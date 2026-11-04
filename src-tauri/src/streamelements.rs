//! StreamElements : points de fidélité et giveaways, lus sur l'API publique
//! (sans connexion), réponses du bot dans le chat pour nos tickets, et, avec
//! le jeton de session du site (compte StreamElements du spectateur), lecture
//! exacte et achat de nos tickets. Adresses relevées dans le code de
//! streamelements.com le 2026-11-01, non documentées officiellement.

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
    #[serde(skip_serializing)]
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

#[derive(Debug, thiserror::Error)]
pub enum SeError {
    #[error("réseau : {0}")]
    Http(#[from] reqwest::Error),
    #[error("jeton StreamElements refusé")]
    Unauthorized,
    #[error("{0}")]
    Api(String),
}

/// Compte StreamElements du jeton
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeMe {
    pub username: String,
    /// Identifiant Twitch du compte
    #[serde(default)]
    pub provider_id: String,
}

/// En-tête d'authentification : jeton de session (JWT) ou jeton d'accès personnel
fn authorization(token: &str) -> String {
    if token.split('.').count() == 3 {
        format!("Bearer {token}")
    } else {
        format!("apikey {token}")
    }
}

async fn api_error(res: reqwest::Response) -> SeError {
    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
        return SeError::Unauthorized;
    }
    let status = res.status();
    let body: serde_json::Value = res.json().await.unwrap_or_default();
    let message = body.get("message").and_then(|m| m.as_str()).map(str::to_owned);
    SeError::Api(message.unwrap_or_else(|| format!("HTTP {status}")))
}

pub async fn me(http: &reqwest::Client, token: &str) -> Result<SeMe, SeError> {
    let res = http
        .get(format!("{API}/kappa/v2/channels/me"))
        .header("Authorization", authorization(token))
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(api_error(res).await);
    }
    Ok(res.json().await?)
}

#[derive(Debug, Deserialize)]
struct Entry {
    #[serde(default)]
    tickets: i64,
}

/// Nos tickets dans un giveaway (0 : pas encore inscrit)
pub async fn my_entry(http: &reqwest::Client, token: &str, se_channel: &str, giveaway: &str) -> Result<i64, SeError> {
    let res = http
        .get(format!("{API}/kappa/v3/giveaways/{se_channel}/{giveaway}/entry"))
        .header("Authorization", authorization(token))
        .send()
        .await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(0);
    }
    if !res.status().is_success() {
        return Err(api_error(res).await);
    }
    Ok(res.json::<Entry>().await?.tickets)
}

/// Achète `tickets` tickets avec nos points (même appel que le bouton du site)
pub async fn enter(
    http: &reqwest::Client,
    token: &str,
    se_channel: &str,
    giveaway: &str,
    tickets: i64,
) -> Result<(), SeError> {
    let res = http
        .post(format!("{API}/kappa/v3/giveaways/{se_channel}/{giveaway}/entry"))
        .header("Authorization", authorization(token))
        .json(&serde_json::json!({ "tickets": tickets }))
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(api_error(res).await);
    }
    Ok(())
}

/// Giveaway StreamElements précis : en cours ou non, et pseudos tirés (dans l'ordre du tirage ;
/// un gagnant apparaît dès le tirage, avant que le streamer ne termine le giveaway)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayOutcome {
    /// Participations ouvertes (« running »)
    pub running: bool,
    /// État donné par StreamElements : « running », « closed » (participations fermées, tirage
    /// à venir), « completed » (tiré), « canceled »…
    pub state: String,
    pub winners: Vec<String>,
    /// Heure du tirage de chaque gagnant (même ordre que `winners` ; None : inconnue)
    #[serde(skip)]
    pub drawn_at: Vec<Option<String>>,
    /// Titre et description (ils annoncent souvent les lots)
    pub title: String,
    pub description: String,
    /// Tickets achetés et participants
    pub total_tickets: i64,
    pub total_users: i64,
    /// Tickets maximum par personne (None : pas de limite)
    pub max_tickets: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutcomeRaw {
    #[serde(default)]
    state: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    total_amount: i64,
    #[serde(default)]
    total_users: i64,
    #[serde(default)]
    max_tickets: i64,
    #[serde(default)]
    winners: Vec<WinnerRaw>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WinnerRaw {
    #[serde(default)]
    username: String,
    /// Heure du tirage de ce gagnant
    #[serde(default)]
    created_at: Option<String>,
}

impl GiveawayOutcome {
    /// Terminé sans tirage à venir : tiré (completed) ou annulé ; « closed » attend son tirage
    pub fn finished(&self) -> bool {
        matches!(self.state.as_str(), "completed" | "ended" | "canceled" | "cancelled")
    }
}

fn to_outcome(raw: OutcomeRaw) -> GiveawayOutcome {
    let (winners, drawn_at): (Vec<String>, Vec<Option<String>>) = raw
        .winners
        .into_iter()
        .map(|w| (w.username.trim().to_lowercase(), w.created_at))
        .filter(|(w, _)| !w.is_empty())
        .unzip();
    GiveawayOutcome {
        running: raw.state == "running",
        state: raw.state.clone(),
        winners,
        drawn_at,
        title: raw.title.trim().to_owned(),
        description: raw.description.trim().to_owned(),
        total_tickets: raw.total_amount,
        total_users: raw.total_users,
        max_tickets: (raw.max_tickets > 0).then_some(raw.max_tickets),
    }
}

/// État d'un giveaway par son identifiant (None : inconnu de StreamElements), sans compte
pub async fn giveaway_outcome(http: &reqwest::Client, se_channel: &str, id: &str) -> Result<Option<GiveawayOutcome>, reqwest::Error> {
    let res = http.get(format!("{API}/kappa/v3/giveaways/{se_channel}/{id}")).send().await?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(to_outcome(res.error_for_status()?.json::<OutcomeRaw>().await?)))
}

/// Chaîne et identifiant d'un lien de giveaway (https://streamelements.com/<chaîne>/giveaway/<id>)
pub fn parse_giveaway_url(url: &str) -> Option<(String, String)> {
    const PREFIX: &str = "https://streamelements.com/";
    let url = url.trim().trim_end_matches('/');
    let rest = url.get(..PREFIX.len()).filter(|head| head.eq_ignore_ascii_case(PREFIX)).map(|_| &url[PREFIX.len()..])?;
    let mut parts = rest.split('/');
    let (channel, kind, id) = (parts.next()?, parts.next()?, parts.next()?);
    let ok = parts.next().is_none()
        && kind == "giveaway"
        && !channel.is_empty()
        && channel.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !id.is_empty()
        && id.chars().all(|c| c.is_ascii_hexdigit());
    ok.then(|| (channel.to_lowercase(), id.to_owned()))
}

/// Pseudos StreamElements des streamers dont la page n'a pas leur pseudo Twitch (fiches des
/// streamers) : pseudo Twitch → pseudo StreamElements
static PAGE_NAMES: LazyLock<std::sync::RwLock<std::collections::HashMap<String, String>>> = LazyLock::new(Default::default);

pub fn set_page_names(names: &[(String, String)]) {
    *PAGE_NAMES.write().unwrap() = names.iter().map(|(l, n)| (l.to_lowercase(), n.to_lowercase())).collect();
}

/// Pseudo de la page StreamElements d'une chaîne Twitch (le même, sauf fiche du streamer)
pub fn page_name(login: &str) -> String {
    let login = login.to_lowercase();
    PAGE_NAMES.read().unwrap().get(&login).cloned().unwrap_or(login)
}

/// Identifiant StreamElements d'une chaîne Twitch (None : la chaîne n'utilise pas StreamElements)
pub async fn channel_id(http: &reqwest::Client, login: &str) -> Result<Option<String>, reqwest::Error> {
    channel_id_of_page(http, &page_name(login)).await
}

/// Identifiant StreamElements d'une page (streamelements.com/<page>), sans passer par la fiche
pub async fn channel_id_of_page(http: &reqwest::Client, name: &str) -> Result<Option<String>, reqwest::Error> {
    let res = http.get(format!("{API}/kappa/v2/channels/{name}")).send().await?;
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
    #[test]
    fn page_names_follow_streamer_sheets() {
        super::set_page_names(&[("TheOriginalPirateMaterial".into(), "0riginalPirateMaterial".into())]);
        assert_eq!(super::page_name("theoriginalpiratematerial"), "0riginalpiratematerial");
        assert_eq!(super::page_name("Saenz203"), "saenz203");
        super::set_page_names(&[]);
    }

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
        assert_eq!(
            parse_giveaway_url("https://streamelements.com/Saenz203/giveaway/6ae91894a0a183147ae56a71/"),
            Some(("saenz203".into(), "6ae91894a0a183147ae56a71".into()))
        );
        assert_eq!(
            parse_giveaway_url("https://StreamElements.com/saenz203/giveaway/6ae91894a0a183147ae56a71").map(|(c, _)| c).as_deref(),
            Some("saenz203")
        );
        assert_eq!(parse_giveaway_url("https://streamelements.com/saenz203/store"), None);
        assert_eq!(parse_giveaway_url("https://evil.com/saenz203/giveaway/6ae9"), None);
        let drawn: OutcomeRaw =
            serde_json::from_str(r#"{"_id":"1","state":"running","title":"Gilded Phoenix Ship Crest ","winners":[{"username":"DreKing_","tickets":189}]}"#).unwrap();
        let drawn = to_outcome(drawn);
        assert!(drawn.running && drawn.winners == ["dreking_"] && drawn.title == "Gilded Phoenix Ship Crest" && drawn.description.is_empty());
        assert_eq!(drawn.max_tickets, None);
        assert_eq!(drawn.drawn_at, [None]);
        let timed: OutcomeRaw = serde_json::from_str(r#"{"state":"completed","winners":[{"username":"dreking_","createdAt":"2026-11-02T01:07:27.772Z"}]}"#).unwrap();
        assert_eq!(to_outcome(timed).drawn_at, [Some("2026-11-02T01:07:27.772Z".to_owned())]);
        let limited: OutcomeRaw = serde_json::from_str(r#"{"_id":"1","state":"running","maxTickets":20}"#).unwrap();
        assert_eq!(to_outcome(limited).max_tickets, Some(20));
        let done: OutcomeRaw = serde_json::from_str(r#"{"_id":"1","state":"completed"}"#).unwrap();
        let done = to_outcome(done);
        assert!(!done.running && done.finished() && done.winners.is_empty());
        let waiting: OutcomeRaw = serde_json::from_str(r#"{"_id":"1","state":"closed"}"#).unwrap();
        let waiting = to_outcome(waiting);
        assert!(!waiting.running && !waiting.finished());
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
    fn auth_header() {
        assert_eq!(authorization("aaa.bbb.ccc"), "Bearer aaa.bbb.ccc");
        assert_eq!(authorization("0123abcd"), "apikey 0123abcd");
    }

    #[test]
    fn me_parsing() {
        let body = r#"{"_id":"692f","username":"powange2","providerId":"181074025","provider":"twitch"}"#;
        let me: SeMe = serde_json::from_str(body).unwrap();
        assert_eq!((me.username.as_str(), me.provider_id.as_str()), ("powange2", "181074025"));
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
