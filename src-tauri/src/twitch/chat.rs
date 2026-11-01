//! Présence dans le chat Twitch (IRC sur WebSocket).
//!
//! Un compte connecté au chat d'une chaîne apparaît dans la liste des
//! spectateurs : c'est ce que StreamElements regarde pour donner ses points de
//! fidélité toutes les 10 min. Aucune connexion anonyme : elle n'apparaît pas
//! dans cette liste. Aucun message n'est envoyé. Les messages des bots de
//! giveaway (StreamElements…) sont remontés : ils annoncent nos tickets.

use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, sleep, timeout, Instant};
use tokio_tungstenite::tungstenite::Message;

const IRC_URL: &str = "wss://irc-ws.chat.twitch.tv:443";
// Twitch limite les JOIN à 20 par 10 s pour un compte ordinaire : on reste en dessous
const JOINS_PER_WINDOW: usize = 15;
const JOIN_WINDOW: Duration = Duration::from_secs(11);
const PING_EVERY: Duration = Duration::from_secs(240);
const PONG_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BACKOFF: Duration = Duration::from_secs(120);
/// Bots dont les messages sont remontés (seuls eux font foi : un spectateur
/// ne doit pas pouvoir imiter une annonce)
const BOTS: &[&str] = &["streamelements", "wizebot", "nightbot", "phuzzybot"];

#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    Connected(bool),
    Joined(String),
    Parted(String),
    BotMessage { channel: String, bot: String, text: String },
    /// Jeton refusé par le chat : inutile de réessayer avec le même
    AuthFailed,
}

/// Garde le compte présent dans les chaînes de `wanted` (logins), jusqu'à ce que
/// `events` soit fermé côté réception
pub async fn run(
    login: String,
    token: String,
    mut wanted: watch::Receiver<BTreeSet<String>>,
    events: mpsc::Sender<ChatEvent>,
) {
    let mut backoff = Duration::from_secs(2);
    loop {
        // Rien à rejoindre : pas de connexion ouverte
        if wanted.borrow().is_empty() && wanted.changed().await.is_err() {
            return;
        }
        if wanted.borrow().is_empty() {
            continue;
        }
        let started = Instant::now();
        let result = connect_once(&login, &token, &mut wanted, &events).await;
        if events.is_closed() {
            return;
        }
        let _ = events.send(ChatEvent::Connected(false)).await;
        match result {
            Err(Failure::Auth) => {
                let _ = events.send(ChatEvent::AuthFailed).await;
                return;
            }
            Err(Failure::Other(e)) => tracing::warn!("chat de {login} : {e}"),
            Ok(()) => {}
        }
        if started.elapsed() > Duration::from_secs(300) {
            backoff = Duration::from_secs(2);
        }
        sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

enum Failure {
    Auth,
    Other(String),
}

impl<E: std::fmt::Display> From<E> for Failure {
    fn from(e: E) -> Self {
        Failure::Other(e.to_string())
    }
}

async fn connect_once(
    login: &str,
    token: &str,
    wanted: &mut watch::Receiver<BTreeSet<String>>,
    events: &mpsc::Sender<ChatEvent>,
) -> Result<(), Failure> {
    let (mut ws, _) = tokio_tungstenite::connect_async(IRC_URL).await?;
    ws.send(Message::text(format!("PASS oauth:{token}"))).await?;
    ws.send(Message::text(format!("NICK {login}"))).await?;

    let mut joined: HashSet<String> = HashSet::new();
    let mut pending_join: Vec<String> = vec![];
    let mut join_window_start = Instant::now();
    let mut joins_in_window = 0usize;
    let mut welcomed = false;
    let mut ping = interval(PING_EVERY);
    ping.tick().await;
    let mut awaiting_pong: Option<Instant> = None;
    let mut join_tick = interval(Duration::from_secs(1));

    loop {
        let pong_wait = awaiting_pong.map(|t| PONG_TIMEOUT.saturating_sub(t.elapsed())).unwrap_or(PING_EVERY);
        tokio::select! {
            _ = ping.tick() => {
                ws.send(Message::text("PING :twitch-desk")).await?;
                awaiting_pong = Some(Instant::now());
            }
            _ = sleep(pong_wait), if awaiting_pong.is_some() => {
                return Err(Failure::Other("pas de réponse au PING".into()));
            }
            changed = wanted.changed(), if welcomed => {
                if changed.is_err() {
                    return Ok(());
                }
                let target = wanted.borrow().clone();
                for channel in joined.iter().filter(|c| !target.contains(*c)).cloned().collect::<Vec<_>>() {
                    ws.send(Message::text(format!("PART #{channel}"))).await?;
                    joined.remove(&channel);
                    let _ = events.send(ChatEvent::Parted(channel)).await;
                }
                pending_join = target.into_iter().filter(|c| !joined.contains(c)).collect();
                if wanted.borrow().is_empty() {
                    return Ok(());
                }
            }
            _ = join_tick.tick(), if welcomed && !pending_join.is_empty() => {
                if join_window_start.elapsed() >= JOIN_WINDOW {
                    join_window_start = Instant::now();
                    joins_in_window = 0;
                }
                let room = JOINS_PER_WINDOW.saturating_sub(joins_in_window);
                if room > 0 {
                    let batch: Vec<String> = pending_join.drain(..room.min(pending_join.len())).collect();
                    joins_in_window += batch.len();
                    let list = batch.iter().map(|c| format!("#{c}")).collect::<Vec<_>>().join(",");
                    ws.send(Message::text(format!("JOIN {list}"))).await?;
                }
            }
            msg = timeout(PING_EVERY * 2, ws.next()) => {
                let text = match msg {
                    Err(_) => return Err(Failure::Other("connexion muette".into())),
                    Ok(None) => return Err(Failure::Other("connexion fermée".into())),
                    Ok(Some(Err(e))) => return Err(e.into()),
                    Ok(Some(Ok(Message::Text(t)))) => t,
                    Ok(Some(Ok(_))) => continue,
                };
                for line in text.lines() {
                    match parse_line(line, login) {
                        Line::Ping(arg) => ws.send(Message::text(format!("PONG :{arg}"))).await?,
                        Line::Pong => awaiting_pong = None,
                        Line::Welcome => {
                            welcomed = true;
                            let _ = events.send(ChatEvent::Connected(true)).await;
                            pending_join = wanted.borrow().iter().cloned().collect();
                        }
                        Line::AuthFailed => return Err(Failure::Auth),
                        Line::Reconnect => return Err(Failure::Other("Twitch demande une reconnexion".into())),
                        Line::SelfJoin(channel) => {
                            joined.insert(channel.clone());
                            let _ = events.send(ChatEvent::Joined(channel)).await;
                        }
                        Line::SelfPart(channel) => {
                            joined.remove(&channel);
                            let _ = events.send(ChatEvent::Parted(channel)).await;
                        }
                        Line::Privmsg { nick, channel, text } => {
                            if BOTS.contains(&nick.as_str()) {
                                let _ = events.send(ChatEvent::BotMessage { channel, bot: nick, text }).await;
                            }
                        }
                        Line::Other => {}
                    }
                }
            }
        }
        if events.is_closed() {
            return Ok(());
        }
    }
}

#[derive(Debug, PartialEq)]
enum Line {
    Ping(String),
    Pong,
    Welcome,
    AuthFailed,
    Reconnect,
    SelfJoin(String),
    SelfPart(String),
    Privmsg { nick: String, channel: String, text: String },
    Other,
}

fn parse_line(line: &str, login: &str) -> Line {
    let line = line.trim_end_matches('\r');
    // Les étiquettes IRCv3 (@a=b;…) ne nous servent pas ici
    let line = match line.strip_prefix('@') {
        Some(rest) => rest.split_once(' ').map_or("", |(_, r)| r),
        None => line,
    };
    if let Some(arg) = line.strip_prefix("PING ") {
        return Line::Ping(arg.trim_start_matches(':').to_owned());
    }
    let (prefix, rest) = match line.strip_prefix(':') {
        Some(l) => l.split_once(' ').unwrap_or((l, "")),
        None => ("", line),
    };
    let mut parts = rest.splitn(3, ' ');
    let command = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let nick = prefix.split('!').next().unwrap_or("");
    let own = nick.eq_ignore_ascii_case(login);
    match command {
        "PONG" => Line::Pong,
        "001" => Line::Welcome,
        "RECONNECT" => Line::Reconnect,
        "NOTICE" if rest.contains("Login authentication failed") || rest.contains("Improperly formatted auth") => {
            Line::AuthFailed
        }
        "JOIN" if own => Line::SelfJoin(target.trim_start_matches('#').to_lowercase()),
        "PART" if own => Line::SelfPart(target.trim_start_matches('#').to_lowercase()),
        "PRIVMSG" => Line::Privmsg {
            nick: nick.to_lowercase(),
            channel: target.trim_start_matches('#').to_lowercase(),
            text: parts.next().unwrap_or("").trim_start_matches(':').to_owned(),
        },
        _ => Line::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        assert_eq!(parse_line("PING :tmi.twitch.tv", "me"), Line::Ping("tmi.twitch.tv".into()));
        assert_eq!(parse_line(":tmi.twitch.tv PONG tmi.twitch.tv :twitch-desk", "me"), Line::Pong);
        assert_eq!(parse_line(":tmi.twitch.tv 001 me :Welcome, GLHF!\r", "me"), Line::Welcome);
        assert_eq!(parse_line(":tmi.twitch.tv NOTICE * :Login authentication failed", "me"), Line::AuthFailed);
        assert_eq!(parse_line(":me!me@me.tmi.twitch.tv JOIN #LunarSea", "me"), Line::SelfJoin("lunarsea".into()));
        assert_eq!(parse_line(":Me!me@me.tmi.twitch.tv PART #lunarsea", "me"), Line::SelfPart("lunarsea".into()));
        // JOIN d'un autre spectateur : ignoré
        assert_eq!(parse_line(":bob!bob@bob.tmi.twitch.tv JOIN #lunarsea", "me"), Line::Other);
        assert_eq!(
            parse_line("@badge-info=;color=#FF0000 :bob!bob@bob.tmi.twitch.tv PRIVMSG #lunarsea :!ticket 5", "me"),
            Line::Privmsg { nick: "bob".into(), channel: "lunarsea".into(), text: "!ticket 5".into() }
        );
        assert_eq!(parse_line(":tmi.twitch.tv RECONNECT", "me"), Line::Reconnect);
    }
}
