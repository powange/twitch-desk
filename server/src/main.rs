//! Serveur de synchronisation de Twitch Desk.
//!
//! Chaque application envoie ses modifications (ajout, modification, suppression) et récupère
//! celles des autres depuis son dernier passage. Les données sont des documents JSON rangés
//! par collection ; la dernière modification reçue l'emporte. Chaque modification est notée
//! dans un historique (qui, quand, quel document ; sans le contenu).
//!
//! Configuration (variables d'environnement) :
//! - `SYNC_CODE` (obligatoire) : code à communiquer aux utilisateurs, 8 caractères au moins
//! - `SYNC_COLLECTIONS` : collections synchronisées, séparées par des virgules
//!   (défaut : toutes : groups, gifts, giveaways, channels, streamers, detection)
//! - `SYNC_DB` : fichier SQLite (défaut : /data/sync.db)
//! - `SYNC_PORT` : port d'écoute (défaut : 8787)
//! - `SYNC_HISTORY_DAYS` : durée de conservation de l'historique (défaut : 90 jours)
//! - `SYNC_PROXY_HOPS` : proxys devant le serveur, pour lire l'adresse du client dans
//!   `X-Forwarded-For` (défaut : 1 ; 0 quand le serveur est joint directement)
//!
//! `GET /api/v1/events` (Server-Sent Events) annonce chaque nouvelle modification : les
//! applications abonnées relisent `/api/v1/changes` tout de suite, sans attendre leur passage.

use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, DefaultBodyLimit, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Semaphore};
use tokio_stream::wrappers::WatchStream;
use tokio_stream::StreamExt as _;
use tower::limit::GlobalConcurrencyLimitLayer;
use tower_http::timeout::TimeoutLayer;

/// Collections connues, dans l'ordre où les appliquer (les groupes avant les cadeaux, etc.)
const COLLECTIONS: &[&str] = &["groups", "gifts", "giveaways", "channels", "streamers", "detection"];
/// Un document : une image de cadeau en data URL fait 2 Mo au plus
const MAX_DOCUMENT: usize = 4 * 1024 * 1024;
/// Envoi : l'application regroupe jusqu'à 8 Mo de documents (PUSH_BATCH_BYTES), plus le
/// dernier ajouté (4 Mo au plus)
const MAX_BODY: usize = 16 * 1024 * 1024;
const MAX_PAGE: usize = 500;
/// Réception : page coupée à 16 Mo de documents (au moins un document par page)
const MAX_PAGE_BYTES: usize = 16 * 1024 * 1024;
/// Requêtes de l'API traitées en même temps (les suivantes attendent leur tour)
const MAX_CONCURRENT: usize = 16;
/// Au-delà, la requête est abandonnée (l'application attend 60 s)
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
/// Codes refusés : après MAX_FAILURES essais en FAILURE_WINDOW, l'adresse attend BLOCK_TIME
const MAX_FAILURES: u32 = 10;
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);
const BLOCK_TIME: Duration = Duration::from_secs(15 * 60);
/// Adresses suivies au plus (au-delà, les plus anciennes sont oubliées)
const MAX_TRACKED: usize = 10_000;
/// Flux d'événements ouverts en même temps au plus (au-delà : 503) ; ils échappent à
/// MAX_CONCURRENT et REQUEST_TIMEOUT, qui ne visent que les requêtes courtes
const MAX_STREAMS: usize = 64;
/// Commentaire envoyé sur un flux sans nouvelle, pour que les proxys ne le ferment pas
const KEEP_ALIVE: Duration = Duration::from_secs(20);

struct Server {
    code: String,
    collections: BTreeSet<String>,
    db: Mutex<Connection>,
    /// Proxys devant le serveur (voir `SYNC_PROXY_HOPS`)
    proxy_hops: usize,
    guard: Guard,
    /// Numéro de la dernière modification, suivi par les flux d'événements ; `None` : arrêt du
    /// serveur, les flux se ferment
    seq: watch::Sender<Option<i64>>,
    /// Places des flux d'événements (MAX_STREAMS)
    streams: Arc<Semaphore>,
}

impl Server {
    fn new(code: String, collections: BTreeSet<String>, conn: Connection, proxy_hops: usize) -> rusqlite::Result<Self> {
        let seq = last_seq(&conn)?;
        Ok(Server {
            code,
            collections,
            db: Mutex::new(conn),
            proxy_hops,
            guard: Guard::default(),
            seq: watch::Sender::new(Some(seq)),
            streams: Arc::new(Semaphore::new(MAX_STREAMS)),
        })
    }

    /// Nouvelle modification annoncée aux flux (le numéro ne fait qu'avancer : deux envois
    /// simultanés peuvent finir dans le désordre)
    fn announce(&self, seq: i64) {
        self.seq.send_if_modified(|current| match current {
            Some(c) if *c < seq => {
                *c = seq;
                true
            }
            _ => false,
        });
    }
}

type Shared = Arc<Server>;

/// Essais de code refusés, par adresse
#[derive(Default)]
struct Guard {
    attempts: Mutex<HashMap<IpAddr, Attempts>>,
}

struct Attempts {
    count: u32,
    since: Instant,
    blocked_until: Option<Instant>,
}

impl Attempts {
    fn fresh(now: Instant) -> Self {
        Attempts { count: 0, since: now, blocked_until: None }
    }

    fn expired(&self, now: Instant) -> bool {
        match self.blocked_until {
            Some(until) => until <= now,
            None => now.duration_since(self.since) > FAILURE_WINDOW,
        }
    }
}

impl Guard {
    /// Temps d'attente restant quand l'adresse est bloquée
    fn blocked(&self, ip: IpAddr, now: Instant) -> Option<Duration> {
        let map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&ip).and_then(|a| a.blocked_until).filter(|until| *until > now).map(|until| until - now)
    }

    fn failed(&self, ip: IpAddr, now: Instant) {
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= MAX_TRACKED && !map.contains_key(&ip) {
            map.retain(|_, a| !a.expired(now));
            if map.len() >= MAX_TRACKED {
                map.clear();
            }
        }
        let a = map.entry(ip).or_insert_with(|| Attempts::fresh(now));
        if a.expired(now) {
            *a = Attempts::fresh(now);
        }
        a.count += 1;
        if a.count >= MAX_FAILURES {
            a.blocked_until = Some(now + BLOCK_TIME);
        }
    }

    fn succeeded(&self, ip: IpAddr) {
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(&ip);
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Info {
    name: &'static str,
    version: &'static str,
    /// Collections synchronisées par ce serveur
    collections: Vec<String>,
    /// Numéro de la dernière modification
    seq: i64,
    /// Documents présents (hors suppressions) par collection synchronisée
    counts: std::collections::BTreeMap<String, i64>,
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

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    /// Trop de codes refusés depuis cette adresse : temps d'attente
    TooManyAttempts(Duration),
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Code de synchronisation incorrect".to_owned()),
            ApiError::TooManyAttempts(wait) => {
                let minutes = wait.as_secs().div_ceil(60).max(1);
                let body = Json(serde_json::json!({
                    "error": format!("Trop de codes incorrects : réessayer dans {minutes} min")
                }));
                return (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, wait.as_secs().max(1).to_string())], body).into_response();
            }
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

fn code_given(server: &Server, headers: &HeaderMap) -> bool {
    let given = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    same_code(given.trim(), &server.code)
}

/// Adresse du client : derrière `hops` proxys, celle ajoutée par le premier d'entre eux à
/// `X-Forwarded-For` (en partant de la droite : les adresses plus à gauche viennent du client
/// et ne prouvent rien) ; sinon celle de la connexion
fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>, hops: usize) -> IpAddr {
    let peer = peer.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    if hops == 0 {
        return peer;
    }
    let forwarded: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect();
    if forwarded.is_empty() {
        return peer;
    }
    let pick = forwarded[forwarded.len().saturating_sub(hops)];
    pick.parse().unwrap_or(peer)
}

/// Code vérifié avant toute lecture du corps de la requête ; trop d'essais refusés depuis
/// une adresse : refus (429) pendant BLOCK_TIME
async fn require_code(State(server): State<Shared>, request: Request, next: Next) -> Response {
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let ip = client_ip(request.headers(), peer, server.proxy_hops);
    let now = Instant::now();
    if let Some(wait) = server.guard.blocked(ip, now) {
        return ApiError::TooManyAttempts(wait).into_response();
    }
    if !code_given(&server, request.headers()) {
        server.guard.failed(ip, now);
        tracing::warn!("code refusé pour {ip}");
        return ApiError::Unauthorized.into_response();
    }
    server.guard.succeeded(ip);
    next.run(request).await
}

/// Travail sur la base hors des fils de tokio (une grosse transaction ne bloque pas les autres
/// requêtes)
async fn with_db<T: Send + 'static>(
    server: &Shared,
    f: impl FnOnce(&mut Connection) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let server = server.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = server.db.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut conn)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("tâche de la base : {e}")))?
}

fn last_seq(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM history", [], |r| r.get(0))
}

async fn info(State(server): State<Shared>) -> Result<Json<Info>, ApiError> {
    let s = server.clone();
    let (seq, counts) = with_db(&server, move |conn| {
        let seq = last_seq(conn)?;
        let mut counts = std::collections::BTreeMap::new();
        let mut stmt = conn.prepare("SELECT collection, COUNT(*) FROM docs WHERE deleted = 0 GROUP BY collection")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (collection, n) = row?;
            if s.collections.contains(&collection) {
                counts.insert(collection, n);
            }
        }
        Ok((seq, counts))
    })
    .await?;
    Ok(Json(Info {
        name: "twitch-desk-sync",
        version: env!("CARGO_PKG_VERSION"),
        collections: COLLECTIONS.iter().filter(|c| server.collections.contains(**c)).map(|c| c.to_string()).collect(),
        seq,
        counts,
    }))
}

/// Modifications après `since` : `limit` documents et `max_bytes` de contenu au plus (au
/// moins un document)
fn read_changes(
    conn: &Connection,
    collections: &BTreeSet<String>,
    since: i64,
    limit: usize,
    max_bytes: usize,
) -> Result<ChangesPage, ApiError> {
    let mut stmt = conn.prepare(
        "SELECT seq, collection, id, data, deleted, updated_at, author FROM docs WHERE seq > ?1 ORDER BY seq LIMIT ?2",
    )?;
    let mut rows = stmt.query(params![since, limit as i64 + 1])?;
    let mut list: Vec<Change> = vec![];
    let mut bytes = 0;
    let mut more = false;
    while let Some(r) = rows.next()? {
        let data: Option<String> = r.get(3)?;
        let size = data.as_ref().map_or(0, |d| d.len());
        if list.len() >= limit || (!list.is_empty() && bytes + size > max_bytes) {
            more = true;
            break;
        }
        bytes += size;
        list.push(Change {
            seq: r.get(0)?,
            collection: r.get(1)?,
            id: r.get(2)?,
            data: data.and_then(|d| serde_json::from_str(&d).ok()),
            deleted: r.get(4)?,
            updated_at: r.get(5)?,
            author: r.get(6)?,
        });
    }
    let seq = match list.last() {
        Some(c) => c.seq,
        None => since.max(0).min(last_seq(conn)?),
    };
    // Une collection désactivée depuis n'est plus envoyée
    list.retain(|c| collections.contains(&c.collection));
    Ok(ChangesPage { changes: list, seq, more })
}

async fn changes(State(server): State<Shared>, Query(q): Query<Since>) -> Result<Json<ChangesPage>, ApiError> {
    let limit = q.limit.unwrap_or(200).clamp(1, MAX_PAGE);
    let s = server.clone();
    let page = with_db(&server, move |conn| read_changes(conn, &s.collections, q.since, limit, MAX_PAGE_BYTES)).await?;
    Ok(Json(page))
}

async fn push(State(server): State<Shared>, Json(body): Json<Push>) -> Result<Json<PushResult>, ApiError> {
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
    let who = author.clone();
    let (accepted, seq) = with_db(&server, move |conn| {
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
            // Historique : qui, quand, quel document ; le contenu n'est gardé que dans `docs`
            tx.execute(
                "INSERT INTO history (collection, id, data, deleted, at, author) VALUES (?1, ?2, NULL, ?3, ?4, ?5)",
                params![c.collection, c.id, c.deleted, now, author],
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
        Ok((accepted, last_seq(conn)?))
    })
    .await?;
    if accepted > 0 {
        tracing::info!("{accepted} modification(s) de {}", who.as_deref().unwrap_or("?"));
        server.announce(seq);
    }
    Ok(Json(PushResult { accepted, seq }))
}

/// `/api/v1/events` : flux Server-Sent Events. À l'ouverture puis à chaque modification
/// enregistrée, un événement `seq` porte le numéro de la dernière modification
/// (`data: {"seq":42}`) ; plusieurs modifications rapprochées peuvent n'en donner qu'un.
/// Un commentaire toutes les KEEP_ALIVE garde la connexion ouverte
async fn events(State(server): State<Shared>) -> Response {
    let Ok(place) = server.streams.clone().try_acquire_owned() else {
        let body = Json(serde_json::json!({ "error": "Trop de connexions en temps réel : réessayer plus tard" }));
        return (StatusCode::SERVICE_UNAVAILABLE, [(header::RETRY_AFTER, "60")], body).into_response();
    };
    let stream = WatchStream::new(server.seq.subscribe()).map_while(move |seq| {
        // La place est rendue à la fermeture du flux
        let _place = &place;
        let seq = seq?;
        Some(Ok::<_, Infallible>(Event::default().event("seq").data(serde_json::json!({ "seq": seq }).to_string())))
    });
    // `X-Accel-Buffering` : Nginx (et Nginx Proxy Manager) transmet chaque événement sans attendre
    (
        [(header::HeaderName::from_static("x-accel-buffering"), "no")],
        Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEP_ALIVE)),
    )
        .into_response()
}

/// `/health` : le serveur répond et la base aussi
async fn health(State(server): State<Shared>) -> Response {
    let select = |conn: &Connection| conn.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)).is_ok();
    let ok = match server.db.try_lock() {
        Ok(conn) => select(&conn),
        // Base occupée par une requête : elle fonctionne
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Poisoned(e)) => select(&e.into_inner()),
    };
    if ok {
        "ok".into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "base de données indisponible").into_response()
    }
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
    // Bases d'avant la 0.4 : l'historique gardait le contenu de chaque version (lu par
    // personne) ; il est vidé de ce contenu et la place libérée au fil des ménages
    // (auto_vacuum incrémental, pris en compte par un VACUUM complet, une seule fois)
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
    if auto_vacuum != 2 {
        let emptied = conn.execute("UPDATE history SET data = NULL WHERE data IS NOT NULL", [])?;
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM;")?;
        if emptied > 0 {
            tracing::info!("historique : contenu retiré de {emptied} entrée(s), base compactée");
        }
    }
    Ok(conn)
}

/// Historique plus ancien que `days` jours effacé (l'état actuel des documents est gardé)
fn prune_history(conn: &Connection, days: i64) -> rusqlite::Result<usize> {
    let before = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // La dernière modification de chaque document reste : c'est elle qui porte le curseur
    let n = conn.execute("DELETE FROM history WHERE at < ?1 AND seq NOT IN (SELECT seq FROM docs)", [before])?;
    // Place libérée rendue au système
    conn.execute_batch("PRAGMA incremental_vacuum;")?;
    Ok(n)
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
fn health_check(port: u16) -> std::process::ExitCode {
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
    let api = Router::new()
        .route("/api/v1/info", get(info))
        .route("/api/v1/changes", get(changes).post(push))
        // Code vérifié avant la lecture du corps (route_layer : routes connues seulement)
        .route_layer(middleware::from_fn_with_state(server.clone(), require_code))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(GlobalConcurrencyLimitLayer::new(MAX_CONCURRENT))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, REQUEST_TIMEOUT));
    // Flux d'événements : connexions longues, hors de la limite de requêtes simultanées et du
    // délai maximal (plafonnés à part, MAX_STREAMS)
    let live = Router::new()
        .route("/api/v1/events", get(events))
        .route_layer(middleware::from_fn_with_state(server.clone(), require_code));
    Router::new().route("/health", get(health)).merge(api).merge(live).with_state(server)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let port: u16 = std::env::var("SYNC_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8787);
    if std::env::args().nth(1).as_deref() == Some("health") {
        return health_check(port);
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
    let proxy_hops: usize = std::env::var("SYNC_PROXY_HOPS").ok().and_then(|d| d.trim().parse().ok()).unwrap_or(1);
    let server = match Server::new(code, collections, conn, proxy_hops) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!("lecture de {path} : {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    // Ménage quotidien de l'historique
    let pruner = server.clone();
    tokio::spawn(async move {
        loop {
            let result = with_db(&pruner, move |conn| Ok(prune_history(conn, days)?)).await;
            match result {
                Ok(n) if n > 0 => tracing::info!("historique : {n} entrée(s) de plus de {days} jours effacée(s)"),
                Ok(_) => {}
                Err(ApiError::Internal(e)) => tracing::warn!("ménage de l'historique : {e}"),
                Err(_) => {}
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
    let stopping = server.clone();
    let shutdown = async move {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
            tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        let _ = ctrl_c.await;
        // Flux d'événements fermés : l'arrêt n'attend pas qu'ils se terminent d'eux-mêmes
        stopping.seq.send_replace(None);
    };
    let service = app(server).into_make_service_with_connect_info::<SocketAddr>();
    if let Err(e) = axum::serve(listener, service).with_graceful_shutdown(shutdown).await {
        tracing::error!("serveur : {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn server(collections: &[&str]) -> Shared {
        let collections = collections.iter().map(|c| c.to_string()).collect();
        Arc::new(Server::new("code-secret".into(), collections, open_db(":memory:").unwrap(), 1).unwrap())
    }

    fn change(collection: &str, id: &str, data: Option<serde_json::Value>) -> IncomingChange {
        IncomingChange { collection: collection.into(), id: id.into(), deleted: data.is_none(), data }
    }

    async fn pull(s: &Shared, since: i64) -> ChangesPage {
        let Json(page) = changes(State(s.clone()), Query(Since { since, limit: None })).await.ok().unwrap();
        page
    }

    fn request(method: &str, uri: &str, code: Option<&str>, ip: &str, body: &str) -> Request {
        let mut r = Request::builder().method(method).uri(uri).header("x-forwarded-for", ip).header("content-type", "application/json");
        if let Some(code) = code {
            r = r.header("authorization", format!("Bearer {code}"));
        }
        r.body(Body::from(body.to_owned())).unwrap()
    }

    async fn status(s: &Shared, r: Request) -> StatusCode {
        app(s.clone()).oneshot(r).await.unwrap().status()
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

    #[test]
    fn client_address() {
        let peer: IpAddr = "10.0.0.2".parse().unwrap();
        let mut h = HeaderMap::new();
        assert_eq!(client_ip(&h, Some(peer), 1), peer);
        h.insert("x-forwarded-for", "1.1.1.1, 2.2.2.2".parse().unwrap());
        // Le proxy ajoute l'adresse réelle à droite ; celle de gauche vient du client
        assert_eq!(client_ip(&h, Some(peer), 1), "2.2.2.2".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip(&h, Some(peer), 2), "1.1.1.1".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip(&h, Some(peer), 5), "1.1.1.1".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip(&h, Some(peer), 0), peer);
    }

    #[test]
    fn repeated_wrong_codes_block_the_address() {
        let g = Guard::default();
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        let t0 = Instant::now();
        for _ in 0..MAX_FAILURES - 1 {
            g.failed(ip, t0);
        }
        assert!(g.blocked(ip, t0).is_none());
        g.failed(ip, t0);
        assert!(g.blocked(ip, t0).is_some());
        assert!(g.blocked("1.2.3.5".parse().unwrap(), t0).is_none());
        assert!(g.blocked(ip, t0 + BLOCK_TIME + Duration::from_secs(1)).is_none());
        // Essais espacés : la fenêtre repart de zéro
        let other: IpAddr = "5.6.7.8".parse().unwrap();
        for _ in 0..MAX_FAILURES - 1 {
            g.failed(other, t0);
        }
        let later = t0 + FAILURE_WINDOW + Duration::from_secs(1);
        g.failed(other, later);
        assert!(g.blocked(other, later).is_none());
    }

    #[tokio::test]
    async fn code_checked_before_the_body() {
        let s = server(&["gifts"]);
        assert_eq!(status(&s, request("GET", "/health", None, "9.9.9.9", "")).await, StatusCode::OK);
        assert_eq!(status(&s, request("GET", "/api/v1/info", None, "9.9.9.9", "")).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(&s, request("GET", "/api/v1/info", Some("code-secret"), "9.9.9.9", "")).await, StatusCode::OK);
        // Corps invalide sans le bon code : refusé pour le code, le corps n'est pas lu
        assert_eq!(status(&s, request("POST", "/api/v1/changes", Some("faux"), "9.9.9.9", "{pas du json")).await, StatusCode::UNAUTHORIZED);
        assert_eq!(
            status(&s, request("POST", "/api/v1/changes", Some("code-secret"), "9.9.9.9", "{pas du json")).await,
            StatusCode::BAD_REQUEST
        );
        // Trop d'essais : l'adresse attend, même avec le bon code ; les autres non
        for _ in 0..MAX_FAILURES {
            assert_eq!(status(&s, request("GET", "/api/v1/info", Some("faux"), "8.8.8.8", "")).await, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(status(&s, request("GET", "/api/v1/info", Some("code-secret"), "8.8.8.8", "")).await, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(status(&s, request("GET", "/api/v1/info", Some("code-secret"), "9.9.9.9", "")).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn push_pull_and_last_write_wins() {
        let s = server(&["gifts", "groups"]);

        let body = Push {
            author: Some("alice".into()),
            changes: vec![
                change("groups", "g1", Some(serde_json::json!({"name": "Voiles"}))),
                change("gifts", "a", Some(serde_json::json!({"title": "Obsidian Sails"}))),
            ],
        };
        let Json(r) = push(State(s.clone()), Json(body)).await.ok().unwrap();
        assert_eq!((r.accepted, r.seq), (2, 2));

        // Le même contenu renvoyé n'est pas une modification
        let again = Push { author: None, changes: vec![change("gifts", "a", Some(serde_json::json!({"title": "Obsidian Sails"})))] };
        let Json(r) = push(State(s.clone()), Json(again)).await.ok().unwrap();
        assert_eq!(r.accepted, 0);

        let page = pull(&s, 0).await;
        assert_eq!(page.changes.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["g1", "a"]);
        assert_eq!(page.seq, 2);
        assert!(pull(&s, 2).await.changes.is_empty());

        // Modification puis suppression : seule la dernière version est renvoyée
        let edit = Push { author: Some("bob".into()), changes: vec![change("gifts", "a", Some(serde_json::json!({"title": "Voiles"})))] };
        let _ = push(State(s.clone()), Json(edit)).await.ok().unwrap();
        let delete = Push { author: Some("bob".into()), changes: vec![change("gifts", "a", None)] };
        let _ = push(State(s.clone()), Json(delete)).await.ok().unwrap();
        let page = pull(&s, 2).await;
        assert_eq!(page.changes.len(), 1);
        assert!(page.changes[0].deleted && page.changes[0].data.is_none());
        assert_eq!(page.seq, 4);

        // L'historique ne garde pas le contenu
        let stored: i64 = s.db.lock().unwrap().query_row("SELECT COUNT(*) FROM history WHERE data IS NOT NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, 0);

        // Collection non synchronisée refusée
        let other = Push { author: None, changes: vec![change("channels", "x", Some(serde_json::json!({})))] };
        assert!(push(State(s.clone()), Json(other)).await.is_err());
    }

    #[tokio::test]
    async fn paging() {
        let s = server(&["gifts"]);
        let many = (0..250).map(|i| change("gifts", &format!("g{i}"), Some(serde_json::json!({"n": i})))).collect();
        let _ = push(State(s.clone()), Json(Push { author: None, changes: many })).await.ok().unwrap();
        let first = pull(&s, 0).await;
        assert_eq!((first.changes.len(), first.more), (200, true));
        let second = pull(&s, first.seq).await;
        assert_eq!((second.changes.len(), second.more, second.seq), (50, false, 250));

        // Page coupée par la taille : au moins un document, la suite au prochain appel
        let conn = s.db.lock().unwrap();
        let page = read_changes(&conn, &s.collections, 0, 200, 20).unwrap();
        assert_eq!((page.changes.len(), page.more, page.seq), (2, true, 2));
        let page = read_changes(&conn, &s.collections, 0, 200, 1).unwrap();
        assert_eq!((page.changes.len(), page.more, page.seq), (1, true, 1));
    }

    /// Prochain morceau du flux, en texte (5 s au plus)
    async fn next_event(body: &mut axum::body::BodyDataStream) -> String {
        let chunk = tokio::time::timeout(Duration::from_secs(5), body.next()).await.expect("événement attendu");
        String::from_utf8(chunk.unwrap().unwrap().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn events_announce_new_changes() {
        let s = server(&["gifts"]);
        assert_eq!(status(&s, request("GET", "/api/v1/events", None, "7.7.7.7", "")).await, StatusCode::UNAUTHORIZED);

        let res = app(s.clone()).oneshot(request("GET", "/api/v1/events", Some("code-secret"), "7.7.7.7", "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/event-stream"));
        assert_eq!(res.headers()["x-accel-buffering"], "no");
        let mut body = res.into_body().into_data_stream();
        // À l'ouverture : la dernière modification
        assert_eq!(next_event(&mut body).await, "event: seq\ndata: {\"seq\":0}\n\n");

        // Envoi accepté par une autre application : annoncé
        let push = r#"{"author":"alice","changes":[{"collection":"gifts","id":"a","data":{"title":"Voiles"}}]}"#;
        assert_eq!(status(&s, request("POST", "/api/v1/changes", Some("code-secret"), "7.7.7.7", push)).await, StatusCode::OK);
        assert_eq!(next_event(&mut body).await, "event: seq\ndata: {\"seq\":1}\n\n");

        // Rien de nouveau (même contenu) : pas d'événement
        assert_eq!(status(&s, request("POST", "/api/v1/changes", Some("code-secret"), "7.7.7.7", push)).await, StatusCode::OK);
        assert!(tokio::time::timeout(Duration::from_millis(200), body.next()).await.is_err());

        // Le numéro ne recule pas
        s.announce(0);
        assert!(tokio::time::timeout(Duration::from_millis(200), body.next()).await.is_err());

        // Arrêt du serveur : le flux se ferme
        s.seq.send_replace(None);
        assert!(tokio::time::timeout(Duration::from_secs(5), body.next()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn event_streams_are_capped() {
        let s = server(&["gifts"]);
        let all = s.streams.clone().try_acquire_many_owned(MAX_STREAMS as u32).unwrap();
        assert_eq!(status(&s, request("GET", "/api/v1/events", Some("code-secret"), "6.6.6.6", "")).await, StatusCode::SERVICE_UNAVAILABLE);
        drop(all);
        let res = app(s.clone()).oneshot(request("GET", "/api/v1/events", Some("code-secret"), "6.6.6.6", "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(s.streams.available_permits(), MAX_STREAMS - 1);
        // Flux fermé : place rendue
        drop(res);
        assert_eq!(s.streams.available_permits(), MAX_STREAMS);
    }

    #[test]
    fn old_history_content_is_dropped() {
        let dir = std::env::temp_dir().join(format!("tdsync-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.db");
        let path = path.to_str().unwrap();
        {
            // Base d'avant la 0.4 : contenu dans l'historique, sans auto_vacuum
            let conn = Connection::open(path).unwrap();
            conn.execute_batch(
                "CREATE TABLE history (seq INTEGER PRIMARY KEY AUTOINCREMENT, collection TEXT NOT NULL, id TEXT NOT NULL,
                     data TEXT, deleted INTEGER NOT NULL, at TEXT NOT NULL, author TEXT);
                 INSERT INTO history (collection, id, data, deleted, at) VALUES ('gifts', 'a', '{\"n\":1}', 0, '2020-01-01T00:00:00Z');",
            )
            .unwrap();
        }
        let conn = open_db(path).unwrap();
        let (rows, with_data): (i64, i64) =
            conn.query_row("SELECT COUNT(*), COUNT(data) FROM history", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((rows, with_data), (1, 0));
        let mode: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, 2);
        assert_eq!(prune_history(&conn, 1).unwrap(), 1);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
