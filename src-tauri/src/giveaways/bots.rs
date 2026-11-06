//! Lecture des messages des bots de giveaway (StreamElements, WizeBot, Nightbot,
//! PhuzzyBot). Port de `shared/utils/botMessages.ts` du site Giveaway Twitch,
//! avec ses cas de test. Module pur : il extrait des événements, l'appelant
//! décide quoi en faire. Seuls les bots de confiance sont lus : un viewer ne
//! doit pas pouvoir falsifier un gagnant en imitant le format.

use std::sync::{LazyLock, RwLock};

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
    /// Lien de la page du giveaway Lingo Echo (https://lingo-echo.eu/giveaway/<chaîne>)
    LingoEchoUrl(String),
}

pub const CHAT_BOTS: &[&str] = &["streamelements", "wizebot", "nightbot", "phuzzybot", "lingo_echo"];

// Faux « gagnants » : « Nobody won… », « Personne a gagné… », « @x, you won… »
const NON_WINNER_NAMES: &[&str] = &["nobody", "noone", "no", "one", "personne", "someone", "somebody", "you"];
// Un login Twitch fait au plus 25 caractères
const MAX_NAME_LENGTH: usize = 25;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("expression valide")
}

static SE_URL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)https://streamelements\.com/[^/\s]+/giveaway/[a-f0-9]+"));
static SE_WIN_EN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)@?([A-Za-z0-9_]+)\s+(?:has\s+)?won the giveaway(?:\s+with\s+(?:an?\s+)?([\d.,]+\s?%)\s+chance(?:\s+to\s+win)?)?"));
static SE_WIN_FR: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)@?([A-Za-z0-9_]+)\s+a gagné le concours(?:\s+avec une chance de\s+([\d.,]+\s?%))?"));
static NO_ONE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\bno\s+one\s+(?:has\s+)?won\b"));
static SE_TICKETS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)@?([A-Za-z0-9_]+)\s+got\s+\d+\s+ticket\(?s?\)?.*?\((\d+)/(\d+)\)"));
static WIZEBOT_WIN: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)tombe sur .*?viewers?\s+@?([A-Za-z0-9_]+)"));
static NIGHTBOT_WIN: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)congratulations,?\s+@?([A-Za-z0-9_]+).*?won the giveaway"));
// Lingo Echo : « La personne gagnante est : @taz2_0 [✅ Suit la chaîne] 🎉 Participations : 8 ;
// chance de gagner : 12.50% »
static LINGO_WIN: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)personne\s+gagnante\s+est\s*:?\s*@?([A-Za-z0-9_]{3,25})\b"));
static LINGO_URL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)https://lingo-echo\.eu/giveaway/[A-Za-z0-9_]+"));
static LINGO_CHANCE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)chance\s+de\s+gagner\s*:?\s*([\d.,]+\s?%)"));
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
// « 🎁 Le gagnant du tirage est wazagoat. », « The winner of the giveaway is Bob »
static GENERIC_WIN_IS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)(?:\ble\s+gagnant|\bla\s+(?:personne\s+)?gagnante|\bles\s+gagnants?|\bthe\s+winners?)(?:\s+(?:du|de\s+la|of\s+the)\s+(?:tirage|giveaway|concours|raffle|loterie))?\s*(?:est|sont|is|are)\s*:?\s*@?([A-Za-z0-9_]{3,25})\b")
});
static GENERIC_WIN_EN: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)congrat(?:ulation)?s?\s*,?\s*@?([A-Za-z0-9_]{3,25})\b[^@]*?\b(?:you\s+(?:have\s+)?won|has\s+won)\b")
});

/// Bot du chat : connu, ou pseudo en « …bot » (moobot, fossabot, sery_bot…)
/// Pseudos ajoutés par l'utilisateur comme bots (Giveaways → Réglages)
static EXTRA_BOTS: LazyLock<RwLock<Vec<String>>> = LazyLock::new(|| RwLock::new(vec![]));

pub fn set_extra_bots(names: &[String]) {
    *EXTRA_BOTS.write().unwrap() = names.iter().map(|n| n.trim().trim_start_matches('@').to_lowercase()).filter(|n| !n.is_empty()).collect();
}

/// Bot du chat : connu, pseudo en « …bot », ou ajouté dans les réglages
pub fn is_bot_like(user: &str) -> bool {
    let user = user.to_lowercase();
    is_trusted_bot(&user) || user.ends_with("bot")
}

/// Comptes vus avec le badge « Chat Bot » de Twitch (bot-badge) : donné par Twitch aux bots
/// ajoutés par un streamer, un spectateur ne peut pas se l'attribuer
static BADGED_BOTS: LazyLock<RwLock<std::collections::HashSet<String>>> = LazyLock::new(|| RwLock::new(Default::default()));
const BADGED_BOTS_MAX: usize = 500;

/// Auteur d'un message portant le badge « Chat Bot » : bot de confiance désormais
pub fn note_badged_bot(user: &str) {
    let user = user.to_lowercase();
    if BADGED_BOTS.read().unwrap().contains(&user) {
        return;
    }
    let mut set = BADGED_BOTS.write().unwrap();
    if set.len() < BADGED_BOTS_MAX {
        set.insert(user);
    }
}

/// Bot identifié par son pseudo exact (connu, ajouté dans les réglages) ou par le badge « Chat
/// Bot » : un spectateur ne peut pas le prendre. Un pseudo en « …bot » seul n'en est pas un
/// (« viewerbot » est libre)
pub fn is_trusted_bot(user: &str) -> bool {
    let user = user.to_lowercase();
    is_chat_bot(&user) || user == "streamlabs" || EXTRA_BOTS.read().unwrap().contains(&user) || BADGED_BOTS.read().unwrap().contains(&user)
}

/// Formulations ajoutées par l'utilisateur (Giveaways → Réglages), compilées
static CUSTOM_WINS: LazyLock<RwLock<Vec<Regex>>> = LazyLock::new(|| RwLock::new(vec![]));

/// Formulation saisie (« Le gagnant est {pseudo} ») en expression : majuscules ignorées,
/// espaces souples, « @ » facultatif devant le pseudo. Erreur si {pseudo} manque ou se répète
pub fn compile_template(template: &str) -> Result<Regex, String> {
    let template = template.trim();
    let parts: Vec<&str> = template.split("{pseudo}").collect();
    if parts.len() != 2 {
        return Err(format!("« {template} » : mets {{pseudo}} une fois à la place du nom du gagnant"));
    }
    if template.chars().count() > 200 {
        return Err(format!("« {template} » : 200 caractères au plus"));
    }
    let literal = |s: &str| s.split_whitespace().map(regex::escape).collect::<Vec<_>>().join(r"\s+");
    let (before, after) = (literal(parts[0]), literal(parts[1]));
    let pattern = format!(r"(?i){before}\s*@?([A-Za-z0-9_]{{3,25}})\b\s*{after}");
    Regex::new(&pattern).map_err(|e| format!("« {template} » : {e}"))
}

/// Remplace les formulations personnalisées ; renvoie les erreurs des formulations écartées
pub fn set_custom_patterns(templates: &[String]) -> Vec<String> {
    let mut compiled = vec![];
    let mut errors = vec![];
    for t in templates.iter().filter(|t| !t.trim().is_empty()) {
        match compile_template(t) {
            Ok(re) => compiled.push(re),
            Err(e) => errors.push(e),
        }
    }
    *CUSTOM_WINS.write().unwrap() = compiled;
    errors
}

fn custom_winner(bot: &str, message: &str) -> Option<BotEvent> {
    let customs = CUSTOM_WINS.read().unwrap();
    customs.iter().find_map(|re| re.captures(message).and_then(|c| winner(bot, c.get(1).map(|m| m.as_str()), None)))
}

/// Teste une phrase avec les formulations intégrées et celles données (non enregistrées) :
/// (gagnant, formulation personnalisée ?)
pub fn test_phrase(message: &str, templates: &[String]) -> Result<Option<(String, bool)>, String> {
    for t in templates.iter().filter(|t| !t.trim().is_empty()) {
        let re = compile_template(t)?;
        if let Some(BotEvent::Winner { winner, .. }) = re.captures(message).and_then(|c| winner("test", c.get(1).map(|m| m.as_str()), None)) {
            return Ok(Some((winner, true)));
        }
    }
    let builtin = [&SE_WIN_EN, &SE_WIN_FR, &WIZEBOT_WIN, &NIGHTBOT_WIN, &LINGO_WIN]
        .iter()
        .find_map(|re| re.captures(message).and_then(|c| winner("test", c.get(1).map(|m| m.as_str()), None)))
        .or_else(|| generic_winner_builtin("test", message));
    Ok(builtin.and_then(|e| match e {
        BotEvent::Winner { winner, .. } => Some((winner, false)),
        _ => None,
    }))
}

fn generic_winner(bot: &str, message: &str) -> Option<BotEvent> {
    generic_winner_builtin(bot, message).or_else(|| custom_winner(bot, message))
}

fn generic_winner_builtin(bot: &str, message: &str) -> Option<BotEvent> {
    let c = GENERIC_WIN_FR
        .captures(message)
        .or_else(|| GENERIC_WIN_EN.captures(message))
        .or_else(|| GENERIC_WIN_SUBJECT.captures(message))
        .or_else(|| GENERIC_WIN_IS.captures(message))?;
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
    Some(BotEvent::Winner { bot: bot.to_owned(), winner: name.to_owned(), chance: chance.and_then(normalize_chance) })
}

/// Chance de gagner écrite « 20,92 % », « 20.92% » ou « 20.92 » : « 20.92% » (None : illisible)
pub fn normalize_chance(text: &str) -> Option<String> {
    let number = text.trim().trim_end_matches('%').trim().replace(',', ".");
    let value: f64 = number.parse().ok()?;
    ((0.0..=100.0).contains(&value) && !number.is_empty()).then(|| format!("{number}%"))
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

/// Événements reconnus dans le message d'un bot connu (vide sinon). `privileged` : l'auteur
/// a le badge modérateur ou est le streamer ; exigé pour les annonces de gagnant d'un pseudo
/// en « …bot » qui n'est ni un bot connu ni un bot ajouté dans les réglages
pub fn parse_bot_message(user: &str, message: &str, privileged: bool) -> Vec<BotEvent> {
    let bot = user.to_lowercase();
    let events = match bot.as_str() {
        "streamelements" => vec![
            // Domaine écrit en minuscules (« https://StreamElements.com/… » vu dans les chats)
            SE_URL.find(message).map(|m| {
                let url = m.as_str();
                BotEvent::StreamElementsUrl(format!("https://streamelements.com/{}", &url["https://streamelements.com/".len()..]))
            }),
            streamelements_winner(message),
            ticket_info(message),
            ticket_purchase(message),
        ],
        "wizebot" => vec![WIZEBOT_WIN.captures(message).and_then(|c| winner("wizebot", c.get(1).map(|m| m.as_str()), None))],
        "nightbot" => vec![NIGHTBOT_WIN.captures(message).and_then(|c| winner("nightbot", c.get(1).map(|m| m.as_str()), None))],
        "phuzzybot" => vec![ticket_purchase(message)],
        "lingo_echo" => vec![
            LINGO_WIN.captures(message).and_then(|c| {
                let chance = LINGO_CHANCE.captures(message).and_then(|m| m.get(1)).map(|m| m.as_str().to_owned());
                winner("lingo_echo", c.get(1).map(|m| m.as_str()), chance.as_deref())
            }),
            LINGO_URL.find(message).map(|m| BotEvent::LingoEchoUrl(m.as_str().to_lowercase())),
        ],
        _ => vec![],
    };
    let mut events: Vec<BotEvent> = events.into_iter().flatten().collect();
    // Pseudo en « …bot » sans badge : un spectateur a pu le choisir, ses annonces sont ignorées
    let trusted = is_trusted_bot(&bot) || (is_bot_like(&bot) && privileged);
    // Autre bot (moobot, sery_bot…) : formulations générales ; les bots connus gardent leurs
    // formats, et un spectateur n'est jamais pris pour un bot
    if trusted && !is_chat_bot(&bot) && !events.iter().any(|e| matches!(e, BotEvent::Winner { .. })) {
        events.extend(generic_winner(&bot, message));
    }
    // Formulations personnalisées : pour tous les bots, connus compris
    if trusted && !events.iter().any(|e| matches!(e, BotEvent::Winner { .. })) {
        events.extend(custom_winner(&bot, message));
    }
    events
}

/// Événements d'un message de bot : son format, puis, s'il a le badge modérateur (ou est le
/// streamer), les formulations générales quand son format n'annonce pas de gagnant
/// (« 🎁 Le gagnant du tirage est nervi253. » écrit par WizeBot)
pub fn bot_events(user: &str, message: &str, privileged: bool) -> Vec<BotEvent> {
    let mut events = parse_bot_message(user, message, privileged);
    if privileged && !events.iter().any(|e| matches!(e, BotEvent::Winner { .. })) {
        events.extend(moderator_winner(&user.to_lowercase(), message));
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
    fn lingo_echo_winner() {
        let msg = "⠛⠛⠛⠛ 🟥 FBtouchdown La personne gagnante est : @taz2_0 [✅ Suit la chaîne] 🎉 Participations : 8 ; chance de gagner : 12.50% 🟥 Vous avez 90 secondes pour écrire dans le chat ! CoolCat HSWP Détails : https://lingo-echo.eu/giveaway/vulvyqueen ⣤⣤⣤";
        assert_eq!(
            parse_bot_message("Lingo_Echo", msg, false),
            vec![win("lingo_echo", "taz2_0", Some("12.50%")), BotEvent::LingoEchoUrl("https://lingo-echo.eu/giveaway/vulvyqueen".into())]
        );
        // Même phrase écrite par un autre bot modérateur : formulation générale
        assert_eq!(bot_events("autrebot", "La personne gagnante est : @taz2_0", true), vec![win("autrebot", "taz2_0", None)]);
        // Un spectateur qui imite le message n'est pas lu
        assert!(parse_bot_message("viewer42", msg, false).is_empty());
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
    fn custom_templates() {
        let re = compile_template("Bravo à {pseudo} qui remporte").unwrap();
        let c = re.captures("🎉 BRAVO   à @Zoe_77 qui remporte le lot !").unwrap();
        assert_eq!(&c[1], "Zoe_77");
        assert!(compile_template("pas de pseudo").is_err());
        assert!(compile_template("{pseudo} et {pseudo}").is_err());
        // Caractères spéciaux pris tels quels
        assert!(compile_template("(gagnant) {pseudo} ?").unwrap().is_match("(gagnant) bob42 ?"));
        assert_eq!(test_phrase("Bravo à bob42 qui remporte", &["Bravo à {pseudo} qui remporte".into()]).unwrap(), Some(("bob42".into(), true)));
        assert_eq!(test_phrase("Le gagnant du tirage est wazagoat.", &[]).unwrap(), Some(("wazagoat".into(), false)));
        assert_eq!(test_phrase("rien à voir", &[]).unwrap(), None);
    }

    #[test]
    fn generic_winner_announcements() {
        assert_eq!(
            parse_bot_message("Sery_Bot", "🎉 Félicitations @julsr8 ! Vous avez gagné le giveaway ! 🎉", true),
            vec![win("sery_bot", "julsr8", None)]
        );
        assert_eq!(parse_bot_message("moobot", "Congrats @Bob_42, you won the raffle!", true), vec![win("moobot", "Bob_42", None)]);
        assert_eq!(parse_bot_message("fossabot", "Félicitations à tous, tu as gagné quoi ?", true), vec![]);
        assert_eq!(
            parse_bot_message("moobot", "🎉🎉🎉 zalmomo a remporté le giveaway, bravo ! 🎉🎉🎉", true),
            vec![win("moobot", "zalmomo", None)]
        );
        assert_eq!(parse_bot_message("moobot", "Personne a gagné le tirage", true), vec![]);
        // Annonce réelle de MwettonBot sur seaofthieves_france
        assert_eq!(
            parse_bot_message("MwettonBot", "🎉🎉🎉 zalmomo a remporté le giveaway, bravo ! 🎉🎉🎉", true),
            vec![win("mwettonbot", "zalmomo", None)]
        );
        assert_eq!(moderator_winner("lunarsea", "Bob_42 wins the raffle!"), Some(win("lunarsea", "Bob_42", None)));
        // Annonce réelle de lcalypsl (modérateur)
        assert_eq!(moderator_winner("lcalypsl", "🎁 Le gagnant du tirage est wazagoat."), Some(win("lcalypsl", "wazagoat", None)));
        assert_eq!(moderator_winner("x", "La gagnante est @Alice_99 !"), Some(win("x", "Alice_99", None)));
        assert_eq!(moderator_winner("x", "The winner of the giveaway is: Bob42"), Some(win("x", "Bob42", None)));
        assert_eq!(moderator_winner("x", "le gagnant est personne"), None);
        assert_eq!(moderator_winner("x", "qui sera le gagnant ? tapez !join"), None);
        assert_eq!(moderator_winner("lunarsea", "le giveaway commence, tapez !join"), None);
        assert!(is_bot_like("Sery_Bot") && is_bot_like("streamelements") && !is_bot_like("bobby"));
    }

    #[test]
    fn streamelements_winners() {
        assert_eq!(
            parse_bot_message("StreamElements", "@Pseudo_42 won the giveaway with 3.94% chance to win", false),
            vec![win("streamelements", "Pseudo_42", Some("3.94%"))]
        );
        assert_eq!(parse_bot_message("streamelements", "pseudo won the giveaway!", false), vec![win("streamelements", "pseudo", None)]);
        // « with a 20.92% chance », message /me (ACTION)
        assert_eq!(
            parse_bot_message("StreamElements", "ACTION @bilge_rat_bust3r267 won the giveaway with a 20.92% chance to win PogChamp", false),
            vec![win("streamelements", "bilge_rat_bust3r267", Some("20.92%"))]
        );
        assert_eq!(normalize_chance("20,5 %").as_deref(), Some("20.5%"));
        assert_eq!(normalize_chance("12"), Some("12%".into()));
        assert_eq!(normalize_chance("abc"), None);
        assert_eq!(normalize_chance("150%"), None);
        assert_eq!(
            parse_bot_message("streamelements", "@foo has won the giveaway with 100% chance to win PogChamp", false),
            vec![win("streamelements", "foo", Some("100%"))]
        );
        assert_eq!(
            parse_bot_message("streamelements", "@Gagnant a gagné le concours avec une chance de 0.57%", false),
            vec![win("streamelements", "Gagnant", Some("0.57%"))]
        );
        assert_eq!(parse_bot_message("streamelements", "gagnant a gagné le concours", false), vec![win("streamelements", "gagnant", None)]);
    }

    #[test]
    fn no_winner() {
        for m in ["Nobody won the giveaway", "No one won the giveaway", "Personne a gagné le concours", "@viewer, you won the giveaway!"] {
            assert!(parse_bot_message("streamelements", m, false).is_empty(), "{m}");
        }
        assert!(parse_bot_message("streamelements", &format!("{} won the giveaway", "a".repeat(26)), false).is_empty());
        assert!(parse_bot_message("nightbot", "Congratulations, nobody! You won the giveaway!", false).is_empty());
    }

    #[test]
    fn streamelements_url_and_tickets() {
        assert_eq!(
            parse_bot_message("streamelements", "Join the giveaway here: https://streamelements.com/some_channel/giveaway/65f1a2b3c4d5e6f7a8b9c0d1 !", false),
            vec![BotEvent::StreamElementsUrl("https://streamelements.com/some_channel/giveaway/65f1a2b3c4d5e6f7a8b9c0d1".into())]
        );
        assert_eq!(
            parse_bot_message("streamelements", "@Viewer got 5 tickets! (5/10)", false),
            vec![BotEvent::TicketInfo { pseudo: "Viewer".into(), current: 5, max: 10 }]
        );
        assert_eq!(
            parse_bot_message("streamelements", "viewer got 1 ticket(s)! (1/1)", false),
            vec![BotEvent::TicketInfo { pseudo: "viewer".into(), current: 1, max: 1 }]
        );
        assert_eq!(
            parse_bot_message("streamelements", "@Viewer, you successfully bought 3 tickets!", false),
            vec![BotEvent::TicketPurchase { pseudo: "Viewer".into(), count: 3 }]
        );
        assert!(parse_bot_message("streamelements", "Follow the channel to get notified!", false).is_empty());
    }

    #[test]
    fn wizebot_and_nightbot() {
        assert_eq!(
            parse_bot_message("WizeBot", "Le tirage au sort tombe sur le/les viewers Pseudo [SUB].", false),
            vec![win("wizebot", "Pseudo", None)]
        );
        assert_eq!(parse_bot_message("wizebot", "Le tirage au sort tombe sur le viewer @pseudo !", false), vec![win("wizebot", "pseudo", None)]);
        assert!(parse_bot_message("wizebot", "@foo won the giveaway", false).is_empty());
        assert_eq!(
            parse_bot_message("Nightbot", "Congratulations, nickname1334! You won the giveaway!", false),
            vec![win("nightbot", "nickname1334", None)]
        );
        assert_eq!(parse_bot_message("nightbot", "Congratulations @Foo, you won the giveaway", false), vec![win("nightbot", "Foo", None)]);
    }

    #[test]
    fn moderator_bot_with_general_phrasing() {
        // WizeBot modérateur : son format, ou les formulations générales
        assert_eq!(bot_events("WizeBot", "🎁 Le gagnant du tirage est nervi253.", true), vec![win("wizebot", "nervi253", None)]);
        assert_eq!(
            bot_events("wizebot", "Le tirage au sort tombe sur le viewer @pseudo !", true),
            vec![win("wizebot", "pseudo", None)]
        );
        // Sans badge : son format seulement
        assert!(bot_events("wizebot", "🎁 Le gagnant du tirage est nervi253.", false).is_empty());
    }

    #[test]
    fn phuzzybot() {
        assert_eq!(
            parse_bot_message("PhuzzyBot", "@pseudo, successfully bought 4 tickets!", false),
            vec![BotEvent::TicketPurchase { pseudo: "pseudo".into(), count: 4 }]
        );
        assert_eq!(
            parse_bot_message("phuzzybot", "@Pseudo, you successfully bought 1 ticket!", false),
            vec![BotEvent::TicketPurchase { pseudo: "Pseudo".into(), count: 1 }]
        );
        assert!(parse_bot_message("phuzzybot", "@pseudo, successfully bought 0 tickets!", false).is_empty());
        assert!(parse_bot_message("phuzzybot", "@foo won the giveaway", false).is_empty());
        assert!(parse_bot_message("phuzzybot", "@foo got 2 tickets! (2/5)", false).is_empty());
    }

    #[test]
    fn bot_like_needs_badge() {
        // Pseudo en « …bot » sans badge : annonce ignorée (tout spectateur peut s'appeler ainsi)
        assert!(parse_bot_message("viewerbot", "Congrats @viewerbot, you won the raffle!", false).is_empty());
        assert_eq!(
            parse_bot_message("viewerbot", "Congrats @Bob_42, you won the raffle!", true),
            vec![win("viewerbot", "Bob_42", None)]
        );
        // Bot ajouté dans les réglages : cru sans badge
        set_extra_bots(&["@MyGiveawayBot".into()]);
        assert_eq!(
            parse_bot_message("mygiveawaybot", "Congrats @Bob_42, you won the raffle!", false),
            vec![win("mygiveawaybot", "Bob_42", None)]
        );
        set_extra_bots(&[]);
        assert!(is_trusted_bot("Nightbot") && !is_trusted_bot("sery_bot") && is_bot_like("sery_bot"));
        // Badge « Chat Bot » vu sur un message : bot de confiance, quel que soit son pseudo
        assert!(!is_trusted_bot("badgedgamebotless") && !is_bot_like("badgedgamebotless"));
        note_badged_bot("BadgedGameBotless");
        assert!(is_trusted_bot("badgedgamebotless") && is_bot_like("badgedgamebotless"));
    }

    #[test]
    fn impostors_ignored() {
        assert!(parse_bot_message("viewer123", "@viewer123 won the giveaway with 99% chance to win", false).is_empty());
        assert!(parse_bot_message("viewer123", "https://streamelements.com/x/giveaway/abc123", false).is_empty());
        assert!(parse_bot_message("streamelements_fan", "@foo won the giveaway", false).is_empty());
    }
}
