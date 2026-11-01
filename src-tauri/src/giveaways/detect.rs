//! Détection d'un giveaway dans le chat : une même commande reprise par une part
//! du chat. Règles du monitor du site Giveaway Twitch (server/utils/twitchMonitor.ts) :
//! 7 occurrences sur les 20 derniers messages, 2 au plus par pseudo, réarmement
//! quand la commande retombe sous 3, une alerte au plus toutes les 10 min par
//! (chaîne, commande).

use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};

pub const BUFFER_SIZE: usize = 20;
pub const COMMAND_MIN_COUNT: usize = 7;
const COMMAND_REARM_COUNT: usize = 3;
const MAX_PER_USER: usize = 2;
pub const COMMAND_COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// Commande d'achat de tickets StreamElements : tapée en boucle pendant tout un
/// giveaway StreamElements, elle masquerait la vraie commande ; ignorée seulement
/// quand un giveaway StreamElements est actif sur la chaîne
pub const STREAMELEMENTS_TICKET_COMMAND: &str = "!ticket";

static COMMAND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)(![A-Za-z0-9_]+(?:\s+[A-Za-z0-9_]+)?)").expect("expression valide"));

/// Ce qui a déclenché l'alerte (« Pourquoi cette alerte ? »)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// Occurrences comptées (plafonnées par pseudo) sur les derniers messages
    pub count: usize,
    pub window: usize,
    /// Pseudos distincts qui l'ont tapée
    pub users: Vec<String>,
}

/// Commande lue dans un message (« !enter », « !enter mot »), normalisée
pub fn extract_command(message: &str) -> Option<String> {
    let c = COMMAND.captures(message)?;
    let cmd = c[1].to_lowercase();
    Some(cmd.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Normalise une liste de commandes exclues : minuscules, préfixe « ! », sans doublons
pub fn normalize_excluded(list: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for raw in list {
        let cmd = raw.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
        if cmd.is_empty() {
            continue;
        }
        let cmd = if cmd.starts_with('!') { cmd } else { format!("!{cmd}") };
        if !out.contains(&cmd) {
            out.push(cmd);
        }
    }
    out
}

pub fn is_excluded(cmd: &str, excluded: &[String]) -> bool {
    let base = cmd.split(' ').next().unwrap_or(cmd);
    excluded.iter().any(|e| e == cmd || e == base)
}

#[derive(Default)]
struct ChannelState {
    buffer: VecDeque<(String, String)>,
    alerted: Option<String>,
}

#[derive(Default)]
pub struct Detector {
    channels: HashMap<String, ChannelState>,
    last_alert: HashMap<(String, String), Instant>,
}

impl Detector {
    pub fn forget(&mut self, channel: &str) {
        self.channels.remove(channel);
        self.last_alert.retain(|(c, _), _| c != channel);
    }

    /// Ajoute un message ; renvoie la commande détectée quand une alerte est due
    pub fn push(&mut self, channel: &str, user: &str, message: &str, ignored: &[String]) -> Option<(String, Evidence)> {
        self.push_at(channel, user, message, ignored, Instant::now())
    }

    fn push_at(&mut self, channel: &str, user: &str, message: &str, ignored: &[String], now: Instant) -> Option<(String, Evidence)> {
        let state = self.channels.entry(channel.to_owned()).or_default();
        state.buffer.push_back((user.to_lowercase(), message.to_owned()));
        while state.buffer.len() > BUFFER_SIZE {
            state.buffer.pop_front();
        }
        if state.buffer.len() < COMMAND_MIN_COUNT {
            return None;
        }

        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut per_user: HashMap<(String, String), usize> = HashMap::new();
        let mut users: HashMap<String, Vec<String>> = HashMap::new();
        for (user, message) in &state.buffer {
            let Some(cmd) = extract_command(message) else { continue };
            if is_excluded(&cmd, ignored) {
                continue;
            }
            // Contribution plafonnée par pseudo : un spammeur seul n'atteint pas le seuil
            let from_user = per_user.entry((cmd.clone(), user.clone())).or_default();
            if *from_user >= MAX_PER_USER {
                continue;
            }
            *from_user += 1;
            *counts.entry(cmd.clone()).or_default() += 1;
            let list = users.entry(cmd).or_default();
            if !list.contains(user) {
                list.push(user.clone());
            }
        }

        // Réarmer seulement quand la commande déjà signalée est vraiment retombée
        if let Some(alerted) = &state.alerted {
            if counts.get(alerted).copied().unwrap_or(0) < COMMAND_REARM_COUNT {
                state.alerted = None;
            }
        }

        let (top, count) = counts.iter().max_by_key(|(_, c)| **c).map(|(k, c)| (k.clone(), *c))?;
        if count < COMMAND_MIN_COUNT || state.alerted.as_deref() == Some(top.as_str()) {
            return None;
        }
        state.alerted = Some(top.clone());
        let key = (channel.to_owned(), top.clone());
        if self.last_alert.get(&key).is_some_and(|t| now.duration_since(*t) < COMMAND_COOLDOWN) {
            return None;
        }
        self.last_alert.insert(key, now);
        let window = state.buffer.len();
        let users = users.remove(&top).unwrap_or_default();
        Some((top, Evidence { count, window, users }))
    }
}

/// Pseudos de `ours` cités dans le message (hors auteur), en minuscules
pub fn mentions(ours: &[String], author: &str, message: &str) -> Vec<String> {
    let lower = message.to_lowercase();
    let author = author.to_lowercase();
    ours.iter()
        .filter(|login| **login != author)
        .filter(|login| {
            lower.match_indices(login.as_str()).any(|(i, _)| {
                let before = lower[..i].chars().next_back();
                let after = lower[i + login.len()..].chars().next();
                let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
                !word(before) && !word(after)
            })
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands() {
        assert_eq!(extract_command("!Enter"), Some("!enter".into()));
        assert_eq!(extract_command("je tape  !join   now svp"), Some("!join now".into()));
        assert_eq!(extract_command("salut tout le monde"), None);
        assert_eq!(normalize_excluded(&["Enter ".into(), "!enter".into(), "".into(), "!points".into()]), vec!["!enter", "!points"]);
        assert!(is_excluded("!enter mot", &["!enter".into()]));
        assert!(!is_excluded("!join", &["!enter".into()]));
    }

    #[test]
    fn detects_after_seven_distinct_users() {
        let mut d = Detector::default();
        let now = Instant::now();
        let mut hit = None;
        for i in 0..7 {
            hit = d.push_at("c", &format!("u{i}"), "!coffre", &[], now);
        }
        let (cmd, ev) = hit.expect("détecté");
        assert_eq!(cmd, "!coffre");
        assert_eq!((ev.count, ev.users.len()), (7, 7));
        // Déjà signalée : pas de nouvelle alerte tant qu'elle ne retombe pas
        assert!(d.push_at("c", "u8", "!coffre", &[], now).is_none());
    }

    #[test]
    fn spammer_alone_is_capped() {
        let mut d = Detector::default();
        let now = Instant::now();
        for _ in 0..20 {
            assert!(d.push_at("c", "spam", "!coffre", &[], now).is_none());
        }
        // 2 par pseudo : 3 autres pseudos ne suffisent pas (2 + 3×2 = 8 ≥ 7 : si)
        let mut d = Detector::default();
        for i in 0..2 {
            assert!(d.push_at("c", &format!("u{i}"), "!coffre", &[], now).is_none());
        }
    }

    #[test]
    fn ignored_and_cooldown() {
        let mut d = Detector::default();
        let now = Instant::now();
        for i in 0..10 {
            assert!(d.push_at("c", &format!("u{i}"), "!ticket", &["!ticket".into()], now).is_none());
        }
        // Retombée puis remontée dans les 10 min : pas de nouvelle alerte
        let mut d = Detector::default();
        for i in 0..7 {
            d.push_at("c", &format!("u{i}"), "!coffre", &[], now);
        }
        for i in 0..20 {
            d.push_at("c", &format!("x{i}"), "bonjour", &[], now);
        }
        let mut again = None;
        for i in 0..7 {
            again = again.or(d.push_at("c", &format!("v{i}"), "!coffre", &[], now));
        }
        assert!(again.is_none());
        // Après 10 min : nouvelle alerte
        for i in 0..20 {
            d.push_at("c", &format!("y{i}"), "bonjour", &[], now);
        }
        let later = now + COMMAND_COOLDOWN;
        let mut again = None;
        for i in 0..7 {
            again = again.or(d.push_at("c", &format!("w{i}"), "!coffre", &[], later));
        }
        assert!(again.is_some());
    }

    #[test]
    fn mention_detection() {
        let ours = vec!["powange2".to_owned(), "alt".to_owned()];
        assert_eq!(mentions(&ours, "bob", "bien joué @PoWange2 !"), vec!["powange2"]);
        assert!(mentions(&ours, "bob", "powange22 n'est pas moi").is_empty());
        assert!(mentions(&ours, "powange2", "moi powange2 je participe").is_empty());
        assert!(mentions(&ours, "bob", "halte").is_empty());
    }
}
