//! Web app mobile (0.6) : applications identifiées (instances), association d'un téléphone par
//! un code à 6 chiffres, appareils associés (jeton à usage de lecture seule), état publié par
//! chaque application (participations, statut live) et vue d'ensemble lue par la web app.
//!
//! Un appareil associé ne lit que la vue d'ensemble de son application et le flux
//! d'événements ; il n'écrit que son propre abonnement aux notifications (`push`). Son jeton n'est gardé que sous forme
//! d'empreinte SHA-256 ; le supprimer (révocation) le refuse aussitôt et ferme son flux.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Extension, Path, Request, State};
use axum::http::StatusCode;
use axum::Json;
use rand::{Rng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{client_ip, now_text, with_db, Access, ApiError, Shared};

/// Durée de validité d'un code d'association
pub const PAIR_TTL: Duration = Duration::from_secs(5 * 60);
/// Codes d'association refusés : après PAIR_MAX_FAILURES essais en PAIR_WINDOW, l'adresse attend PAIR_BLOCK
pub const PAIR_MAX_FAILURES: u32 = 5;
pub const PAIR_WINDOW: Duration = Duration::from_secs(10 * 60);
pub const PAIR_BLOCK: Duration = Duration::from_secs(10 * 60);
/// Toutes adresses confondues : au-delà de PAIR_GLOBAL_MAX codes refusés en PAIR_WINDOW, les codes
/// en attente sont annulés (deviner un code à 6 chiffres depuis beaucoup d'adresses n'aboutit pas ;
/// il suffit d'en demander un nouveau sur le PC)
pub const PAIR_GLOBAL_MAX: usize = 30;
/// Corps d'une demande d'association (code et nom de l'appareil)
const MAX_CLAIM_BODY: usize = 4 * 1024;
/// État publié par une application (participations, statut live) : quelques Ko d'habitude
pub const MAX_STATE: usize = 256 * 1024;
/// Appareils associés à une même application, au plus
const MAX_DEVICES: i64 = 20;
/// Nom d'une application ou d'un appareil, en caractères
const MAX_NAME: usize = 60;

/// Code d'association en attente
pub struct Pairing {
    pub instance: String,
    pub expires: Instant,
}

/// Nouvelle annoncée aux flux d'événements des appareils
#[derive(Debug, Clone)]
pub enum Notice {
    /// État d'une application publié (identifiant de l'application)
    State(String),
    /// Appareil révoqué : son flux se ferme
    Revoked(String),
    /// Demande d'un téléphone pour cette application (identifiant de l'application)
    Action(String),
    /// Demande traitée par le PC (identifiant de l'appareil qui l'a faite)
    ActionDone(String),
}

/// Identifiant d'application : tiré au hasard par l'application (uuid)
pub fn valid_instance_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn clean_name(name: Option<&str>) -> Option<String> {
    let name: String = name?.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    (!name.is_empty()).then_some(name)
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Empreinte d'un jeton d'appareil (seule gardée en base)
pub fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Application enregistrée si besoin (nom vide tant qu'elle ne l'a pas donné)
fn ensure_instance(conn: &Connection, id: &str, name: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO instances (id, name, updated_at) VALUES (?1, COALESCE(?2, ''), ?3)
         ON CONFLICT(id) DO UPDATE SET name = COALESCE(?2, name), updated_at = excluded.updated_at",
        params![id, name, now_text()],
    )?;
    Ok(())
}

fn check_instance(id: &str) -> Result<(), ApiError> {
    if valid_instance_id(id) {
        Ok(())
    } else {
        Err(ApiError::BadRequest("Identifiant d'application invalide".into()))
    }
}

/// Précision de la dernière visite d'un appareil
const LAST_SEEN_EVERY: chrono::Duration = chrono::Duration::minutes(5);

/// Appareil reconnu par son jeton : (appareil, application) ; dernière visite notée
pub fn device_for_token(conn: &Connection, token: &str) -> rusqlite::Result<Option<(String, String)>> {
    let found: Option<(String, String)> = conn
        .query_row("SELECT id, instance FROM devices WHERE token_hash = ?1", [token_hash(token)], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    // Dernière visite réécrite au plus toutes les LAST_SEEN_EVERY (pas une écriture par requête)
    if let Some((id, _)) = &found {
        let now = chrono::Utc::now();
        let before = (now - LAST_SEEN_EVERY).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        conn.execute(
            "UPDATE devices SET last_seen = ?2 WHERE id = ?1 AND (last_seen IS NULL OR last_seen < ?3)",
            params![id, now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true), before],
        )?;
    }
    Ok(found)
}

pub fn device_exists(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    conn.query_row("SELECT COUNT(*) FROM devices WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).map(|n| n > 0)
}

// --- Routes de l'application (code partagé) ---

#[derive(Debug, Deserialize)]
pub struct InstanceBody {
    name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InstanceInfo {
    id: String,
    name: String,
}

/// `PUT /api/v1/instances/{id}` : l'application s'enregistre (ou change de nom)
pub async fn put_instance(
    State(server): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<InstanceBody>,
) -> Result<Json<InstanceInfo>, ApiError> {
    check_instance(&id)?;
    let name = clean_name(body.name.as_deref());
    let info = with_db(&server, move |conn| {
        ensure_instance(conn, &id, name.as_deref())?;
        let name: String = conn.query_row("SELECT name FROM instances WHERE id = ?1", [&id], |r| r.get(0))?;
        Ok(InstanceInfo { id, name })
    })
    .await?;
    Ok(Json(info))
}

/// `PUT /api/v1/instances/{id}/state` : état publié par l'application (objet JSON libre,
/// MAX_STATE au plus), annoncé aux appareils de cette application
pub async fn put_state(State(server): State<Shared>, Path(id): Path<String>, Json(state): Json<Value>) -> Result<StatusCode, ApiError> {
    check_instance(&id)?;
    if !state.is_object() {
        return Err(ApiError::BadRequest("État invalide".into()));
    }
    let text = state.to_string();
    if text.len() > MAX_STATE {
        return Err(ApiError::BadRequest("État trop volumineux".into()));
    }
    let instance = id.clone();
    let changed = with_db(&server, move |conn| {
        ensure_instance(conn, &id, None)?;
        let n = conn.execute(
            "UPDATE instances SET state = ?2, state_at = ?3 WHERE id = ?1 AND state IS NOT ?2",
            params![id, text, now_text()],
        )?;
        Ok(n > 0)
    })
    .await?;
    if changed {
        let _ = server.notices.send(Notice::State(instance));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairStart {
    instance: String,
    name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairCode {
    code: String,
    expires_at: String,
    /// Secondes de validité
    expires_in: u64,
}

/// `POST /api/v1/pair/start` : code d'association à 6 chiffres, valable PAIR_TTL, à usage
/// unique ; un nouveau code remplace le précédent de la même application
pub async fn pair_start(State(server): State<Shared>, Json(body): Json<PairStart>) -> Result<Json<PairCode>, ApiError> {
    check_instance(&body.instance)?;
    let name = clean_name(body.name.as_deref());
    let instance = body.instance.clone();
    with_db(&server, move |conn| Ok(ensure_instance(conn, &instance, name.as_deref())?)).await?;
    let now = Instant::now();
    let code = {
        let mut pairings = server.pairings.lock().unwrap_or_else(|e| e.into_inner());
        pairings.retain(|_, p| p.expires > now && p.instance != body.instance);
        let mut rng = rand::rng();
        let code = loop {
            let code = format!("{:06}", rng.random_range(0..1_000_000u32));
            if !pairings.contains_key(&code) {
                break code;
            }
        };
        pairings.insert(code.clone(), Pairing { instance: body.instance, expires: now + PAIR_TTL });
        code
    };
    let expires_at = (chrono::Utc::now() + chrono::Duration::from_std(PAIR_TTL).expect("durée"))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Ok(Json(PairCode { code, expires_at, expires_in: PAIR_TTL.as_secs() }))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    id: String,
    name: String,
    created_at: String,
    last_seen: Option<String>,
}

/// `GET /api/v1/instances/{id}/devices` : appareils associés à l'application
pub async fn list_devices(State(server): State<Shared>, Path(id): Path<String>) -> Result<Json<Vec<DeviceInfo>>, ApiError> {
    check_instance(&id)?;
    let list = with_db(&server, move |conn| {
        let mut stmt = conn.prepare("SELECT id, name, created_at, last_seen FROM devices WHERE instance = ?1 ORDER BY created_at, id")?;
        let rows = stmt.query_map([id], |r| Ok(DeviceInfo { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)?, last_seen: r.get(3)? }))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await?;
    Ok(Json(list))
}

/// `DELETE /api/v1/instances/{id}/devices/{device}` : jeton refusé aussitôt, flux fermé
pub async fn revoke_device(State(server): State<Shared>, Path((id, device)): Path<(String, String)>) -> Result<StatusCode, ApiError> {
    check_instance(&id)?;
    let target = device.clone();
    let n = with_db(&server, move |conn| {
        let n = conn.execute("DELETE FROM devices WHERE id = ?1 AND instance = ?2", params![target, id])?;
        // Plus de notifications pour lui
        conn.execute("DELETE FROM push_subscriptions WHERE device = ?1", [&target])?;
        Ok(n)
    })
    .await?;
    if n == 0 {
        return Err(ApiError::NotFound("Appareil inconnu".into()));
    }
    tracing::info!("appareil {device} révoqué");
    let _ = server.notices.send(Notice::Revoked(device));
    Ok(StatusCode::NO_CONTENT)
}

// --- Association (sans code partagé) ---

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairClaim {
    code: String,
    device_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paired {
    token: String,
    device_id: String,
    instance: InstanceInfo,
}

/// `POST /api/v1/pair/claim` : le téléphone donne le code affiché par l'application et reçoit
/// son jeton. Sans code partagé : limité à PAIR_MAX_FAILURES codes refusés par adresse
/// (vérifié avant la lecture du corps)
pub async fn pair_claim(State(server): State<Shared>, request: Request) -> Result<Json<Paired>, ApiError> {
    let peer = request.extensions().get::<ConnectInfo<std::net::SocketAddr>>().map(|c| c.0.ip());
    let ip = client_ip(request.headers(), peer, server.proxy_hops);
    let now = Instant::now();
    if let Some(wait) = server.pair_guard.blocked(ip, now) {
        return Err(ApiError::TooManyAttempts(wait));
    }
    let bytes = axum::body::to_bytes(request.into_body(), MAX_CLAIM_BODY)
        .await
        .map_err(|_| ApiError::BadRequest("Demande trop volumineuse".into()))?;
    let body: PairClaim = serde_json::from_slice(&bytes).map_err(|_| ApiError::BadRequest("Demande invalide".into()))?;
    let code: String = body.code.chars().filter(|c| !c.is_whitespace()).collect();
    let instance = {
        let mut pairings = server.pairings.lock().unwrap_or_else(|e| e.into_inner());
        pairings.retain(|_, p| p.expires > now);
        // Usage unique : retiré dès qu'il sert (remis si l'association est refusée)
        pairings.remove(&code)
    };
    let Some(pairing) = instance else {
        server.pair_guard.failed(ip, now);
        tracing::warn!("code d'association refusé pour {ip}");
        let mut failures = server.pair_failures.lock().unwrap_or_else(|e| e.into_inner());
        while failures.front().is_some_and(|t| now.duration_since(*t) > PAIR_WINDOW) {
            failures.pop_front();
        }
        failures.push_back(now);
        if failures.len() >= PAIR_GLOBAL_MAX {
            failures.clear();
            let cancelled = {
                let mut pairings = server.pairings.lock().unwrap_or_else(|e| e.into_inner());
                let n = pairings.len();
                pairings.clear();
                n
            };
            tracing::warn!("{PAIR_GLOBAL_MAX} codes d'association refusés en {} min : {cancelled} code(s) en attente annulé(s)", PAIR_WINDOW.as_secs() / 60);
        }
        return Err(ApiError::BadPairingCode);
    };
    let name = clean_name(body.device_name.as_deref()).unwrap_or_else(|| "Téléphone".to_owned());
    let token = random_hex(32);
    let device_id = random_hex(12);
    let (hash, id) = (token_hash(&token), device_id.clone());
    let instance = pairing.instance.clone();
    let result = with_db(&server, move |conn| {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM devices WHERE instance = ?1", [&instance], |r| r.get(0))?;
        if count >= MAX_DEVICES {
            return Err(ApiError::BadRequest(format!("{MAX_DEVICES} appareils sont déjà associés : révoques-en un depuis l'application")));
        }
        let at = now_text();
        conn.execute(
            "INSERT INTO devices (id, instance, name, token_hash, created_at, last_seen) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, instance, name, hash, at],
        )?;
        let name: String = conn.query_row("SELECT name FROM instances WHERE id = ?1", [&instance], |r| r.get(0))?;
        Ok(InstanceInfo { id: instance, name })
    })
    .await;
    // Limite d'appareils atteinte (ou erreur) : le code reste valable, utilisable une fois un
    // appareil révoqué
    let instance = match result {
        Ok(i) => i,
        Err(e) => {
            server.pairings.lock().unwrap_or_else(|e| e.into_inner()).insert(code, pairing);
            return Err(e);
        }
    };
    tracing::info!("appareil {device_id} associé à {}", instance.id);
    Ok(Json(Paired { token, device_id, instance }))
}

// --- Vue d'ensemble (appareil associé) ---

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    instance: InstanceInfo,
    device: DeviceInfo,
    /// Dernier état publié par l'application (null : jamais publié)
    state: Option<Value>,
    state_at: Option<String>,
    /// Giveaways ouverts puis en pause, cadeaux résolus
    giveaways: Vec<Value>,
    /// Numéro de la dernière modification
    seq: i64,
}

/// Documents présents d'une collection : identifiant → contenu
fn docs_of(conn: &Connection, collection: &str) -> rusqlite::Result<HashMap<String, Value>> {
    let mut stmt = conn.prepare_cached("SELECT id, data FROM docs WHERE collection = ?1 AND deleted = 0 AND data IS NOT NULL")?;
    let rows = stmt.query_map([collection], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut map = HashMap::new();
    for row in rows {
        let (id, data) = row?;
        if let Some(v) = object_of(&data) {
            map.insert(id, v);
        }
    }
    Ok(map)
}

fn object_of(data: &str) -> Option<Value> {
    serde_json::from_str::<Value>(data).ok().filter(Value::is_object)
}

/// Un document présent (None : absent, supprimé ou illisible)
fn doc_of(conn: &Connection, collection: &str, id: &str) -> rusqlite::Result<Option<Value>> {
    let data: Option<String> = conn
        .prepare_cached("SELECT data FROM docs WHERE collection = ?1 AND id = ?2 AND deleted = 0 AND data IS NOT NULL")?
        .query_row(params![collection, id], |r| r.get(0))
        .optional()?;
    Ok(data.as_deref().and_then(object_of))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Image d'un cadeau envoyée à part (`gift_image`) : adresse avec l'empreinte de l'image (gardée
/// en cache par le téléphone tant qu'elle ne change pas). Image en ligne : son adresse si elle est
/// chez Twitch ; ailleurs, relayée par ce serveur (0.8.10), qui la lit lui-même : l'adresse IP des
/// téléphones n'est jamais connue de son hébergeur
fn gift_image_url(gift_id: &str, image: &str) -> Option<String> {
    if image.starts_with("https://") && twitch_image(image) {
        return Some(image.to_owned());
    }
    if !image.starts_with("data:image/") && !image.starts_with("https://") {
        return None;
    }
    let hash: String = Sha256::digest(image.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
    let id: String = gift_id.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    Some(format!("/api/v1/app/gifts/{id}/image?v={hash}"))
}

/// Image servie par Twitch (photos, jeux, récompenses de drops)
fn twitch_image(url: &str) -> bool {
    let Ok(uri) = url.parse::<axum::http::Uri>() else { return false };
    let host = uri.host().unwrap_or_default().to_ascii_lowercase();
    uri.scheme_str() == Some("https")
        && uri.port().is_none()
        && !url.contains('@')
        && ["jtvnw.net", "twitchcdn.net", "twitch.tv"].iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Giveaways ouverts ou en pause, ouverts d'abord, les plus récents en premier ; cadeaux
/// remplacés par leur titre, adresse de leur image, jeu et groupe. Seuls ces giveaways et leurs
/// cadeaux sont lus (pas les terminés, ni les images des autres cadeaux)
pub fn open_giveaways(conn: &Connection, collections: &std::collections::BTreeSet<String>) -> rusqlite::Result<Vec<Value>> {
    if !collections.contains("giveaways") {
        return Ok(vec![]);
    }
    let with_gifts = collections.contains("gifts");
    let groups = if with_gifts && collections.contains("groups") { docs_of(conn, "groups")? } else { HashMap::new() };
    let mut stmt = conn.prepare_cached(
        "SELECT id, data FROM docs WHERE collection = 'giveaways' AND deleted = 0 AND data IS NOT NULL
           AND json_valid(data) AND (json_extract(data, '$.open') = 1 OR json_extract(data, '$.status') = 'paused')",
    )?;
    let rows: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut gifts: HashMap<String, Option<Value>> = HashMap::new();
    let mut list: Vec<(bool, String, Value)> = vec![];
    for (id, data) in rows {
        let Some(mut g) = object_of(&data) else { continue };
        let open = g.get("open").and_then(Value::as_bool).unwrap_or(false);
        let paused = g.get("status").and_then(Value::as_str) == Some("paused");
        if !open && !paused {
            continue;
        }
        let ids: Vec<String> = g.get("gifts").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
        let mut resolved = vec![];
        if with_gifts {
            for gid in ids {
                if !gifts.contains_key(&gid) {
                    let doc = doc_of(conn, "gifts", &gid)?;
                    gifts.insert(gid.clone(), doc);
                }
                let Some(Some(gift)) = gifts.get(&gid) else { continue };
                let group = text(gift, "group").and_then(|gr| groups.get(&gr)).and_then(|gr| text(gr, "name"));
                resolved.push(serde_json::json!({
                    "id": gid,
                    "title": text(gift, "title"),
                    "image": text(gift, "image").and_then(|i| gift_image_url(&gid, &i)),
                    "gameName": text(gift, "gameName"),
                    "group": group,
                }));
            }
        }
        let detected = text(&g, "detectedAt").unwrap_or_default();
        g["id"] = Value::String(id);
        g["gifts"] = Value::Array(resolved);
        g["status"] = Value::String(if open { "open" } else { "paused" }.into());
        list.push((open, detected, g));
    }
    list.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    Ok(list.into_iter().map(|(_, _, g)| g).collect())
}

/// Image d'un cadeau décodée (type, octets) ; None : pas de cadeau ou image non intégrée
pub fn gift_image(conn: &Connection, id: &str) -> rusqlite::Result<Option<(String, Vec<u8>)>> {
    use base64ct::{Base64, Encoding};
    let Some(gift) = doc_of(conn, "gifts", id)? else { return Ok(None) };
    let Some(image) = text(&gift, "image") else { return Ok(None) };
    let Some(rest) = image.strip_prefix("data:") else { return Ok(None) };
    let Some((meta, payload)) = rest.split_once(',') else { return Ok(None) };
    let Some(mime) = meta.strip_suffix(";base64") else { return Ok(None) };
    // Types d'images seulement (pas de SVG : il peut porter du script)
    if !matches!(mime, "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/avif") {
        return Ok(None);
    }
    Ok(Base64::decode_vec(payload.trim()).ok().map(|bytes| (mime.to_owned(), bytes)))
}

/// Images en ligne relayées : lues au plus 5 Mo, gardées en mémoire (200 au plus)
const RELAY_MAX_BYTES: usize = 5 * 1024 * 1024;
const RELAY_CACHE: usize = 200;
/// Image relayée : type, octets
type Relayed = (String, Vec<u8>);
static RELAYED: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Relayed>>> = std::sync::LazyLock::new(Default::default);

/// Adresse IP publique : ni locale, ni privée, ni réservée (le relais ne doit jamais viser le
/// réseau du serveur)
fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && (b == 18 || b == 19))
                || a >= 240)
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_ip(std::net::IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// Image en ligne lue par le serveur : https seulement, hôte à adresse publique (vérifiée puis
/// imposée à la connexion), pas de redirection, type d'image (pas de SVG), 5 Mo au plus
async fn relay_image(url: &str) -> Option<(String, Vec<u8>)> {
    if let Some(found) = RELAYED.lock().unwrap_or_else(|e| e.into_inner()).get(url) {
        return Some(found.clone());
    }
    let parsed = url.parse::<axum::http::Uri>().ok()?;
    if parsed.scheme_str() != Some("https") || url.contains('@') {
        return None;
    }
    let host = parsed.host()?.trim_start_matches('[').trim_end_matches(']').to_owned();
    let port = parsed.port_u16().unwrap_or(443);
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), port)).await.ok()?.collect();
    let addr = *addrs.first()?;
    if addrs.iter().any(|a| !public_ip(a.ip())) {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .resolve(&host, addr)
        .user_agent(concat!("twitch-desk-sync/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let mut res = client.get(url).send().await.ok()?.error_for_status().ok()?;
    let mime = res.headers().get(reqwest::header::CONTENT_TYPE)?.to_str().ok()?.split(';').next()?.trim().to_ascii_lowercase();
    if !matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/avif") {
        return None;
    }
    if res.content_length().is_some_and(|n| n as usize > RELAY_MAX_BYTES) {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = res.chunk().await.ok()? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > RELAY_MAX_BYTES {
            return None;
        }
    }
    let mut cache = RELAYED.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= RELAY_CACHE {
        cache.clear();
    }
    cache.insert(url.to_owned(), (mime.clone(), bytes.clone()));
    Some((mime, bytes))
}

/// Adresse en ligne de l'image d'un cadeau (hors Twitch), à relayer
fn gift_online_image(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    let Some(gift) = doc_of(conn, "gifts", id)? else { return Ok(None) };
    Ok(text(&gift, "image").filter(|i| i.starts_with("https://")))
}

/// `GET /api/v1/app/gifts/{id}/image` (appareil associé) : image d'un cadeau, gardée en cache par
/// le téléphone (l'adresse donnée par la vue change avec l'image)
pub async fn gift_image_route(State(server): State<Shared>, Extension(access): Extension<Access>, Path(id): Path<String>) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    if !matches!(access, Access::Device { .. }) {
        return Err(ApiError::BadRequest("Réservé aux appareils associés".into()));
    }
    if !server.collections.contains("gifts") {
        return Err(ApiError::NotFound("Pas de cadeaux sur ce serveur".into()));
    }
    let (found, online) = with_db(&server, move |conn| Ok((gift_image(conn, &id)?, gift_online_image(conn, &id)?))).await?;
    let found = match (found, online) {
        (Some(f), _) => Some(f),
        (None, Some(url)) => relay_image(&url).await,
        (None, None) => None,
    };
    let Some((mime, bytes)) = found else { return Err(ApiError::NotFound("Image introuvable".into())) };
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, mime),
            (axum::http::header::CACHE_CONTROL, "private, max-age=31536000, immutable".to_owned()),
            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        bytes,
    )
        .into_response())
}

/// `GET /api/v1/app/state` (appareil associé) : état publié par son application seulement (léger,
/// relu à chaque annonce « state », sans les giveaways ni leurs images)
pub async fn state(State(server): State<Shared>, Extension(access): Extension<Access>) -> Result<Json<Value>, ApiError> {
    let Access::Device { instance, .. } = access else {
        return Err(ApiError::BadRequest("Réservé aux appareils associés".into()));
    };
    let (state, at) = with_db(&server, move |conn| {
        Ok(conn.query_row("SELECT state, state_at FROM instances WHERE id = ?1", [&instance], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))
        })?)
    })
    .await?;
    Ok(Json(serde_json::json!({ "state": state.and_then(|t| serde_json::from_str::<Value>(&t).ok()), "stateAt": at })))
}

/// `GET /api/v1/app/overview` (appareil associé) : giveaways ouverts et en pause, état de
/// son application
pub async fn overview(State(server): State<Shared>, Extension(access): Extension<Access>) -> Result<Json<Overview>, ApiError> {
    let Access::Device { id, instance } = access else {
        return Err(ApiError::BadRequest("Réservé aux appareils associés".into()));
    };
    let s = server.clone();
    let overview = with_db(&server, move |conn| {
        let (name, state, state_at): (String, Option<String>, Option<String>) =
            conn.query_row("SELECT name, state, state_at FROM instances WHERE id = ?1", [&instance], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let device = conn.query_row("SELECT id, name, created_at, last_seen FROM devices WHERE id = ?1", [&id], |r| {
            Ok(DeviceInfo { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)?, last_seen: r.get(3)? })
        })?;
        Ok(Overview {
            instance: InstanceInfo { id: instance, name },
            device,
            state: state.and_then(|t| serde_json::from_str(&t).ok()),
            state_at,
            giveaways: open_giveaways(conn, &s.collections)?,
            seq: crate::last_seq(conn)?,
        })
    })
    .await?;
    Ok(Json(overview))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_gift_images_direct_only_from_twitch() {
        // Chez Twitch : adresse directe ; ailleurs (ou faux domaine Twitch) : relayée par le serveur
        assert_eq!(gift_image_url("a", "https://static-cdn.jtvnw.net/twitch-quests-assets/REWARD/x.png").as_deref(), Some("https://static-cdn.jtvnw.net/twitch-quests-assets/REWARD/x.png"));
        for url in ["https://exemple.fr/x.png", "https://jtvnw.net.exemple.fr/x.png", "https://static-cdn.jtvnw.net:8443/x.png"] {
            assert!(gift_image_url("a", url).unwrap().starts_with("/api/v1/app/gifts/a/image?v="), "{url}");
        }
        assert!(gift_image_url("a", "http://static-cdn.jtvnw.net/x.png").is_none());
        assert!(gift_image_url("a", "data:image/png;base64,iVBORw==").unwrap().starts_with("/api/v1/app/gifts/a/image?v="));
    }
}

#[cfg(test)]
mod relay_tests {
    use super::*;

    #[test]
    fn relay_never_targets_the_server_network() {
        for ip in ["127.0.0.1", "10.1.2.3", "192.168.1.10", "172.16.0.1", "169.254.169.254", "0.0.0.0", "100.64.0.1", "::1", "fc00::1", "fe80::1", "::ffff:127.0.0.1"] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "2606:2800:220:1:248:1893:25c8:1946"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
        // Image hors Twitch : relayée par le serveur ; Twitch : adresse directe
        assert!(gift_image_url("g1", "https://i.ebayimg.com/a.jpg").unwrap().starts_with("/api/v1/app/gifts/g1/image?v="));
        assert_eq!(gift_image_url("g1", "https://static-cdn.jtvnw.net/a.png").as_deref(), Some("https://static-cdn.jtvnw.net/a.png"));
        assert!(gift_image_url("g1", "http://insecure.example/a.png").is_none());
    }
}
