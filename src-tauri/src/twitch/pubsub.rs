//! Événements en direct de Twitch (PubSub) : bonus disponible, points gagnés,
//! début / fin de live, raids. Twitch accepte 50 sujets par connexion : au-delà,
//! on ouvre une connexion de plus par tranche.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::{interval, sleep, timeout, Instant};
use tokio_tungstenite::tungstenite::Message;

const PUBSUB_URL: &str = "wss://pubsub-edge.twitch.tv/v1";
/// Tous les sujets passent par Hermes, comme le site de Twitch : depuis le 4 novembre 2026,
/// PubSub coupe les connexions de l'application au bout de quelques secondes (PubSub reste en
/// secours si Hermes refuse le jeton)
const HERMES_URL: &str = "wss://hermes.twitch.tv/v1";
/// Silence toléré au-delà du keepalive annoncé par Hermes (welcome.keepaliveSec)
const HERMES_SILENCE_MARGIN: Duration = Duration::from_secs(10);
const HERMES_HANDSHAKE: Duration = Duration::from_secs(15);
// 50 par connexion : limite de PubSub ; Hermes en accepte 150 (au-delà : SUB006, « too many
// subscriptions »)
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
    /// true quand toutes les connexions sont abonnées (réponse de LISTEN reçue)
    Connection { connected: bool },
    /// Jeton refusé par PubSub (ERR_BADAUTH) : la connexion s'arrête, un nouveau jeton est nécessaire
    AuthFailed,
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

/// Connexions abonnées, sur le total : « temps réel » seulement quand toutes le sont
struct Presence {
    up: AtomicUsize,
    total: usize,
}

impl Presence {
    async fn set(&self, mine: &mut bool, connected: bool, events: &mpsc::Sender<PubSubEvent>) {
        if *mine == connected {
            return;
        }
        *mine = connected;
        let up = if connected {
            self.up.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            self.up.fetch_sub(1, Ordering::SeqCst) - 1
        };
        let _ = events.send(PubSubEvent::Connection { connected: up == self.total }).await;
    }
}

enum Failure {
    /// Jeton refusé : inutile de réessayer avec le même
    Auth,
    /// Hermes refuse l'authentification ou un abonnement : essai suivant par PubSub
    Refused(String),
    Other(String),
}

/// Garde les connexions ouvertes jusqu'à ce que `events` soit fermé côté réception.
/// Les connexions sont des tâches filles d'un JoinSet : arrêter `run` les arrête toutes
pub async fn run(token: String, topics: Vec<String>, events: mpsc::Sender<PubSubEvent>) {
    // Sujets du compte d'abord (première connexion), puis ceux des chaînes
    let (mut topics, channels): (Vec<String>, Vec<String>) = topics.into_iter().partition(|t| is_user_topic(t));
    topics.extend(channels);
    let chunks: Vec<Vec<String>> = topics.chunks(TOPICS_PER_CONNECTION).map(<[String]>::to_vec).collect();
    let presence = Arc::new(Presence { up: AtomicUsize::new(0), total: chunks.len() });
    let mut tasks = JoinSet::new();
    for chunk in chunks {
        let token = token.clone();
        let events = events.clone();
        let presence = presence.clone();
        tasks.spawn(async move { connection_loop(token, chunk, events, presence).await });
    }
    while tasks.join_next().await.is_some() {}
}

fn is_user_topic(topic: &str) -> bool {
    topic.starts_with("community-points-user-v1.") || topic.starts_with("user-drop-events.")
}

/// Connexion Hermes ; refusée par Hermes, essai suivant par PubSub (ancienne méthode), puis de
/// nouveau Hermes
async fn connection_loop(token: String, topics: Vec<String>, events: mpsc::Sender<PubSubEvent>, presence: Arc<Presence>) {
    let mut backoff = Duration::from_secs(2);
    let mut up = false;
    let mut legacy_next = false;
    loop {
        let started = Instant::now();
        let via_hermes = !legacy_next;
        let result = match via_hermes {
            true => hermes_once(&token, &topics, &events, &presence, &mut up).await,
            false => connect_once(&token, &topics, &events, &presence, &mut up).await,
        };
        legacy_next = via_hermes && matches!(&result, Err(Failure::Refused(_)));
        if events.is_closed() {
            return;
        }
        presence.set(&mut up, false, &events).await;
        match result {
            Err(Failure::Auth) => {
                tracing::warn!("PubSub : jeton refusé (ERR_BADAUTH)");
                let _ = events.send(PubSubEvent::AuthFailed).await;
                return;
            }
            Err(Failure::Other(e)) => tracing::warn!("{} : {e}", if via_hermes { "Hermes" } else { "PubSub" }),
            Err(Failure::Refused(e)) => tracing::warn!("Hermes : {e}, essai par PubSub"),
            Ok(()) => {}
        }
        // Connexion restée stable un moment : on repart du délai minimal
        if started.elapsed() > Duration::from_secs(300) {
            backoff = Duration::from_secs(2);
        }
        tokio::select! {
            _ = sleep(backoff) => {}
            _ = events.closed() => return,
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Réponse à notre LISTEN : Ok abonné, Err refusé ; None si ce n'est pas notre réponse
fn listen_response(envelope: &Value, nonce: &str) -> Option<Result<(), Failure>> {
    if envelope.get("type").and_then(Value::as_str) != Some("RESPONSE") {
        return None;
    }
    if envelope.get("nonce").and_then(Value::as_str).is_some_and(|n| n != nonce) {
        return None;
    }
    let error = envelope.get("error").and_then(Value::as_str).unwrap_or("");
    Some(match error {
        "" => Ok(()),
        "ERR_BADAUTH" => Err(Failure::Auth),
        other => Err(Failure::Other(format!("abonnement refusé : {other}"))),
    })
}

async fn connect_once(
    token: &str,
    topics: &[String],
    events: &mpsc::Sender<PubSubEvent>,
    presence: &Presence,
    up: &mut bool,
) -> Result<(), Failure> {
    let other = |e: tokio_tungstenite::tungstenite::Error| Failure::Other(e.to_string());
    let (mut ws, _) = tokio_tungstenite::connect_async(PUBSUB_URL).await.map_err(other)?;
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let listen = json!({ "type": "LISTEN", "nonce": nonce, "data": { "topics": topics, "auth_token": token } });
    ws.send(Message::text(listen.to_string())).await.map_err(other)?;

    let mut ping = interval(PING_EVERY);
    ping.tick().await;
    let mut awaiting_pong: Option<Instant> = None;
    loop {
        let wait = awaiting_pong.map(|t| PONG_TIMEOUT.saturating_sub(t.elapsed())).unwrap_or(PING_EVERY);
        tokio::select! {
            _ = events.closed() => return Ok(()),
            _ = ping.tick() => {
                ws.send(Message::text(r#"{"type":"PING"}"#)).await.map_err(other)?;
                awaiting_pong = Some(Instant::now());
            }
            _ = sleep(wait), if awaiting_pong.is_some() => {
                return Err(Failure::Other("pas de réponse au PING".into()));
            }
            msg = timeout(PING_EVERY * 2, ws.next()) => {
                let msg = match msg {
                    Err(_) => return Err(Failure::Other("connexion muette".into())),
                    Ok(None) => return Err(Failure::Other("connexion fermée".into())),
                    Ok(Some(Err(e))) => return Err(other(e)),
                    Ok(Some(Ok(m))) => m,
                };
                let Message::Text(text) = msg else { continue };
                let Ok(envelope) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(response) = listen_response(&envelope, &nonce) {
                    response?;
                    // Abonnement confirmé : seulement maintenant la connexion compte
                    presence.set(up, true, events).await;
                    continue;
                }
                match envelope.get("type").and_then(Value::as_str) {
                    Some("PONG") => awaiting_pong = None,
                    Some("RECONNECT") => return Err(Failure::Other("Twitch demande une reconnexion".into())),
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

/// Trame Hermes à envoyer : `kind` est aussi le nom du champ qui porte `body`
fn hermes_frame(kind: &str, id: &str, body: Value) -> Message {
    let mut frame = json!({ "type": kind, "id": id, "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true) });
    frame[kind] = body;
    Message::text(frame.to_string())
}

/// Réponse d'Hermes à la trame `id` : Ok accepté, Err(code) refusé ; None pour une autre trame
fn hermes_result(frame: &Value, field: &str, id: &str) -> Option<Result<(), String>> {
    if frame.get("type").and_then(Value::as_str) != Some(field) || frame.get("parentId").and_then(Value::as_str) != Some(id) {
        return None;
    }
    let response = frame.get(field)?;
    Some(match response.get("result").and_then(Value::as_str) {
        Some("ok") => Ok(()),
        other => Err(response.get("errorCode").and_then(Value::as_str).or(other).unwrap_or("refus").chars().take(40).collect()),
    })
}

/// Notification Hermes d'un abonnement : sujet et message (même contenu que PubSub), sous la
/// forme d'une enveloppe PubSub pour `parse_message`
fn hermes_notification(frame: &Value, subscriptions: &std::collections::HashMap<String, String>) -> Option<Value> {
    if frame.get("type").and_then(Value::as_str) != Some("notification") {
        return None;
    }
    let n = frame.get("notification")?;
    let topic = subscriptions.get(str_at(n, "/subscription/id")?)?;
    let message = str_at(n, "/pubsub")?;
    Some(json!({ "type": "MESSAGE", "data": { "topic": topic, "message": message } }))
}

/// Une connexion Hermes : bienvenue, authentification avec le jeton, un abonnement par sujet ;
/// « temps réel » une fois tous les abonnements acceptés
async fn hermes_once(
    token: &str,
    topics: &[String],
    events: &mpsc::Sender<PubSubEvent>,
    presence: &Presence,
    up: &mut bool,
) -> Result<(), Failure> {
    let other = |e: tokio_tungstenite::tungstenite::Error| Failure::Other(e.to_string());
    let url = format!("{HERMES_URL}?clientId={}", super::CLIENT_ID);
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.map_err(other)?;
    let auth_id = uuid::Uuid::new_v4().to_string();
    // Identifiant de la trame d'abonnement -> (identifiant de l'abonnement, sujet)
    let mut pending: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
    // Abonnements acceptés : identifiant -> sujet
    let mut subscriptions: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut keepalive = Duration::from_secs(15);
    let mut welcomed = false;
    let opened = Instant::now();
    loop {
        let wait = if welcomed && *up { keepalive + HERMES_SILENCE_MARGIN } else { HERMES_HANDSHAKE.saturating_sub(opened.elapsed()).max(Duration::from_secs(1)) };
        let msg = tokio::select! {
            _ = events.closed() => return Ok(()),
            msg = timeout(wait, ws.next()) => msg,
        };
        let msg = match msg {
            Err(_) if *up => return Err(Failure::Other("connexion muette".into())),
            Err(_) => return Err(Failure::Other("pas de réponse à la connexion".into())),
            Ok(None) => return Err(Failure::Other("connexion fermée".into())),
            Ok(Some(Err(e))) => return Err(other(e)),
            Ok(Some(Ok(m))) => m,
        };
        let Message::Text(text) = msg else { continue };
        let Ok(frame) = serde_json::from_str::<Value>(&text) else { continue };
        match frame.get("type").and_then(Value::as_str) {
            Some("welcome") => {
                welcomed = true;
                if let Some(s) = frame.pointer("/welcome/keepaliveSec").and_then(Value::as_u64).filter(|s| (1..=600).contains(s)) {
                    keepalive = Duration::from_secs(s);
                }
                ws.send(hermes_frame("authenticate", &auth_id, json!({ "token": token }))).await.map_err(other)?;
                continue;
            }
            Some("keepalive") => continue,
            Some("reconnect") => return Err(Failure::Other("Twitch demande une reconnexion".into())),
            _ => {}
        }
        if let Some(result) = hermes_result(&frame, "authenticateResponse", &auth_id) {
            result.map_err(|code| Failure::Refused(format!("authentification refusée ({code})")))?;
            for topic in topics {
                let (frame_id, sub_id) = (uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string());
                let body = json!({ "id": sub_id, "type": "pubsub", "pubsub": { "topic": topic } });
                ws.send(hermes_frame("subscribe", &frame_id, body)).await.map_err(other)?;
                pending.insert(frame_id, (sub_id, topic.clone()));
            }
            continue;
        }
        let answered = pending.keys().find(|id| hermes_result(&frame, "subscribeResponse", id).is_some()).cloned();
        if let Some(frame_id) = answered {
            let (sub_id, topic) = pending.remove(&frame_id).expect("abonnement en attente");
            match hermes_result(&frame, "subscribeResponse", &frame_id) {
                // Sujet du compte refusé : essai par PubSub ; sujet d'une chaîne : ignoré (le
                // statut live est aussi relu régulièrement)
                Some(Err(code)) if is_user_topic(&topic) => {
                    return Err(Failure::Refused(format!("abonnement à {} refusé ({code})", topic.split('.').next().unwrap_or(""))));
                }
                Some(Err(code)) => tracing::warn!("Hermes : abonnement à {topic} refusé ({code})"),
                _ => {
                    subscriptions.insert(sub_id, topic);
                }
            }
            if pending.is_empty() {
                tracing::info!("Hermes : connecté ({} sujet(s) sur {})", subscriptions.len(), topics.len());
                presence.set(up, true, events).await;
            }
            continue;
        }
        // Abonnement retiré par Hermes : reconnexion (tous les sujets de nouveau)
        if frame.get("type").and_then(Value::as_str) == Some("subscriptionRevocation") {
            return Err(Failure::Other("abonnement retiré par Twitch".into()));
        }
        if let Some(envelope) = hermes_notification(&frame, &subscriptions) {
            for event in parse_message(&envelope) {
                if events.send(event).await.is_err() {
                    return Ok(());
                }
            }
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
    fn listen_responses() {
        let ok = json!({"type":"RESPONSE","nonce":"n1","error":""});
        assert!(matches!(listen_response(&ok, "n1"), Some(Ok(()))));
        let bad = json!({"type":"RESPONSE","nonce":"n1","error":"ERR_BADAUTH"});
        assert!(matches!(listen_response(&bad, "n1"), Some(Err(Failure::Auth))));
        let other = json!({"type":"RESPONSE","nonce":"n1","error":"ERR_BADTOPIC"});
        assert!(matches!(listen_response(&other, "n1"), Some(Err(Failure::Other(_)))));
        // Réponse à une autre demande, ou message ordinaire
        assert!(listen_response(&json!({"type":"RESPONSE","nonce":"x","error":""}), "n1").is_none());
        assert!(listen_response(&json!({"type":"PONG"}), "n1").is_none());
    }

    #[test]
    fn hermes_frames() {
        // Réponses à nos trames : acceptée, refusée avec son code, ou réponse à une autre trame
        let ok = json!({"type":"authenticateResponse","parentId":"a1","authenticateResponse":{"result":"ok"}});
        assert_eq!(hermes_result(&ok, "authenticateResponse", "a1"), Some(Ok(())));
        let bad = json!({"type":"authenticateResponse","parentId":"a1","authenticateResponse":{"result":"error","errorCode":"AUTH002"}});
        assert_eq!(hermes_result(&bad, "authenticateResponse", "a1"), Some(Err("AUTH002".into())));
        assert_eq!(hermes_result(&ok, "authenticateResponse", "a2"), None);
        assert_eq!(hermes_result(&ok, "subscribeResponse", "a1"), None);
        // Notification : enveloppe PubSub du sujet de l'abonnement, lue comme avant
        let subs = std::collections::HashMap::from([("s1".to_owned(), "community-points-user-v1.42".to_owned())]);
        let inner = json!({"type":"points-earned","data":{"channel_id":"7","point_gain":{"total_points":10,"reason_code":"WATCH"},"balance":{"balance":110}}});
        let frame = json!({"type":"notification","notification":{"subscription":{"id":"s1"},"type":"pubsub","pubsub":inner.to_string()}});
        let envelope = hermes_notification(&frame, &subs).unwrap();
        assert_eq!(
            parse_message(&envelope),
            vec![PubSubEvent::PointsEarned { channel_id: "7".into(), gain: 10, reason: "WATCH".into(), balance: Some(110) }]
        );
        // Abonnement inconnu : ignorée
        let unknown = json!({"type":"notification","notification":{"subscription":{"id":"zz"},"pubsub":inner.to_string()}});
        assert!(hermes_notification(&unknown, &subs).is_none());
        assert!(is_user_topic("user-drop-events.1") && !is_user_topic("raid.1"));
    }

    #[tokio::test]
    async fn realtime_needs_every_connection() {
        let (tx, mut rx) = mpsc::channel(8);
        let p = Presence { up: AtomicUsize::new(0), total: 2 };
        let (mut a, mut b) = (false, false);
        p.set(&mut a, true, &tx).await;
        assert_eq!(rx.recv().await, Some(PubSubEvent::Connection { connected: false }));
        p.set(&mut b, true, &tx).await;
        assert_eq!(rx.recv().await, Some(PubSubEvent::Connection { connected: true }));
        // Pas de double comptage
        p.set(&mut b, true, &tx).await;
        p.set(&mut a, false, &tx).await;
        assert_eq!(rx.recv().await, Some(PubSubEvent::Connection { connected: false }));
        assert_eq!(p.up.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn topics() {
        let t = topics_for("33", &["11".into(), "12".into()]);
        assert_eq!(t.len(), 6);
        assert_eq!(t[1], "user-drop-events.33");
        assert_eq!(t[0], "community-points-user-v1.33");
    }
}
