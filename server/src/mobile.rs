//! Web app mobile (0.6) : applications identifiées (instances), association d'un téléphone par
//! un code à 6 chiffres, appareils associés (jeton à usage de lecture seule), état publié par
//! chaque application (participations, statut live) et vue d'ensemble lue par la web app.
//!
//! Un appareil associé ne lit que la vue d'ensemble de son application et le flux
//! d'événements : aucune route d'écriture ne l'accepte. Son jeton n'est gardé que sous forme
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

/// Appareil reconnu par son jeton : (appareil, application) ; dernière visite notée
pub fn device_for_token(conn: &Connection, token: &str) -> rusqlite::Result<Option<(String, String)>> {
    let found: Option<(String, String)> = conn
        .query_row("SELECT id, instance FROM devices WHERE token_hash = ?1", [token_hash(token)], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    if let Some((id, _)) = &found {
        conn.execute("UPDATE devices SET last_seen = ?2 WHERE id = ?1", params![id, now_text()])?;
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
    let n = with_db(&server, move |conn| Ok(conn.execute("DELETE FROM devices WHERE id = ?1 AND instance = ?2", params![target, id])?))
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
        // Usage unique : retiré dès qu'il sert
        pairings.remove(&code).map(|p| p.instance)
    };
    let Some(instance) = instance else {
        server.pair_guard.failed(ip, now);
        tracing::warn!("code d'association refusé pour {ip}");
        return Err(ApiError::BadPairingCode);
    };
    let name = clean_name(body.device_name.as_deref()).unwrap_or_else(|| "Téléphone".to_owned());
    let token = random_hex(32);
    let device_id = random_hex(12);
    let (hash, id) = (token_hash(&token), device_id.clone());
    let instance = with_db(&server, move |conn| {
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
    .await?;
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
        if let Ok(v) = serde_json::from_str::<Value>(&data) {
            if v.is_object() {
                map.insert(id, v);
            }
        }
    }
    Ok(map)
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Giveaways ouverts ou en pause, ouverts d'abord, les plus récents en premier ; cadeaux
/// remplacés par leur titre, image, jeu et groupe
pub fn open_giveaways(conn: &Connection, collections: &std::collections::BTreeSet<String>) -> rusqlite::Result<Vec<Value>> {
    if !collections.contains("giveaways") {
        return Ok(vec![]);
    }
    let gifts = if collections.contains("gifts") { docs_of(conn, "gifts")? } else { HashMap::new() };
    let groups = if collections.contains("groups") { docs_of(conn, "groups")? } else { HashMap::new() };
    let mut list: Vec<(bool, String, Value)> = vec![];
    for (id, mut g) in docs_of(conn, "giveaways")? {
        let open = g.get("open").and_then(Value::as_bool).unwrap_or(false);
        let paused = g.get("status").and_then(Value::as_str) == Some("paused");
        if !open && !paused {
            continue;
        }
        let resolved: Vec<Value> = g
            .get("gifts")
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .filter_map(|gid| {
                        let gift = gifts.get(gid)?;
                        let group = text(gift, "group").and_then(|gr| groups.get(&gr)).and_then(|gr| text(gr, "name"));
                        Some(serde_json::json!({
                            "id": gid,
                            "title": text(gift, "title"),
                            "image": text(gift, "image").filter(|i| i.starts_with("data:image/") || i.starts_with("https://")),
                            "gameName": text(gift, "gameName"),
                            "group": group,
                        }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let detected = text(&g, "detectedAt").unwrap_or_default();
        g["id"] = Value::String(id);
        g["gifts"] = Value::Array(resolved);
        g["status"] = Value::String(if open { "open" } else { "paused" }.into());
        list.push((open, detected, g));
    }
    list.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    Ok(list.into_iter().map(|(_, _, g)| g).collect())
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
