//! Actions à distance (0.8) : un téléphone associé demande à son application d'agir (aujourd'hui :
//! acheter des tickets StreamElements). La demande est gardée ici et annoncée au PC (flux
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
    account: String,
    channel: String,
    giveaway: String,
    tickets: i64,
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
    })
}

const SELECT: &str = "SELECT id, kind, payload, status, message, created_at, updated_at FROM actions";

/// `POST /api/v1/app/actions` (appareil associé) : nouvelle demande pour son application
pub async fn create(State(server): State<Shared>, Extension(access): Extension<Access>, Json(body): Json<NewAction>) -> Result<Json<ActionView>, ApiError> {
    let Access::Device { id: device, instance } = access else {
        return Err(ApiError::BadRequest("Réservé aux appareils associés".into()));
    };
    if body.kind != "se_buy" {
        return Err(ApiError::BadRequest("Action inconnue".into()));
    }
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
    let payload = serde_json::json!({ "account": account, "channel": channel, "giveaway": body.giveaway, "tickets": body.tickets }).to_string();
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
    let device = with_db(&server, move |conn| {
        let device: Option<String> = conn
            .query_row("SELECT device FROM actions WHERE id = ?1 AND instance = ?2 AND status = 'pending'", params![action, instance], |r| {
                r.get(0)
            })
            .optional()?;
        if device.is_some() {
            conn.execute(
                "UPDATE actions SET status = ?2, message = ?3, updated_at = ?4 WHERE id = ?1",
                params![action, if body.ok { "done" } else { "failed" }, message, now_text()],
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
