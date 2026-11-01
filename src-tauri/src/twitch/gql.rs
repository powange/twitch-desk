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
use tokio::sync::Mutex;

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

fn hashes_for(name: &str) -> Vec<String> {
    OPERATIONS.read().unwrap().operations.get(name).cloned().unwrap_or_default()
}

static CLIENT_VERSION: LazyLock<Mutex<Option<(String, Instant)>>> = LazyLock::new(|| Mutex::new(None));

async fn client_version(http: &reqwest::Client) -> String {
    let mut cached = CLIENT_VERSION.lock().await;
    if let Some((v, at)) = cached.as_ref() {
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
    *cached = Some((version.clone(), Instant::now()));
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

    /// Exécute l'opération `name` ; renvoie le champ `data` de la réponse
    pub async fn request(&self, name: &str, variables: Value) -> Result<Value> {
        let hashes = hashes_for(name);
        if hashes.is_empty() {
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
                Outcome::Data(data) => return Ok(data),
                Outcome::UnknownHash => continue,
                Outcome::Unauthorized => return Err(TwitchError::Unauthorized),
                Outcome::Error(e) => return Err(TwitchError::Gql(format!("{name} : {e}"))),
            }
        }
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
        let data = self
            .request(
                "ClaimCommunityPoints",
                json!({ "input": { "channelID": channel_id, "claimID": claim_id } }),
            )
            .await?;
        match data.pointer("/claimCommunityPoints/error").filter(|e| !e.is_null()) {
            None => Ok(()),
            Some(error) => {
                let code = error
                    .get("code")
                    .or_else(|| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("inconnu");
                if code.eq_ignore_ascii_case("ALREADY_CLAIMED") {
                    Ok(())
                } else {
                    Err(TwitchError::Gql(format!("bonus refusé : {code}")))
                }
            }
        }
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
    Data(Value),
    UnknownHash,
    Unauthorized,
    Error(String),
}

fn classify_response(value: Value) -> Outcome {
    // { "error": "Unauthorized", "status": 401 }
    if value.get("status").and_then(Value::as_u64) == Some(401) {
        return Outcome::Unauthorized;
    }
    if let Some(errors) = value.get("errors").and_then(Value::as_array) {
        let messages: Vec<&str> =
            errors.iter().filter_map(|e| e.get("message").and_then(Value::as_str)).collect();
        if messages.iter().any(|m| m.contains("PersistedQueryNotFound")) {
            return Outcome::UnknownHash;
        }
        if value.get("data").is_none_or(Value::is_null) {
            return Outcome::Error(messages.join(", "));
        }
    }
    match value.get("data") {
        Some(data) if !data.is_null() => Outcome::Data(data.clone()),
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
        assert!(matches!(classify_response(json!({"data":{"user":null}})), Outcome::Data(_)));
        assert!(matches!(
            classify_response(json!({"errors":[{"message":"service error"}],"data":null})),
            Outcome::Error(_)
        ));
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
