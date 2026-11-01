//! Serveur de synchronisation de Twitch Desk.
//!
//! Chaque application envoie ses modifications (ajout, modification, suppression) et récupère
//! celles des autres depuis son dernier passage. Les données sont des documents JSON rangés
//! par collection ; la dernière modification reçue l'emporte. Chaque modification est gardée
//! dans un historique.
//!
//! Configuration (variables d'environnement) :
//! - `SYNC_CODE` (obligatoire) : code à communiquer aux utilisateurs, 8 caractères au moins
//! - `SYNC_COLLECTIONS` : collections synchronisées, séparées par des virgules
//!   (défaut : toutes : groups, gifts, giveaways, channels, streamers)
//! - `SYNC_DB` : fichier SQLite (défaut : /data/sync.db)
//! - `SYNC_PORT` : port d'écoute (défaut : 8787)
//! - `SYNC_HISTORY_DAYS` : durée de conservation de l'historique (défaut : 90 jours)

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Collections connues, dans l'ordre où les appliquer (les groupes avant les cadeaux, etc.)
const COLLECTIONS: &[&str] = &["groups", "gifts", "giveaways", "channels", "streamers"];
/// Un document : une image de cadeau en data URL fait 2 Mo au plus
const MAX_DOCUMENT: usize = 4 * 1024 * 1024;
const MAX_BODY: usize = 32 * 1024 * 1024;
const MAX_PAGE: usize = 500;

struct Server {
    code: String,
    collections: BTreeSet<String>,
    db: Mutex<Connection>,
}

type Shared = Arc<Server>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Info {
    name: &'static str,
    version: &'static str,
    /// Collections synchronisées par ce serveur
    collections: Vec<String>,
    /// Numéro de la dernière modification
    seq: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Change {
    seq: i64,
    collection: String,
    id: String,
    data: Option<serde_json::Value>,
    deleted: bool,
    updated_at: String,
    author: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChangesPage {
    changes: Vec<Change>,
    /// Curseur à renvoyer au prochain appel
    seq: i64,
    /// D'autres modifications attendent : rappeler tout de suite
    more: bool,
}

#[derive(Debug, Deserialize)]
struct Since {
    #[serde(default)]
    since: i64,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct IncomingChange {
    collection: String,
    id: String,
    #[serde(default)]
    data: Option<serde_json::Value>,
    #[serde(default)]
    deleted: bool,
}

#[derive(Debug, Deserialize)]
struct Push {
    /// Qui envoie (pseudo affiché dans l'historique)
    #[serde(default)]
    author: Option<String>,
    changes: Vec<IncomingChange>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PushResult {
    /// Modifications enregistrées (les envois identiques au contenu actuel sont ignorés)
    accepted: usize,
    seq: i64,
}

enum ApiError {
    Unauthorized,
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Code de synchronisation incorrect".to_owned()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Internal(m) => {
                tracing::error!("{m}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Erreur du serveur".to_owned())
            }
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError::Internal(format!("base de données : {e}"))
    }
}

/// Comparaison en temps constant, pour ne rien laisser deviner du code
fn same_code(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as usize;
    }
    diff == 0
}

fn authorize(server: &Server, headers: &HeaderMap) -> Result<(), ApiError> {
    let given = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if same_code(given.trim(), &server.code) {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

fn last_seq(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM history", [], |r| r.get(0))
}

async fn info(State(server): State<Shared>, headers: HeaderMap) -> Result<Json<Info>, ApiError> {
    authorize(&server, &headers)?;
    let seq = last_seq(&server.db.lock().unwrap())?;
    Ok(Json(Info {
        name: "twitch-desk-sync",
        version: env!("CARGO_PKG_VERSION"),
        collections: COLLECTIONS.iter().filter(|c| server.collections.contains(**c)).map(|c| c.to_string()).collect(),
        seq,
    }))
}

async fn changes(
    State(server): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<Since>,
) -> Result<Json<ChangesPage>, ApiError> {
    authorize(&server, &headers)?;
    let limit = q.limit.unwrap_or(200).clamp(1, MAX_PAGE);
    let conn = server.db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT seq, collection, id, data, deleted, updated_at, author FROM docs WHERE seq > ?1 ORDER BY seq LIMIT ?2",
    )?;
    let mut list: Vec<Change> = stmt
        .query_map(params![q.since, limit as i64 + 1], |r| {
            let data: Option<String> = r.get(3)?;
            Ok(Change {
                seq: r.get(0)?,
                collection: r.get(1)?,
                id: r.get(2)?,
                data: data.and_then(|d| serde_json::from_str(&d).ok()),
                deleted: r.get(4)?,
                updated_at: r.get(5)?,
                author: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    // Une collection désactivée depuis n'est plus envoyée
    let more = list.len() > limit;
    list.truncate(limit);
    let seq = match list.last() {
        Some(c) => c.seq,
        None => q.since.max(0).min(last_seq(&conn)?),
    };
    list.retain(|c| server.collections.contains(&c.collection));
    Ok(Json(ChangesPage { changes: list, seq, more }))
}

async fn push(State(server): State<Shared>, headers: HeaderMap, Json(body): Json<Push>) -> Result<Json<PushResult>, ApiError> {
    authorize(&server, &headers)?;
    for c in &body.changes {
        if !server.collections.contains(&c.collection) {
            return Err(ApiError::BadRequest(format!("Collection non synchronisée par ce serveur : {}", c.collection)));
        }
        if c.id.trim().is_empty() || c.id.len() > 200 {
            return Err(ApiError::BadRequest("Identifiant de document invalide".into()));
        }
        if !c.deleted && !c.data.as_ref().is_some_and(|d| d.is_object()) {
            return Err(ApiError::BadRequest(format!("Document sans contenu : {}/{}", c.collection, c.id)));
        }
    }
    let author = body.author.map(|a| a.chars().take(60).collect::<String>());
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut conn = server.db.lock().unwrap();
    let tx = conn.transaction()?;
    let mut accepted = 0;
    for c in body.changes {
        let data = if c.deleted { None } else { c.data.map(|d| d.to_string()) };
        if data.as_ref().is_some_and(|d| d.len() > MAX_DOCUMENT) {
            return Err(ApiError::BadRequest(format!("Document trop volumineux : {}/{}", c.collection, c.id)));
        }
        let current: Option<(Option<String>, bool)> = tx
            .query_row(
                "SELECT data, deleted FROM docs WHERE collection = ?1 AND id = ?2",
                params![c.collection, c.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        // Rien de nouveau : pas de modification enregistrée (évite les allers-retours)
        let unchanged = match &current {
            Some((d, deleted)) => *deleted == c.deleted && (c.deleted || d == &data),
            None => c.deleted,
        };
        if unchanged {
            continue;
        }
        tx.execute(
            "INSERT INTO history (collection, id, data, deleted, at, author) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![c.collection, c.id, data, c.deleted, now, author],
        )?;
        let seq = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO docs (collection, id, data, deleted, seq, updated_at, author) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(collection, id) DO UPDATE SET data = excluded.data, deleted = excluded.deleted,
                 seq = excluded.seq, updated_at = excluded.updated_at, author = excluded.author",
            params![c.collection, c.id, data, c.deleted, seq, now, author],
        )?;
        accepted += 1;
    }
    tx.commit()?;
    let seq = last_seq(&conn)?;
    if accepted > 0 {
        tracing::info!("{accepted} modification(s) de {}", author.as_deref().unwrap_or("?"));
    }
    Ok(Json(PushResult { accepted, seq }))
}

fn open_db(path: &str) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS history (
             seq INTEGER PRIMARY KEY AUTOINCREMENT,
             collection TEXT NOT NULL,
             id TEXT NOT NULL,
             data TEXT,
             deleted INTEGER NOT NULL,
             at TEXT NOT NULL,
             author TEXT
         );
         CREATE TABLE IF NOT EXISTS docs (
             collection TEXT NOT NULL,
             id TEXT NOT NULL,
             data TEXT,
             deleted INTEGER NOT NULL,
             seq INTEGER NOT NULL,
             updated_at TEXT NOT NULL,
             author TEXT,
             PRIMARY KEY (collection, id)
         );
         CREATE INDEX IF NOT EXISTS docs_seq ON docs (seq);",
    )?;
    Ok(conn)
}

/// Historique plus ancien que `days` jours effacé (l'état actuel des documents est gardé)
fn prune_history(conn: &Connection, days: i64) -> rusqlite::Result<usize> {
    let before = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // La dernière modification de chaque document reste : c'est elle qui porte le curseur
    conn.execute("DELETE FROM history WHERE at < ?1 AND seq NOT IN (SELECT seq FROM docs)", [before])
}

fn parse_collections(value: Option<String>) -> Result<BTreeSet<String>, String> {
    let Some(value) = value.filter(|v| !v.trim().is_empty()) else {
        return Ok(COLLECTIONS.iter().map(|c| c.to_string()).collect());
    };
    let mut set = BTreeSet::new();
    for c in value.split(',').map(|c| c.trim().to_lowercase()).filter(|c| !c.is_empty()) {
        if !COLLECTIONS.contains(&c.as_str()) {
            return Err(format!("SYNC_COLLECTIONS : collection inconnue « {c} » (connues : {})", COLLECTIONS.join(", ")));
        }
        set.insert(c);
    }
    Ok(set)
}

/// `twitch-desk-sync health` : vérification de santé pour Docker (sans curl dans l'image)
fn health(port: u16) -> std::process::ExitCode {
    use std::io::{Read, Write};
    let ok = (|| -> std::io::Result<bool> {
        let mut s = std::net::TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(3))?;
        s.set_read_timeout(Some(Duration::from_secs(3)))?;
        s.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
        let mut buf = String::new();
        s.read_to_string(&mut buf)?;
        Ok(buf.starts_with("HTTP/1.0 200") || buf.starts_with("HTTP/1.1 200"))
    })()
    .unwrap_or(false);
    if ok {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

fn app(server: Shared) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/v1/info", get(info))
        .route("/api/v1/changes", get(changes).post(push))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(server)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let port: u16 = std::env::var("SYNC_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8787);
    if std::env::args().nth(1).as_deref() == Some("health") {
        return health(port);
    }

    let code = std::env::var("SYNC_CODE").unwrap_or_default().trim().to_owned();
    if code.chars().count() < 8 {
        tracing::error!("SYNC_CODE doit contenir au moins 8 caractères : c'est le code à donner aux utilisateurs");
        return std::process::ExitCode::FAILURE;
    }
    let collections = match parse_collections(std::env::var("SYNC_COLLECTIONS").ok()) {
        Ok(c) if !c.is_empty() => c,
        Ok(_) => {
            tracing::error!("SYNC_COLLECTIONS ne contient aucune collection");
            return std::process::ExitCode::FAILURE;
        }
        Err(e) => {
            tracing::error!("{e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let path = std::env::var("SYNC_DB").unwrap_or_else(|_| "/data/sync.db".into());
    let conn = match open_db(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("ouverture de {path} : {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let days: i64 = std::env::var("SYNC_HISTORY_DAYS").ok().and_then(|d| d.parse().ok()).unwrap_or(90).max(1);
    let server = Arc::new(Server { code, collections, db: Mutex::new(conn) });

    // Ménage quotidien de l'historique
    let pruner = server.clone();
    tokio::spawn(async move {
        loop {
            match prune_history(&pruner.db.lock().unwrap(), days) {
                Ok(n) if n > 0 => tracing::info!("historique : {n} entrée(s) de plus de {days} jours effacée(s)"),
                Ok(_) => {}
                Err(e) => tracing::warn!("ménage de l'historique : {e}"),
            }
            tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
        }
    });

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("écoute sur {addr} : {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    tracing::info!(
        "Twitch Desk Sync {} sur {addr}, base {path}, collections : {}",
        env!("CARGO_PKG_VERSION"),
        server.collections.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
            tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        let _ = ctrl_c.await;
    };
    if let Err(e) = axum::serve(listener, app(server)).with_graceful_shutdown(shutdown).await {
        tracing::error!("serveur : {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(collections: &[&str]) -> Shared {
        Arc::new(Server {
            code: "code-secret".into(),
            collections: collections.iter().map(|c| c.to_string()).collect(),
            db: Mutex::new(open_db(":memory:").unwrap()),
        })
    }

    fn auth() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("authorization", "Bearer code-secret".parse().unwrap());
        h
    }

    fn change(collection: &str, id: &str, data: Option<serde_json::Value>) -> IncomingChange {
        IncomingChange { collection: collection.into(), id: id.into(), deleted: data.is_none(), data }
    }

    async fn pull(s: &Shared, since: i64) -> ChangesPage {
        let Json(page) = changes(State(s.clone()), auth(), Query(Since { since, limit: None })).await.ok().unwrap();
        page
    }

    #[test]
    fn code_comparison() {
        assert!(same_code("abcdefgh", "abcdefgh"));
        assert!(!same_code("abcdefgh", "abcdefgi"));
        assert!(!same_code("abcdefgh", "abcdefg"));
        assert!(!same_code("", "abcdefgh"));
    }

    #[test]
    fn collections_from_env() {
        assert_eq!(parse_collections(None).unwrap().len(), COLLECTIONS.len());
        assert_eq!(parse_collections(Some(" Gifts, groups ".into())).unwrap().into_iter().collect::<Vec<_>>(), vec!["gifts", "groups"]);
        assert!(parse_collections(Some("gifts,accounts".into())).is_err());
    }

    #[tokio::test]
    async fn push_pull_and_last_write_wins() {
        let s = server(&["gifts", "groups"]);
        assert!(info(State(s.clone()), HeaderMap::new()).await.is_err());

        let body = Push {
            author: Some("alice".into()),
            changes: vec![
                change("groups", "g1", Some(serde_json::json!({"name": "Voiles"}))),
                change("gifts", "a", Some(serde_json::json!({"title": "Obsidian Sails"}))),
            ],
        };
        let Json(r) = push(State(s.clone()), auth(), Json(body)).await.ok().unwrap();
        assert_eq!((r.accepted, r.seq), (2, 2));

        // Le même contenu renvoyé n'est pas une modification
        let again = Push { author: None, changes: vec![change("gifts", "a", Some(serde_json::json!({"title": "Obsidian Sails"})))] };
        let Json(r) = push(State(s.clone()), auth(), Json(again)).await.ok().unwrap();
        assert_eq!(r.accepted, 0);

        let page = pull(&s, 0).await;
        assert_eq!(page.changes.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["g1", "a"]);
        assert_eq!(page.seq, 2);
        assert!(pull(&s, 2).await.changes.is_empty());

        // Modification puis suppression : seule la dernière version est renvoyée
        let edit = Push { author: Some("bob".into()), changes: vec![change("gifts", "a", Some(serde_json::json!({"title": "Voiles"})))] };
        let _ = push(State(s.clone()), auth(), Json(edit)).await.ok().unwrap();
        let delete = Push { author: Some("bob".into()), changes: vec![change("gifts", "a", None)] };
        let _ = push(State(s.clone()), auth(), Json(delete)).await.ok().unwrap();
        let page = pull(&s, 2).await;
        assert_eq!(page.changes.len(), 1);
        assert!(page.changes[0].deleted && page.changes[0].data.is_none());
        assert_eq!(page.seq, 4);

        // Collection non synchronisée refusée
        let other = Push { author: None, changes: vec![change("channels", "x", Some(serde_json::json!({})))] };
        assert!(push(State(s.clone()), auth(), Json(other)).await.is_err());
    }

    #[tokio::test]
    async fn paging() {
        let s = server(&["gifts"]);
        let many = (0..250).map(|i| change("gifts", &format!("g{i}"), Some(serde_json::json!({"n": i})))).collect();
        let _ = push(State(s.clone()), auth(), Json(Push { author: None, changes: many })).await.ok().unwrap();
        let first = pull(&s, 0).await;
        assert_eq!((first.changes.len(), first.more), (200, true));
        let second = pull(&s, first.seq).await;
        assert_eq!((second.changes.len(), second.more, second.seq), (50, false, 250));
    }
}
