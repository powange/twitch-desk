//! Notifications push (0.7) : un appareil associé abonne son navigateur (Web Push) aux
//! alertes de son choix ; l'application du PC envoie chaque alerte au serveur, qui la
//! transmet aux téléphones abonnés, même web app fermée.
//!
//! Clé VAPID tirée au premier démarrage et gardée en base (table `meta`) : la changer
//! désabonnerait tous les téléphones. Le contenu est chiffré pour chaque abonnement
//! (aes128gcm) : le service de push du navigateur ne le lit pas.

use std::time::Duration;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::Json;
use base64ct::{Base64UrlUnpadded, Encoding};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike as _, ES256KeyPair};
use web_push_native::{p256, Auth, WebPushBuilder};

use crate::{now_text, with_db, Access, ApiError, Shared};

/// Alertes de l'application (clés de `Alert` côté application)
pub const ALERTS: [&str; 6] = ["giveaway", "win", "mention", "reminder", "scanner", "drop"];
/// Durée de vie d'une notification chez le service de push (téléphone éteint plus longtemps :
/// perdue, elle ne serait plus d'actualité) ; aussi celle de la signature VAPID
const TTL: Duration = Duration::from_secs(3600);
const SEND_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_ENDPOINT: usize = 1024;
const MAX_TITLE: usize = 120;
const MAX_BODY: usize = 500;
/// Contact donné aux services de push (`SYNC_PUSH_CONTACT`), en cas d'abus de leur côté
pub const DEFAULT_CONTACT: &str = "https://github.com/powange/twitch-desk";

pub struct Push {
    key: ES256KeyPair,
    /// Clé publique (non compressée, base64url) : `applicationServerKey` du navigateur
    pub public_key: String,
    pub contact: String,
    http: reqwest::Client,
}

impl Push {
    /// Clé VAPID de la base, tirée et enregistrée la première fois
    pub fn load(conn: &Connection, contact: String) -> rusqlite::Result<Self> {
        let saved: Option<String> = conn.query_row("SELECT value FROM meta WHERE key = 'vapid_key'", [], |r| r.get(0)).optional()?;
        let key = match saved.and_then(|k| Base64UrlUnpadded::decode_vec(&k).ok()).and_then(|b| ES256KeyPair::from_bytes(&b).ok()) {
            Some(key) => key,
            None => {
                let key = ES256KeyPair::generate();
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('vapid_key', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [Base64UrlUnpadded::encode_string(&key.to_bytes())],
                )?;
                tracing::info!("notifications push : clé VAPID créée");
                key
            }
        };
        let public_key = Base64UrlUnpadded::encode_string(&key.public_key().public_key().to_bytes_uncompressed());
        // Aucune redirection suivie : la requête reste chez le service de push
        let http = reqwest::Client::builder()
            .timeout(SEND_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("twitch-desk-sync/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("client HTTP");
        Ok(Push { key, public_key, contact, http })
    }
}

/// Abonnement d'un navigateur, tel que donné par `PushSubscription.toJSON()`
#[derive(Debug, Deserialize)]
pub struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}

#[derive(Debug, Deserialize)]
pub struct SubscribeBody {
    endpoint: String,
    keys: SubscriptionKeys,
    /// Alertes voulues (clés de ALERTS)
    events: Vec<String>,
}

/// Adresse d'un vrai service de push des navigateurs (Chrome et Android, Firefox, Safari, Edge),
/// en https sur le port par défaut : le serveur n'envoie rien ailleurs (ni vers son réseau, ni
/// vers un nom qui y mènerait)
fn valid_endpoint(endpoint: &str) -> bool {
    if endpoint.len() > MAX_ENDPOINT {
        return false;
    }
    let Ok(uri) = endpoint.parse::<axum::http::Uri>() else {
        return false;
    };
    let Some(host) = uri.host() else { return false };
    let host = host.to_ascii_lowercase();
    let known = matches!(host.as_str(), "fcm.googleapis.com" | "updates.push.services.mozilla.com" | "web.push.apple.com")
        || host.ends_with(".push.apple.com")
        || host.ends_with(".notify.windows.com");
    uri.scheme_str() == Some("https") && uri.port().is_none() && known && !endpoint.contains('@')
}

/// Clé publique (65 octets) et secret (16 octets) de l'abonnement
fn subscription_keys(keys: &SubscriptionKeys) -> Option<(p256::PublicKey, Auth)> {
    let public = Base64UrlUnpadded::decode_vec(keys.p256dh.trim_end_matches('=')).ok()?;
    let auth = Base64UrlUnpadded::decode_vec(keys.auth.trim_end_matches('=')).ok()?;
    if auth.len() != 16 {
        return None;
    }
    Some((p256::PublicKey::from_sec1_bytes(&public).ok()?, Auth::clone_from_slice(&auth)))
}

fn device_of(access: Access) -> Result<String, ApiError> {
    match access {
        Access::Device { id, .. } => Ok(id),
        Access::Full => Err(ApiError::BadRequest("Réservé aux appareils associés".into())),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushInfo {
    /// Clé publique VAPID du serveur
    public_key: String,
    /// Adresse de l'abonnement enregistré pour cet appareil (null : aucun)
    endpoint: Option<String>,
    /// Alertes choisies par cet appareil
    events: Vec<String>,
    /// Alertes possibles
    alerts: Vec<&'static str>,
}

fn info_of(conn: &Connection, server: &Shared, device: &str) -> rusqlite::Result<PushInfo> {
    let found: Option<(String, String)> = conn
        .query_row("SELECT endpoint, events FROM push_subscriptions WHERE device = ?1", [device], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let (endpoint, events) = match found {
        Some((endpoint, events)) => (Some(endpoint), serde_json::from_str(&events).unwrap_or_default()),
        None => (None, vec![]),
    };
    Ok(PushInfo { public_key: server.push.public_key.clone(), endpoint, events, alerts: ALERTS.to_vec() })
}

/// `GET /api/v1/app/push` (appareil associé) : clé du serveur et abonnement de cet appareil
pub async fn get_push(State(server): State<Shared>, Extension(access): Extension<Access>) -> Result<Json<PushInfo>, ApiError> {
    let device = device_of(access)?;
    let s = server.clone();
    Ok(Json(with_db(&server, move |conn| Ok(info_of(conn, &s, &device)?)).await?))
}

/// `PUT /api/v1/app/push` (appareil associé) : abonnement de ce navigateur et alertes voulues
/// (remplace le précédent : un abonnement par appareil)
pub async fn put_push(
    State(server): State<Shared>,
    Extension(access): Extension<Access>,
    Json(body): Json<SubscribeBody>,
) -> Result<Json<PushInfo>, ApiError> {
    let device = device_of(access)?;
    if !valid_endpoint(&body.endpoint) {
        return Err(ApiError::BadRequest("Adresse d'abonnement refusée".into()));
    }
    if subscription_keys(&body.keys).is_none() {
        return Err(ApiError::BadRequest("Clés d'abonnement invalides".into()));
    }
    let mut events: Vec<String> = body.events.into_iter().filter(|e| ALERTS.contains(&e.as_str())).collect();
    events.sort_unstable();
    events.dedup();
    let events = serde_json::to_string(&events).expect("liste");
    let s = server.clone();
    let (info, replaced) = with_db(&server, move |conn| {
        // Même navigateur associé de nouveau (nouveau QR code) : l'ancien appareil de cette
        // application, abonné à la même adresse, est remplacé (sinon chaque alerte arriverait deux
        // fois, et il compterait dans la limite d'appareils)
        let replaced: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT s.device FROM push_subscriptions s JOIN devices old ON old.id = s.device
                 WHERE s.endpoint = ?1 AND s.device <> ?2
                   AND old.instance = (SELECT instance FROM devices WHERE id = ?2)",
            )?;
            let rows = stmt.query_map(params![body.endpoint, device], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for old in &replaced {
            conn.execute("DELETE FROM push_subscriptions WHERE device = ?1", [old])?;
            conn.execute("DELETE FROM devices WHERE id = ?1", [old])?;
        }
        // Autre application (autre PC) sur le même navigateur : seulement l'abonnement en double
        conn.execute("DELETE FROM push_subscriptions WHERE endpoint = ?1 AND device <> ?2", params![body.endpoint, device])?;
        conn.execute(
            "INSERT INTO push_subscriptions (device, endpoint, p256dh, auth, events, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(device) DO UPDATE SET endpoint = excluded.endpoint, p256dh = excluded.p256dh, auth = excluded.auth,
             events = excluded.events",
            params![device, body.endpoint, body.keys.p256dh, body.keys.auth, events, now_text()],
        )?;
        Ok((info_of(conn, &s, &device)?, replaced))
    })
    .await?;
    for old in replaced {
        tracing::info!("appareil {old} remplacé (même navigateur associé de nouveau)");
        let _ = server.notices.send(crate::mobile::Notice::Revoked(old));
    }
    Ok(Json(info))
}

/// `DELETE /api/v1/app/push` (appareil associé) : plus de notifications sur cet appareil
pub async fn delete_push(State(server): State<Shared>, Extension(access): Extension<Access>) -> Result<StatusCode, ApiError> {
    let device = device_of(access)?;
    with_db(&server, move |conn| Ok(conn.execute("DELETE FROM push_subscriptions WHERE device = ?1", [device])?)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct NotifyBody {
    alert: String,
    title: String,
    #[serde(default)]
    body: String,
    /// Cible ouverte au clic sur le téléphone (0.8.13) : chaîne, ou compte pour un drop
    #[serde(default)]
    target: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NotifyResult {
    /// Appareils à qui la notification part
    devices: usize,
}

/// Abonnement visé par une alerte
#[derive(Debug, Clone)]
pub struct Target {
    device: String,
    endpoint: String,
    p256dh: String,
    auth: String,
}

/// Abonnements des appareils de l'application qui ont choisi cette alerte
pub fn targets(conn: &Connection, instance: &str, alert: &str) -> rusqlite::Result<Vec<Target>> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.device, s.endpoint, s.p256dh, s.auth, s.events FROM push_subscriptions s
         JOIN devices d ON d.id = s.device WHERE d.instance = ?1",
    )?;
    let rows = stmt.query_map([instance], |r| {
        Ok((Target { device: r.get(0)?, endpoint: r.get(1)?, p256dh: r.get(2)?, auth: r.get(3)? }, r.get::<_, String>(4)?))
    })?;
    let mut list = vec![];
    for row in rows {
        let (target, events) = row?;
        if serde_json::from_str::<Vec<String>>(&events).unwrap_or_default().iter().any(|e| e == alert) {
            list.push(target);
        }
    }
    Ok(list)
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

/// Requête chiffrée pour un abonnement (signée VAPID, à délivrer vite)
fn build_request(push: &Push, target: &Target, payload: &[u8]) -> Result<reqwest::Request, String> {
    let (public, auth) =
        subscription_keys(&SubscriptionKeys { p256dh: target.p256dh.clone(), auth: target.auth.clone() }).ok_or("clés invalides")?;
    let endpoint = target.endpoint.parse().map_err(|_| "adresse invalide")?;
    let mut request = WebPushBuilder::new(endpoint, public, auth)
        .with_valid_duration(TTL)
        .with_vapid(&push.key, &push.contact)
        .build(payload.to_vec())
        .map_err(|e| e.to_string())?;
    // Délivrée tout de suite, même téléphone en veille
    request.headers_mut().insert("urgency", axum::http::HeaderValue::from_static("high"));
    reqwest::Request::try_from(request).map_err(|e| e.to_string())
}

/// Envoie la notification à chaque abonnement ; abonnement expiré (404, 410) : effacé
async fn deliver(server: Shared, targets: Vec<Target>, payload: Vec<u8>) {
    let mut tasks = tokio::task::JoinSet::new();
    for target in targets {
        // Abonnement enregistré avant la liste des services connus : ignoré
        if !valid_endpoint(&target.endpoint) {
            tracing::warn!("notification pour l'appareil {} : service de push inconnu, ignorée", target.device);
            continue;
        }
        let request = match build_request(&server.push, &target, &payload) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("notification pour l'appareil {} : {e}", target.device);
                continue;
            }
        };
        let http = server.push.http.clone();
        tasks.spawn(async move { (target, http.execute(request).await) });
    }
    while let Some(Ok((target, result))) = tasks.join_next().await {
        match result {
            Ok(res) if res.status().is_success() => {}
            Ok(res) if matches!(res.status().as_u16(), 404 | 410) => {
                tracing::info!("abonnement push expiré pour l'appareil {} : effacé", target.device);
                let (device, endpoint) = (target.device, target.endpoint);
                let _ = with_db(&server, move |conn| {
                    Ok(conn.execute("DELETE FROM push_subscriptions WHERE device = ?1 AND endpoint = ?2", params![device, endpoint])?)
                })
                .await;
            }
            Ok(res) => tracing::warn!("notification pour l'appareil {} refusée ({})", target.device, res.status()),
            Err(e) => tracing::warn!("notification pour l'appareil {} : {e}", target.device),
        }
    }
}

/// `POST /api/v1/instances/{id}/notify` (application) : alerte transmise aux téléphones de
/// cette application qui l'ont choisie ; envoi en arrière-plan
pub async fn notify(
    State(server): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<NotifyBody>,
) -> Result<Json<NotifyResult>, ApiError> {
    if !crate::mobile::valid_instance_id(&id) {
        return Err(ApiError::BadRequest("Identifiant d'application invalide".into()));
    }
    let Some(alert) = ALERTS.iter().copied().find(|a| *a == body.alert) else {
        return Err(ApiError::BadRequest("Alerte inconnue".into()));
    };
    let title = clip(&body.title, MAX_TITLE);
    if title.is_empty() {
        return Err(ApiError::BadRequest("Titre manquant".into()));
    }
    let targets = with_db(&server, move |conn| Ok(targets(conn, &id, alert)?)).await?;
    let devices = targets.len();
    if devices > 0 {
        // Cible : un pseudo Twitch seulement (sinon ignorée)
        let target = body
            .target
            .map(|t| t.trim().to_lowercase())
            .filter(|t| (1..=25).contains(&t.len()) && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        let payload = serde_json::json!({ "alert": alert, "title": title, "body": clip(&body.body, MAX_BODY), "target": target }).to_string().into_bytes();
        tokio::spawn(deliver(server.clone(), targets, payload));
    }
    Ok(Json(NotifyResult { devices }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints() {
        assert!(valid_endpoint("https://fcm.googleapis.com/fcm/send/abc"));
        assert!(valid_endpoint("https://web.push.apple.com/QGx"));
        assert!(valid_endpoint("https://updates.push.services.mozilla.com/wpush/v2/x"));
        assert!(!valid_endpoint("http://fcm.googleapis.com/fcm/send/abc"));
        assert!(!valid_endpoint("https://127.0.0.1/x"));
        assert!(!valid_endpoint("https://[::1]/x"));
        assert!(!valid_endpoint("https://localhost/x"));
        assert!(!valid_endpoint("https://nas.local/x"));
        assert!(!valid_endpoint("https://sync/x"));
        assert!(valid_endpoint("https://wns2-par02p.notify.windows.com/w/?token=x"));
        assert!(!valid_endpoint("https://fcm.googleapis.com:8443/x"));
        assert!(!valid_endpoint("https://exemple.fr/x"));
        assert!(!valid_endpoint("https://fcm.googleapis.com.exemple.fr/x"));
        assert!(!valid_endpoint("https://notify.windows.com.exemple.fr/x"));
        assert!(!valid_endpoint("https://x@fcm.googleapis.com/x"));
        assert!(!valid_endpoint(&format!("https://a.b/{}", "x".repeat(MAX_ENDPOINT))));
    }

    #[test]
    fn clipped_text() {
        assert_eq!(clip("  court ", 10), "court");
        assert_eq!(clip("abcdefghijkl", 5), "abcd…");
    }

    #[test]
    fn payload_is_readable_by_the_phone_only() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)").unwrap();
        let push = Push::load(&conn, DEFAULT_CONTACT.into()).unwrap();
        // Même clé au redémarrage
        assert_eq!(Push::load(&conn, DEFAULT_CONTACT.into()).unwrap().public_key, push.public_key);
        // Navigateur simulé : sa clé privée et son secret
        let phone = p256::SecretKey::from_slice(&[7u8; 32]).unwrap();
        let auth = [3u8; 16];
        let target = Target {
            device: "d".into(),
            endpoint: "https://fcm.googleapis.com/fcm/send/sub/1".into(),
            p256dh: Base64UrlUnpadded::encode_string(phone.public_key().to_sec1_bytes().as_ref()),
            auth: Base64UrlUnpadded::encode_string(&auth),
        };
        let request = build_request(&push, &target, br#"{"title":"Tu as gagne"}"#).unwrap();
        assert_eq!(request.url().as_str(), "https://fcm.googleapis.com/fcm/send/sub/1");
        assert_eq!(request.headers()["urgency"], "high");
        assert_eq!(request.headers()["content-encoding"], "aes128gcm");
        let vapid = request.headers()["authorization"].to_str().unwrap();
        assert!(vapid.starts_with("vapid t=") && vapid.ends_with(&format!("k={}", push.public_key)));
        let body = request.body().and_then(|b| b.as_bytes()).unwrap().to_vec();
        let clear = web_push_native::decrypt(body, &phone, &Auth::clone_from_slice(&auth)).unwrap();
        assert_eq!(clear, br#"{"title":"Tu as gagne"}"#);
    }
}
