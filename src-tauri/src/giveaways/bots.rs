//! Lecture des messages des bots de giveaway (StreamElements, WizeBot, Nightbot,
//! PhuzzyBot). Port de `shared/utils/botMessages.ts` du site Giveaway Twitch,
//! avec ses cas de test. Module pur : il extrait des événements, l'appelant
//! décide quoi en faire. Seuls les bots de confiance sont lus : un viewer ne
//! doit pas pouvoir falsifier un gagnant en imitant le format.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
pub enum BotEvent {
    Winner { bot: String, winner: String, chance: Option<String> },
    /// « @pseudo got N ticket(s)! (N/MAX) » (StreamElements)
    TicketInfo { pseudo: String, current: i64, max: i64 },
    /// « @pseudo, [you] successfully bought N ticket(s)! » (StreamElements, PhuzzyBot)
    TicketPurchase { pseudo: String, count: i64 },
    /// Lien du giveaway StreamElements posté par le bot
    StreamElementsUrl(String),
}

pub const CHAT_BOTS: &[&str] = &["streamelements", "wizebot", "nightbot", "phuzzybot"];

// Faux « gagnants » : « Nobody won… », « Personne a gagné… », « @x, you won… »
const NON_WINNER_NAMES: &[&str] = &["nobody", "noone", "no", "one", "personne", "someone", "somebody", "you"];
// Un login Twitch fait au plus 25 caractères
const MAX_NAME_LENGTH: usize = 25;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("expression valide")
}

static SE_URL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)https://streamelements\.com/[^/\s]+/giveaway/[a-f0-9]+"));
static SE_WIN_EN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)@?([A-Za-z0-9_]+)\s+(?:has\s+)?won the giveaway(?:\s+with\s+([\d.]+%)\s+chance to win)?"));
static SE_WIN_FR: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)@?([A-Za-z0-9_]+)\s+a gagné le concours(?:\s+avec une chance de\s+([\d.]+%))?"));
static NO_ONE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\bno\s+one\s+(?:has\s+)?won\b"));
static SE_TICKETS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)@?([A-Za-z0-9_]+)\s+got\s+\d+\s+ticket\(?s?\)?.*?\((\d+)/(\d+)\)"));
static WIZEBOT_WIN: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)tombe sur .*?viewers?\s+@?([A-Za-z0-9_]+)"));
static NIGHTBOT_WIN: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)congratulations,?\s+@?([A-Za-z0-9_]+).*?won the giveaway"));
static PURCHASE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)@?([A-Za-z0-9_]+),?\s+(?:you\s+)?successfully bought\s+(\d+)\s+ticket"));

// Annonces de gagnant de tout bot : « 🎉 Félicitations @pseudo ! Vous avez gagné le giveaway ! »,
// « Congrats @pseudo, you won… »
static GENERIC_WIN_FR: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)f[ée]licitations?\s*,?\s*@?([A-Za-z0-9_]{3,25})\b[^@]*?\b(?:vous\s+avez|tu\s+as|a)\s+gagn[ée]")
});
// « 🎉 zalmomo a remporté le giveaway, bravo ! », « Bob wins the raffle »
static GENERIC_WIN_SUBJECT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)(?:^|[\s:!,.])@?([A-Za-z0-9_]{3,25})\s+(?:a\s+(?:gagn[ée]|remport[ée])|remporte|gagne|has\s+won|won|wins)\s+(?:le|la|l'|the)\s*(?:giveaway|concours|tirage|raffle|loterie|cadeau)")
});
static GENERIC_WIN_EN: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)congrat(?:ulation)?s?\s*,?\s*@?([A-Za-z0-9_]{3,25})\b[^@]*?\b(?:you\s+(?:have\s+)?won|has\s+won)\b")
});

/// Bot du chat : connu, ou pseudo en « …bot » (moobot, fossabot, sery_bot…)
pub fn is_bot_like(user: &str) -> bool {
    let user = user.to_lowercase();
    is_chat_bot(&user) || user.ends_with("bot") || user == "streamlabs"
}

fn generic_winner(bot: &str, message: &str) -> Option<BotEvent> {
    let c = GENERIC_WIN_FR
        .captures(message)
        .or_else(|| GENERIC_WIN_EN.captures(message))
        .or_else(|| GENERIC_WIN_SUBJECT.captures(message))?;
    winner(bot, c.get(1).map(|m| m.as_str()), None)
}

/// Annonce de gagnant écrite par un modérateur ou le streamer (formulations générales seulement)
pub fn moderator_winner(author: &str, message: &str) -> Option<BotEvent> {
    generic_winner(author, message)
}

pub fn is_non_winner_name(name: &str) -> bool {
    NON_WINNER_NAMES.contains(&name.to_lowercase().as_str())
}

pub fn is_chat_bot(user: &str) -> bool {
    CHAT_BOTS.contains(&user.to_lowercase().as_str())
}

fn winner(bot: &str, name: Option<&str>, chance: Option<&str>) -> Option<BotEvent> {
    let name = name?;
    if name.len() > MAX_NAME_LENGTH || is_non_winner_name(name) {
        return None;
    }
    Some(BotEvent::Winner { bot: bot.to_owned(), winner: name.to_owned(), chance: chance.map(str::to_owned) })
}

fn streamelements_winner(message: &str) -> Option<BotEvent> {
    if NO_ONE.is_match(message) {
        return None;
    }
    // Une annonce est en anglais OU en français : si l'anglais a reconnu un faux
    // gagnant (« Nobody won… »), inutile de tenter le français
    let caps = SE_WIN_EN.captures(message).or_else(|| SE_WIN_FR.captures(message))?;
    winner("streamelements", caps.get(1).map(|m| m.as_str()), caps.get(2).map(|m| m.as_str()))
}

fn ticket_info(message: &str) -> Option<BotEvent> {
    let c = SE_TICKETS.captures(message)?;
    Some(BotEvent::TicketInfo { pseudo: c[1].to_owned(), current: c[2].parse().ok()?, max: c[3].parse().ok()? })
}

fn ticket_purchase(message: &str) -> Option<BotEvent> {
    let c = PURCHASE.captures(message)?;
    let count: i64 = c[2].parse().ok()?;
    (count > 0).then(|| BotEvent::TicketPurchase { pseudo: c[1].to_owned(), count })
}

/// Événements reconnus dans le message d'un bot connu (vide sinon)
pub fn parse_bot_message(user: &str, message: &str) -> Vec<BotEvent> {
    let bot = user.to_lowercase();
    let events = match bot.as_str() {
        "streamelements" => vec![
            SE_URL.find(message).map(|m| BotEvent::StreamElementsUrl(m.as_str().to_owned())),
            streamelements_winner(message),
            ticket_info(message),
            ticket_purchase(message),
        ],
        "wizebot" => vec![WIZEBOT_WIN.captures(message).and_then(|c| winner("wizebot", c.get(1).map(|m| m.as_str()), None))],
        "nightbot" => vec![NIGHTBOT_WIN.captures(message).and_then(|c| winner("nightbot", c.get(1).map(|m| m.as_str()), None))],
        "phuzzybot" => vec![ticket_purchase(message)],
        _ => vec![],
    };
    let mut events: Vec<BotEvent> = events.into_iter().flatten().collect();
    // Autre bot (moobot, sery_bot…) : formulations générales ; les bots connus gardent leurs
    // formats, et un spectateur n'est jamais pris pour un bot
    if !is_chat_bot(&bot) && is_bot_like(&bot) && !events.iter().any(|e| matches!(e, BotEvent::Winner { .. })) {
        events.extend(generic_winner(&bot, message));
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(bot: &str, w: &str, chance: Option<&str>) -> BotEvent {
        BotEvent::Winner { bot: bot.into(), winner: w.into(), chance: chance.map(Into::into) }
    }

    #[test]
    fn non_winner_names() {
        for n in ["Nobody", "NOONE", "personne", "You"] {
            assert!(is_non_winner_name(n));
        }
        for n in ["pseudo", "nobody_x"] {
            assert!(!is_non_winner_name(n));
        }
    }

    #[test]
    fn generic_winner_announcements() {
        assert_eq!(
            parse_bot_message("Sery_Bot", "🎉 Félicitations @julsr8 ! Vous avez gagné le giveaway ! 🎉"),
            vec![win("sery_bot", "julsr8", None)]
        );
        assert_eq!(parse_bot_message("moobot", "Congrats @Bob_42, you won the raffle!"), vec![win("moobot", "Bob_42", None)]);
        assert_eq!(parse_bot_message("fossabot", "Félicitations à tous, tu as gagné quoi ?"), vec![]);
        assert_eq!(
            parse_bot_message("moobot", "🎉🎉🎉 zalmomo a remporté le giveaway, bravo ! 🎉🎉🎉"),
            vec![win("moobot", "zalmomo", None)]
        );
        assert_eq!(parse_bot_message("moobot", "Personne a gagné le tirage"), vec![]);
        // Annonce réelle de MwettonBot sur seaofthieves_france
        assert_eq!(
            parse_bot_message("MwettonBot", "🎉🎉🎉 zalmomo a remporté le giveaway, bravo ! 🎉🎉🎉"),
            vec![win("mwettonbot", "zalmomo", None)]
        );
        assert_eq!(moderator_winner("lunarsea", "Bob_42 wins the raffle!"), Some(win("lunarsea", "Bob_42", None)));
        assert_eq!(moderator_winner("lunarsea", "le giveaway commence, tapez !join"), None);
        assert!(is_bot_like("Sery_Bot") && is_bot_like("streamelements") && !is_bot_like("bobby"));
    }

    #[test]
    fn streamelements_winners() {
        assert_eq!(
            parse_bot_message("StreamElements", "@Pseudo_42 won the giveaway with 3.94% chance to win"),
            vec![win("streamelements", "Pseudo_42", Some("3.94%"))]
        );
        assert_eq!(parse_bot_message("streamelements", "pseudo won the giveaway!"), vec![win("streamelements", "pseudo", None)]);
        assert_eq!(
            parse_bot_message("streamelements", "@foo has won the giveaway with 100% chance to win PogChamp"),
            vec![win("streamelements", "foo", Some("100%"))]
        );
        assert_eq!(
            parse_bot_message("streamelements", "@Gagnant a gagné le concours avec une chance de 0.57%"),
            vec![win("streamelements", "Gagnant", Some("0.57%"))]
        );
        assert_eq!(parse_bot_message("streamelements", "gagnant a gagné le concours"), vec![win("streamelements", "gagnant", None)]);
    }

    #[test]
    fn no_winner() {
        for m in ["Nobody won the giveaway", "No one won the giveaway", "Personne a gagné le concours", "@viewer, you won the giveaway!"] {
            assert!(parse_bot_message("streamelements", m).is_empty(), "{m}");
        }
        assert!(parse_bot_message("streamelements", &format!("{} won the giveaway", "a".repeat(26))).is_empty());
        assert!(parse_bot_message("nightbot", "Congratulations, nobody! You won the giveaway!").is_empty());
    }

    #[test]
    fn streamelements_url_and_tickets() {
        assert_eq!(
            parse_bot_message("streamelements", "Join the giveaway here: https://streamelements.com/some_channel/giveaway/65f1a2b3c4d5e6f7a8b9c0d1 !"),
            vec![BotEvent::StreamElementsUrl("https://streamelements.com/some_channel/giveaway/65f1a2b3c4d5e6f7a8b9c0d1".into())]
        );
        assert_eq!(
            parse_bot_message("streamelements", "@Viewer got 5 tickets! (5/10)"),
            vec![BotEvent::TicketInfo { pseudo: "Viewer".into(), current: 5, max: 10 }]
        );
        assert_eq!(
            parse_bot_message("streamelements", "viewer got 1 ticket(s)! (1/1)"),
            vec![BotEvent::TicketInfo { pseudo: "viewer".into(), current: 1, max: 1 }]
        );
        assert_eq!(
            parse_bot_message("streamelements", "@Viewer, you successfully bought 3 tickets!"),
            vec![BotEvent::TicketPurchase { pseudo: "Viewer".into(), count: 3 }]
        );
        assert!(parse_bot_message("streamelements", "Follow the channel to get notified!").is_empty());
    }

    #[test]
    fn wizebot_and_nightbot() {
        assert_eq!(
            parse_bot_message("WizeBot", "Le tirage au sort tombe sur le/les viewers Pseudo [SUB]."),
            vec![win("wizebot", "Pseudo", None)]
        );
        assert_eq!(parse_bot_message("wizebot", "Le tirage au sort tombe sur le viewer @pseudo !"), vec![win("wizebot", "pseudo", None)]);
        assert!(parse_bot_message("wizebot", "@foo won the giveaway").is_empty());
        assert_eq!(
            parse_bot_message("Nightbot", "Congratulations, nickname1334! You won the giveaway!"),
            vec![win("nightbot", "nickname1334", None)]
        );
        assert_eq!(parse_bot_message("nightbot", "Congratulations @Foo, you won the giveaway"), vec![win("nightbot", "Foo", None)]);
    }

    #[test]
    fn phuzzybot() {
        assert_eq!(
            parse_bot_message("PhuzzyBot", "@pseudo, successfully bought 4 tickets!"),
            vec![BotEvent::TicketPurchase { pseudo: "pseudo".into(), count: 4 }]
        );
        assert_eq!(
            parse_bot_message("phuzzybot", "@Pseudo, you successfully bought 1 ticket!"),
            vec![BotEvent::TicketPurchase { pseudo: "Pseudo".into(), count: 1 }]
        );
        assert!(parse_bot_message("phuzzybot", "@pseudo, successfully bought 0 tickets!").is_empty());
        assert!(parse_bot_message("phuzzybot", "@foo won the giveaway").is_empty());
        assert!(parse_bot_message("phuzzybot", "@foo got 2 tickets! (2/5)").is_empty());
    }

    #[test]
    fn impostors_ignored() {
        assert!(parse_bot_message("viewer123", "@viewer123 won the giveaway with 99% chance to win").is_empty());
        assert!(parse_bot_message("viewer123", "https://streamelements.com/x/giveaway/abc123").is_empty());
        assert!(parse_bot_message("streamelements_fan", "@foo won the giveaway").is_empty());
    }
}
