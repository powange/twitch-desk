//! StreamElements : points de fidélité et giveaways, lus sur l'API publique
//! (sans connexion), et réponses du bot dans le chat pour nos tickets.
//! Adresses non documentées officiellement, relevées le 2026-11-01.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

const API: &str = "https://api.streamelements.com";

#[derive(Debug, Deserialize)]
struct SeChannel {
    #[serde(rename = "_id")]
    id: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SePoints {
    pub points: i64,
    #[serde(default)]
    pub rank: Option<i64>,
}

/// Réglage de fidélité de la chaîne : comment on gagne les points
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Loyalty {
    pub enabled: bool,
    pub name: String,
    /// Points donnés à chaque distribution…
    pub amount: i64,
    /// …toutes les `interval` minutes, aux spectateurs présents pendant le live
    pub interval: i64,
    pub subscriber_multiplier: f64,
    /// Bonus ponctuels non nuls (follow, sub, tip, cheer, raid, host)
    pub bonuses: Vec<(String, i64)>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoyaltyRaw {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    name: String,
    #[serde(default)]
    amount: i64,
    #[serde(default)]
    interval: i64,
    #[serde(default)]
    subscriber_multiplier: Option<f64>,
    #[serde(default)]
    bonuses: std::collections::BTreeMap<String, i64>,
}

#[derive(Debug, Deserialize)]
struct LoyaltyResponse {
    loyalty: LoyaltyRaw,
}

/// Giveaway StreamElements en cours sur une chaîne
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Giveaway {
    pub id: String,
    pub title: String,
    pub description: String,
    pub command: Option<String>,
    /// Prix d'un ticket en points StreamElements
    pub cost: i64,
    /// Tickets maximum par personne (None : pas de limite)
    pub max_tickets: Option<i64>,
    pub free_ticket: bool,
    pub subscriber_only: bool,
    pub total_tickets: i64,
    pub total_users: i64,
    /// Le bot annonce-t-il les achats dans le chat ? Sinon, nos tickets restent inconnus
    pub bot_responses: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GiveawayRaw {
    #[serde(rename = "_id")]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    cost: i64,
    #[serde(default)]
    max_tickets: i64,
    #[serde(default)]
    free_ticket: bool,
    #[serde(default)]
    subscriber_only: bool,
    #[serde(default)]
    total_amount: i64,
    #[serde(default)]
    total_users: i64,
    #[serde(default = "yes")]
    bot_responses: bool,
    #[serde(default)]
    state: String,
}

fn yes() -> bool {
    true
}

/// Identifiant StreamElements d'une chaîne Twitch (None : la chaîne n'utilise pas StreamElements)
pub async fn channel_id(http: &reqwest::Client, login: &str) -> Result<Option<String>, reqwest::Error> {
    let res = http.get(format!("{API}/kappa/v2/channels/{login}")).send().await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(res.error_for_status()?.json::<SeChannel>().await?.id))
}

/// Solde d'un spectateur (0 tant qu'il n'a rien gagné : StreamElements répond 404)
pub async fn points(http: &reqwest::Client, se_channel: &str, username: &str) -> Result<SePoints, reqwest::Error> {
    let res = http.get(format!("{API}/kappa/v2/points/{se_channel}/{username}")).send().await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(SePoints { points: 0, rank: None });
    }
    res.error_for_status()?.json().await
}

pub async fn loyalty(http: &reqwest::Client, se_channel: &str) -> Result<Option<Loyalty>, reqwest::Error> {
    let res = http.get(format!("{API}/kappa/v2/loyalty/{se_channel}")).send().await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let raw = res.error_for_status()?.json::<LoyaltyResponse>().await?.loyalty;
    Ok(Some(to_loyalty(raw)))
}

fn to_loyalty(raw: LoyaltyRaw) -> Loyalty {
    Loyalty {
        enabled: raw.enabled,
        name: raw.name,
        amount: raw.amount,
        interval: raw.interval,
        subscriber_multiplier: raw.subscriber_multiplier.unwrap_or(1.0),
        bonuses: raw.bonuses.into_iter().filter(|(_, v)| *v > 0).collect(),
    }
}

/// Giveaway en cours (None : aucun ; StreamElements répond 404)
pub async fn active_giveaway(http: &reqwest::Client, se_channel: &str) -> Result<Option<Giveaway>, reqwest::Error> {
    let res = http.get(format!("{API}/kappa/v3/giveaways/{se_channel}/active")).send().await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let raw = res.error_for_status()?.json::<GiveawayRaw>().await?;
    Ok(to_giveaway(raw))
}

fn to_giveaway(raw: GiveawayRaw) -> Option<Giveaway> {
    if raw.state != "running" {
        return None;
    }
    Some(Giveaway {
        command: extract_command(&raw.description),
        id: raw.id,
        title: raw.title,
        description: raw.description,
        cost: raw.cost,
        max_tickets: (raw.max_tickets > 0).then_some(raw.max_tickets),
        free_ticket: raw.free_ticket,
        subscriber_only: raw.subscriber_only,
        total_tickets: raw.total_amount,
        total_users: raw.total_users,
        bot_responses: raw.bot_responses,
    })
}

/// Commande de participation lue dans la description (même règles que le site Giveaway Twitch)
pub fn extract_command(description: &str) -> Option<String> {
    static BRACKET: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)\[\s*(![a-z0-9_]+)\s*\]").unwrap());
    static TYPED: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)(?:type|tapez?|tape|écris|ecris|write)\s+\[?\s*(![a-z0-9_]+)").unwrap()
    });
    static ANY: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)(?:^|\s)(![a-z0-9_]+)").unwrap());
    [&*BRACKET, &*TYPED, &*ANY]
        .iter()
        .find_map(|re| re.captures(description).map(|c| c[1].to_lowercase()))
}

/// Ce qu'une réponse du bot StreamElements dit de nos tickets
#[derive(Debug, PartialEq)]
pub enum TicketUpdate {
    /// « @pseudo got 3 tickets! (3/210) » : total actuel et maximum
    Total { current: i64, max: i64 },
    /// « @pseudo, you successfully bought 5 tickets » : achat à ajouter
    Bought(i64),
}

/// Réponse du bot concernant `my_login` (les messages d'un autre pseudo sont ignorés)
pub fn parse_ticket_message(text: &str, my_login: &str) -> Option<TicketUpdate> {
    static TOTAL: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)@?(\w+)\s+got\s+\d+\s+ticket\(?s?\)?.*?\((\d+)/(\d+)\)").unwrap()
    });
    static BOUGHT: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)@?(\w+),?\s+(?:you\s+)?successfully bought\s+(\d+)\s+ticket").unwrap()
    });
    if let Some(c) = TOTAL.captures(text) {
        if c[1].eq_ignore_ascii_case(my_login) {
            return Some(TicketUpdate::Total { current: c[2].parse().ok()?, max: c[3].parse().ok()? });
        }
        return None;
    }
    let c = BOUGHT.captures(text)?;
    if !c[1].eq_ignore_ascii_case(my_login) {
        return None;
    }
    let n: i64 = c[2].parse().ok()?;
    (n > 0).then_some(TicketUpdate::Bought(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_points() {
        let body = r#"{"channel":"600abc","username":"powange2","points":1153400,"pointsAlltime":4655400,"watchtime":76520,"rank":173}"#;
        assert_eq!(serde_json::from_str::<SePoints>(body).unwrap(), SePoints { points: 1153400, rank: Some(173) });
    }

    #[test]
    fn parse_loyalty() {
        let body = r#"{"loyalty":{"bonuses":{"raid":0,"follow":30,"tip":150,"subscriber":500,"cheer":0,"host":50},"name":"RiggedBucks","enabled":true,"amount":50,"subscriberMultiplier":3,"ignored":["streamelements"],"interval":10},"_id":"x","channel":"y"}"#;
        let l = to_loyalty(serde_json::from_str::<LoyaltyResponse>(body).unwrap().loyalty);
        assert_eq!((l.amount, l.interval, l.subscriber_multiplier), (50, 10, 3.0));
        assert_eq!(l.bonuses, vec![("follow".into(), 30), ("host".into(), 50), ("subscriber".into(), 500), ("tip".into(), 150)]);
    }

    #[test]
    fn parse_giveaway() {
        let body = r#"{"_id":"6ae6","channel":"600a","title":"GIVEAWAY","description":"🎁 Tente de gagner un Navire en tapant !ticket dans le tchat","cost":1000,"subscriberLuck":1,"subscriberOnly":false,"freeTicket":true,"totalAmount":227,"totalUsers":6,"maxTickets":210,"botResponses":true,"state":"running","winners":[]}"#;
        let g = to_giveaway(serde_json::from_str(body).unwrap()).unwrap();
        assert_eq!(g.command.as_deref(), Some("!ticket"));
        assert_eq!((g.cost, g.max_tickets, g.total_tickets, g.total_users), (1000, Some(210), 227, 6));
        assert!(g.free_ticket);
        let ended = r#"{"_id":"1","state":"ended","maxTickets":0}"#;
        assert_eq!(to_giveaway(serde_json::from_str(ended).unwrap()), None);
    }

    #[test]
    fn commands() {
        assert_eq!(extract_command("type [!win] in chat").as_deref(), Some("!win"));
        assert_eq!(extract_command("Tapez !Coffre pour participer").as_deref(), Some("!coffre"));
        assert_eq!(extract_command("Bonne chance !join"), Some("!join".into()));
        assert_eq!(extract_command("aucune commande ici"), None);
    }

    #[test]
    fn ticket_messages() {
        assert_eq!(
            parse_ticket_message("@powange2 got 3 tickets! (3/210)", "powange2"),
            Some(TicketUpdate::Total { current: 3, max: 210 })
        );
        assert_eq!(
            parse_ticket_message("PoWange2, you successfully bought 5 tickets", "powange2"),
            Some(TicketUpdate::Bought(5))
        );
        assert_eq!(parse_ticket_message("@bob got 1 ticket! (1/10)", "powange2"), None);
        assert_eq!(parse_ticket_message("Le giveaway est terminé", "powange2"), None);
    }
}
