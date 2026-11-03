//! Drops Twitch : campagnes, progression et réclamation.
//!
//! Sources (requêtes GQL internes, mêmes que TwitchDropsMiner, relevées le
//! 2026-11-01) : Inventory (campagnes commencées, drops déjà obtenus),
//! ViewerDropsDashboard (toutes les campagnes en cours ou à venir),
//! DropCampaignDetails (drops, chaînes autorisées, compte de jeu lié).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use crate::app::{App, EventKind};
use crate::db::{Account, DropGame};
use crate::twitch::gql::{Gql, StreamInfo};
use crate::twitch::watch::{ChannelWatch, WatchContext};
use crate::twitch::{Result, TwitchError};

const INVENTORY_EVERY: Duration = Duration::from_secs(10 * 60);
// Après le début d'un visionnage sans campagne connue : relire l'inventaire tôt
const INVENTORY_AFTER_START: Duration = Duration::from_secs(3 * 60);
const STREAMS_EVERY: Duration = Duration::from_secs(5 * 60);
const STREAM_INFO_EVERY: Duration = Duration::from_secs(2 * 60);
const FAILED_CHANNEL_COOLDOWN: Duration = Duration::from_secs(10 * 60);
const FAILURES_BEFORE_SWITCH: u32 = 5;
const CLAIM_RETRY: Duration = Duration::from_secs(60 * 60);
// Visionnage sans campagne visible dans l'inventaire : campagne déjà terminée ou
// non ouverte à ce compte ; le jeu est mis de côté un moment
const NO_PROGRESS_AFTER: Duration = Duration::from_secs(20 * 60);
const NO_PROGRESS_PAUSE: Duration = Duration::from_secs(60 * 60);
// Sans campagne connue dans l'inventaire, le tag « drops activés » ne suffit pas (un
// streamer peut le garder après la fin d'une campagne) : Twitch doit confirmer une
// campagne ouverte sur la chaîne (AvailableDrops). Vérification gardée 10 min.
const VERIFY_EVERY: Duration = Duration::from_secs(10 * 60);
/// Campagne vue sur une chaîne mais absente de l'inventaire (terminée, ou pas commencée) : gardée
/// ce temps, puis relue
const CHANNEL_CAMPAIGN_KEEP: Duration = Duration::from_secs(6 * 3600);
// Détail d'une campagne (drops, chaînes imposées) : il change peu, gardé 1 h
const DETAILS_TTL: Duration = Duration::from_secs(3600);
// Progression d'un drop inconnu : relecture forcée de l'inventaire au plus toutes les 10 min
const FORCED_INVENTORY_GAP: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub id: String,
    pub name: String,
    pub slug: Option<String>,
    pub box_art: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reward {
    pub name: String,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimedDrop {
    pub id: String,
    pub name: String,
    pub required_minutes: i64,
    pub current_minutes: i64,
    pub claimed: bool,
    /// Identifiant à envoyer pour réclamer le drop (présent une fois commencé)
    #[serde(skip_serializing)]
    pub instance_id: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub rewards: Vec<Reward>,
    /// Drops à obtenir avant celui-ci
    #[serde(skip_serializing)]
    pub preconditions: Vec<String>,
}

impl TimedDrop {
    pub fn claimable(&self) -> bool {
        !self.claimed && self.instance_id.is_some() && self.current_minutes >= self.required_minutes
    }

    pub fn remaining_minutes(&self) -> i64 {
        if self.claimed {
            0
        } else {
            (self.required_minutes - self.current_minutes).max(0)
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Campaign {
    pub id: String,
    pub name: String,
    pub game: Game,
    /// ACTIVE, UPCOMING ou EXPIRED
    pub status: String,
    pub starts_at: String,
    pub ends_at: String,
    /// Compte du jeu lié au compte Twitch (sinon les drops ne sont pas attribués)
    pub account_linked: bool,
    pub link_url: Option<String>,
    /// Chaînes imposées par la campagne (vide : toute chaîne du jeu avec drops activés)
    pub allowed_channels: Vec<String>,
    pub drops: Vec<TimedDrop>,
}

impl Campaign {
    pub fn active(&self) -> bool {
        self.status == "ACTIVE"
    }

    /// Reste-t-il des minutes à regarder ?
    pub fn unfinished(&self) -> bool {
        self.drops.iter().any(|d| !d.claimed && d.remaining_minutes() > 0)
    }
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn parse_game(v: &Value) -> Option<Game> {
    let id = str_of(v, "id")?;
    let name = str_of(v, "displayName").or_else(|| str_of(v, "name"))?;
    Some(Game {
        id,
        name,
        slug: str_of(v, "slug"),
        box_art: str_of(v, "boxArtURL"),
    })
}

fn parse_drop(v: &Value, claimed_benefits: &HashSet<String>) -> Option<TimedDrop> {
    let me = v.get("self").filter(|s| !s.is_null());
    let rewards: Vec<Reward> = v
        .get("benefitEdges")
        .and_then(Value::as_array)
        .map(|edges| {
            edges
                .iter()
                .filter_map(|e| e.get("benefit"))
                .filter_map(|b| Some(Reward { name: str_of(b, "name")?, image: str_of(b, "imageAssetURL") }))
                .collect()
        })
        .unwrap_or_default();
    // Drop déjà obtenu lors d'une session précédente : ses récompenses sont dans l'inventaire
    let benefit_ids: Vec<String> = v
        .get("benefitEdges")
        .and_then(Value::as_array)
        .map(|edges| edges.iter().filter_map(|e| e.get("benefit").and_then(|b| str_of(b, "id"))).collect())
        .unwrap_or_default();
    let already = !benefit_ids.is_empty() && benefit_ids.iter().all(|id| claimed_benefits.contains(id));
    Some(TimedDrop {
        id: str_of(v, "id")?,
        name: str_of(v, "name").unwrap_or_default(),
        required_minutes: v.get("requiredMinutesWatched").and_then(Value::as_i64).unwrap_or(0),
        current_minutes: me.and_then(|s| s.get("currentMinutesWatched")).and_then(Value::as_i64).unwrap_or(0),
        claimed: me.and_then(|s| s.get("isClaimed")).and_then(Value::as_bool).unwrap_or(false) || (me.is_none() && already),
        instance_id: me.and_then(|s| str_of(s, "dropInstanceID")),
        starts_at: str_of(v, "startAt").unwrap_or_default(),
        ends_at: str_of(v, "endAt").unwrap_or_default(),
        rewards,
        preconditions: v
            .get("preconditionDrops")
            .and_then(Value::as_array)
            .map(|p| p.iter().filter_map(|d| str_of(d, "id")).collect())
            .unwrap_or_default(),
    })
}

/// Campagne complète (inventaire et/ou détail fusionnés)
pub fn parse_campaign(v: &Value, claimed_benefits: &HashSet<String>) -> Option<Campaign> {
    let game = parse_game(v.get("game")?)?;
    let allow = v.get("allow").filter(|a| !a.is_null());
    let allow_enabled = allow.and_then(|a| a.get("isEnabled")).and_then(Value::as_bool).unwrap_or(true);
    let allowed_channels = match (allow, allow_enabled) {
        (Some(a), true) => a
            .get("channels")
            .and_then(Value::as_array)
            .map(|c| c.iter().filter_map(|ch| str_of(ch, "name")).map(|s| s.to_lowercase()).collect())
            .unwrap_or_default(),
        _ => vec![],
    };
    Some(Campaign {
        id: str_of(v, "id")?,
        name: str_of(v, "name").unwrap_or_default(),
        game,
        status: str_of(v, "status").unwrap_or_else(|| "ACTIVE".into()),
        starts_at: str_of(v, "startAt").unwrap_or_default(),
        ends_at: str_of(v, "endAt").unwrap_or_default(),
        account_linked: v.pointer("/self/isAccountConnected").and_then(Value::as_bool).unwrap_or(false),
        link_url: str_of(v, "accountLinkURL"),
        allowed_channels,
        drops: v
            .get("timeBasedDrops")
            .and_then(Value::as_array)
            .map(|d| d.iter().filter_map(|x| parse_drop(x, claimed_benefits)).collect())
            .unwrap_or_default(),
    })
}

/// Fusion récursive : les valeurs de `primary` l'emportent (progression de l'inventaire)
pub fn merge(primary: &Value, secondary: &Value) -> Value {
    match (primary, secondary) {
        (Value::Object(p), Value::Object(s)) => {
            let mut out = s.clone();
            for (k, v) in p {
                let merged = match s.get(k) {
                    Some(sv) => merge(v, sv),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        (Value::Array(p), Value::Array(s)) => {
            // Listes d'objets avec un id (drops, chaînes) : fusion par id
            let ids = |a: &Vec<Value>| a.iter().all(|x| x.get("id").is_some());
            if ids(p) && ids(s) {
                let mut out: Vec<Value> = s.clone();
                for item in p {
                    match out.iter_mut().find(|o| o.get("id") == item.get("id")) {
                        Some(o) => *o = merge(item, o),
                        None => out.push(item.clone()),
                    }
                }
                Value::Array(out)
            } else {
                primary.clone()
            }
        }
        (Value::Null, other) => other.clone(),
        _ => primary.clone(),
    }
}

/// Ce que Twitch renvoie pour un compte : campagnes et récompenses déjà obtenues
pub struct DropsSnapshot {
    pub campaigns: Vec<Campaign>,
    /// Données brutes par campagne (inventaire + tableau), pour y fusionner les détails lus ensuite
    raw: HashMap<String, Value>,
    claimed: HashSet<String>,
    /// Campagnes dont le détail reste à lire, hors du passage
    background: Vec<String>,
}

/// Stream en live d'un jeu, drops activés (annuaire du jeu)
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DropStream {
    pub login: String,
    pub channel_id: String,
    pub viewers: i64,
    pub title: String,
}

/// Résultat d'une réclamation
#[derive(Debug, PartialEq)]
pub enum ClaimOutcome {
    Claimed,
    AlreadyClaimed,
    Refused(String),
}

impl Gql {
    /// Jeu Twitch d'après son nom exact : (identifiant, slug)
    pub async fn game_by_name(&self, name: &str) -> Result<Option<(String, String)>> {
        let data = self.request("DirectoryGameRedirect", json!({ "name": name })).await?;
        let game = data.get("game").filter(|g| !g.is_null());
        Ok(game.and_then(|g| Some((str_of(g, "id")?, str_of(g, "slug")?))))
    }

    /// Jeu d'après son slug (adresse de la catégorie)
    pub async fn game_by_slug(&self, slug: &str) -> Result<Option<Game>> {
        let data = self.request("DirectoryPage_Game", directory_vars(slug)).await?;
        Ok(data.get("game").filter(|g| !g.is_null()).and_then(parse_game))
    }

    /// Streams en live du jeu avec les drops activés, les plus regardés d'abord
    /// Campagnes de drops ouvertes sur une chaîne : (identifiant de campagne, identifiant du jeu)
    /// None : réponse vide, Twitch ne donne pas l'information à ce jeton
    pub async fn available_drops(&self, channel_id: &str) -> Result<Option<Vec<(String, String)>>> {
        let data = self.request("DropsHighlightService_AvailableDrops", json!({ "channelID": channel_id })).await?;
        Ok(parse_available_drops(&data))
    }

    /// Streams avec drops, et jaquette du jeu au passage
    pub async fn drop_streams(&self, slug: &str) -> Result<(Vec<DropStream>, Option<String>)> {
        let data = self.request("DirectoryPage_Game", directory_vars(slug)).await?;
        let box_art = data.get("game").and_then(|g| str_of(g, "boxArtURL"));
        Ok((parse_drop_streams(&data), box_art))
    }

    pub async fn claim_drop(&self, instance_id: &str) -> Result<ClaimOutcome> {
        let data = self
            .request("DropsPage_ClaimDropRewards", json!({ "input": { "dropInstanceID": instance_id } }))
            .await?;
        let status = data.pointer("/claimDropRewards/status").and_then(Value::as_str).unwrap_or("");
        Ok(match status {
            "ELIGIBLE_FOR_ALL" => ClaimOutcome::Claimed,
            "DROP_INSTANCE_ALREADY_CLAIMED" => ClaimOutcome::AlreadyClaimed,
            "" => ClaimOutcome::Refused("réponse vide".into()),
            other => ClaimOutcome::Refused(other.into()),
        })
    }

    pub async fn drops_inventory(&self) -> Result<Value> {
        self.request("Inventory", json!({ "fetchRewardCampaigns": false })).await
    }

    pub async fn drops_dashboard(&self) -> Result<Value> {
        self.request("ViewerDropsDashboard", json!({ "fetchRewardCampaigns": false })).await
    }

    /// Détail de plusieurs campagnes, en requêtes groupées ; un résultat par campagne, dans l'ordre
    pub async fn drop_campaign_details(&self, user_id: &str, campaign_ids: &[String]) -> Vec<Result<Value>> {
        let vars = campaign_ids.iter().map(|id| json!({ "channelLogin": user_id, "dropID": id })).collect();
        self.request_batch("DropCampaignDetails", vars).await
    }
}

/// Détails de campagnes déjà lus, par identifiant
pub type DetailsCache = HashMap<String, (Value, Instant)>;

/// Détails lus en tâche de fond : (identifiant de campagne, détail si Twitch l'a donné)
type DetailsBatch = Vec<(String, Option<Value>)>;

/// Campagnes à détailler : commencées (inventaire) ou d'un jeu suivi pour ses drops
fn needs_details(id: &str, game_id: Option<&str>, in_inventory: &HashSet<String>, wanted_games: &HashSet<String>) -> bool {
    in_inventory.contains(id) || game_id.is_some_and(|g| wanted_games.contains(g))
}

/// Campagnes sans détail en cache (ou périmé), en deux listes triées : celles qui comptent pour
/// le minage (`needs_details`), lues tout de suite, et les autres (des centaines), lues en tâche
/// de fond pour la page Drops
fn details_to_fetch(
    campaigns: &[(String, Option<String>)],
    cache: &DetailsCache,
    in_inventory: &HashSet<String>,
    wanted_games: &HashSet<String>,
    now: Instant,
) -> (Vec<String>, Vec<String>) {
    let (mut urgent, mut later): (Vec<String>, Vec<String>) = (vec![], vec![]);
    let fresh = |id: &String| cache.get(id).is_some_and(|(_, at)| now.duration_since(*at) < DETAILS_TTL);
    for (id, game_id) in campaigns.iter().filter(|(id, _)| !fresh(id)) {
        if needs_details(id, game_id.as_deref(), in_inventory, wanted_games) {
            urgent.push(id.clone());
        } else {
            later.push(id.clone());
        }
    }
    urgent.sort();
    later.sort();
    (urgent, later)
}

/// Détail utile d'une réponse DropCampaignDetails
fn campaign_details(data: &Value) -> Option<Value> {
    data.pointer("/user/dropCampaign").filter(|c| !c.is_null()).cloned()
}

/// Campagnes du compte : commencées (inventaire) + en cours ou à venir (tableau). Les détails
/// absents du cache des campagnes qui comptent sont lus ici (requêtes groupées) ; les autres
/// sont renvoyés dans `background`, à lire hors du passage
pub async fn fetch_campaigns(
    gql: &Gql,
    user_id: &str,
    wanted_games: &HashSet<String>,
    cache: &mut DetailsCache,
) -> Result<DropsSnapshot> {
    let inventory = gql.drops_inventory().await?;
    let inv = inventory.pointer("/currentUser/inventory").cloned().unwrap_or(Value::Null);
    let claimed: HashSet<String> = inv
        .get("gameEventDrops")
        .and_then(Value::as_array)
        .map(|d| d.iter().filter_map(|x| str_of(x, "id")).collect())
        .unwrap_or_default();
    let mut raw: HashMap<String, Value> = inv
        .get("dropCampaignsInProgress")
        .and_then(Value::as_array)
        .map(|c| c.iter().filter_map(|x| Some((str_of(x, "id")?, x.clone()))).collect())
        .unwrap_or_default();
    let in_inventory: HashSet<String> = raw.keys().cloned().collect();

    let dashboard = gql.drops_dashboard().await;
    let listed: Vec<Value> = match &dashboard {
        Ok(d) => d.pointer("/currentUser/dropCampaigns").and_then(Value::as_array).cloned().unwrap_or_default(),
        Err(e) => {
            tracing::warn!("drops : tableau des campagnes indisponible ({e})");
            vec![]
        }
    };
    for c in listed {
        let status = str_of(&c, "status").unwrap_or_default();
        if status != "ACTIVE" && status != "UPCOMING" {
            continue;
        }
        if let Some(id) = str_of(&c, "id") {
            let entry = raw.remove(&id).map(|inv| merge(&inv, &c)).unwrap_or(c);
            raw.insert(id, entry);
        }
    }

    let now = Instant::now();
    // Détail périmé gardé pour l'affichage jusqu'à sa relecture (pas de liste de drops qui
    // disparaît le temps de la tâche de fond)
    cache.retain(|id, _| raw.contains_key(id));
    let listed: Vec<(String, Option<String>)> =
        raw.iter().map(|(id, v)| (id.clone(), v.get("game").and_then(|g| str_of(g, "id")))).collect();
    let (urgent, background) = details_to_fetch(&listed, cache, &in_inventory, wanted_games, now);
    if !urgent.is_empty() {
        for (id, res) in urgent.iter().zip(gql.drop_campaign_details(user_id, &urgent).await) {
            match res {
                Ok(d) => {
                    if let Some(details) = campaign_details(&d) {
                        cache.insert(id.clone(), (details, now));
                    }
                }
                Err(TwitchError::Unauthorized) => return Err(TwitchError::Unauthorized),
                Err(e) => tracing::warn!("drops : détail de la campagne {id} indisponible ({e})"),
            }
        }
    }

    let mut campaigns: Vec<Campaign> = raw
        .iter()
        .filter_map(|(id, value)| match cache.get(id) {
            Some((details, _)) => parse_campaign(&merge(value, details), &claimed),
            None => parse_campaign(value, &claimed),
        })
        .collect();
    campaigns.sort_by(|a, b| a.ends_at.cmp(&b.ends_at));
    Ok(DropsSnapshot { campaigns, raw, claimed, background })
}

/// Lit en tâche de fond le détail des campagnes `ids` (requêtes groupées), sans bloquer le passage
fn spawn_details(gql: &Gql, user_id: &str, ids: Vec<String>) -> tokio::task::JoinHandle<DetailsBatch> {
    let (gql, user_id) = (gql.clone(), user_id.to_owned());
    tokio::spawn(async move {
        let results = gql.drop_campaign_details(&user_id, &ids).await;
        let mut failed = 0;
        let mut batch = DetailsBatch::new();
        for (id, res) in ids.into_iter().zip(results) {
            let details = match res {
                Ok(d) => campaign_details(&d),
                // Jeton refusé : les requêtes du passage le verront et reconnecteront le compte
                Err(TwitchError::Unauthorized) => None,
                Err(_) => {
                    failed += 1;
                    None
                }
            };
            batch.push((id, details));
        }
        if failed > 0 {
            tracing::warn!("drops : {failed} détail(s) de campagne indisponible(s)");
        }
        batch
    })
}

/// Garde la progression connue en direct (PubSub, réclamation) quand une campagne est relue
fn keep_live_progress(fresh: &mut Campaign, old: &Campaign) {
    for d in &mut fresh.drops {
        if let Some(o) = old.drops.iter().find(|o| o.id == d.id) {
            d.current_minutes = d.current_minutes.max(o.current_minutes);
            d.claimed |= o.claimed;
        }
    }
}

fn directory_vars(slug: &str) -> Value {
    json!({
        "limit": 30, "slug": slug, "imageWidth": 50, "includeCostreaming": false,
        "options": {
            "broadcasterLanguages": [], "freeformTags": null, "includeRestricted": ["SUB_ONLY_LIVE"],
            "recommendationsContext": { "platform": "web" }, "sort": "VIEWER_COUNT",
            "systemFilters": ["DROPS_ENABLED"], "tags": [], "requestID": "JIRA-VXP-2397"
        },
        "sortTypeIsRecency": false
    })
}

/// Saisie de l'utilisateur : slug si c'est l'adresse d'une catégorie, sinon nom du jeu
pub fn category_slug(input: &str) -> Option<String> {
    let s = input.trim();
    let idx = s.find("/directory/category/").map(|i| i + "/directory/category/".len())
        .or_else(|| s.find("/directory/game/").map(|i| i + "/directory/game/".len()))?;
    let slug = s[idx..].split(['/', '?', '#']).next()?.trim().to_lowercase();
    (!slug.is_empty()).then_some(slug)
}

pub fn parse_available_drops(data: &Value) -> Option<Vec<(String, String)>> {
    let list = data.pointer("/channel/viewerDropCampaigns")?;
    if list.is_null() {
        // Chaîne connue mais aucune campagne : liste vide, pas une absence d'information
        return data.get("channel").filter(|c| !c.is_null()).map(|_| vec![]);
    }
    Some(
        list.as_array()?
            .iter()
            .filter_map(|c| Some((str_of(c, "id")?, c.get("game").and_then(|g| str_of(g, "id"))?)))
            .collect(),
    )
}

pub fn parse_drop_streams(data: &Value) -> Vec<DropStream> {
    data.pointer("/game/streams/edges")
        .and_then(Value::as_array)
        .map(|edges| {
            edges
                .iter()
                .filter_map(|e| e.get("node"))
                .filter_map(|n| {
                    let b = n.get("broadcaster")?;
                    Some(DropStream {
                        login: str_of(b, "login")?.to_lowercase(),
                        channel_id: str_of(b, "id")?,
                        viewers: n.get("viewersCount").and_then(Value::as_i64).unwrap_or(0),
                        title: str_of(n, "title").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// --- Moteur : un par compte, appelé par le superviseur à chaque passage ---

/// Où en est un jeu souhaité
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum GameStatus {
    /// Pas encore vérifié
    Unknown,
    /// Un stream de ce jeu est regardé pour ses drops
    Watching,
    /// Des streams avec drops existent, mais un jeu plus haut dans la liste passe avant
    Waiting,
    /// Aucun stream en live avec les drops activés : pas de campagne en ce moment
    NoStreams,
    /// Tous les drops de la campagne sont obtenus
    Done,
    /// Regardé sans qu'aucune campagne n'apparaisse : déjà terminée ou pas ouverte à ce compte
    NoProgress,
    Disabled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropGameView {
    pub id: i64,
    pub game_id: String,
    pub name: String,
    pub slug: String,
    pub box_art: Option<String>,
    pub enabled: bool,
    pub status: GameStatus,
    pub streams: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropTargetView {
    pub game_name: String,
    pub login: String,
    pub viewers: i64,
    pub title: String,
    /// Campagne pour laquelle la chaîne est regardée (None : pas encore connue)
    pub campaign_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropsView {
    pub enabled: bool,
    pub games: Vec<DropGameView>,
    pub campaigns: Vec<Campaign>,
    pub target: Option<DropTargetView>,
    pub inventory_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Default)]
struct GameRt {
    streams: Vec<DropStream>,
    streams_at: Option<Instant>,
    status: Option<GameStatus>,
    paused_until: Option<Instant>,
    /// Campagne confirmée par Twitch sur une chaîne du jeu (None : pas encore vérifié)
    verified: Option<(bool, Instant)>,
}

struct Target {
    game_id: String,
    game_name: String,
    stream: DropStream,
    live: Option<StreamInfo>,
    live_at: Option<Instant>,
    watch: ChannelWatch,
    failures: u32,
    started: Instant,
}

#[derive(Default)]
pub struct DropsEngine {
    games: Vec<DropGame>,
    rt: HashMap<String, GameRt>,
    campaigns: Vec<Campaign>,
    inventory_at: Option<Instant>,
    inventory_wall: Option<String>,
    force_inventory_at: Option<Instant>,
    /// Dernière relecture forcée par la progression d'un drop inconnu
    unknown_progress_at: Option<Instant>,
    details: DetailsCache,
    /// Dernières données brutes des campagnes, pour y fusionner les détails lus en tâche de fond
    raw: HashMap<String, Value>,
    claimed: HashSet<String>,
    /// Lecture en cours du détail des campagnes qui ne servent qu'à la page Drops
    details_task: Option<tokio::task::JoinHandle<DetailsBatch>>,
    target: Option<Target>,
    failed: HashMap<String, Instant>,
    claim_attempts: HashMap<String, Instant>,
    error: Option<String>,
    /// Campagnes vues sur une chaîne et absentes de l'inventaire (une campagne terminée en
    /// disparaît) : détail brut et date de lecture, gardés d'une relecture de l'inventaire à
    /// l'autre (sinon une campagne déjà obtenue serait de nouveau inconnue, donc regardée)
    channel_campaigns: HashMap<String, (Value, Instant)>,
}

impl DropsEngine {
    pub fn set_games(&mut self, games: Vec<DropGame>) {
        // Jeu ajouté : ses campagnes n'ont pas encore été détaillées, relire l'inventaire
        let before: HashSet<String> = self.wanted_games();
        self.games = games;
        if !self.wanted_games().is_subset(&before) {
            self.inventory_at = None;
        }
        let ids: HashSet<&str> = self.games.iter().map(|g| g.game_id.as_str()).collect();
        self.rt.retain(|id, _| ids.contains(id.as_str()));
        if let Some(t) = &self.target {
            if !self.games.iter().any(|g| g.game_id == t.game_id && g.enabled) {
                self.target = None;
            }
        }
    }

    fn wanted_games(&self) -> HashSet<String> {
        self.games.iter().filter(|g| g.enabled).map(|g| g.game_id.clone()).collect()
    }

    /// Chaîne regardée pour les drops (elle occupe une des 2 places du compte)
    pub fn watching_login(&self) -> Option<&str> {
        self.target.as_ref().map(|t| t.stream.login.as_str())
    }

    /// Relire l'inventaire au prochain passage
    pub fn refresh_inventory(&mut self) {
        self.inventory_at = None;
        self.claim_attempts.clear();
        // Streams avec drops et vérification des campagnes relus aussi (bouton « Actualiser »)
        for r in self.rt.values_mut() {
            r.streams_at = None;
            r.verified = None;
        }
    }

    /// Progression annoncée en direct (PubSub) ; drop inconnu : nouvelle campagne, relire l'inventaire
    pub fn on_progress(&mut self, drop_id: &str, current: i64) {
        let mut known = false;
        for c in &mut self.campaigns {
            for d in &mut c.drops {
                if d.id == drop_id {
                    known = true;
                    d.current_minutes = d.current_minutes.max(current);
                }
            }
        }
        if !known && self.unknown_progress_at.is_none_or(|t| t.elapsed() >= FORCED_INVENTORY_GAP) {
            self.unknown_progress_at = Some(Instant::now());
            self.inventory_at = None;
        }
    }

    /// Range les détails lus en tâche de fond : cache, puis campagne affichée mise à jour
    fn apply_details(&mut self, batch: DetailsBatch) {
        let now = Instant::now();
        let mut added = false;
        for (id, details) in batch {
            let (Some(details), Some(value)) = (details, self.raw.get(&id)) else { continue };
            let Some(mut fresh) = parse_campaign(&merge(value, &details), &self.claimed) else { continue };
            self.details.insert(id.clone(), (details, now));
            match self.campaigns.iter_mut().find(|c| c.id == id) {
                Some(old) => {
                    keep_live_progress(&mut fresh, old);
                    *old = fresh;
                }
                None => {
                    self.campaigns.push(fresh);
                    added = true;
                }
            }
        }
        if added {
            self.campaigns.sort_by(|a, b| a.ends_at.cmp(&b.ends_at));
        }
    }

    /// Détails de fond arrivés depuis le dernier passage
    async fn collect_details(&mut self) {
        if !self.details_task.as_ref().is_some_and(|t| t.is_finished()) {
            return;
        }
        let Some(task) = self.details_task.take() else { return };
        match task.await {
            Ok(batch) => self.apply_details(batch),
            Err(e) => tracing::warn!("drops : lecture des détails de campagnes interrompue ({e})"),
        }
    }

    /// Campagnes proposées par une chaîne, lues en détail et ajoutées aux campagnes connues (la
    /// page Drops les affiche, et une campagne déjà obtenue n'est plus regardée)
    async fn campaigns_on_channel(&mut self, gql: &Gql, user_id: &str, ids: &[String]) -> Vec<Campaign> {
        let mut found = vec![];
        for (id, res) in ids.iter().zip(gql.drop_campaign_details(user_id, ids).await) {
            let raw = match res {
                Ok(d) => d.pointer("/user/dropCampaign").filter(|c| !c.is_null()).cloned(),
                Err(e) => {
                    tracing::warn!("drops : détail de la campagne {id} indisponible ({e})");
                    None
                }
            };
            let Some(raw) = raw else { continue };
            if let Some(c) = parse_campaign(&raw, &self.claimed) {
                self.channel_campaigns.insert(c.id.clone(), (raw, Instant::now()));
                if !self.campaigns.iter().any(|x| x.id == c.id) {
                    self.campaigns.push(c.clone());
                }
                found.push(c);
            }
        }
        found
    }

    /// Campagnes vues sur les chaînes et absentes de l'inventaire relu : ajoutées de nouveau (avec
    /// les récompenses déjà obtenues à jour) tant qu'elles sont récentes
    fn keep_channel_campaigns(&mut self) {
        self.channel_campaigns.retain(|_, (_, at)| at.elapsed() < CHANNEL_CAMPAIGN_KEEP);
        for (id, (raw, _)) in &self.channel_campaigns {
            if self.campaigns.iter().any(|c| c.id == *id) {
                continue;
            }
            if let Some(c) = parse_campaign(raw, &self.claimed) {
                self.campaigns.push(c);
            }
        }
        self.campaigns.sort_by(|a, b| a.ends_at.cmp(&b.ends_at));
    }

    fn campaign_for(&self, game_id: &str) -> Option<&Campaign> {
        self.campaigns.iter().find(|c| c.game.id == game_id && c.active())
    }

    pub async fn step(&mut self, app: &App, account: &Account, gql: &Gql, active: bool) -> Result<()> {
        let now = Instant::now();
        self.collect_details().await;
        if !active || !account.drops_enabled {
            self.target = None;
            return Ok(());
        }
        let wants_any = self.games.iter().any(|g| g.enabled);

        // 1. Inventaire : campagnes commencées, progression, drops à réclamer
        let due = self.inventory_at.is_none_or(|t| now.duration_since(t) >= INVENTORY_EVERY)
            || self.force_inventory_at.is_some_and(|t| now >= t);
        if due {
            self.inventory_at = Some(now);
            self.force_inventory_at = None;
            let wanted = self.wanted_games();
            match fetch_campaigns(gql, &account.user_id, &wanted, &mut self.details).await {
                Ok(snapshot) => {
                    self.campaigns = snapshot.campaigns;
                    self.raw = snapshot.raw;
                    self.claimed = snapshot.claimed;
                    self.keep_channel_campaigns();
                    // Une seule lecture de fond à la fois ; les campagnes restantes attendent la
                    // relecture suivante de l'inventaire
                    if self.details_task.is_none() && !snapshot.background.is_empty() {
                        self.details_task = Some(spawn_details(gql, &account.user_id, snapshot.background));
                    }
                    self.inventory_wall = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
                    self.error = None;
                }
                Err(TwitchError::Unauthorized) => return Err(TwitchError::Unauthorized),
                Err(e) => self.error = Some(format!("Inventaire des drops : {e}")),
            }
            self.claim_ready(app, account, gql).await?;
        }
        if !wants_any {
            self.target = None;
            return Ok(());
        }

        // 2. Streams avec drops du jeu le plus prioritaire qui en a besoin (une requête par passage)
        let mut fetched = false;
        let mut verified_once = false;
        let mut chosen: Option<(String, String, DropStream)> = None;
        let games = self.games.clone();
        for g in games.iter().filter(|g| g.enabled) {
            if self.rt.get(&g.game_id).and_then(|r| r.paused_until).is_some_and(|t| t > now) {
                continue;
            }
            if let Some(c) = self.campaign_for(&g.game_id) {
                if !c.unfinished() {
                    self.rt.entry(g.game_id.clone()).or_default().status = Some(GameStatus::Done);
                    continue;
                }
            }
            let stale = self.rt.get(&g.game_id).is_none_or(|r| r.streams_at.is_none_or(|t| now.duration_since(t) >= STREAMS_EVERY));
            if stale && !fetched {
                fetched = true;
                match gql.drop_streams(&g.slug).await {
                    Ok((streams, box_art)) => {
                        if let (None, Some(art)) = (&g.box_art, box_art) {
                            let _ = app.db.set_drop_game_box_art(&g.game_id, &art);
                            if let Some(game) = self.games.iter_mut().find(|x| x.game_id == g.game_id) {
                                game.box_art = Some(art);
                            }
                        }
                        let r = self.rt.entry(g.game_id.clone()).or_default();
                        r.streams = streams;
                        r.streams_at = Some(now);
                    }
                    Err(TwitchError::Unauthorized) => return Err(TwitchError::Unauthorized),
                    Err(e) => {
                        self.error = Some(format!("{} : {e}", g.name));
                        self.rt.entry(g.game_id.clone()).or_default().streams_at = Some(now);
                    }
                }
            }
            let known_campaign = self.campaign_for(&g.game_id).is_some();
            let allowed: Vec<String> = self.campaign_for(&g.game_id).map(|c| c.allowed_channels.clone()).unwrap_or_default();
            let r = self.rt.entry(g.game_id.clone()).or_default();
            let usable: Vec<&DropStream> = r
                .streams
                .iter()
                .filter(|s| allowed.is_empty() || allowed.contains(&s.login))
                .filter(|s| self.failed.get(&s.login).is_none_or(|t| now.duration_since(*t) >= FAILED_CHANNEL_COOLDOWN))
                .collect();
            if usable.is_empty() {
                r.status = Some(if r.streams_at.is_some() { GameStatus::NoStreams } else { GameStatus::Unknown });
                continue;
            }
            // Pas de campagne connue : Twitch confirme-t-il une campagne sur la chaîne candidate ?
            if !known_campaign {
                let candidate = usable[0].clone();
                let stale = r.verified.is_none_or(|(_, at)| now.duration_since(at) >= VERIFY_EVERY);
                if stale && !verified_once {
                    verified_once = true;
                    // Regardée seulement si Twitch confirme une campagne active de ce jeu sur la chaîne,
                    // avec des drops encore à obtenir : sans cette preuve (détail illisible, information
                    // refusée, erreur), rien n'est regardé ; nouvel essai VERIFY_EVERY plus tard
                    let confirmed = match gql.available_drops(&candidate.channel_id).await {
                        Ok(Some(list)) => {
                            let ids: Vec<String> = list.iter().filter(|(_, game)| *game == g.game_id).map(|(id, _)| id.clone()).collect();
                            if ids.is_empty() {
                                false
                            } else {
                                let found = self.campaigns_on_channel(gql, &account.user_id, &ids).await;
                                if found.is_empty() {
                                    tracing::info!("drops : {} sur {} : détail de la campagne illisible, pas de visionnage", g.name, candidate.login);
                                }
                                found.iter().any(|c| c.active() && c.unfinished())
                            }
                        }
                        Ok(None) => {
                            tracing::info!("drops : {} sur {} : campagnes de la chaîne refusées par Twitch, pas de visionnage", g.name, candidate.login);
                            false
                        }
                        Err(TwitchError::Unauthorized) => return Err(TwitchError::Unauthorized),
                        Err(e) => {
                            tracing::warn!("drops : vérification de {} impossible ({e})", candidate.login);
                            false
                        }
                    };
                    tracing::info!("drops : {} sur {} : campagne {}", g.name, candidate.login, if confirmed { "confirmée" } else { "absente" });
                    self.rt.entry(g.game_id.clone()).or_default().verified = Some((confirmed, now));
                }
                let r = self.rt.entry(g.game_id.clone()).or_default();
                if !r.verified.is_some_and(|(ok, _)| ok) {
                    r.status = Some(if r.verified.is_some() { GameStatus::NoStreams } else { GameStatus::Unknown });
                    continue;
                }
            }
            let r = self.rt.entry(g.game_id.clone()).or_default();
            let usable: Vec<&DropStream> = r
                .streams
                .iter()
                .filter(|s| allowed.is_empty() || allowed.contains(&s.login))
                .filter(|s| self.failed.get(&s.login).is_none_or(|t| now.duration_since(*t) >= FAILED_CHANNEL_COOLDOWN))
                .collect();
            if chosen.is_none() {
                // Garder la chaîne actuelle si elle est toujours valable
                let keep = self
                    .target
                    .as_ref()
                    .filter(|t| t.game_id == g.game_id)
                    .and_then(|t| usable.iter().find(|s| s.login == t.stream.login).copied());
                let stream = keep.unwrap_or(usable[0]).clone();
                chosen = Some((g.game_id.clone(), g.name.clone(), stream));
                r.status = Some(GameStatus::Watching);
            } else {
                r.status = Some(GameStatus::Waiting);
            }
        }

        // 3. Changement de cible
        match (&self.target, &chosen) {
            (Some(t), Some((gid, _, s))) if t.game_id == *gid && t.stream.login == s.login => {}
            (_, Some((gid, name, stream))) => {
                app.log(&account.login, EventKind::Info, format!("Drops {name} : visionnage de {}", stream.login));
                if self.campaign_for(gid).is_none() {
                    self.force_inventory_at = Some(now + INVENTORY_AFTER_START);
                }
                self.target = Some(Target {
                    game_id: gid.clone(),
                    game_name: name.clone(),
                    stream: stream.clone(),
                    live: None,
                    live_at: None,
                    watch: ChannelWatch::default(),
                    failures: 0,
                    started: now,
                });
            }
            (Some(_), None) => self.target = None,
            (None, None) => {}
        }

        // 4. Visionnage de la cible
        if let Some(t) = &self.target {
            let inventory_after_start = self.inventory_at.is_some_and(|at| at > t.started + INVENTORY_AFTER_START);
            if self.campaign_for(&t.game_id).is_none() && now.duration_since(t.started) >= NO_PROGRESS_AFTER && inventory_after_start {
                let (game_id, name) = (t.game_id.clone(), t.game_name.clone());
                app.log(
                    &account.login,
                    EventKind::Info,
                    format!("Drops {name} : aucune progression, campagne terminée ou pas ouverte à ce compte. Nouvel essai dans 1 h"),
                );
                let r = self.rt.entry(game_id).or_default();
                r.paused_until = Some(now + NO_PROGRESS_PAUSE);
                r.status = Some(GameStatus::NoProgress);
                self.target = None;
                return Ok(());
            }
        }
        let Some(t) = self.target.as_mut() else { return Ok(()) };
        if t.live_at.is_none_or(|at| now.duration_since(at) >= STREAM_INFO_EVERY) {
            t.live_at = Some(now);
            match gql.stream_info(&t.stream.login).await {
                Ok(Some(info)) if info.game_id.as_deref() == Some(t.game_id.as_str()) => t.live = Some(info),
                Ok(_) => {
                    // Hors ligne ou autre jeu : on cherchera une autre chaîne
                    let login = t.stream.login.clone();
                    let game_id = t.game_id.clone();
                    self.failed.insert(login, now);
                    if let Some(r) = self.rt.get_mut(&game_id) {
                        r.streams_at = None;
                    }
                    self.target = None;
                    return Ok(());
                }
                Err(TwitchError::Unauthorized) => return Err(TwitchError::Unauthorized),
                Err(e) => tracing::warn!("drops : {e}"),
            }
        }
        let Some(live) = t.live.clone() else { return Ok(()) };
        let ctx = WatchContext {
            login: &t.stream.login,
            channel_id: &t.stream.channel_id,
            broadcast_id: &live.broadcast_id,
            user_id: &account.user_id,
            game_id: live.game_id.as_deref(),
            game_name: live.game_name.as_deref(),
        };
        match t.watch.tick(gql, &ctx).await {
            Ok(_) => t.failures = 0,
            Err(TwitchError::Unauthorized) => return Err(TwitchError::Unauthorized),
            Err(e) => {
                t.failures += 1;
                if t.failures >= FAILURES_BEFORE_SWITCH {
                    let login = t.stream.login.clone();
                    app.log(&account.login, EventKind::Error, format!("Drops : visionnage de {login} en échec ({e}), autre chaîne"));
                    self.failed.insert(login, now);
                    self.target = None;
                }
            }
        }
        Ok(())
    }

    /// Réclame les drops terminés (au plus une tentative par heure et par drop)
    async fn claim_ready(&mut self, app: &App, account: &Account, gql: &Gql) -> Result<()> {
        let now = Instant::now();
        let ready: Vec<(String, String, String)> = self
            .campaigns
            .iter()
            .flat_map(|c| {
                c.drops
                    .iter()
                    .filter(|d| d.claimable())
                    .map(move |d| (d.instance_id.clone().unwrap_or_default(), d.name.clone(), c.game.name.clone()))
            })
            .filter(|(id, _, _)| self.claim_attempts.get(id).is_none_or(|t| now.duration_since(*t) >= CLAIM_RETRY))
            .collect();
        for (instance_id, name, game) in ready {
            self.claim_attempts.insert(instance_id.clone(), now);
            match gql.claim_drop(&instance_id).await {
                Ok(ClaimOutcome::Claimed) => {
                    app.log(&account.login, EventKind::Drop, format!("Drop réclamé : {name} ({game})"));
                    self.mark_claimed(&instance_id);
                }
                // Déjà réclamé (sur le site, ou avant un redémarrage) : rien de nouveau à annoncer
                Ok(ClaimOutcome::AlreadyClaimed) => self.mark_claimed(&instance_id),
                Ok(ClaimOutcome::Refused(why)) => app.log(
                    &account.login,
                    EventKind::Error,
                    format!("Drop {name} ({game}) non réclamé : {why}. Le compte du jeu est-il lié à Twitch ?"),
                ),
                Err(TwitchError::Unauthorized) => return Err(TwitchError::Unauthorized),
                Err(e) => app.log(&account.login, EventKind::Error, format!("Drop {name} ({game}) non réclamé : {e}")),
            }
        }
        Ok(())
    }

    fn mark_claimed(&mut self, instance_id: &str) {
        for c in &mut self.campaigns {
            for d in &mut c.drops {
                if d.instance_id.as_deref() == Some(instance_id) {
                    d.claimed = true;
                }
            }
        }
    }

    /// Drop terminé annoncé en direct : réclamation immédiate
    pub async fn claim_now(&mut self, app: &App, account: &Account, gql: &Gql, instance_id: &str) -> Result<()> {
        self.claim_attempts.remove(instance_id);
        let known = self.campaigns.iter().any(|c| c.drops.iter().any(|d| d.instance_id.as_deref() == Some(instance_id)));
        if !known {
            // Campagne pas encore dans l'inventaire : on le relit, la réclamation suivra
            self.inventory_at = None;
            return Ok(());
        }
        for c in &mut self.campaigns {
            for d in &mut c.drops {
                if d.instance_id.as_deref() == Some(instance_id) {
                    d.current_minutes = d.current_minutes.max(d.required_minutes);
                }
            }
        }
        self.claim_ready(app, account, gql).await
    }

    pub fn view(&self, enabled: bool) -> DropsView {
        DropsView {
            enabled,
            games: self
                .games
                .iter()
                .map(|g| {
                    let r = self.rt.get(&g.game_id);
                    DropGameView {
                        id: g.id,
                        game_id: g.game_id.clone(),
                        name: g.name.clone(),
                        slug: g.slug.clone(),
                        box_art: g.box_art.clone(),
                        enabled: g.enabled,
                        status: if !g.enabled {
                            GameStatus::Disabled
                        } else {
                            r.and_then(|r| r.status).unwrap_or(GameStatus::Unknown)
                        },
                        streams: r.map_or(0, |r| r.streams.len()),
                    }
                })
                .collect(),
            campaigns: self.campaigns.clone(),
            target: self.target.as_ref().map(|t| DropTargetView {
                game_name: t.game_name.clone(),
                login: t.stream.login.clone(),
                viewers: t.live.as_ref().map_or(t.stream.viewers, |l| l.viewers as i64),
                title: t.live.as_ref().map_or(t.stream.title.clone(), |l| l.title.clone()),
                campaign_id: self.campaign_for(&t.game_id).map(|c| c.id.clone()),
            }),
            inventory_at: self.inventory_wall.clone(),
            error: self.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "id": "c1", "name": "Saison 42", "status": "ACTIVE",
            "startAt": "2026-10-28T00:00:00Z", "endAt": "2026-11-04T00:00:00Z",
            "accountLinkURL": "https://example.com/link",
            "self": { "isAccountConnected": true },
            "game": { "id": "263490", "displayName": "Rust", "slug": "rust", "boxArtURL": "https://x/rust.jpg" },
            "allow": { "isEnabled": true, "channels": [{ "id": "1", "name": "RustBuilder_FR" }] },
            "timeBasedDrops": [
                { "id": "d1", "name": "Sac", "requiredMinutesWatched": 120, "startAt": "a", "endAt": "b",
                  "benefitEdges": [{ "benefit": { "id": "b1", "name": "Sac à dos camo", "imageAssetURL": "https://x/sac.png" } }],
                  "self": { "currentMinutesWatched": 42, "isClaimed": false, "dropInstanceID": "u#c1#d1" },
                  "preconditionDrops": null },
                { "id": "d2", "name": "Hache", "requiredMinutesWatched": 60, "startAt": "a", "endAt": "b",
                  "benefitEdges": [{ "benefit": { "id": "b2", "name": "Skin hache" } }],
                  "preconditionDrops": [{ "id": "d1" }] }
            ]
        })
    }

    #[test]
    fn channel_campaigns_survive_inventory_reads() {
        // Campagne finie : absente de l'inventaire, vue sur la chaîne (récompenses obtenues)
        let mut done = sample();
        done["timeBasedDrops"][0]["self"] = Value::Null;
        let mut engine = DropsEngine::default();
        engine.channel_campaigns.insert("c1".into(), (done, Instant::now()));
        engine.claimed = ["b1".to_owned(), "b2".to_owned()].into();
        // Relecture de l'inventaire : la campagne revient, terminée, donc plus regardée
        engine.campaigns = vec![];
        engine.keep_channel_campaigns();
        let c = engine.campaign_for("263490").unwrap();
        assert!(c.active() && !c.unfinished());
        // Déjà dans l'inventaire : pas de doublon
        engine.keep_channel_campaigns();
        assert_eq!(engine.campaigns.len(), 1);
        // Trop ancienne : oubliée, relue sur la chaîne au besoin
        engine.channel_campaigns.get_mut("c1").unwrap().1 = Instant::now() - CHANNEL_CAMPAIGN_KEEP;
        engine.campaigns = vec![];
        engine.keep_channel_campaigns();
        assert!(engine.campaign_for("263490").is_none());
    }

    #[test]
    fn campaign_parsing() {
        let c = parse_campaign(&sample(), &HashSet::new()).unwrap();
        assert_eq!(c.game.name, "Rust");
        assert!(c.account_linked && c.active() && c.unfinished());
        assert_eq!(c.allowed_channels, vec!["rustbuilder_fr"]);
        assert_eq!(c.drops[0].remaining_minutes(), 78);
        assert_eq!(c.drops[0].instance_id.as_deref(), Some("u#c1#d1"));
        assert!(!c.drops[0].claimable());
        assert_eq!(c.drops[1].preconditions, vec!["d1"]);
    }

    #[test]
    fn claimed_from_inventory_benefits() {
        let claimed: HashSet<String> = ["b2".to_owned()].into();
        let c = parse_campaign(&sample(), &claimed).unwrap();
        assert!(c.drops[1].claimed);
        assert!(!c.drops[0].claimed);
    }

    #[test]
    fn claimable_drop() {
        let mut v = sample();
        v["timeBasedDrops"][0]["self"]["currentMinutesWatched"] = json!(120);
        let c = parse_campaign(&v, &HashSet::new()).unwrap();
        assert!(c.drops[0].claimable());
    }

    #[test]
    fn category_urls() {
        assert_eq!(category_slug("https://www.twitch.tv/directory/category/sea-of-thieves").as_deref(), Some("sea-of-thieves"));
        assert_eq!(category_slug("twitch.tv/directory/category/arc-raiders?sort=VIEWER_COUNT").as_deref(), Some("arc-raiders"));
        assert_eq!(category_slug("https://www.twitch.tv/directory/game/Rust/").as_deref(), Some("rust"));
        assert_eq!(category_slug("Sea of Thieves"), None);
    }

    #[test]
    fn available_drops() {
        let open = json!({ "channel": { "id": "1", "viewerDropCampaigns": [{ "id": "c1", "game": { "id": "464339927" } }] } });
        assert_eq!(parse_available_drops(&open), Some(vec![("c1".into(), "464339927".into())]));
        let none = json!({ "channel": { "id": "1", "viewerDropCampaigns": null } });
        assert_eq!(parse_available_drops(&none), Some(vec![]));
        assert_eq!(parse_available_drops(&json!({ "channel": null })), None);
    }

    #[test]
    fn directory_streams() {
        let data = json!({ "game": { "streams": { "edges": [
            { "node": { "title": "Drops ON", "viewersCount": 20714, "broadcaster": { "id": "545", "login": "Kato_Junichi0817" } } },
            { "node": { "title": "x", "broadcaster": null } }
        ] } } });
        let s = parse_drop_streams(&data);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].login.as_str(), s[0].viewers), ("kato_junichi0817", 20714));
        assert!(parse_drop_streams(&json!({ "game": null })).is_empty());
    }

    #[test]
    fn details_only_for_started_or_wanted() {
        let inv: HashSet<String> = ["c1".to_owned()].into();
        let wanted: HashSet<String> = ["263490".to_owned()].into();
        assert!(needs_details("c1", Some("999"), &inv, &wanted));
        assert!(needs_details("c2", Some("263490"), &inv, &wanted));
        assert!(!needs_details("c3", Some("999"), &inv, &wanted));
        assert!(!needs_details("c4", None, &inv, &wanted));
    }

    #[test]
    fn details_selection_splits_urgent_and_background() {
        let now = Instant::now();
        let inv: HashSet<String> = ["c1".to_owned()].into();
        let wanted: HashSet<String> = ["263490".to_owned()].into();
        let mut cache = DetailsCache::new();
        cache.insert("c2".into(), (json!({}), now));
        cache.insert("c5".into(), (json!({}), now - DETAILS_TTL - Duration::from_secs(1)));
        cache.insert("c6".into(), (json!({}), now));
        let listed: Vec<(String, Option<String>)> = vec![
            ("c4".into(), None),
            ("c1".into(), Some("999".into())),
            ("c2".into(), Some("263490".into())),
            ("c3".into(), Some("263490".into())),
            ("c5".into(), Some("999".into())),
            ("c6".into(), Some("999".into())),
        ];
        let (urgent, background) = details_to_fetch(&listed, &cache, &inv, &wanted, now);
        // c2 et c6 : détail récent en cache ; c5 : périmé, relu
        assert_eq!(urgent, vec!["c1", "c3"]);
        assert_eq!(background, vec!["c4", "c5"]);
    }

    #[test]
    fn background_details_fill_campaign_and_keep_progress() {
        let mut bare = sample();
        bare.as_object_mut().unwrap().remove("timeBasedDrops");
        let claimed = HashSet::new();
        let mut e = DropsEngine {
            raw: [("c1".to_owned(), bare.clone())].into(),
            campaigns: vec![parse_campaign(&bare, &claimed).unwrap()],
            ..Default::default()
        };
        assert!(e.campaigns[0].drops.is_empty());
        let details = json!({ "id": "c1", "timeBasedDrops": sample()["timeBasedDrops"] });
        e.apply_details(vec![("c1".into(), Some(details.clone())), ("absente".into(), Some(json!({}))), ("c9".into(), None)]);
        assert_eq!(e.campaigns.len(), 1);
        assert_eq!(e.campaigns[0].drops.len(), 2);
        assert!(e.details.contains_key("c1") && !e.details.contains_key("absente"));

        // Progression reçue en direct entre-temps : pas écrasée par une relecture plus ancienne
        e.on_progress("d1", 100);
        e.apply_details(vec![("c1".into(), Some(details))]);
        assert_eq!(e.campaigns[0].drops[0].current_minutes, 100);
    }

    #[test]
    fn unknown_progress_refresh_is_rate_limited() {
        let mut e = DropsEngine { inventory_at: Some(Instant::now()), ..Default::default() };
        e.on_progress("inconnu", 5);
        assert!(e.inventory_at.is_none());
        e.inventory_at = Some(Instant::now());
        e.on_progress("inconnu", 6);
        assert!(e.inventory_at.is_some(), "deuxième relecture forcée trop tôt");
    }

    #[test]
    fn merge_keeps_progress_and_adds_details() {
        let inventory = json!({ "id": "c1", "timeBasedDrops": [{ "id": "d1", "self": { "currentMinutesWatched": 42 } }] });
        let details = json!({ "id": "c1", "name": "Saison", "timeBasedDrops": [{ "id": "d1", "requiredMinutesWatched": 120, "self": null }, { "id": "d2" }] });
        let m = merge(&inventory, &details);
        assert_eq!(m["name"], "Saison");
        assert_eq!(m["timeBasedDrops"].as_array().unwrap().len(), 2);
        assert_eq!(m["timeBasedDrops"][0]["self"]["currentMinutesWatched"], 42);
        assert_eq!(m["timeBasedDrops"][0]["requiredMinutesWatched"], 120);
    }
}
