//! Actions à distance (0.8) : un téléphone associé demande à son application d'agir : acheter
//! des tickets StreamElements, vérifier ou installer une mise à jour (0.8.2). La demande est gardée ici et annoncée au PC (flux
//! d'événements, « action ») ; le PC l'exécute s'il l'autorise, puis rend le résultat, annoncé au
//! téléphone. Le serveur ne fait que transmettre : il ne détient aucun jeton Twitch ni
//! StreamElements.

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::Json;
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mobile::Notice;
use crate::{now_text, with_db, Access, ApiError, Shared};

/// Demande non prise en charge par le PC dans ce délai : abandonnée (PC éteint)
const PENDING_MINUTES: i64 = 5;
/// Demandes en attente par appareil, au plus
const MAX_PENDING: i64 = 5;
/// Demandes gardées (et listées) par appareil
const KEEP: i64 = 20;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAction {
    kind: String,
    /// Achat de tickets : compte (pseudo), chaîne, identifiant du giveaway StreamElements, nombre
    #[serde(default)]
    account: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    giveaway: String,
    #[serde(default)]
    tickets: i64,
    /// Giveaway de l'application (identifiant de synchronisation) : masquer, journal, conditions
    #[serde(default)]
    item: String,
    /// Masquer (true) ou ne plus masquer (false)
    #[serde(default)]
    hidden: bool,
    /// Conditions du giveaway (mêmes champs que sa fiche synchronisée)
    #[serde(default)]
    conditions: Option<Value>,
    /// Ordre des chaînes de la page Points de chaîne d'un compte (pseudos)
    #[serde(default)]
    order: Vec<String>,
    /// Réglage d'une chaîne StreamElements : « chat » (présence dans le chat) ou « autoBuy »
    #[serde(default)]
    option: String,
    #[serde(default)]
    enabled: bool,
    /// Solde maximum d'une chaîne Points (absent ou 0 : pas de limite)
    #[serde(default)]
    max: Option<i64>,
}

/// Identifiant de synchronisation (uuid) d'un document de l'application
fn sync_id_ok(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Conditions d'un giveaway : booléens ou null, nombres ou null ; rien d'autre
fn conditions_of(v: &Value) -> Result<Value, ApiError> {
    let bad = || ApiError::BadRequest("Conditions invalides".into());
    let obj = v.as_object().ok_or_else(bad)?;
    let mut out = serde_json::Map::new();
    for (key, value) in obj {
        match key.as_str() {
            "requireFollow" | "requirePresence" | "requireDiscord" | "requireGreeting" | "canGift" => {
                if !(value.is_null() || value.is_boolean()) {
                    return Err(bad());
                }
            }
            "followMinSecs" | "minChatMinutes" => {
                if !(value.is_null() || value.as_i64().is_some_and(|n| (0..=315_360_000).contains(&n))) {
                    return Err(bad());
                }
            }
            _ => return Err(bad()),
        }
        out.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(out))
}

/// Contenu vérifié d'une demande, selon son type
fn payload_of(body: &NewAction) -> Result<Value, ApiError> {
    match body.kind.as_str() {
        // Mise à jour de l'application : vérifier, ou installer puis redémarrer
        "update_check" | "update_install" => Ok(serde_json::json!({})),
        // Giveaway (0.8.10) : masquer ou non, journal du chat, conditions
        "giveaway_hide" | "giveaway_chat_log" | "giveaway_conditions" => {
            if !sync_id_ok(&body.item) {
                return Err(ApiError::BadRequest("Giveaway invalide".into()));
            }
            Ok(match body.kind.as_str() {
                "giveaway_hide" => serde_json::json!({ "item": body.item, "hidden": body.hidden }),
                "giveaway_conditions" => {
                    let conditions = conditions_of(body.conditions.as_ref().ok_or_else(|| ApiError::BadRequest("Conditions manquantes".into()))?)?;
                    serde_json::json!({ "item": body.item, "conditions": conditions })
                }
                _ => serde_json::json!({ "item": body.item }),
            })
        }
        // Ordre des chaînes de la page Points de chaîne d'un compte (0.8.10)
        "points_order" => {
            let account = body.account.trim().to_lowercase();
            let order: Vec<String> = body.order.iter().map(|l| l.trim().to_lowercase()).collect();
            if !login_ok(&account) || order.is_empty() || order.len() > 200 || !order.iter().all(|l| login_ok(l)) {
                return Err(ApiError::BadRequest("Ordre invalide".into()));
            }
            Ok(serde_json::json!({ "account": account, "order": order }))
        }
        // Présence dans le chat ou achat automatique d'une chaîne StreamElements (0.8.10)
        "se_option" => {
            let account = body.account.trim().to_lowercase();
            let channel = body.channel.trim().to_lowercase();
            if !login_ok(&account) || !login_ok(&channel) || !matches!(body.option.as_str(), "chat" | "autoBuy") {
                return Err(ApiError::BadRequest("Réglage invalide".into()));
            }
            Ok(serde_json::json!({ "account": account, "channel": channel, "option": body.option, "enabled": body.enabled }))
        }
        // Giveaway (0.8.11) : participer, suivre la chaîne, dire bonjour ; réponse au gain à annuler
        "giveaway_participate" | "giveaway_follow" | "giveaway_hello" | "win_reply_cancel" => {
            if !sync_id_ok(&body.item) {
                return Err(ApiError::BadRequest("Demande invalide".into()));
            }
            Ok(serde_json::json!({ "item": body.item }))
        }
        // Application du PC en pause ou relancée (0.8.11)
        "app_pause" => Ok(serde_json::json!({ "enabled": body.enabled })),
        // Compte (0.8.11) : récolte ou bonus auto d'une chaîne Points, solde maximum, campagne de
        // drops ignorée ou rétablie, drops relus
        "points_option" | "points_max" | "drop_campaign_ignore" | "drops_refresh" => {
            let account = body.account.trim().to_lowercase();
            if !login_ok(&account) {
                return Err(ApiError::BadRequest("Compte invalide".into()));
            }
            let channel = body.channel.trim().to_lowercase();
            match body.kind.as_str() {
                "points_option" => {
                    if !login_ok(&channel) || !matches!(body.option.as_str(), "harvest" | "bonus") {
                        return Err(ApiError::BadRequest("Réglage invalide".into()));
                    }
                    Ok(serde_json::json!({ "account": account, "channel": channel, "option": body.option, "enabled": body.enabled }))
                }
                "points_max" => {
                    if !login_ok(&channel) || body.max.is_some_and(|m| !(0..=2_000_000_000).contains(&m)) {
                        return Err(ApiError::BadRequest("Solde maximum invalide".into()));
                    }
                    Ok(serde_json::json!({ "account": account, "channel": channel, "max": body.max }))
                }
                "drop_campaign_ignore" => {
                    if !sync_id_ok(&body.item) {
                        return Err(ApiError::BadRequest("Campagne invalide".into()));
                    }
                    Ok(serde_json::json!({ "account": account, "item": body.item, "enabled": body.enabled }))
                }
                _ => Ok(serde_json::json!({ "account": account })),
            }
        }
        // Chaîne du scanner à surveiller (0.8.10)
        "channel_monitor" => {
            let channel = body.channel.trim().to_lowercase();
            if !login_ok(&channel) {
                return Err(ApiError::BadRequest("Chaîne invalide".into()));
            }
            Ok(serde_json::json!({ "channel": channel }))
        }
        "se_buy" => {
            let account = body.account.trim().to_lowercase();
            let channel = body.channel.trim().to_lowercase();
            if !login_ok(&account) || !login_ok(&channel) {
                return Err(ApiError::BadRequest("Compte ou chaîne invalide".into()));
            }
            if body.giveaway.is_empty() || body.giveaway.len() > 64 || !body.giveaway.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(ApiError::BadRequest("Giveaway invalide".into()));
            }
            if !(1..=100_000).contains(&body.tickets) {
                return Err(ApiError::BadRequest("Nombre de tickets invalide".into()));
            }
            Ok(serde_json::json!({ "account": account, "channel": channel, "giveaway": body.giveaway, "tickets": body.tickets }))
        }
        _ => Err(ApiError::BadRequest("Action inconnue".into())),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionView {
    id: String,
    kind: String,
    payload: Value,
    /// « pending », « done » ou « failed »
    status: String,
    message: Option<String>,
    /// Résultat détaillé rendu par le PC (journal du chat…), 0.8.10
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    created_at: String,
    updated_at: String,
}

fn random_id() -> String {
    let mut buf = [0u8; 12];
    rand::rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn login_ok(s: &str) -> bool {
    (1..=25).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Demandes en attente trop anciennes : abandonnées (le PC ne les exécutera plus)
fn expire(conn: &Connection) -> rusqlite::Result<()> {
    let before = (chrono::Utc::now() - chrono::Duration::minutes(PENDING_MINUTES)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    conn.execute(
        "UPDATE actions SET status = 'failed', message = 'Le PC n''a pas répondu à temps (éteint ou hors ligne)', updated_at = ?2
         WHERE status = 'pending' AND created_at < ?1",
        params![before, now_text()],
    )?;
    Ok(())
}

fn view_of(r: &rusqlite::Row) -> rusqlite::Result<ActionView> {
    Ok(ActionView {
        id: r.get(0)?,
        kind: r.get(1)?,
        payload: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null),
        status: r.get(3)?,
        message: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        data: r.get::<_, Option<String>>(7)?.and_then(|d| serde_json::from_str(&d).ok()),
    })
}

const SELECT: &str = "SELECT id, kind, payload, status, message, created_at, updated_at, data FROM actions";
/// Résultat détaillé d'une demande, au plus (journal du chat d'un giveaway)
const MAX_DATA: usize = 512 * 1024;

/// `POST /api/v1/app/actions` (appareil associé) : nouvelle demande pour son application
pub async fn create(State(server): State<Shared>, Extension(access): Extension<Access>, Json(body): Json<NewAction>) -> Result<Json<ActionView>, ApiError> {
    let Access::Device { id: device, instance } = access else {
        return Err(ApiError::BadRequest("Réservé aux appareils associés".into()));
    };
    let payload = payload_of(&body)?.to_string();
    let target = instance.clone();
    let view = with_db(&server, move |conn| {
        expire(conn)?;
        let pending: i64 =
            conn.query_row("SELECT COUNT(*) FROM actions WHERE device = ?1 AND status = 'pending'", [&device], |r| r.get(0))?;
        if pending >= MAX_PENDING {
            return Err(ApiError::BadRequest("Trop de demandes en attente : attends la réponse du PC".into()));
        }
        let id = random_id();
        let at = now_text();
        conn.execute(
            "INSERT INTO actions (id, instance, device, kind, payload, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?6)",
            params![id, instance, device, body.kind, payload, at],
        )?;
        // Seules les KEEP dernières demandes de l'appareil restent
        conn.execute(
            "DELETE FROM actions WHERE device = ?1 AND id NOT IN (SELECT id FROM actions WHERE device = ?1 ORDER BY created_at DESC LIMIT ?2)",
            params![device, KEEP],
        )?;
        Ok(conn.query_row(&format!("{SELECT} WHERE id = ?1"), [&id], view_of)?)
    })
    .await?;
    let _ = server.notices.send(Notice::Action(target));
    Ok(Json(view))
}

/// `GET /api/v1/app/actions` (appareil associé) : ses dernières demandes et leur résultat
pub async fn mine(State(server): State<Shared>, Extension(access): Extension<Access>) -> Result<Json<Vec<ActionView>>, ApiError> {
    let Access::Device { id: device, .. } = access else {
        return Err(ApiError::BadRequest("Réservé aux appareils associés".into()));
    };
    let list = with_db(&server, move |conn| {
        expire(conn)?;
        let mut stmt = conn.prepare(&format!("{SELECT} WHERE device = ?1 ORDER BY created_at DESC LIMIT ?2"))?;
        let rows = stmt.query_map(params![device, KEEP], view_of)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await?;
    Ok(Json(list))
}

/// `GET /api/v1/instances/{id}/actions` (application) : demandes en attente de ses téléphones
pub async fn pending(State(server): State<Shared>, Path(instance): Path<String>) -> Result<Json<Vec<ActionView>>, ApiError> {
    if !crate::mobile::valid_instance_id(&instance) {
        return Err(ApiError::BadRequest("Identifiant d'application invalide".into()));
    }
    let list = with_db(&server, move |conn| {
        expire(conn)?;
        let mut stmt = conn.prepare(&format!("{SELECT} WHERE instance = ?1 AND status = 'pending' ORDER BY created_at"))?;
        let rows = stmt.query_map([instance], view_of)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    .await?;
    Ok(Json(list))
}

#[derive(Debug, Deserialize)]
pub struct ActionResult {
    ok: bool,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    data: Option<Value>,
}

/// `PUT /api/v1/instances/{id}/actions/{action}` (application) : résultat d'une demande, annoncé
/// au téléphone qui l'a faite
pub async fn finish(
    State(server): State<Shared>,
    Path((instance, action)): Path<(String, String)>,
    Json(body): Json<ActionResult>,
) -> Result<StatusCode, ApiError> {
    if !crate::mobile::valid_instance_id(&instance) {
        return Err(ApiError::BadRequest("Identifiant d'application invalide".into()));
    }
    let message: Option<String> = body.message.map(|m| m.chars().take(300).collect());
    let data = body.data.map(|d| d.to_string());
    if data.as_ref().is_some_and(|d| d.len() > MAX_DATA) {
        return Err(ApiError::BadRequest("Résultat trop volumineux".into()));
    }
    let device = with_db(&server, move |conn| {
        let device: Option<String> = conn
            .query_row("SELECT device FROM actions WHERE id = ?1 AND instance = ?2 AND status = 'pending'", params![action, instance], |r| {
                r.get(0)
            })
            .optional()?;
        if device.is_some() {
            conn.execute(
                "UPDATE actions SET status = ?2, message = ?3, updated_at = ?4, data = ?5 WHERE id = ?1",
                params![action, if body.ok { "done" } else { "failed" }, message, now_text(), data],
            )?;
        }
        Ok(device)
    })
    .await?;
    let Some(device) = device else {
        return Err(ApiError::NotFound("Demande inconnue, déjà traitée ou expirée".into()));
    };
    let _ = server.notices.send(Notice::ActionDone(device));
    Ok(StatusCode::NO_CONTENT)
}
