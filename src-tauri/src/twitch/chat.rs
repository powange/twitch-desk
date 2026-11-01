//! Présence dans le chat Twitch (IRC sur WebSocket).
//!
//! Un compte connecté au chat d'une chaîne apparaît dans la liste des
//! spectateurs : c'est ce que StreamElements regarde pour donner ses points de
//! fidélité toutes les 10 min. Aucune connexion anonyme : elle n'apparaît pas
//! dans cette liste. Les messages des bots de giveaway (StreamElements…) sont
//! remontés : ils annoncent nos tickets.
//!
//! Mode anonyme (lecture seule, aucun compte) : le moniteur de giveaways lit tous
//! les messages des chaînes surveillées sans apparaître pour aucun de nos comptes.
//! L'envoi d'une commande de participation passe par une connexion courte du compte.

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

/// Ce qu'une connexion transmet au moniteur de giveaways : il lit les messages
/// des chaînes surveillées via la connexion déjà présente dans le chat (un seul
/// lecteur par chaîne), sans en ouvrir une de plus
#[derive(Debug, Clone, PartialEq)]
pub enum TapEvent {
    Joined { source: i64, channel: String },
    Parted { source: i64, channel: String },
    Disconnected { source: i64 },
    Message { source: i64, channel: String, user: String, text: String },
}

/// Connexion vue par le moniteur : identifiant de la source (0 : anonyme, sinon compte)
pub type Tap = Option<(i64, mpsc::Sender<TapEvent>)>;

fn tap_send(tap: &Tap, event: impl FnOnce(i64) -> TapEvent) {
    if let Some((source, tx)) = tap {
        // Moniteur débordé : on perd un message plutôt que de bloquer la lecture du chat
        let _ = tx.try_send(event(*source));
    }
}

/// Identité de la connexion
#[derive(Clone)]
pub enum ChatAuth {
    Account { login: String, token: String },
    /// Lecture seule, sans compte : tous les messages sont remontés
    Anonymous,
}

impl ChatAuth {
    fn nick(&self) -> String {
        match self {
            ChatAuth::Account { login, .. } => login.clone(),
            // Pseudo réservé par Twitch aux connexions anonymes
            ChatAuth::Anonymous => format!("justinfan{}", rand::random_range(10_000..99_999)),
        }
    }
}

/// Garde la connexion présente dans les chaînes de `wanted` (logins), jusqu'à ce
/// que `events` soit fermé côté réception
pub async fn run(auth: ChatAuth, mut wanted: watch::Receiver<BTreeSet<String>>, events: mpsc::Sender<ChatEvent>, tap: Tap) {
    let login = auth.nick();
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
        let result = connect_once(&auth, &login, &mut wanted, &events, &tap).await;
        tap_send(&tap, |source| TapEvent::Disconnected { source });
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
    auth: &ChatAuth,
    login: &str,
    wanted: &mut watch::Receiver<BTreeSet<String>>,
    events: &mpsc::Sender<ChatEvent>,
    tap: &Tap,
) -> Result<(), Failure> {
    let (mut ws, _) = tokio_tungstenite::connect_async(IRC_URL).await?;
    if let ChatAuth::Account { token, .. } = auth {
        ws.send(Message::text(format!("PASS oauth:{token}"))).await?;
    }
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
                    tap_send(tap, |source| TapEvent::Parted { source, channel: channel.clone() });
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
                            tap_send(tap, |source| TapEvent::Joined { source, channel: channel.clone() });
                            let _ = events.send(ChatEvent::Joined(channel)).await;
                        }
                        Line::SelfPart(channel) => {
                            joined.remove(&channel);
                            tap_send(tap, |source| TapEvent::Parted { source, channel: channel.clone() });
                            let _ = events.send(ChatEvent::Parted(channel)).await;
                        }
                        Line::Privmsg { nick, channel, text } => {
                            tap_send(tap, |source| TapEvent::Message {
                                source,
                                channel: channel.clone(),
                                user: nick.clone(),
                                text: text.clone(),
                            });
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

/// Envoie un message dans le chat d'une chaîne avec le compte : connexion courte
/// (connexion, JOIN, message, départ). Sert à la participation aux giveaways.
pub async fn send_message(login: &str, token: &str, channel: &str, text: &str) -> Result<(), String> {
    let channel = channel.trim_start_matches('#').to_lowercase();
    let text = text.replace(['\r', '\n'], " ");
    let work = async {
        let (mut ws, _) = tokio_tungstenite::connect_async(IRC_URL).await.map_err(|e| e.to_string())?;
        let send = |m: String| Message::text(m);
        ws.send(send(format!("PASS oauth:{token}"))).await.map_err(|e| e.to_string())?;
        ws.send(send(format!("NICK {login}"))).await.map_err(|e| e.to_string())?;
        while let Some(msg) = ws.next().await {
            let Message::Text(t) = msg.map_err(|e| e.to_string())? else { continue };
            for line in t.lines() {
                match parse_line(line, login) {
                    Line::Ping(arg) => ws.send(send(format!("PONG :{arg}"))).await.map_err(|e| e.to_string())?,
                    Line::Welcome => ws.send(send(format!("JOIN #{channel}"))).await.map_err(|e| e.to_string())?,
                    Line::AuthFailed => return Err("jeton refusé par le chat : reconnecte ce compte".to_owned()),
                    Line::SelfJoin(c) if c == channel => {
                        ws.send(send(format!("PRIVMSG #{channel} :{text}"))).await.map_err(|e| e.to_string())?;
                        // Laisser au serveur le temps de relayer avant de partir
                        sleep(Duration::from_millis(800)).await;
                        let _ = ws.send(send(format!("PART #{channel}"))).await;
                        let _ = ws.close(None).await;
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
        Err("connexion au chat fermée".to_owned())
    };
    timeout(Duration::from_secs(20), work).await.map_err(|_| "le chat n'a pas répondu à temps".to_owned())?
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
