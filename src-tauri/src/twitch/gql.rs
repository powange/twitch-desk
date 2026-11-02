//! Requêtes GQL internes de Twitch (requêtes « persistées » identifiées par une empreinte).
//!
//! Twitch change ces empreintes sans prévenir. Elles sont rangées dans
//! operations.json : la version embarquée sert de base, une copie plus récente
//! est relue sur GitHub au démarrage. Chaque opération peut avoir plusieurs
//! empreintes, essayées dans l'ordre tant que Twitch répond PersistedQueryNotFound.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex, Notify};

use super::{random_device_id, Result, TwitchError, CLIENT_ID, CLIENT_ORIGIN};

const GQL_URL: &str = "https://gql.twitch.tv/gql";
pub const REMOTE_OPERATIONS_URL: &str =
    "https://raw.githubusercontent.com/powange/twitch-desk/main/operations.json";
const EMBEDDED_OPERATIONS: &str = include_str!("../../../operations.json");
// Numéro de build du site, envoyé en Client-Version ; repli si twitch.tv ne répond pas
const CLIENT_VERSION_FALLBACK: &str = "ef928475-9403-42f2-8a34-55784bd08e16";
const CLIENT_VERSION_TTL: Duration = Duration::from_secs(3600);
// Twitch casse tout au-delà de quelques requêtes par seconde : 250 ms minimum entre deux
const MIN_REQUEST_GAP: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationsFile {
    pub version: u32,
    pub updated: String,
    pub operations: HashMap<String, Vec<String>>,
}

static OPERATIONS: LazyLock<RwLock<OperationsFile>> = LazyLock::new(|| {
    RwLock::new(serde_json::from_str(EMBEDDED_OPERATIONS).expect("operations.json embarqué valide"))
});

/// Remplace les empreintes si le fichier fourni est valide et au moins aussi récent
pub fn install_operations(file: OperationsFile) -> bool {
    if file.operations.is_empty() {
        return false;
    }
    let mut current = OPERATIONS.write().unwrap();
    if file.version < current.version {
        return false;
    }
    *current = file;
    true
}

pub fn operations_info() -> (u32, String) {
    let ops = OPERATIONS.read().unwrap();
    (ops.version, ops.updated.clone())
}

/// Dernière empreinte acceptée par Twitch pour chaque opération : essayée en premier
static PREFERRED: LazyLock<RwLock<HashMap<String, String>>> = LazyLock::new(|| RwLock::new(HashMap::new()));

/// Empreinte inconnue de Twitch : signal pour relire operations.json plus tôt que prévu
static OPERATIONS_STALE: Notify = Notify::const_new();

/// Attend qu'une requête tombe sur une empreinte inconnue de Twitch
pub async fn operations_stale() {
    OPERATIONS_STALE.notified().await;
}

fn hashes_for(name: &str) -> Vec<String> {
    let hashes = OPERATIONS.read().unwrap().operations.get(name).cloned().unwrap_or_default();
    let preferred = PREFERRED.read().unwrap().get(name).cloned();
    prefer_first(hashes, preferred.as_deref())
}

/// Met l'empreinte qui a marché la dernière fois en tête (si elle est toujours listée)
fn prefer_first(mut hashes: Vec<String>, preferred: Option<&str>) -> Vec<String> {
    if let Some(i) = preferred.and_then(|p| hashes.iter().position(|h| h == p)) {
        let h = hashes.remove(i);
        hashes.insert(0, h);
    }
    hashes
}

fn remember_hash(name: &str, hash: &str) {
    let known = PREFERRED.read().unwrap().get(name).is_some_and(|h| h == hash);
    if !known {
        PREFERRED.write().unwrap().insert(name.to_owned(), hash.to_owned());
    }
}

static CLIENT_VERSION: LazyLock<Mutex<Option<(String, Instant)>>> = LazyLock::new(|| Mutex::new(None));

async fn client_version(http: &reqwest::Client) -> String {
    // Le verrou n'est pas gardé pendant la lecture de twitch.tv (jusqu'à 20 s)
    if let Some((v, at)) = CLIENT_VERSION.lock().await.as_ref() {
        if at.elapsed() < CLIENT_VERSION_TTL {
            return v.clone();
        }
    }
    let fetched = async {
        let html = http
            .get("https://www.twitch.tv")
            .header("User-Agent", super::BROWSER_USER_AGENT)
            .send()
            .await
            .ok()?
            .text()
            .await
            .ok()?;
        extract_build_id(&html)
    }
    .await;
    let version = fetched.unwrap_or_else(|| CLIENT_VERSION_FALLBACK.to_owned());
    *CLIENT_VERSION.lock().await = Some((version.clone(), Instant::now()));
    version
}

fn extract_build_id(html: &str) -> Option<String> {
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#"window\.__twilightBuildID\s*=\s*"([0-9a-f-]{36})""#).unwrap());
    RE.captures(html).map(|c| c[1].to_owned())
}

/// Client GQL d'un compte : jeton, identifiant d'appareil et de session propres
#[derive(Clone)]
pub struct Gql {
    http: reqwest::Client,
    token: Arc<RwLock<String>>,
    device_id: String,
    session_id: String,
    last_request: Arc<Mutex<Instant>>,
}

impl Gql {
    pub fn new(http: reqwest::Client, token: String, device_id: Option<String>) -> Self {
        Self {
            http,
            token: Arc::new(RwLock::new(token)),
            device_id: device_id.unwrap_or_else(random_device_id),
            session_id: uuid::Uuid::new_v4().simple().to_string()[..16].to_owned(),
            last_request: Arc::new(Mutex::new(Instant::now() - MIN_REQUEST_GAP)),
        }
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn token(&self) -> String {
        self.token.read().unwrap().clone()
    }

    pub fn set_token(&self, token: String) {
        *self.token.write().unwrap() = token;
    }

    async fn throttle(&self) {
        let mut last = self.last_request.lock().await;
        let wait = MIN_REQUEST_GAP.saturating_sub(last.elapsed());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        *last = Instant::now();
    }

    /// Requête écrite en clair (sans empreinte), avec le jeton du compte ; renvoie `data`
    pub async fn query(&self, query: &str, variables: Value) -> Result<Value> {
        self.throttle().await;
        let version = client_version(&self.http).await;
        let res = self
            .http
            .post(GQL_URL)
            .header("Client-Id", CLIENT_ID)
            .header("Authorization", format!("OAuth {}", self.token()))
            .header("Client-Session-Id", &self.session_id)
            .header("Client-Version", &version)
            .header("X-Device-Id", &self.device_id)
            .header("Origin", CLIENT_ORIGIN)
            .header("Referer", format!("{CLIENT_ORIGIN}/"))
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await?;
        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(TwitchError::Unauthorized);
        }
        let value: Value = res.json().await?;
        match classify_response(value) {
            Outcome::Data(data, _) => Ok(data),
            Outcome::Unauthorized => Err(TwitchError::Unauthorized),
            Outcome::UnknownHash => Err(TwitchError::Gql("requête refusée".into())),
            Outcome::Error(e) => Err(TwitchError::Gql(e)),
        }
    }

    /// Depuis quand le compte suit la chaîne (None : ne la suit pas), avec l'identifiant de la
    /// chaîne ; None si la chaîne est introuvable
    pub async fn follow_state(&self, login: &str) -> Result<Option<(String, Option<String>)>> {
        let data = self
            .query("query FollowState($login: String!) { user(login: $login) { id self { follower { followedAt } } } }", json!({ "login": login }))
            .await?;
        let Some(user) = data.get("user").filter(|u| !u.is_null()) else { return Ok(None) };
        let id = user.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
        let since = user.pointer("/self/follower/followedAt").and_then(Value::as_str).map(str::to_owned);
        Ok(Some((id, since)))
    }

    /// Suit une chaîne (notifications de live laissées à Twitch)
    pub async fn follow(&self, channel_id: &str) -> Result<()> {
        let data = self
            .query(
                "mutation Follow($id: ID!) { followUser(input: { targetID: $id, disableNotifications: false }) { follow { followedAt } error { code } } }",
                json!({ "id": channel_id }),
            )
            .await?;
        if let Some(code) = data.pointer("/followUser/error/code").and_then(Value::as_str) {
            return Err(TwitchError::Gql(format!("follow refusé ({code})")));
        }
        if data.pointer("/followUser/follow/followedAt").is_none_or(Value::is_null) {
            return Err(TwitchError::Gql("follow non confirmé par Twitch".into()));
        }
        Ok(())
    }

    /// Exécute l'opération `name` ; renvoie le champ `data` de la réponse
    pub async fn request(&self, name: &str, variables: Value) -> Result<Value> {
        self.request_full(name, variables).await.map(|(data, _)| data)
    }

    /// Comme `request`, avec les erreurs partielles renvoyées à côté des données
    async fn request_full(&self, name: &str, variables: Value) -> Result<(Value, Vec<String>)> {
        let hashes = hashes_for(name);
        if hashes.is_empty() {
            OPERATIONS_STALE.notify_one();
            return Err(TwitchError::PersistedQueryNotFound(name.to_owned()));
        }
        let version = client_version(&self.http).await;
        for hash in &hashes {
            self.throttle().await;
            let body = json!({
                "operationName": name,
                "variables": variables,
                "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } }
            });
            let res = self
                .http
                .post(GQL_URL)
                .header("Client-Id", CLIENT_ID)
                .header("Authorization", format!("OAuth {}", self.token()))
                .header("Client-Session-Id", &self.session_id)
                .header("Client-Version", &version)
                .header("X-Device-Id", &self.device_id)
                .header("Origin", CLIENT_ORIGIN)
                .header("Referer", format!("{CLIENT_ORIGIN}/"))
                .json(&body)
                .send()
                .await?;
            if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(TwitchError::Unauthorized);
            }
            let value: Value = res.json().await?;
            match classify_response(value) {
                Outcome::Data(data, errors) => {
                    remember_hash(name, hash);
                    return Ok((data, errors));
                }
                Outcome::UnknownHash => continue,
                Outcome::Unauthorized => return Err(TwitchError::Unauthorized),
                Outcome::Error(e) => return Err(TwitchError::Gql(format!("{name} : {e}"))),
            }
        }
        OPERATIONS_STALE.notify_one();
        Err(TwitchError::PersistedQueryNotFound(name.to_owned()))
    }

    // --- Opérations utilisées par les points de chaîne ---

    /// Identifiant numérique d'une chaîne (None si le login n'existe pas)
    pub async fn channel_id(&self, login: &str) -> Result<Option<String>> {
        let data = self.request("GetIDFromLogin", json!({ "login": login })).await?;
        Ok(data.pointer("/user/id").and_then(Value::as_str).map(str::to_owned))
    }

    /// État du live (None = hors ligne)
    pub async fn stream_info(&self, login: &str) -> Result<Option<StreamInfo>> {
        let data = self
            .request("VideoPlayerStreamInfoOverlayChannel", json!({ "channel": login }))
            .await?;
        Ok(parse_stream_info(&data))
    }

    /// Solde de points et bonus en attente sur une chaîne
    pub async fn points_context(&self, login: &str) -> Result<Option<PointsContext>> {
        let data = self.request("ChannelPointsContext", json!({ "channelLogin": login })).await?;
        Ok(parse_points_context(&data))
    }

    pub async fn claim_bonus(&self, channel_id: &str, claim_id: &str) -> Result<()> {
        let (data, errors) = self
            .request_full(
                "ClaimCommunityPoints",
                json!({ "input": { "channelID": channel_id, "claimID": claim_id } }),
            )
            .await?;
        check_claim(&data, &errors)
    }

    pub async fn playback_token(&self, login: &str) -> Result<(String, String)> {
        let data = self
            .request(
                "PlaybackAccessToken",
                json!({
                    "login": login, "isLive": true, "isVod": false, "vodID": "",
                    "playerType": "site", "platform": "web"
                }),
            )
            .await?;
        let token = data
            .get("streamPlaybackAccessToken")
            .ok_or_else(|| TwitchError::Unexpected("pas de jeton de lecture".into()))?;
        let value = token.get("value").and_then(Value::as_str);
        let signature = token.get("signature").and_then(Value::as_str);
        match (value, signature) {
            (Some(v), Some(s)) => Ok((v.to_owned(), s.to_owned())),
            _ => Err(TwitchError::Unexpected("jeton de lecture incomplet".into())),
        }
    }
}

/// Identifiant client du site web de Twitch : requêtes écrites à la main, sans
/// connexion (même méthode que le site Giveaway Twitch)
const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// Langue de diffusion de chaque chaîne (code ISO 639-1 en minuscules), par lots
pub async fn channel_languages(http: &reqwest::Client, logins: &[String]) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    for chunk in logins.chunks(50) {
        let list: Vec<String> = chunk.iter().filter(|l| is_login(l)).map(|l| format!("\"{l}\"")).collect();
        if list.is_empty() {
            continue;
        }
        let query = format!("query {{ users(logins: [{}]) {{ login broadcastSettings {{ language }} }} }}", list.join(","));
        let res = http
            .post(GQL_URL)
            .header("Client-Id", WEB_CLIENT_ID)
            .header("User-Agent", super::BROWSER_USER_AGENT)
            .json(&json!({ "query": query }))
            .send()
            .await?;
        let value: Value = res.json().await?;
        out.extend(parse_languages(&value));
    }
    Ok(out)
}

/// Fiche publique d'une chaîne (requête sans connexion, comme les langues)
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelProfile {
    pub login: String,
    pub display_name: String,
    pub description: Option<String>,
    pub profile_image: Option<String>,
    pub language: Option<String>,
    pub followers: Option<i64>,
    /// En live : spectateurs, catégorie, titre, début du live
    pub live: bool,
    pub viewers: Option<i64>,
    pub game_id: Option<String>,
    pub game_name: Option<String>,
    pub title: Option<String>,
    pub started_at: Option<String>,
    /// Message épinglé dans le chat (meilleur effort : Twitch peut ne pas le donner)
    pub pinned_message: Option<String>,
    pub pinned_by: Option<String>,
    /// Invitation Discord trouvée dans les liens de la chaîne ou sa description
    pub discord: Option<String>,
}

/// Invitation Discord dans un texte (discord.gg/…, discord.com/invite/…), en adresse complète
pub fn find_discord(text: &str) -> Option<String> {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)(?:https?://)?(?:www\.)?(discord\.gg|discord(?:app)?\.com/invite)/([A-Za-z0-9-]{2,64})").unwrap()
    });
    let c = RE.captures(text)?;
    let host = if c[1].eq_ignore_ascii_case("discord.gg") { "discord.gg" } else { "discord.com/invite" };
    Some(format!("https://{host}/{}", &c[2]))
}

pub async fn channel_profile(http: &reqwest::Client, login: &str) -> Result<Option<ChannelProfile>> {
    if !is_login(login) {
        return Ok(None);
    }
    let query = format!(
        "query {{ user(login: \"{login}\") {{ login displayName description profileImageURL(width: 150) \
         followers {{ totalCount }} channel {{ socialMedias {{ name url }} }} \
         broadcastSettings {{ language title game {{ id displayName }} }} \
         stream {{ viewersCount createdAt title game {{ id displayName }} }} }} }}"
    );
    let value = raw_query(http, &query).await?;
    let mut profile = match parse_profile(&value) {
        Some(p) => p,
        None => return Ok(None),
    };
    // Message épinglé : requête à part, une erreur ne doit pas faire perdre la fiche
    let pinned = format!(
        "query {{ channel(name: \"{login}\") {{ pinnedChatMessages(first: 1) {{ edges {{ node {{ \
         pinnedBy {{ displayName }} pinnedMessage {{ content {{ text }} sender {{ displayName }} }} }} }} }} }} }}"
    );
    match raw_query(http, &pinned).await {
        Ok(v) => {
            let node = v.pointer("/data/channel/pinnedChatMessages/edges/0/node");
            profile.pinned_message = node
                .and_then(|n| n.pointer("/pinnedMessage/content/text"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            profile.pinned_by = node
                .and_then(|n| n.pointer("/pinnedMessage/sender/displayName").or_else(|| n.pointer("/pinnedBy/displayName")))
                .and_then(Value::as_str)
                .map(str::to_owned);

        }
        Err(e) => tracing::debug!("message épinglé de {login} : {e}"),
    }
    Ok(Some(profile))
}

/// Stream en live d'une catégorie, pour le scanner
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CategoryStream {
    pub login: String,
    pub display_name: String,
    pub title: String,
    pub viewers: i64,
    pub started_at: Option<String>,
    pub language: Option<String>,
    pub profile_image: Option<String>,
    pub game_id: String,
    pub game_name: String,
    pub tags: Vec<String>,
}

/// Streams en live d'une catégorie (slug), les plus regardés d'abord (requête sans connexion)
/// Requête des lives d'une catégorie (une page de 100), triés par spectateurs
fn category_query(slug: &str, sort: &str, after: Option<&str>) -> String {
    let after_arg = after.map(|c| format!(", after: \"{c}\"")).unwrap_or_default();
    format!(
        "query {{ game(slug: \"{slug}\") {{ id displayName streams(first: 100{after_arg}, options: {{ sort: {sort} }}) {{ \
         edges {{ cursor node {{ title viewersCount createdAt freeformTags {{ name }} \
         broadcaster {{ login displayName profileImageURL(width: 50) broadcastSettings {{ language }} }} }} }} \
         pageInfo {{ hasNextPage }} }} }} }}"
    )
}

/// Curseur du dernier live d'une page (caractères sûrs seulement : il est recopié dans la requête)
fn last_cursor(data: &Value) -> Option<String> {
    data.pointer("/game/streams/edges")
        .and_then(Value::as_array)
        .and_then(|e| e.last())
        .and_then(|e| e.get("cursor"))
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_alphanumeric() || "=_-+/".contains(ch)))
        .map(str::to_owned)
}

fn valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Lives d'une catégorie jusqu'à `max`, les plus regardés d'abord.
/// Avec un compte (`gql`), toutes les pages sont lues : Twitch exige une vérification
/// d'intégrité pour les pages suivantes d'une requête anonyme. Sans compte, ou si Twitch
/// refuse : les 100 plus regardés et les 100 moins regardés (deux requêtes anonymes), pour ne
/// pas manquer les petits streamers. Tri explicite : sans lui, Twitch ne renvoie qu'une page
/// « par pertinence »
pub async fn category_streams(http: &reqwest::Client, gql: Option<&Gql>, slug: &str, max: usize) -> Result<Vec<CategoryStream>> {
    if !valid_slug(slug) {
        return Ok(vec![]);
    }
    if let Some(gql) = gql {
        let mut out: Vec<CategoryStream> = vec![];
        let mut after: Option<String> = None;
        let mut failed = false;
        while out.len() < max {
            match gql.query(&category_query(slug, "VIEWER_COUNT", after.as_deref()), json!({})).await {
                Ok(data) => {
                    let page = parse_category_streams(&json!({ "data": data }));
                    if page.is_empty() {
                        break;
                    }
                    out.extend(page);
                    let next = data.pointer("/game/streams/pageInfo/hasNextPage").and_then(Value::as_bool).unwrap_or(false);
                    after = last_cursor(&data);
                    if !next || after.is_none() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!("scanner : lives de {slug} avec le compte : {e}");
                    failed = out.is_empty();
                    break;
                }
            }
        }
        if !failed {
            out.truncate(max);
            return Ok(out);
        }
    }
    // Anonyme : les plus regardés, puis les moins regardés
    let mut out: Vec<CategoryStream> = vec![];
    for sort in ["VIEWER_COUNT", "VIEWER_COUNT_ASC"] {
        let value = raw_query(http, &category_query(slug, sort, None)).await?;
        if value.pointer("/data/game").is_none_or(Value::is_null) {
            if let Some(errors) = value.get("errors") {
                if out.is_empty() {
                    return Err(TwitchError::Gql(format!("streams de {slug} : {errors}")));
                }
            }
            continue;
        }
        for s in parse_category_streams(&value) {
            if !out.iter().any(|x| x.login == s.login) {
                out.push(s);
            }
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.viewers));
    out.truncate(max);
    Ok(out)
}

fn parse_category_streams(value: &Value) -> Vec<CategoryStream> {
    let Some(game) = value.pointer("/data/game").filter(|g| !g.is_null()) else { return vec![] };
    let game_id = game.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
    let game_name = game.get("displayName").and_then(Value::as_str).unwrap_or_default().to_owned();
    game.pointer("/streams/edges")
        .and_then(Value::as_array)
        .map(|edges| {
            edges
                .iter()
                .filter_map(|e| e.get("node"))
                .filter_map(|n| {
                    let b = n.get("broadcaster").filter(|b| !b.is_null())?;
                    let s = |v: &Value, p: &str| v.pointer(p).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_owned);
                    Some(CategoryStream {
                        login: s(b, "/login")?.to_lowercase(),
                        display_name: s(b, "/displayName").unwrap_or_default(),
                        title: s(n, "/title").unwrap_or_default(),
                        viewers: n.get("viewersCount").and_then(Value::as_i64).unwrap_or(0),
                        started_at: s(n, "/createdAt"),
                        language: s(b, "/broadcastSettings/language").map(|l| l.to_lowercase()),
                        profile_image: s(b, "/profileImageURL"),
                        game_id: game_id.clone(),
                        game_name: game_name.clone(),
                        tags: n
                            .get("freeformTags")
                            .and_then(Value::as_array)
                            .map(|t| t.iter().filter_map(|x| x.get("name").and_then(Value::as_str).map(str::to_owned)).collect())
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Catégorie trouvée par la recherche
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CategoryHit {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub box_art: Option<String>,
}

/// Recherche de catégories Twitch par nom (autocomplétion), sans compte
pub async fn search_categories(http: &reqwest::Client, text: &str) -> Result<Vec<CategoryHit>> {
    let text = text.trim();
    if text.chars().count() < 2 {
        return Ok(vec![]);
    }
    let value = raw_query(
        http,
        &format!(
            "query {{ searchCategories(query: {}, first: 8) {{ edges {{ node {{ id displayName slug boxArtURL(width: 285, height: 380) }} }} }} }}",
            Value::String(text.chars().take(60).collect())
        ),
    )
    .await?;
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_owned);
    Ok(value
        .pointer("/data/searchCategories/edges")
        .and_then(Value::as_array)
        .map(|edges| {
            edges
                .iter()
                .filter_map(|e| e.get("node"))
                .filter_map(|n| Some(CategoryHit { id: s(n, "id")?, name: s(n, "displayName")?, slug: s(n, "slug")?, box_art: s(n, "boxArtURL") }))
                .collect()
        })
        .unwrap_or_default())
}

/// Jaquettes de jeux d'après leur identifiant (50 au plus par appel), sans compte. L'adresse ne
/// se déduit pas de l'identifiant : les jeux récents ont une forme `<id>_IGDB-…`
pub async fn box_arts(http: &reqwest::Client, ids: &[String]) -> Result<HashMap<String, String>> {
    let ids: Vec<&String> = ids.iter().filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).take(50).collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let fields: String = ids
        .iter()
        .enumerate()
        .map(|(i, id)| format!("g{i}: game(id: \"{id}\") {{ id boxArtURL(width: 285, height: 380) }} "))
        .collect();
    let value = raw_query(http, &format!("query {{ {fields}}}")).await?;
    let mut out = HashMap::new();
    if let Some(data) = value.get("data").and_then(Value::as_object) {
        for game in data.values() {
            if let (Some(id), Some(url)) = (game.get("id").and_then(Value::as_str), game.get("boxArtURL").and_then(Value::as_str)) {
                out.insert(id.to_owned(), url.to_owned());
            }
        }
    }
    Ok(out)
}

/// Chaînes au plus par requête de statut live groupée
pub const STREAMS_BATCH: usize = 50;

/// Statut live de plusieurs chaînes, sans compte, en une requête par lot de 50 (mêmes champs que
/// `stream_info`). Une chaîne demandée absente de la réponse n'existe pas : hors ligne. Les
/// logins invalides ne sont pas demandés (absents du résultat)
pub async fn streams_info(http: &reqwest::Client, logins: &[String]) -> Result<HashMap<String, Option<StreamInfo>>> {
    let mut out = HashMap::new();
    let valid: Vec<&String> = logins.iter().filter(|l| is_login(l)).collect();
    for chunk in valid.chunks(STREAMS_BATCH) {
        let list: Vec<String> = chunk.iter().map(|l| format!("\"{l}\"")).collect();
        let query = format!(
            "query {{ users(logins: [{}]) {{ login stream {{ id viewersCount }} \
             broadcastSettings {{ title game {{ id displayName }} }} }} }}",
            list.join(",")
        );
        let value = raw_query(http, &query).await?;
        let found = parse_streams(&value).ok_or_else(|| {
            let why = value.get("errors").map(Value::to_string).unwrap_or_else(|| "réponse sans données".into());
            TwitchError::Gql(format!("statut live groupé : {why}"))
        })?;
        for login in chunk {
            out.insert((*login).clone(), found.get(&login.to_lowercase()).cloned().flatten());
        }
    }
    Ok(out)
}

/// login (minuscules) → statut live ; None si la réponse n'a pas de liste
fn parse_streams(value: &Value) -> Option<HashMap<String, Option<StreamInfo>>> {
    let users = value.pointer("/data/users")?.as_array()?;
    Some(
        users
            .iter()
            .filter_map(|u| {
                let login = u.get("login")?.as_str()?.to_lowercase();
                Some((login, parse_stream_info(&json!({ "user": u }))))
            })
            .collect(),
    )
}

async fn raw_query(http: &reqwest::Client, query: &str) -> Result<Value> {
    let res = http
        .post(GQL_URL)
        .header("Client-Id", WEB_CLIENT_ID)
        .header("User-Agent", super::BROWSER_USER_AGENT)
        .json(&json!({ "query": query }))
        .send()
        .await?;
    Ok(res.json().await?)
}

fn parse_profile(value: &Value) -> Option<ChannelProfile> {
    let u = value.pointer("/data/user").filter(|u| !u.is_null())?;
    let s = |v: &Value, p: &str| v.pointer(p).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_owned);
    let stream = u.get("stream").filter(|x| !x.is_null());
    let settings = u.get("broadcastSettings");
    Some(ChannelProfile {
        login: s(u, "/login")?.to_lowercase(),
        display_name: s(u, "/displayName").unwrap_or_default(),
        description: s(u, "/description"),
        profile_image: s(u, "/profileImageURL"),
        language: settings.and_then(|b| s(b, "/language")).map(|l| l.to_lowercase()),
        followers: u.pointer("/followers/totalCount").and_then(Value::as_i64),
        live: stream.is_some(),
        viewers: stream.and_then(|x| x.get("viewersCount")).and_then(Value::as_i64),
        game_id: stream.and_then(|x| s(x, "/game/id")).or_else(|| settings.and_then(|b| s(b, "/game/id"))),
        game_name: stream.and_then(|x| s(x, "/game/displayName")).or_else(|| settings.and_then(|b| s(b, "/game/displayName"))),
        title: stream.and_then(|x| s(x, "/title")).or_else(|| settings.and_then(|b| s(b, "/title"))),
        started_at: stream.and_then(|x| s(x, "/createdAt")),
        pinned_message: None,
        pinned_by: None,
        // Liens de la chaîne d'abord (bouton Discord du profil), sinon la description
        discord: u
            .pointer("/channel/socialMedias")
            .and_then(Value::as_array)
            .and_then(|links| links.iter().filter_map(|l| l.get("url").and_then(Value::as_str)).find_map(find_discord))
            .or_else(|| s(u, "/description").as_deref().and_then(find_discord)),
    })
}

fn is_login(s: &str) -> bool {
    !s.is_empty() && s.len() <= 25 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_languages(value: &Value) -> HashMap<String, String> {
    value
        .pointer("/data/users")
        .and_then(Value::as_array)
        .map(|users| {
            users
                .iter()
                .filter_map(|u| {
                    let login = u.get("login")?.as_str()?.to_lowercase();
                    let lang = u.pointer("/broadcastSettings/language")?.as_str()?.to_lowercase();
                    Some((login, lang))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub broadcast_id: String,
    pub title: String,
    pub game_id: Option<String>,
    pub game_name: Option<String>,
    pub viewers: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointsContext {
    pub balance: i64,
    pub claim_id: Option<String>,
}

enum Outcome {
    /// Données, avec les erreurs partielles éventuelles
    Data(Value, Vec<String>),
    UnknownHash,
    Unauthorized,
    Error(String),
}

/// Bonus récupéré seulement si Twitch le confirme : `claimCommunityPoints` présent, sans erreur
fn check_claim(data: &Value, errors: &[String]) -> Result<()> {
    let Some(claim) = data.get("claimCommunityPoints").filter(|c| !c.is_null()) else {
        let why = if errors.is_empty() { "réponse vide".to_owned() } else { errors.join(", ") };
        return Err(TwitchError::Gql(format!("bonus non confirmé : {why}")));
    };
    if let Some(error) = claim.get("error").filter(|e| !e.is_null()) {
        let code = error.get("code").or_else(|| error.get("message")).and_then(Value::as_str).unwrap_or("inconnu");
        if code.eq_ignore_ascii_case("ALREADY_CLAIMED") {
            return Ok(());
        }
        return Err(TwitchError::Gql(format!("bonus refusé : {code}")));
    }
    if !errors.is_empty() {
        return Err(TwitchError::Gql(format!("bonus refusé : {}", errors.join(", "))));
    }
    Ok(())
}

fn classify_response(value: Value) -> Outcome {
    // { "error": "Unauthorized", "status": 401 }
    if value.get("status").and_then(Value::as_u64) == Some(401) {
        return Outcome::Unauthorized;
    }
    let mut messages: Vec<String> = vec![];
    if let Some(errors) = value.get("errors").and_then(Value::as_array) {
        messages = errors.iter().filter_map(|e| e.get("message").and_then(Value::as_str)).map(str::to_owned).collect();
        if messages.iter().any(|m| m.contains("PersistedQueryNotFound")) {
            return Outcome::UnknownHash;
        }
        if value.get("data").is_none_or(Value::is_null) {
            return Outcome::Error(messages.join(", "));
        }
    }
    match value.get("data") {
        Some(data) if !data.is_null() => Outcome::Data(data.clone(), messages),
        _ => Outcome::Error("réponse sans données".into()),
    }
}

fn parse_stream_info(data: &Value) -> Option<StreamInfo> {
    let user = data.get("user")?;
    let stream = user.get("stream").filter(|s| !s.is_null())?;
    let settings = user.get("broadcastSettings");
    let game = settings.and_then(|s| s.get("game")).filter(|g| !g.is_null());
    Some(StreamInfo {
        broadcast_id: stream.get("id")?.as_str()?.to_owned(),
        title: settings
            .and_then(|s| s.get("title"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        game_id: game.and_then(|g| g.get("id")).and_then(Value::as_str).map(str::to_owned),
        game_name: game
            .and_then(|g| g.get("displayName").or_else(|| g.get("name")))
            .and_then(Value::as_str)
            .map(str::to_owned),
        viewers: stream.get("viewersCount").and_then(Value::as_u64).unwrap_or(0),
    })
}

fn parse_points_context(data: &Value) -> Option<PointsContext> {
    let points = data.pointer("/community/channel/self/communityPoints")?;
    Some(PointsContext {
        balance: points.get("balance").and_then(Value::as_i64).unwrap_or(0),
        claim_id: points
            .pointer("/availableClaim/id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_operations_are_valid() {
        let (version, _) = operations_info();
        assert!(version >= 1);
        for name in [
            "GetIDFromLogin",
            "VideoPlayerStreamInfoOverlayChannel",
            "ChannelPointsContext",
            "ClaimCommunityPoints",
            "PlaybackAccessToken",
        ] {
            assert!(!hashes_for(name).is_empty(), "{name}");
        }
    }

    #[test]
    fn category_streams_parsing() {
        let v = json!({"data":{"game":{"id":"490377","displayName":"Sea of Thieves","streams":{"edges":[
            {"node":{"title":"GIVEAWAY ce soir","viewersCount":151,"createdAt":"2026-11-01T08:00:00Z","freeformTags":[{"name":"Français"}],
              "broadcaster":{"login":"LevraiDoffy","displayName":"LeVraiDoffy","profileImageURL":"https://x/p.png","broadcastSettings":{"language":"FR"}}}},
            {"node":{"title":"x","viewersCount":1,"broadcaster":null}}]}}}});
        let s = parse_category_streams(&v);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].login.as_str(), s[0].language.as_deref(), s[0].game_name.as_str()), ("levraidoffy", Some("fr"), "Sea of Thieves"));
        assert_eq!(s[0].tags, vec!["Français"]);
        assert!(parse_category_streams(&json!({"data":{"game":null}})).is_empty());
    }

    #[test]
    fn discord_links() {
        assert_eq!(find_discord("Rejoins-nous : discord.gg/SeaOfThievesFR !").as_deref(), Some("https://discord.gg/SeaOfThievesFR"));
        assert_eq!(find_discord("https://discord.com/invite/abc-123"), Some("https://discord.com/invite/abc-123".into()));
        assert_eq!(find_discord("https://www.discordapp.com/invite/xyz"), Some("https://discord.com/invite/xyz".into()));
        assert_eq!(find_discord("pas de discord ici"), None);
        let v = json!({"data":{"user":{"login":"a","displayName":"A","description":"discord.gg/desc",
            "channel":{"socialMedias":[{"name":"x","url":"https://twitter.com/a"},{"name":"discord","url":"https://discord.gg/social"}]}}}});
        assert_eq!(parse_profile(&v).unwrap().discord.as_deref(), Some("https://discord.gg/social"));
    }

    #[test]
    fn profile_parsing() {
        let v = json!({"data":{"user":{"login":"LevraiDoffy","displayName":"LevraiDoffy","description":"Pirate 24/7",
            "profileImageURL":"https://x/p.png","followers":{"totalCount":12345},
            "broadcastSettings":{"language":"FR","title":"Réglages","game":{"id":"490377","displayName":"Sea of Thieves"}},
            "stream":{"viewersCount":151,"createdAt":"2026-10-30T12:00:00Z","title":"En live","game":{"id":"490377","displayName":"Sea of Thieves"}}}}});
        let p = parse_profile(&v).unwrap();
        assert!(p.live);
        assert_eq!((p.login.as_str(), p.viewers, p.followers), ("levraidoffy", Some(151), Some(12345)));
        assert_eq!((p.language.as_deref(), p.title.as_deref()), (Some("fr"), Some("En live")));
        let off = json!({"data":{"user":{"login":"a","displayName":"A","stream":null,"broadcastSettings":{"title":"Hors live","game":null}}}});
        let p = parse_profile(&off).unwrap();
        assert!(!p.live && p.viewers.is_none());
        assert_eq!(p.title.as_deref(), Some("Hors live"));
        assert!(parse_profile(&json!({"data":{"user":null}})).is_none());
    }

    #[test]
    fn languages() {
        let v = json!({"data":{"users":[{"login":"LunarSea","broadcastSettings":{"language":"FR"}},null,{"login":"x","broadcastSettings":null}]}});
        let m = parse_languages(&v);
        assert_eq!(m.get("lunarsea").map(String::as_str), Some("fr"));
        assert_eq!(m.len(), 1);
        assert!(is_login("kraken_tv") && !is_login("a\"b"));
    }

    #[test]
    fn build_id() {
        let html = r#"<script>window.__twilightBuildID="0a1b2c3d-4e5f-6789-abcd-ef0123456789";</script>"#;
        assert_eq!(extract_build_id(html).as_deref(), Some("0a1b2c3d-4e5f-6789-abcd-ef0123456789"));
        assert_eq!(extract_build_id("rien"), None);
    }

    #[test]
    fn response_classification() {
        assert!(matches!(
            classify_response(json!({"errors":[{"message":"PersistedQueryNotFound"}]})),
            Outcome::UnknownHash
        ));
        assert!(matches!(
            classify_response(json!({"error":"Unauthorized","status":401,"message":"..."})),
            Outcome::Unauthorized
        ));
        assert!(matches!(classify_response(json!({"data":{"user":null}})), Outcome::Data(_, ref e) if e.is_empty()));
        assert!(matches!(
            classify_response(json!({"data":{"x":null},"errors":[{"message":"partiel"}]})),
            Outcome::Data(_, ref e) if e == &["partiel".to_owned()]
        ));
        assert!(matches!(
            classify_response(json!({"errors":[{"message":"service error"}],"data":null})),
            Outcome::Error(_)
        ));
    }

    #[test]
    fn claim_needs_confirmation() {
        let ok = json!({"claimCommunityPoints":{"claim":{"id":"c"},"error":null}});
        assert!(check_claim(&ok, &[]).is_ok());
        let already = json!({"claimCommunityPoints":{"error":{"code":"ALREADY_CLAIMED"}}});
        assert!(check_claim(&already, &[]).is_ok());
        let refused = json!({"claimCommunityPoints":{"error":{"code":"NOT_FOUND"}}});
        assert!(check_claim(&refused, &[]).is_err());
        // Données présentes mais réclamation absente, avec une erreur : pas un succès
        let null = json!({"claimCommunityPoints":null});
        assert!(check_claim(&null, &["service timeout".into()]).is_err());
        assert!(check_claim(&json!({}), &[]).is_err());
        assert!(check_claim(&ok, &["partiel".into()]).is_err());
    }

    #[test]
    fn preferred_hash_first() {
        let list = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        assert_eq!(prefer_first(list.clone(), Some("c")), vec!["c", "a", "b"]);
        assert_eq!(prefer_first(list.clone(), Some("z")), vec!["a", "b", "c"]);
        assert_eq!(prefer_first(list, None), vec!["a", "b", "c"]);
    }

    #[test]
    fn stream_info_online_and_offline() {
        let online = json!({"user":{"id":"1","stream":{"id":"42","viewersCount":120},
            "broadcastSettings":{"title":"Giveaway ce soir","game":{"id":"490377","name":"Sea of Thieves","displayName":"Sea of Thieves"}}}});
        let info = parse_stream_info(&online).unwrap();
        assert_eq!(info.broadcast_id, "42");
        assert_eq!(info.viewers, 120);
        assert_eq!(info.game_name.as_deref(), Some("Sea of Thieves"));
        let offline = json!({"user":{"id":"1","stream":null,"broadcastSettings":{"title":"x","game":null}}});
        assert_eq!(parse_stream_info(&offline), None);
        assert_eq!(parse_stream_info(&json!({"user":null})), None);
    }

    #[test]
    fn batched_streams() {
        let value = json!({"data":{"users":[
            {"login":"Alpha","stream":{"id":"7","viewersCount":12},"broadcastSettings":{"title":"t","game":{"id":"1","displayName":"Jeu"}}},
            {"login":"beta","stream":null,"broadcastSettings":{"title":"x","game":null}},
            null
        ]}});
        let map = parse_streams(&value).unwrap();
        assert_eq!(map.len(), 2);
        let alpha = map["alpha"].as_ref().unwrap();
        assert_eq!((alpha.broadcast_id.as_str(), alpha.viewers, alpha.game_name.as_deref()), ("7", 12, Some("Jeu")));
        assert_eq!(map["beta"], None);
        assert_eq!(parse_streams(&json!({"errors":[{"message":"x"}]})), None);
    }

    #[test]
    fn points_context() {
        let with_claim = json!({"community":{"channel":{"self":{"communityPoints":{"balance":4820,"availableClaim":{"id":"abc"}}}}}});
        assert_eq!(
            parse_points_context(&with_claim),
            Some(PointsContext { balance: 4820, claim_id: Some("abc".into()) })
        );
        let without = json!({"community":{"channel":{"self":{"communityPoints":{"balance":10,"availableClaim":null}}}}});
        assert_eq!(parse_points_context(&without).unwrap().claim_id, None);
        assert_eq!(parse_points_context(&json!({"community":{"channel":null}})), None);
    }

}
