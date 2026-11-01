//! Événements en direct de Twitch (PubSub) : bonus disponible, points gagnés,
//! début / fin de live, raids. Twitch accepte 50 sujets par connexion : au-delà,
//! on ouvre une connexion de plus par tranche.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::{interval, sleep, timeout, Instant};
use tokio_tungstenite::tungstenite::Message;

const PUBSUB_URL: &str = "wss://pubsub-edge.twitch.tv/v1";
const TOPICS_PER_CONNECTION: usize = 50;
const PING_EVERY: Duration = Duration::from_secs(180);
const PONG_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq)]
pub enum PubSubEvent {
    PointsEarned { channel_id: String, gain: i64, reason: String, balance: Option<i64> },
    ClaimAvailable { channel_id: String, claim_id: String },
    StreamUp { channel_id: String },
    StreamDown { channel_id: String },
    Viewers { channel_id: String, viewers: u64 },
    Raid { channel_id: String, raid_id: String, target_login: String },
    DropProgress { drop_id: String, current: i64 },
    DropClaim { instance_id: String },
    Connection { connected: bool },
}

/// Sujets écoutés pour un compte et ses chaînes
pub fn topics_for(user_id: &str, channel_ids: &[String]) -> Vec<String> {
    let mut topics = vec![format!("community-points-user-v1.{user_id}"), format!("user-drop-events.{user_id}")];
    for id in channel_ids {
        topics.push(format!("video-playback-by-id.{id}"));
        topics.push(format!("raid.{id}"));
    }
    topics
}

/// Garde les connexions ouvertes jusqu'à ce que `events` soit fermé côté réception
pub async fn run(token: String, topics: Vec<String>, events: mpsc::Sender<PubSubEvent>) {
    let mut tasks = Vec::new();
    for chunk in topics.chunks(TOPICS_PER_CONNECTION) {
        let chunk = chunk.to_vec();
        let token = token.clone();
        let events = events.clone();
        tasks.push(tokio::spawn(async move { connection_loop(token, chunk, events).await }));
    }
    for task in tasks {
        let _ = task.await;
    }
}

async fn connection_loop(token: String, topics: Vec<String>, events: mpsc::Sender<PubSubEvent>) {
    let mut backoff = Duration::from_secs(2);
    loop {
        let started = Instant::now();
        let result = connect_once(&token, &topics, &events).await;
        if events.is_closed() {
            return;
        }
        let _ = events.send(PubSubEvent::Connection { connected: false }).await;
        if let Err(e) = result {
            tracing::warn!("PubSub : {e}");
        }
        // Connexion restée stable un moment : on repart du délai minimal
        if started.elapsed() > Duration::from_secs(300) {
            backoff = Duration::from_secs(2);
        }
        sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn connect_once(
    token: &str,
    topics: &[String],
    events: &mpsc::Sender<PubSubEvent>,
) -> Result<(), String> {
    let (mut ws, _) = tokio_tungstenite::connect_async(PUBSUB_URL).await.map_err(|e| e.to_string())?;
    let listen = json!({
        "type": "LISTEN",
        "nonce": uuid::Uuid::new_v4().simple().to_string(),
        "data": { "topics": topics, "auth_token": token }
    });
    ws.send(Message::text(listen.to_string())).await.map_err(|e| e.to_string())?;
    let _ = events.send(PubSubEvent::Connection { connected: true }).await;

    let mut ping = interval(PING_EVERY);
    ping.tick().await;
    let mut awaiting_pong: Option<Instant> = None;
    loop {
        let wait = awaiting_pong.map(|t| PONG_TIMEOUT.saturating_sub(t.elapsed())).unwrap_or(PING_EVERY);
        tokio::select! {
            _ = ping.tick() => {
                ws.send(Message::text(r#"{"type":"PING"}"#)).await.map_err(|e| e.to_string())?;
                awaiting_pong = Some(Instant::now());
            }
            _ = sleep(wait), if awaiting_pong.is_some() => {
                return Err("pas de réponse au PING".into());
            }
            msg = timeout(PING_EVERY * 2, ws.next()) => {
                let msg = match msg {
                    Err(_) => return Err("connexion muette".into()),
                    Ok(None) => return Err("connexion fermée".into()),
                    Ok(Some(Err(e))) => return Err(e.to_string()),
                    Ok(Some(Ok(m))) => m,
                };
                let Message::Text(text) = msg else { continue };
                let Ok(envelope) = serde_json::from_str::<Value>(&text) else { continue };
                match envelope.get("type").and_then(Value::as_str) {
                    Some("PONG") => awaiting_pong = None,
                    Some("RECONNECT") => return Err("Twitch demande une reconnexion".into()),
                    Some("RESPONSE") => {
                        let error = envelope.get("error").and_then(Value::as_str).unwrap_or("");
                        if !error.is_empty() {
                            return Err(format!("abonnement refusé : {error}"));
                        }
                    }
                    Some("MESSAGE") => {
                        for event in parse_message(&envelope) {
                            if events.send(event).await.is_err() {
                                return Ok(());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if events.is_closed() {
            return Ok(());
        }
    }
}

fn str_at<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str)
}

/// Événements contenus dans un message PubSub de type MESSAGE
pub fn parse_message(envelope: &Value) -> Vec<PubSubEvent> {
    let Some(topic) = str_at(envelope, "/data/topic") else { return vec![] };
    let Some(inner) = str_at(envelope, "/data/message").and_then(|m| serde_json::from_str::<Value>(m).ok()) else {
        return vec![];
    };
    let (kind, topic_id) = topic.split_once('.').unwrap_or((topic, ""));
    let msg_type = inner.get("type").and_then(Value::as_str).unwrap_or("");
    let data = inner.get("data").unwrap_or(&Value::Null);
    match (kind, msg_type) {
        ("community-points-user-v1", "points-earned") => {
            let channel_id = str_at(data, "/channel_id").or_else(|| str_at(data, "/balance/channel_id"));
            match channel_id {
                Some(channel_id) => vec![PubSubEvent::PointsEarned {
                    channel_id: channel_id.to_owned(),
                    gain: data.pointer("/point_gain/total_points").and_then(Value::as_i64).unwrap_or(0),
                    reason: str_at(data, "/point_gain/reason_code").unwrap_or("").to_owned(),
                    balance: data.pointer("/balance/balance").and_then(Value::as_i64),
                }],
                None => vec![],
            }
        }
        ("community-points-user-v1", "claim-available") => {
            match (str_at(data, "/claim/channel_id"), str_at(data, "/claim/id")) {
                (Some(channel_id), Some(claim_id)) => vec![PubSubEvent::ClaimAvailable {
                    channel_id: channel_id.to_owned(),
                    claim_id: claim_id.to_owned(),
                }],
                _ => vec![],
            }
        }
        ("user-drop-events", "drop-progress") => match str_at(data, "/drop_id") {
            Some(drop_id) => vec![PubSubEvent::DropProgress {
                drop_id: drop_id.to_owned(),
                current: data.get("current_progress_min").and_then(Value::as_i64).unwrap_or(0),
            }],
            None => vec![],
        },
        ("user-drop-events", "drop-claim") => match str_at(data, "/drop_instance_id") {
            Some(id) => vec![PubSubEvent::DropClaim { instance_id: id.to_owned() }],
            None => vec![],
        },
        ("video-playback-by-id", "stream-up") => vec![PubSubEvent::StreamUp { channel_id: topic_id.to_owned() }],
        ("video-playback-by-id", "stream-down") => vec![PubSubEvent::StreamDown { channel_id: topic_id.to_owned() }],
        ("video-playback-by-id", "viewcount") => vec![PubSubEvent::Viewers {
            channel_id: topic_id.to_owned(),
            viewers: inner.get("viewers").and_then(Value::as_u64).unwrap_or(0),
        }],
        ("raid", "raid_update_v2") => match (str_at(&inner, "/raid/id"), str_at(&inner, "/raid/target_login")) {
            (Some(raid_id), Some(target)) => vec![PubSubEvent::Raid {
                channel_id: topic_id.to_owned(),
                raid_id: raid_id.to_owned(),
                target_login: target.to_owned(),
            }],
            _ => vec![],
        },
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(topic: &str, message: Value) -> Value {
        json!({ "type": "MESSAGE", "data": { "topic": topic, "message": message.to_string() } })
    }

    #[test]
    fn points_earned() {
        let e = envelope(
            "community-points-user-v1.33",
            json!({"type":"points-earned","data":{"channel_id":"11","point_gain":{"total_points":50,"reason_code":"CLAIM"},"balance":{"balance":1250,"channel_id":"11"}}}),
        );
        assert_eq!(
            parse_message(&e),
            vec![PubSubEvent::PointsEarned { channel_id: "11".into(), gain: 50, reason: "CLAIM".into(), balance: Some(1250) }]
        );
    }

    #[test]
    fn claim_available() {
        let e = envelope(
            "community-points-user-v1.33",
            json!({"type":"claim-available","data":{"claim":{"id":"c1","channel_id":"11"}}}),
        );
        assert_eq!(parse_message(&e), vec![PubSubEvent::ClaimAvailable { channel_id: "11".into(), claim_id: "c1".into() }]);
    }

    #[test]
    fn playback_and_raid() {
        let up = envelope("video-playback-by-id.11", json!({"type":"stream-up","server_time":1.0}));
        assert_eq!(parse_message(&up), vec![PubSubEvent::StreamUp { channel_id: "11".into() }]);
        let views = envelope("video-playback-by-id.11", json!({"type":"viewcount","viewers":321}));
        assert_eq!(parse_message(&views), vec![PubSubEvent::Viewers { channel_id: "11".into(), viewers: 321 }]);
        let raid = envelope("raid.11", json!({"type":"raid_update_v2","raid":{"id":"r1","target_login":"kraken_tv"}}));
        assert_eq!(
            parse_message(&raid),
            vec![PubSubEvent::Raid { channel_id: "11".into(), raid_id: "r1".into(), target_login: "kraken_tv".into() }]
        );
    }

    #[test]
    fn drop_events() {
        let p = envelope("user-drop-events.33", json!({"type":"drop-progress","data":{"drop_id":"d1","current_progress_min":42,"required_progress_min":120}}));
        assert_eq!(parse_message(&p), vec![PubSubEvent::DropProgress { drop_id: "d1".into(), current: 42 }]);
        let c = envelope("user-drop-events.33", json!({"type":"drop-claim","data":{"drop_instance_id":"33#c#d","drop_id":"d1"}}));
        assert_eq!(parse_message(&c), vec![PubSubEvent::DropClaim { instance_id: "33#c#d".into() }]);
    }

    #[test]
    fn unknown_and_broken_messages() {
        assert!(parse_message(&envelope("whatever.1", json!({"type":"x"}))).is_empty());
        assert!(parse_message(&json!({"type":"MESSAGE","data":{"topic":"raid.1","message":"{pas du json"}})).is_empty());
    }

    #[test]
    fn topics() {
        let t = topics_for("33", &["11".into(), "12".into()]);
        assert_eq!(t.len(), 6);
        assert_eq!(t[1], "user-drop-events.33");
        assert_eq!(t[0], "community-points-user-v1.33");
    }
}
