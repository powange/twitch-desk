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
//! - `SYNC_WEB_DIR` : fichiers de la web app mobile, servis sous `/app`
//!   (défaut : /usr/share/twitch-desk-sync/web, rempli dans l'image Docker)
//! - `SYNC_WEB_PORT` : port supplémentaire qui ne sert que la web app (défaut : aucun, elle
//!   est servie sur le port de l'API)
//!
//! `GET /api/v1/events` (Server-Sent Events) annonce chaque nouvelle modification : les
//! applications abonnées relisent `/api/v1/changes` tout de suite, sans attendre leur passage.
//!
//! Web app mobile (0.6, module `mobile`) : un téléphone associé par un code à 6 chiffres lit,
//! avec son propre jeton (`Authorization: Device <jeton>`), les giveaways ouverts et l'état
//! publié par son application ; il n'écrit rien.

mod actions;
mod mobile;
mod push;
mod web;

use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::future::IntoFuture;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, DefaultBodyLimit, Extension, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, watch, Semaphore};
use tokio_stream::wrappers::ReceiverStream;
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
/// Flux ouverts au plus par appareil, et par adresse pour les applications (code partagé) ; au-delà,
/// le plus ancien est fermé (une seule personne ne prend pas toutes les places)
const STREAMS_PER_DEVICE: usize = 2;
const STREAMS_PER_ADDRESS: usize = 4;
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
    /// Dernière modification de ce que montrent les téléphones (giveaways, cadeaux, groupes) :
    /// les autres (chaînes, fiches…) ne leur font pas tout relire
    phone_seq: watch::Sender<Option<i64>>,
    /// Places des flux d'événements (MAX_STREAMS)
    streams: Arc<Semaphore>,
    /// Flux ouverts par appareil ou adresse : numéro et signal de fermeture, du plus ancien au plus récent
    stream_owners: Mutex<HashMap<String, StreamList>>,
    next_stream: std::sync::atomic::AtomicU64,
    /// Codes d'association refusés, par adresse (plus strict que `guard`)
    pair_guard: Guard,
    /// Codes d'association en attente : code → application (en mémoire : perdus au redémarrage)
    pairings: Mutex<HashMap<String, mobile::Pairing>>,
    /// Codes d'association refusés récemment, toutes adresses confondues (limite globale)
    pair_failures: Mutex<std::collections::VecDeque<Instant>>,
    /// États publiés et appareils révoqués, annoncés aux flux des appareils
    notices: broadcast::Sender<mobile::Notice>,
    /// Fichiers de la web app (`SYNC_WEB_DIR`)
    web_dir: PathBuf,
    /// Notifications push : clé VAPID et envoi
    push: push::Push,
    /// Adresse publique de la web app (`SYNC_WEB_URL`), donnée aux applications
    web_url: Option<String>,
    /// Flux temps réel ouverts par chaque application (identifiant donné à l'ouverture) : ouverte
    /// ou fermée, pour le diagnostic
    pc_streams: Mutex<HashMap<String, usize>>,
}

impl Server {
    fn new(code: String, collections: BTreeSet<String>, conn: Connection, proxy_hops: usize) -> rusqlite::Result<Self> {
        let seq = last_seq(&conn)?;
        let contact = std::env::var("SYNC_PUSH_CONTACT").ok().map(|c| c.trim().to_owned()).filter(|c| !c.is_empty());
        if contact.as_deref().is_none_or(|c| c == push::DEFAULT_CONTACT) {
            // Google, Apple ou Mozilla contacteraient l'auteur du projet, pas l'hébergeur de ce
            // serveur (Apple peut aussi refuser un contact trop générique)
            tracing::warn!("SYNC_PUSH_CONTACT non défini : indique un contact de l'hébergeur (mailto:toi@exemple.fr) pour les notifications push");
        }
        let push = push::Push::load(&conn, contact.unwrap_or_else(|| push::DEFAULT_CONTACT.to_owned()))?;
        let web_url = std::env::var("SYNC_WEB_URL").ok().and_then(|u| web_url_of(&u));
        Ok(Server {
            code,
            collections,
            db: Mutex::new(conn),
            proxy_hops,
            guard: Guard::default(),
            seq: watch::Sender::new(Some(seq)),
            phone_seq: watch::Sender::new(Some(seq)),
            streams: Arc::new(Semaphore::new(MAX_STREAMS)),
            stream_owners: Mutex::new(HashMap::new()),
            web_url,
            pc_streams: Mutex::new(HashMap::new()),
            next_stream: std::sync::atomic::AtomicU64::new(0),
            pair_guard: Guard::new(mobile::PAIR_MAX_FAILURES, mobile::PAIR_WINDOW, mobile::PAIR_BLOCK),
            pairings: Mutex::new(HashMap::new()),
            pair_failures: Mutex::new(std::collections::VecDeque::new()),
            notices: broadcast::channel(64).0,
            web_dir: PathBuf::from(web::DEFAULT_WEB_DIR),
            push,
        })
    }

    /// Nouvelle modification annoncée aux flux (le numéro ne fait qu'avancer : deux envois
    /// simultanés peuvent finir dans le désordre)
    fn announce(&self, seq: i64, for_phones: bool) {
        let advance = |current: &mut Option<i64>| match current {
            Some(c) if *c < seq => {
                *c = seq;
                true
            }
            _ => false,
        };
        self.seq.send_if_modified(advance);
        if for_phones {
            self.phone_seq.send_if_modified(advance);
        }
    }
}

type Shared = Arc<Server>;

/// Flux d'un propriétaire : numéro et signal de fermeture, du plus ancien au plus récent
type StreamList = std::collections::VecDeque<(u64, tokio::sync::oneshot::Sender<()>)>;

/// Essais de code refusés, par adresse : après `max_failures` essais en `window`, l'adresse
/// attend `block`
struct Guard {
    attempts: Mutex<HashMap<IpAddr, Attempts>>,
    max_failures: u32,
    window: Duration,
    block: Duration,
}

/// Code partagé et jetons d'appareils
impl Default for Guard {
    fn default() -> Self {
        Guard::new(MAX_FAILURES, FAILURE_WINDOW, BLOCK_TIME)
    }
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

    fn expired(&self, now: Instant, window: Duration) -> bool {
        match self.blocked_until {
            Some(until) => until <= now,
            None => now.duration_since(self.since) > window,
        }
    }
}

impl Guard {
    fn new(max_failures: u32, window: Duration, block: Duration) -> Self {
        Guard { attempts: Mutex::new(HashMap::new()), max_failures, window, block }
    }

    /// Temps d'attente restant quand l'adresse est bloquée
    fn blocked(&self, ip: IpAddr, now: Instant) -> Option<Duration> {
        let ip = guard_key(ip);
        let map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&ip).and_then(|a| a.blocked_until).filter(|until| *until > now).map(|until| until - now)
    }

    fn failed(&self, ip: IpAddr, now: Instant) {
        let ip = guard_key(ip);
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= MAX_TRACKED && !map.contains_key(&ip) {
            map.retain(|_, a| !a.expired(now, self.window));
            // Encore plein : les plus anciennes adresses non bloquées sont oubliées (les
            // blocages en cours restent), jusqu'à un dixième de place libre
            if map.len() >= MAX_TRACKED {
                let mut free: Vec<(IpAddr, Instant)> = map.iter().filter(|(_, a)| a.blocked_until.is_none()).map(|(k, a)| (*k, a.since)).collect();
                free.sort_by_key(|(_, since)| *since);
                for (k, _) in free.into_iter().take(MAX_TRACKED / 10) {
                    map.remove(&k);
                }
            }
            if map.len() >= MAX_TRACKED {
                return;
            }
        }
        let a = map.entry(ip).or_insert_with(|| Attempts::fresh(now));
        if a.expired(now, self.window) {
            *a = Attempts::fresh(now);
        }
        a.count += 1;
        if a.count >= self.max_failures {
            a.blocked_until = Some(now + self.block);
        }
    }

    fn succeeded(&self, ip: IpAddr) {
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(&guard_key(ip));
    }
}

/// Adresse comptée par les blocages : une adresse IPv6 vaut tout son réseau /64 (un abonné en
/// reçoit des milliards), une IPv4 écrite en IPv6 vaut l'IPv4
fn guard_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(std::net::Ipv6Addr::from(u128::from(v6) & !((1u128 << 64) - 1))),
        },
        v4 => v4,
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
    /// Adresse de la web app (`SYNC_WEB_URL`), si l'hébergeur l'a indiquée
    #[serde(rename = "webUrl", skip_serializing_if = "Option::is_none")]
    web_url: Option<String>,
}

/// `SYNC_WEB_URL` : adresse http(s) sans espace, sans / final ; autre chose : ignorée
fn web_url_of(raw: &str) -> Option<String> {
    let url = raw.trim().trim_end_matches('/');
    let ok = (url.starts_with("https://") || url.starts_with("http://")) && url.len() <= 300 && !url.contains(char::is_whitespace) && url.split("://").nth(1).is_some_and(|h| !h.is_empty());
    if !ok && !url.is_empty() {
        tracing::warn!("SYNC_WEB_URL ignorée : adresse http(s) attendue");
    }
    ok.then(|| url.to_owned())
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
    /// Jeton d'appareil inconnu (révoqué) : à associer de nouveau
    UnknownDevice,
    /// Code d'association inconnu, expiré ou déjà utilisé
    BadPairingCode,
    /// Trop de codes refusés depuis cette adresse : temps d'attente
    TooManyAttempts(Duration),
    BadRequest(String),
    NotFound(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Code de synchronisation incorrect".to_owned()),
            ApiError::UnknownDevice => (StatusCode::UNAUTHORIZED, "Appareil inconnu : associe-le de nouveau depuis l'application".to_owned()),
            ApiError::BadPairingCode => (StatusCode::BAD_REQUEST, "Code inconnu, expiré ou déjà utilisé".to_owned()),
            ApiError::TooManyAttempts(wait) => {
                let minutes = wait.as_secs().div_ceil(60).max(1);
                let body = Json(serde_json::json!({
                    "error": format!("Trop de codes incorrects : réessayer dans {minutes} min")
                }));
                return (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, wait.as_secs().max(1).to_string())], body).into_response();
            }
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
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

/// Jeton d'appareil donné (`Authorization: Device <jeton>`)
fn device_token(headers: &HeaderMap) -> Option<String> {
    let token = headers.get("authorization")?.to_str().ok()?.strip_prefix("Device ")?.trim();
    (!token.is_empty() && token.len() <= 128).then(|| token.to_owned())
}

/// Qui fait la requête : une application (code partagé, tout permis) ou un appareil associé
/// (lecture seule, limitée à son application)
#[derive(Debug, Clone)]
enum Access {
    Full,
    Device { id: String, instance: String },
}

fn now_text() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
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
    // Une fois : combien d'adresses le ou les proxys ajoutent, pour vérifier SYNC_PROXY_HOPS
    // (Cloudflare devant le proxy : 2 ; mal réglé, un nœud Cloudflare serait pris pour le client)
    static LOGGED: std::sync::Once = std::sync::Once::new();
    LOGGED.call_once(|| {
        tracing::info!(
            "X-Forwarded-For : {} adresse(s) reçue(s), SYNC_PROXY_HOPS = {hops} (Cloudflare devant le proxy : mettre 2)",
            forwarded.len()
        );
    });
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

/// Code partagé ou jeton d'appareil, vérifié avant toute lecture du corps ; les jetons refusés
/// comptent comme des codes refusés. L'accès accordé est passé à la route (`Access`)
async fn require_access(State(server): State<Shared>, mut request: Request, next: Next) -> Response {
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let ip = client_ip(request.headers(), peer, server.proxy_hops);
    let now = Instant::now();
    // Jeton d'appareil valide : accepté même si l'adresse est bloquée (un PC au code périmé sur
    // le même Wi-Fi ne bloque pas les téléphones), et sans remettre à zéro les échecs du code
    if let Some(token) = device_token(request.headers()) {
        match with_db(&server, move |conn| Ok(mobile::device_for_token(conn, &token)?)).await {
            Ok(Some((id, instance))) => {
                request.extensions_mut().insert(Access::Device { id, instance });
                return next.run(request).await;
            }
            Ok(None) => {}
            Err(e) => return e.into_response(),
        }
    }
    if let Some(wait) = server.guard.blocked(ip, now) {
        return ApiError::TooManyAttempts(wait).into_response();
    }
    let access = code_given(&server, request.headers()).then_some(Access::Full);
    let Some(access) = access else {
        server.guard.failed(ip, now);
        tracing::warn!("code ou jeton refusé pour {ip}");
        let device = request.headers().get("authorization").and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("Device "));
        return if device { ApiError::UnknownDevice } else { ApiError::Unauthorized }.into_response();
    };
    server.guard.succeeded(ip);
    request.extensions_mut().insert(access);
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
        web_url: server.web_url.clone(),
    }))
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    /// Identifiant de l'application qui ouvre le flux (diagnostic : ouverte ou fermée)
    instance: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstanceOverview {
    id: String,
    name: String,
    /// Flux temps réel ouvert en ce moment
    online: bool,
    /// Dernier enregistrement ou dernier état publié
    last_seen: String,
    /// Comptes Twitch de l'application (d'après son dernier état publié)
    accounts: Vec<String>,
    /// Version de l'application (d'après son dernier état publié)
    version: Option<String>,
    /// Appareils associés, et ceux dont la web app est ouverte en ce moment
    devices: i64,
    devices_online: i64,
}

/// `GET /api/v1/admin/instances` : applications connues du serveur, ouvertes ou non, leurs comptes
/// Twitch et leurs appareils (code partagé seulement)
async fn instances_overview(State(server): State<Shared>) -> Result<Json<Vec<InstanceOverview>>, ApiError> {
    let online: HashMap<String, usize> = server.pc_streams.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let devices_live: BTreeSet<String> = server
        .stream_owners
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .filter_map(|k| k.strip_prefix("device:").map(str::to_owned))
        .collect();
    let list = with_db(&server, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT i.id, i.name, MAX(i.updated_at, COALESCE(i.state_at, '')), i.state,
                    (SELECT COUNT(*) FROM devices d WHERE d.instance = i.id)
             FROM instances i ORDER BY i.name COLLATE NOCASE, i.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, i64>(4)?))
        })?;
        let mut device_stmt = conn.prepare("SELECT id FROM devices WHERE instance = ?1")?;
        let mut out = vec![];
        for row in rows {
            let (id, name, last_seen, state, devices) = row?;
            let state = state.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
            let version = state.as_ref().and_then(|v| v.pointer("/pc/version")?.as_str().map(|s| s.chars().take(40).collect()));
            let accounts = state
                .and_then(|v| v.get("accounts").and_then(|a| a.as_array()).cloned())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            let ids: Vec<String> = device_stmt.query_map([&id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            let devices_online = ids.iter().filter(|d| devices_live.contains(*d)).count() as i64;
            out.push(InstanceOverview { online: online.get(&id).is_some_and(|n| *n > 0), id, name, last_seen, accounts, version, devices, devices_online });
        }
        Ok(out)
    })
    .await?;
    Ok(Json(list))
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
    let now = now_text();
    let who = author.clone();
    let (accepted, seq, for_phones) = with_db(&server, move |conn| {
        let tx = conn.transaction()?;
        let mut accepted = 0;
        let mut for_phones = false;
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
            for_phones |= PHONE_COLLECTIONS.contains(&c.collection.as_str());
        }
        tx.commit()?;
        Ok((accepted, last_seq(conn)?, for_phones))
    })
    .await?;
    if accepted > 0 {
        tracing::info!("{accepted} modification(s) de {}", who.as_deref().unwrap_or("?"));
        server.announce(seq, for_phones);
    }
    Ok(Json(PushResult { accepted, seq }))
}

/// Lance une fonction à la fin d'une portée (y compris sur un `return` anticipé)
struct OnDrop<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

/// Collections montrées par la vue des téléphones
const PHONE_COLLECTIONS: [&str; 3] = ["giveaways", "gifts", "groups"];

fn seq_event(seq: i64) -> Event {
    Event::default().event("seq").data(serde_json::json!({ "seq": seq }).to_string())
}

/// `/api/v1/events` : flux Server-Sent Events. À l'ouverture puis à chaque modification
/// enregistrée, un événement `seq` porte le numéro de la dernière modification
/// (`data: {"seq":42}`) ; plusieurs modifications rapprochées peuvent n'en donner qu'un.
/// Un commentaire toutes les KEEP_ALIVE garde la connexion ouverte.
///
/// Appareil associé (0.6) : en plus, un événement `state` (`data: {"instance":"…"}`) quand
/// son application publie un nouvel état ; son flux se ferme dès qu'il est révoqué
async fn events(State(server): State<Shared>, Extension(access): Extension<Access>, Query(q): Query<EventsQuery>, request: Request) -> Response {
    // Application qui se signale : comptée ouverte tant que son flux l'est
    let pc = match (&access, q.instance) {
        (Access::Full, Some(i)) if mobile::valid_instance_id(&i) => Some(i),
        _ => None,
    };
    // Propriétaire du flux : l'appareil, ou l'adresse de l'application ; ses flux en trop
    // (le plus ancien d'abord) sont fermés pour faire de la place
    let (owner, limit) = match &access {
        Access::Device { id, .. } => (format!("device:{id}"), STREAMS_PER_DEVICE),
        Access::Full => {
            let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
            (format!("ip:{}", guard_key(client_ip(request.headers(), peer, server.proxy_hops))), STREAMS_PER_ADDRESS)
        }
    };
    let stream_id = server.next_stream.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (kick_tx, mut kick) = tokio::sync::oneshot::channel::<()>();
    {
        let mut owners = server.stream_owners.lock().unwrap_or_else(|e| e.into_inner());
        let list = owners.entry(owner.clone()).or_default();
        list.push_back((stream_id, kick_tx));
        while list.len() > limit {
            if let Some((_, old)) = list.pop_front() {
                let _ = old.send(());
            }
        }
    }
    if let Some(i) = &pc {
        *server.pc_streams.lock().unwrap_or_else(|e| e.into_inner()).entry(i.clone()).or_default() += 1;
    }
    let release = {
        let server = server.clone();
        let owner = owner.clone();
        move || {
            if let Some(i) = &pc {
                let mut streams = server.pc_streams.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(n) = streams.get_mut(i) {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        streams.remove(i);
                    }
                }
            }
            let mut owners = server.stream_owners.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(list) = owners.get_mut(&owner) {
                list.retain(|(id, _)| *id != stream_id);
                if list.is_empty() {
                    owners.remove(&owner);
                }
            }
        }
    };
    // Flux fermé juste avant : sa place revient dès que sa tâche s'arrête
    tokio::task::yield_now().await;
    let Ok(place) = server.streams.clone().try_acquire_owned() else {
        release();
        let body = Json(serde_json::json!({ "error": "Trop de connexions en temps réel : réessayer plus tard" }));
        return (StatusCode::SERVICE_UNAVAILABLE, [(header::RETRY_AFTER, "60")], body).into_response();
    };
    let (tx, rx) = mpsc::channel::<Event>(16);
    // Téléphones : annoncés seulement pour ce que montre leur vue
    let mut seq = match access {
        Access::Full => server.seq.subscribe(),
        Access::Device { .. } => server.phone_seq.subscribe(),
    };
    let mut notices = server.notices.subscribe();
    let device = match access {
        Access::Full => None,
        Access::Device { id, instance } => Some((id, instance)),
    };
    let s = server.clone();
    tokio::spawn(async move {
        // Flux terminé, quelle qu'en soit la raison : retiré de ceux de son propriétaire
        let _release = OnDrop(Some(release));
        let first = *seq.borrow_and_update();
        let Some(first) = first else { return };
        if tx.send(seq_event(first)).await.is_err() {
            return;
        }
        loop {
            let event = tokio::select! {
                _ = tx.closed() => break,
                // Trop de flux pour ce propriétaire : le plus ancien (celui-ci) se ferme
                _ = &mut kick => break,
                changed = seq.changed() => {
                    let current = *seq.borrow_and_update();
                    match (changed, current) {
                        (Ok(()), Some(n)) => seq_event(n),
                        // Arrêt du serveur : le flux se ferme
                        _ => break,
                    }
                }
                notice = notices.recv() => {
                    // Applications : demande d'un téléphone (chacune ne garde que les siennes)
                    let Some((id, instance)) = device.as_ref() else {
                        match notice {
                            Ok(mobile::Notice::Action(i)) => {
                                let event = Event::default().event("action").data(serde_json::json!({ "instance": i }).to_string());
                                if tx.send(event).await.is_err() {
                                    break;
                                }
                                continue;
                            }
                            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    };
                    match notice {
                        Ok(mobile::Notice::State(i)) if &i == instance => {
                            Event::default().event("state").data(serde_json::json!({ "instance": i }).to_string())
                        }
                        Ok(mobile::Notice::ActionDone(d)) if &d == id => {
                            Event::default().event("action").data(serde_json::json!({ "device": d }).to_string())
                        }
                        Ok(mobile::Notice::Revoked(d)) if &d == id => break,
                        Ok(_) => continue,
                        // Nouvelles perdues : appareil revérifié, puis tout est relu
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let id = id.clone();
                            match with_db(&s, move |conn| Ok(mobile::device_exists(conn, &id)?)).await {
                                Ok(true) => match *seq.borrow() {
                                    Some(n) => seq_event(n),
                                    None => break,
                                },
                                _ => break,
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            };
            if tx.send(event).await.is_err() {
                break;
            }
        }
    });
    let stream = ReceiverStream::new(rx).map(move |event| {
        // La place est rendue à la fermeture du flux
        let _place = &place;
        Ok::<_, Infallible>(event)
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
         CREATE INDEX IF NOT EXISTS docs_seq ON docs (seq);
         -- Web app mobile (0.6) : applications, leur dernier état publié, appareils associés
         -- (jeton gardé sous forme d'empreinte SHA-256)
         CREATE TABLE IF NOT EXISTS instances (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             state TEXT,
             state_at TEXT
         );
         CREATE TABLE IF NOT EXISTS devices (
             id TEXT PRIMARY KEY,
             instance TEXT NOT NULL,
             name TEXT NOT NULL,
             token_hash TEXT NOT NULL UNIQUE,
             created_at TEXT NOT NULL,
             last_seen TEXT
         );
         CREATE INDEX IF NOT EXISTS devices_instance ON devices (instance);
         -- Notifications push (0.7) : réglages du serveur (clé VAPID), abonnement de chaque
         -- appareil et alertes choisies (liste JSON)
         CREATE TABLE IF NOT EXISTS meta (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         -- Actions à distance (0.8) : demandes des téléphones (achat de tickets…), exécutées par le PC
         CREATE TABLE IF NOT EXISTS actions (
             id TEXT PRIMARY KEY,
             instance TEXT NOT NULL,
             device TEXT NOT NULL,
             kind TEXT NOT NULL,
             payload TEXT NOT NULL,
             status TEXT NOT NULL,
             message TEXT,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS actions_instance ON actions (instance, status);
         CREATE INDEX IF NOT EXISTS actions_device ON actions (device, created_at);
         CREATE TABLE IF NOT EXISTS push_subscriptions (
             device TEXT PRIMARY KEY,
             endpoint TEXT NOT NULL,
             p256dh TEXT NOT NULL,
             auth TEXT NOT NULL,
             events TEXT NOT NULL,
             created_at TEXT NOT NULL
         );",
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
    // Code partagé : applications seulement (tout ce qui écrit)
    let full = Router::new()
        .route("/api/v1/info", get(info))
        .route("/api/v1/admin/instances", get(instances_overview))
        .route("/api/v1/changes", get(changes).post(push))
        .route("/api/v1/instances/{id}", put(mobile::put_instance))
        .route("/api/v1/instances/{id}/state", put(mobile::put_state))
        .route("/api/v1/instances/{id}/devices", get(mobile::list_devices))
        .route("/api/v1/instances/{id}/devices/{device}", delete(mobile::revoke_device))
        .route("/api/v1/pair/start", post(mobile::pair_start))
        .route("/api/v1/instances/{id}/notify", post(push::notify))
        .route("/api/v1/instances/{id}/actions", get(actions::pending))
        .route("/api/v1/instances/{id}/actions/{action}", put(actions::finish))
        // Code vérifié avant la lecture du corps (route_layer : routes connues seulement)
        .route_layer(middleware::from_fn_with_state(server.clone(), require_code));
    // Code partagé ou jeton d'appareil : lecture seule, et abonnement aux notifications de
    // l'appareil lui-même
    let read = Router::new()
        .route("/api/v1/app/overview", get(mobile::overview))
        .route("/api/v1/app/state", get(mobile::state))
        .route("/api/v1/app/gifts/{id}/image", get(mobile::gift_image_route))
        .route("/api/v1/app/actions", get(actions::mine).post(actions::create))
        .route("/api/v1/app/push", get(push::get_push).put(push::put_push).delete(push::delete_push))
        .route_layer(middleware::from_fn_with_state(server.clone(), require_access));
    // Association : sans code, limitée par adresse dans la route
    let open = Router::new().route("/api/v1/pair/claim", post(mobile::pair_claim));
    let api = full
        .merge(read)
        .merge(open)
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(GlobalConcurrencyLimitLayer::new(MAX_CONCURRENT))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, REQUEST_TIMEOUT));
    // Flux d'événements : connexions longues, hors de la limite de requêtes simultanées et du
    // délai maximal (plafonnés à part, MAX_STREAMS)
    let live = Router::new()
        .route("/api/v1/events", get(events))
        .route_layer(middleware::from_fn_with_state(server.clone(), require_access));
    let files = web::router(&server.web_dir);
    Router::new().route("/health", get(health)).merge(api).merge(live).with_state(server).merge(files)
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
        Ok(mut s) => {
            if let Some(dir) = std::env::var("SYNC_WEB_DIR").ok().filter(|d| !d.trim().is_empty()) {
                s.web_dir = PathBuf::from(dir.trim());
            }
            Arc::new(s)
        }
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

    let web_port: Option<u16> = std::env::var("SYNC_WEB_PORT").ok().and_then(|p| p.trim().parse().ok()).filter(|p| *p != port);
    let bind = |port: u16| async move {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        tokio::net::TcpListener::bind(addr).await.map_err(|e| tracing::error!("écoute sur {addr} : {e}"))
    };
    let Ok(listener) = bind(port).await else { return std::process::ExitCode::FAILURE };
    let web_listener = match web_port {
        Some(p) => match bind(p).await {
            Ok(l) => Some(l),
            Err(()) => return std::process::ExitCode::FAILURE,
        },
        None => None,
    };
    tracing::info!(
        "Twitch Desk Sync {} sur le port {port}, base {path}, collections : {}",
        env!("CARGO_PKG_VERSION"),
        server.collections.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    if !server.web_dir.join("index.html").is_file() {
        tracing::info!("web app absente de {} : /app non servi", server.web_dir.display());
    } else if let Some(p) = web_port {
        tracing::info!("web app sur /app (ports {port} et {p})");
    }
    // Arrêt demandé : chaque écoute s'arrête
    let (stop_tx, stop_rx) = watch::channel(false);
    let stopping = server.clone();
    tokio::spawn(async move {
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
        stopping.phone_seq.send_replace(None);
        let _ = stop_tx.send(true);
    });
    let stopped = |mut rx: watch::Receiver<bool>| async move {
        let _ = rx.wait_for(|stop| *stop).await;
    };
    let web_dir = server.web_dir.clone();
    let service = app(server).into_make_service_with_connect_info::<SocketAddr>();
    let api = axum::serve(listener, service).with_graceful_shutdown(stopped(stop_rx.clone()));
    let result = match web_listener {
        None => api.await,
        Some(l) => {
            let files: Router = web::standalone(&web_dir);
            let web = axum::serve(l, files.into_make_service()).with_graceful_shutdown(stopped(stop_rx));
            tokio::try_join!(api.into_future(), web.into_future()).map(|_| ())
        }
    };
    if let Err(e) = result {
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
        s.announce(0, true);
        assert!(tokio::time::timeout(Duration::from_millis(200), body.next()).await.is_err());

        // Arrêt du serveur : le flux se ferme
        s.seq.send_replace(None);
        s.phone_seq.send_replace(None);
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

    #[tokio::test]
    async fn too_many_streams_from_one_address_close_the_oldest() {
        let s = server(&["gifts"]);
        let res = app(s.clone()).oneshot(request("GET", "/api/v1/events", Some("code-secret"), "6.6.6.7", "")).await.unwrap();
        let mut oldest = res.into_body().into_data_stream();
        assert_eq!(next_event(&mut oldest).await, "event: seq\ndata: {\"seq\":0}\n\n");
        let mut kept = vec![];
        for _ in 0..STREAMS_PER_ADDRESS {
            kept.push(app(s.clone()).oneshot(request("GET", "/api/v1/events", Some("code-secret"), "6.6.6.7", "")).await.unwrap());
        }
        // Le plus ancien est fermé ; une autre adresse garde les siens
        assert!(tokio::time::timeout(Duration::from_secs(5), oldest.next()).await.unwrap().is_none());
        let other = app(s.clone()).oneshot(request("GET", "/api/v1/events", Some("code-secret"), "6.6.6.8", "")).await.unwrap();
        assert_eq!(other.status(), StatusCode::OK);
        assert_eq!(s.stream_owners.lock().unwrap().get("ip:6.6.6.7").map(|l| l.len()), Some(STREAMS_PER_ADDRESS));
        drop(kept);
    }

    #[test]
    fn ipv6_blocked_by_its_network() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff:1:2:3".parse().unwrap();
        let other: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(guard_key(a), guard_key(b));
        assert_ne!(guard_key(a), guard_key(other));
        assert_eq!(guard_key("::ffff:5.5.5.5".parse().unwrap()), "5.5.5.5".parse::<IpAddr>().unwrap());
        let guard = Guard::new(3, Duration::from_secs(60), Duration::from_secs(60));
        let now = Instant::now();
        for _ in 0..3 {
            guard.failed(a, now);
        }
        assert!(guard.blocked(b, now).is_some() && guard.blocked(other, now).is_none());
    }

    #[tokio::test]
    async fn many_wrong_pairing_codes_cancel_the_pending_ones() {
        let s = server(&["giveaways"]);
        let code = start_pairing(&s, INSTANCE).await;
        let wrong = if code == "000000" { "111111" } else { "000000" };
        // Depuis beaucoup d'adresses (chacune sous sa propre limite)
        for i in 0..mobile::PAIR_GLOBAL_MAX {
            assert_eq!(claim(&s, wrong, &format!("5.6.{}.{}", i / 4, i % 4 + 1)).await.0, StatusCode::BAD_REQUEST);
        }
        assert_eq!(claim(&s, &code, "5.7.7.7").await.0, StatusCode::BAD_REQUEST);
    }

    /// Requête avec un en-tête `Authorization` quelconque
    fn request_auth(method: &str, uri: &str, auth: &str, ip: &str, body: &str) -> Request {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("x-forwarded-for", ip)
            .header("content-type", "application/json")
            .header("authorization", auth)
            .body(Body::from(body.to_owned()))
            .unwrap()
    }

    #[tokio::test]
    async fn admin_overview_shows_open_apps_accounts_and_devices() {
        let s = server(&["gifts"]);
        let put = |uri: String, body: &str| request("PUT", &uri, Some("code-secret"), "7.7.7.7", body);
        assert_eq!(status(&s, put(format!("/api/v1/instances/{INSTANCE}"), r#"{"name":"PC salon"}"#)).await, StatusCode::OK);
        let state = r#"{"accounts":["alice","bob"],"giveaways":{},"live":{},"pc":{"version":"0.48.37"}}"#;
        assert_eq!(status(&s, put(format!("/api/v1/instances/{INSTANCE}/state"), state)).await, StatusCode::NO_CONTENT);
        let overview = || request("GET", "/api/v1/admin/instances", Some("code-secret"), "7.7.7.7", "");
        let (code, list) = json_of(&s, overview()).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(list[0]["name"], "PC salon");
        assert_eq!(list[0]["accounts"], serde_json::json!(["alice", "bob"]));
        assert_eq!(list[0]["version"], "0.48.37");
        assert_eq!((list[0]["online"].clone(), list[0]["devices"].clone()), (serde_json::json!(false), serde_json::json!(0)));
        // Flux temps réel ouvert par l'application : ouverte
        let res = app(s.clone()).oneshot(request("GET", &format!("/api/v1/events?instance={INSTANCE}"), Some("code-secret"), "7.7.7.7", "")).await.unwrap();
        let mut body = res.into_body().into_data_stream();
        next_event(&mut body).await;
        assert_eq!(json_of(&s, overview()).await.1[0]["online"], true);
        // Réservé au code partagé
        assert_eq!(status(&s, request("GET", "/api/v1/admin/instances", None, "7.7.7.7", "")).await, StatusCode::UNAUTHORIZED);
        assert_eq!(web_url_of(" https://app.exemple.fr/ "), Some("https://app.exemple.fr".into()));
        assert_eq!(web_url_of("ftp://x"), None);
    }

    async fn json_of(s: &Shared, r: Request) -> (StatusCode, serde_json::Value) {
        let res = app(s.clone()).oneshot(r).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    const INSTANCE: &str = "0b6f1c1e-1111-4a2b-9c3d-000000000001";

    /// Code d'association demandé par l'application
    async fn start_pairing(s: &Shared, instance: &str) -> String {
        let body = format!(r#"{{"instance":"{instance}","name":"PC de test"}}"#);
        let (status, v) = json_of(s, request("POST", "/api/v1/pair/start", Some("code-secret"), "1.1.1.1", &body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["expiresIn"], 300);
        let code = v["code"].as_str().unwrap().to_owned();
        assert!(code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()));
        code
    }

    async fn claim(s: &Shared, code: &str, ip: &str) -> (StatusCode, serde_json::Value) {
        let body = format!(r#"{{"code":"{code}","deviceName":"iPhone"}}"#);
        json_of(s, request("POST", "/api/v1/pair/claim", None, ip, &body)).await
    }

    /// Téléphone associé : (jeton, identifiant de l'appareil)
    async fn paired_device(s: &Shared, instance: &str) -> (String, String) {
        let code = start_pairing(s, instance).await;
        let (status, v) = claim(s, &code, "2.2.2.2").await;
        assert_eq!(status, StatusCode::OK);
        (v["token"].as_str().unwrap().to_owned(), v["deviceId"].as_str().unwrap().to_owned())
    }

    #[tokio::test]
    async fn pairing_gives_a_single_use_token() {
        let s = server(&["gifts", "giveaways"]);
        // Sans le code partagé : pas de code d'association
        let body = format!(r#"{{"instance":"{INSTANCE}"}}"#);
        assert_eq!(status(&s, request("POST", "/api/v1/pair/start", None, "1.1.1.1", &body)).await, StatusCode::UNAUTHORIZED);
        assert_eq!(
            status(&s, request("POST", "/api/v1/pair/start", Some("code-secret"), "1.1.1.1", r#"{"instance":"x"}"#)).await,
            StatusCode::BAD_REQUEST
        );

        let code = start_pairing(&s, INSTANCE).await;
        let (st, v) = claim(&s, &code, "2.2.2.2").await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["instance"]["name"], "PC de test");
        let token = v["token"].as_str().unwrap().to_owned();
        assert_eq!(token.len(), 64);
        // Seule l'empreinte du jeton est gardée
        let stored: String = s.db.lock().unwrap().query_row("SELECT token_hash FROM devices", [], |r| r.get(0)).unwrap();
        assert_ne!(stored, token);
        assert_eq!(stored, mobile::token_hash(&token));

        // Le jeton ouvre la vue d'ensemble
        let (st, v) = json_of(&s, request_auth("GET", "/api/v1/app/overview", &format!("Device {token}"), "2.2.2.2", "")).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["instance"]["id"], INSTANCE);
        assert_eq!(v["device"]["name"], "iPhone");
        assert!(v["device"]["lastSeen"].is_string());

        // Code déjà utilisé
        assert_eq!(claim(&s, &code, "2.2.2.2").await.0, StatusCode::BAD_REQUEST);
        // Code expiré
        let code = start_pairing(&s, INSTANCE).await;
        s.pairings.lock().unwrap().get_mut(&code).unwrap().expires = Instant::now() - Duration::from_secs(1);
        assert_eq!(claim(&s, &code, "3.3.3.3").await.0, StatusCode::BAD_REQUEST);
        // Un nouveau code remplace le précédent de la même application
        let first = start_pairing(&s, INSTANCE).await;
        let second = start_pairing(&s, INSTANCE).await;
        if first != second {
            assert_eq!(claim(&s, &first, "3.3.3.3").await.0, StatusCode::BAD_REQUEST);
        }
        assert_eq!(claim(&s, &second, "3.3.3.3").await.0, StatusCode::OK);
        // Jeton inconnu
        let (st, v) = json_of(&s, request_auth("GET", "/api/v1/app/overview", "Device inconnu", "4.4.4.4", "")).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        assert!(v["error"].as_str().unwrap().contains("associe-le"));
    }

    #[tokio::test]
    async fn pairing_attempts_are_limited_per_address() {
        let s = server(&["giveaways"]);
        let code = start_pairing(&s, INSTANCE).await;
        let wrong = if code == "000000" { "111111" } else { "000000" };
        for _ in 0..mobile::PAIR_MAX_FAILURES {
            assert_eq!(claim(&s, wrong, "5.5.5.5").await.0, StatusCode::BAD_REQUEST);
        }
        // Bloquée, même avec le bon code ; les autres adresses non
        assert_eq!(claim(&s, &code, "5.5.5.5").await.0, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(claim(&s, &code, "5.5.5.6").await.0, StatusCode::OK);
        // Corps illisible ou trop gros : refusé sans être lu en entier
        assert_eq!(status(&s, request("POST", "/api/v1/pair/claim", None, "5.5.5.7", "{pas du json")).await, StatusCode::BAD_REQUEST);
        let big = format!(r#"{{"code":"{}"}}"#, "1".repeat(10_000));
        assert_eq!(status(&s, request("POST", "/api/v1/pair/claim", None, "5.5.5.7", &big)).await, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn device_token_is_read_only() {
        let s = server(&["gifts", "giveaways"]);
        let (token, device) = paired_device(&s, INSTANCE).await;
        let auth = format!("Device {token}");
        let push = r#"{"author":"x","changes":[{"collection":"gifts","id":"a","data":{"title":"Voiles"}}]}"#;
        assert_eq!(status(&s, request_auth("POST", "/api/v1/changes", &auth, "6.6.6.1", push)).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(&s, request_auth("GET", "/api/v1/changes", &auth, "6.6.6.1", "")).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(&s, request_auth("GET", "/api/v1/info", &auth, "6.6.6.1", "")).await, StatusCode::UNAUTHORIZED);
        let state_uri = format!("/api/v1/instances/{INSTANCE}/state");
        assert_eq!(status(&s, request_auth("PUT", &state_uri, &auth, "6.6.6.1", "{}")).await, StatusCode::UNAUTHORIZED);
        let devices_uri = format!("/api/v1/instances/{INSTANCE}/devices");
        assert_eq!(status(&s, request_auth("GET", &devices_uri, &auth, "6.6.6.1", "")).await, StatusCode::UNAUTHORIZED);
        let revoke_uri = format!("/api/v1/instances/{INSTANCE}/devices/{device}");
        assert_eq!(status(&s, request_auth("DELETE", &revoke_uri, &auth, "6.6.6.1", "")).await, StatusCode::UNAUTHORIZED);
        let start = format!(r#"{{"instance":"{INSTANCE}"}}"#);
        assert_eq!(status(&s, request_auth("POST", "/api/v1/pair/start", &auth, "6.6.6.1", &start)).await, StatusCode::UNAUTHORIZED);
        assert_eq!(pull(&s, 0).await.changes.len(), 0);
        // Lecture permise : vue d'ensemble et flux
        assert_eq!(status(&s, request_auth("GET", "/api/v1/app/overview", &auth, "6.6.6.2", "")).await, StatusCode::OK);
        assert_eq!(status(&s, request_auth("GET", "/api/v1/events", &auth, "6.6.6.2", "")).await, StatusCode::OK);
        // La vue d'ensemble est celle d'un appareil
        assert_eq!(status(&s, request("GET", "/api/v1/app/overview", Some("code-secret"), "6.6.6.2", "")).await, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn overview_lists_open_then_paused_giveaways_with_their_gifts() {
        let s = server(&["groups", "gifts", "giveaways"]);
        let body = Push {
            author: Some("alice".into()),
            changes: vec![
                change("groups", "g1", Some(serde_json::json!({"name": "Voiles"}))),
                change("gifts", "c1", Some(serde_json::json!({"title": "Obsidian Sails", "image": "data:image/png;base64,iVBORw==", "group": "g1", "gameName": "Sea of Thieves"}))),
                change("gifts", "c2", Some(serde_json::json!({"title": "Pirate", "image": "javascript:alert(1)"}))),
                change("giveaways", "old", Some(serde_json::json!({"channel": "a", "kind": "command", "detectedAt": "2026-01-01T00:00:00Z", "open": false}))),
                change("giveaways", "p1", Some(serde_json::json!({"channel": "b", "kind": "command", "detectedAt": "2026-11-01T12:00:00Z", "open": false, "status": "paused"}))),
                change("giveaways", "o1", Some(serde_json::json!({"channel": "c", "kind": "command", "command": "!join", "detectedAt": "2026-11-01T10:00:00Z", "open": true, "gifts": ["c1", "c2", "absent"]}))),
                change("giveaways", "o2", Some(serde_json::json!({"channel": "d", "kind": "ticket", "detectedAt": "2026-11-01T11:00:00Z", "open": true}))),
            ],
        };
        let _ = push(State(s.clone()), Json(body)).await.ok().unwrap();
        let (token, _) = paired_device(&s, INSTANCE).await;
        let auth = format!("Device {token}");

        // État publié par l'application
        let state = r#"{"accounts":["alice"],"giveaways":{"o1":{"participated":["alice"],"handled":true}},"live":{"c":{"live":true,"viewers":42}}}"#;
        let uri = format!("/api/v1/instances/{INSTANCE}/state");
        assert_eq!(status(&s, request("PUT", &uri, Some("code-secret"), "7.7.7.1", state)).await, StatusCode::NO_CONTENT);
        assert_eq!(status(&s, request("PUT", &uri, Some("code-secret"), "7.7.7.1", "[1]")).await, StatusCode::BAD_REQUEST);

        let (st, v) = json_of(&s, request_auth("GET", "/api/v1/app/overview", &auth, "7.7.7.2", "")).await;
        assert_eq!(st, StatusCode::OK);
        let ids: Vec<&str> = v["giveaways"].as_array().unwrap().iter().map(|g| g["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["o2", "o1", "p1"]);
        let o1 = &v["giveaways"][1];
        assert_eq!(o1["status"], "open");
        assert_eq!(o1["command"], "!join");
        assert_eq!(o1["gifts"].as_array().unwrap().len(), 2);
        assert_eq!(o1["gifts"][0]["title"], "Obsidian Sails");
        assert_eq!(o1["gifts"][0]["group"], "Voiles");
        assert_eq!(o1["gifts"][0]["gameName"], "Sea of Thieves");
        // Image qui n'est ni une data URL ni une adresse https : retirée
        assert!(o1["gifts"][1]["image"].is_null());
        // Image intégrée : envoyée à part, à une adresse qui change avec elle
        let image = o1["gifts"][0]["image"].as_str().unwrap().to_owned();
        assert!(image.starts_with("/api/v1/app/gifts/c1/image?v="), "{image}");
        let res = app(s.clone()).oneshot(request_auth("GET", &image, &auth, "7.7.7.2", "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "image/png");
        assert!(res.headers()[header::CACHE_CONTROL].to_str().unwrap().contains("immutable"));
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], &[0x89, b'P', b'N', b'G'][..]);
        // Réservée aux appareils ; cadeau sans image intégrée : introuvable
        assert_eq!(status(&s, request("GET", &image, Some("code-secret"), "7.7.7.2", "")).await, StatusCode::BAD_REQUEST);
        assert_eq!(status(&s, request_auth("GET", "/api/v1/app/gifts/c2/image", &auth, "7.7.7.2", "")).await, StatusCode::NOT_FOUND);
        assert_eq!(v["giveaways"][2]["status"], "paused");
        assert_eq!(v["state"]["giveaways"]["o1"]["participated"][0], "alice");
        assert!(v["stateAt"].is_string());
        assert_eq!(v["instance"]["name"], "PC de test");
    }

    #[tokio::test]
    async fn device_stream_announces_its_state_and_closes_on_revoke() {
        let s = server(&["gifts", "channels"]);
        let (token, device) = paired_device(&s, INSTANCE).await;
        let (other_token, _) = paired_device(&s, "0b6f1c1e-1111-4a2b-9c3d-000000000002").await;
        let res = app(s.clone()).oneshot(request_auth("GET", "/api/v1/events", &format!("Device {token}"), "8.8.8.1", "")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let mut body = res.into_body().into_data_stream();
        assert_eq!(next_event(&mut body).await, "event: seq\ndata: {\"seq\":0}\n\n");

        // État d'une autre application : rien ; de la sienne : annoncé (une fois s'il ne change pas)
        let other = "/api/v1/instances/0b6f1c1e-1111-4a2b-9c3d-000000000002/state";
        assert_eq!(status(&s, request("PUT", other, Some("code-secret"), "8.8.8.2", r#"{"a":1}"#)).await, StatusCode::NO_CONTENT);
        assert!(tokio::time::timeout(Duration::from_millis(200), body.next()).await.is_err());
        let mine = format!("/api/v1/instances/{INSTANCE}/state");
        assert_eq!(status(&s, request("PUT", &mine, Some("code-secret"), "8.8.8.2", r#"{"a":1}"#)).await, StatusCode::NO_CONTENT);
        assert_eq!(next_event(&mut body).await, format!("event: state\ndata: {{\"instance\":\"{INSTANCE}\"}}\n\n"));
        assert_eq!(status(&s, request("PUT", &mine, Some("code-secret"), "8.8.8.2", r#"{"a":1}"#)).await, StatusCode::NO_CONTENT);
        assert!(tokio::time::timeout(Duration::from_millis(200), body.next()).await.is_err());

        // Chaîne modifiée : absente de la vue du téléphone, rien d'annoncé
        let push = r#"{"changes":[{"collection":"channels","id":"lunarsea","data":{"note":"x"}}]}"#;
        assert_eq!(status(&s, request("POST", "/api/v1/changes", Some("code-secret"), "8.8.8.2", push)).await, StatusCode::OK);
        assert!(tokio::time::timeout(Duration::from_millis(200), body.next()).await.is_err());

        // Modification d'un cadeau : annoncée
        let push = r#"{"changes":[{"collection":"gifts","id":"a","data":{"title":"Voiles"}}]}"#;
        assert_eq!(status(&s, request("POST", "/api/v1/changes", Some("code-secret"), "8.8.8.2", push)).await, StatusCode::OK);
        assert_eq!(next_event(&mut body).await, "event: seq\ndata: {\"seq\":2}\n\n");

        // Appareils de l'application, puis révocation : jeton refusé, flux fermé
        let (st, list) = json_of(&s, request("GET", &format!("/api/v1/instances/{INSTANCE}/devices"), Some("code-secret"), "8.8.8.2", "")).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["id"], device.as_str());
        assert_eq!(list[0]["name"], "iPhone");
        let revoke = format!("/api/v1/instances/{INSTANCE}/devices/{device}");
        assert_eq!(status(&s, request("DELETE", &revoke, Some("code-secret"), "8.8.8.2", "")).await, StatusCode::NO_CONTENT);
        assert!(tokio::time::timeout(Duration::from_secs(5), body.next()).await.unwrap().is_none());
        assert_eq!(status(&s, request_auth("GET", "/api/v1/app/overview", &format!("Device {token}"), "8.8.8.3", "")).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(&s, request("DELETE", &revoke, Some("code-secret"), "8.8.8.2", "")).await, StatusCode::NOT_FOUND);
        // L'appareil de l'autre application reste associé
        assert_eq!(status(&s, request_auth("GET", "/api/v1/app/overview", &format!("Device {other_token}"), "8.8.8.3", "")).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn phones_subscribe_to_the_alerts_they_choose() {
        let s = server(&["giveaways"]);
        let (token, device) = paired_device(&s, INSTANCE).await;
        let (token2, _) = paired_device(&s, INSTANCE).await;
        let (auth, auth2) = (format!("Device {token}"), format!("Device {token2}"));
        let (status_code, v) = json_of(&s, request_auth("GET", "/api/v1/app/push", &auth, "7.7.7.1", "")).await;
        assert_eq!(status_code, StatusCode::OK);
        assert_eq!(v["publicKey"], s.push.public_key.as_str());
        assert!(v["endpoint"].is_null());
        // Clés d'un navigateur (point de la courbe P-256 valide)
        let p256dh = "BLn9b-VR0ca83knDNZ32dCHGyjJp-1riX9ZTN40MqV8K_LpQmLqxC_DoHvqvFXO_nGdAB4W9dogZb_sM-uV4JbY";
        let sub = |endpoint: &str, events: &str| {
            format!(r#"{{"endpoint":"{endpoint}","keys":{{"p256dh":"{p256dh}","auth":"_ordMnz7uTCmrpBTeUV4Bw"}},"events":{events}}}"#)
        };
        let local = sub("https://127.0.0.1/x", r#"["win"]"#);
        assert_eq!(status(&s, request_auth("PUT", "/api/v1/app/push", &auth, "7.7.7.1", &local)).await, StatusCode::BAD_REQUEST);
        let body = sub("https://fcm.googleapis.com/fcm/send/a", r#"["win","giveaway","inconnue","win"]"#);
        let (status_code, v) = json_of(&s, request_auth("PUT", "/api/v1/app/push", &auth, "7.7.7.1", &body)).await;
        assert_eq!(status_code, StatusCode::OK);
        assert_eq!(v["events"], serde_json::json!(["giveaway", "win"]));
        let body2 = sub("https://fcm.googleapis.com/fcm/send/b", r#"["win"]"#);
        assert_eq!(status(&s, request_auth("PUT", "/api/v1/app/push", &auth2, "7.7.7.1", &body2)).await, StatusCode::OK);
        // Le code partagé n'a pas d'abonnement ; un appareil n'envoie pas d'alerte
        assert_eq!(status(&s, request("PUT", "/api/v1/app/push", Some("code-secret"), "7.7.7.1", &body)).await, StatusCode::BAD_REQUEST);
        let notify_uri = format!("/api/v1/instances/{INSTANCE}/notify");
        let alert = |a: &str| format!(r#"{{"alert":"{a}","title":"Titre","body":"Texte"}}"#);
        assert_eq!(status(&s, request_auth("POST", &notify_uri, &auth, "7.7.7.1", &alert("win"))).await, StatusCode::UNAUTHORIZED);
        let targets = |a: &str| push::targets(&s.db.lock().unwrap(), INSTANCE, a).unwrap().len();
        assert_eq!((targets("win"), targets("giveaway"), targets("scanner")), (2, 1, 0));
        assert_eq!(push::targets(&s.db.lock().unwrap(), "autre-application-1", "win").unwrap().len(), 0);
        let (status_code, v) = json_of(&s, request("POST", &notify_uri, Some("code-secret"), "7.7.7.1", &alert("scanner"))).await;
        assert_eq!((status_code, v["devices"].as_u64()), (StatusCode::OK, Some(0)));
        assert_eq!(status(&s, request("POST", &notify_uri, Some("code-secret"), "7.7.7.1", &alert("autre"))).await, StatusCode::BAD_REQUEST);
        // Appareil révoqué : son abonnement part avec lui
        let revoke_uri = format!("/api/v1/instances/{INSTANCE}/devices/{device}");
        assert_eq!(status(&s, request("DELETE", &revoke_uri, Some("code-secret"), "7.7.7.1", "")).await, StatusCode::NO_CONTENT);
        assert_eq!((targets("win"), targets("giveaway")), (1, 0));
        // Désabonnement
        assert_eq!(status(&s, request_auth("DELETE", "/api/v1/app/push", &auth2, "7.7.7.1", "")).await, StatusCode::NO_CONTENT);
        assert_eq!(targets("win"), 0);
    }

    #[tokio::test]
    async fn blocked_address_keeps_valid_phones() {
        let s = server(&["giveaways"]);
        let (token, _) = paired_device(&s, INSTANCE).await;
        // Un PC au code périmé derrière la même adresse : elle est bloquée pour le code
        for _ in 0..MAX_FAILURES {
            assert_eq!(status(&s, request("GET", "/api/v1/changes", Some("ancien"), "6.6.6.6", "")).await, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(status(&s, request("GET", "/api/v1/changes", Some("code-secret"), "6.6.6.6", "")).await, StatusCode::TOO_MANY_REQUESTS);
        // Le téléphone de la maison, jeton valide : servi quand même
        assert_eq!(status(&s, request_auth("GET", "/api/v1/app/overview", &format!("Device {token}"), "6.6.6.6", "")).await, StatusCode::OK);
        // Jeton inventé : toujours bloqué
        assert_eq!(status(&s, request_auth("GET", "/api/v1/app/overview", "Device invente", "6.6.6.6", "")).await, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn same_browser_paired_again_replaces_its_old_device() {
        let s = server(&["giveaways"]);
        let (old_token, _) = paired_device(&s, INSTANCE).await;
        let (new_token, _) = paired_device(&s, INSTANCE).await;
        let p256dh = "BLn9b-VR0ca83knDNZ32dCHGyjJp-1riX9ZTN40MqV8K_LpQmLqxC_DoHvqvFXO_nGdAB4W9dogZb_sM-uV4JbY";
        let body = format!(r#"{{"endpoint":"https://fcm.googleapis.com/fcm/send/same","keys":{{"p256dh":"{p256dh}","auth":"_ordMnz7uTCmrpBTeUV4Bw"}},"events":["win"]}}"#);
        assert_eq!(status(&s, request_auth("PUT", "/api/v1/app/push", &format!("Device {old_token}"), "7.7.7.1", &body)).await, StatusCode::OK);
        // Nouveau QR code scanné dans le même navigateur : même adresse d'abonnement
        assert_eq!(status(&s, request_auth("PUT", "/api/v1/app/push", &format!("Device {new_token}"), "7.7.7.1", &body)).await, StatusCode::OK);
        // Une seule notification par alerte, l'ancien appareil n'est plus associé
        assert_eq!(push::targets(&s.db.lock().unwrap(), INSTANCE, "win").unwrap().len(), 1);
        assert_eq!(status(&s, request_auth("GET", "/api/v1/app/overview", &format!("Device {old_token}"), "7.7.7.1", "")).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(&s, request_auth("GET", "/api/v1/app/overview", &format!("Device {new_token}"), "7.7.7.1", "")).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn phone_actions_go_through_the_pc() {
        let s = server(&["giveaways"]);
        let (token, _) = paired_device(&s, INSTANCE).await;
        let auth = format!("Device {token}");
        let body = r#"{"kind":"se_buy","account":"Alice","channel":"lunarsea","giveaway":"6ae9","tickets":3}"#;
        // Le code partagé ne crée pas de demande ; type inconnu refusé
        assert_eq!(status(&s, request("POST", "/api/v1/app/actions", Some("code-secret"), "9.9.9.1", body)).await, StatusCode::BAD_REQUEST);
        let bad = body.replace("se_buy", "delete_all");
        assert_eq!(status(&s, request_auth("POST", "/api/v1/app/actions", &auth, "9.9.9.1", &bad)).await, StatusCode::BAD_REQUEST);
        let (st, v) = json_of(&s, request_auth("POST", "/api/v1/app/actions", &auth, "9.9.9.1", body)).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!((v["status"].as_str(), v["payload"]["account"].as_str()), (Some("pending"), Some("alice")));
        let id = v["id"].as_str().unwrap().to_owned();
        // Le téléphone ne lit pas la file du PC ; le PC la lit avec le code
        let pending_uri = format!("/api/v1/instances/{INSTANCE}/actions");
        assert_eq!(status(&s, request_auth("GET", &pending_uri, &auth, "9.9.9.1", "")).await, StatusCode::UNAUTHORIZED);
        let (st, v) = json_of(&s, request("GET", &pending_uri, Some("code-secret"), "9.9.9.1", "")).await;
        assert_eq!((st, v.as_array().map(Vec::len)), (StatusCode::OK, Some(1)));
        // Résultat rendu par le PC, une seule fois
        let finish_uri = format!("/api/v1/instances/{INSTANCE}/actions/{id}");
        let done = r#"{"ok":true,"message":"3 tickets achetés (5 au total)"}"#;
        assert_eq!(status(&s, request("PUT", &finish_uri, Some("code-secret"), "9.9.9.1", done)).await, StatusCode::NO_CONTENT);
        assert_eq!(status(&s, request("PUT", &finish_uri, Some("code-secret"), "9.9.9.1", done)).await, StatusCode::NOT_FOUND);
        let (_, v) = json_of(&s, request("GET", &pending_uri, Some("code-secret"), "9.9.9.1", "")).await;
        assert_eq!(v.as_array().map(Vec::len), Some(0));
        let (_, v) = json_of(&s, request_auth("GET", "/api/v1/app/actions", &auth, "9.9.9.1", "")).await;
        assert_eq!((v[0]["status"].as_str(), v[0]["message"].as_str()), (Some("done"), Some("3 tickets achetés (5 au total)")));
        // Mise à jour du PC : demande sans contenu
        let (st, v) = json_of(&s, request_auth("POST", "/api/v1/app/actions", &auth, "9.9.9.1", r#"{"kind":"update_check"}"#)).await;
        assert_eq!((st, v["kind"].as_str(), v["payload"].clone()), (StatusCode::OK, Some("update_check"), serde_json::json!({})));
        // État léger de l'application
        let (st, v) = json_of(&s, request_auth("GET", "/api/v1/app/state", &auth, "9.9.9.1", "")).await;
        assert_eq!((st, v["state"].is_null()), (StatusCode::OK, true));
    }

    #[tokio::test]
    async fn instance_registration() {
        let s = server(&["gifts"]);
        let uri = format!("/api/v1/instances/{INSTANCE}");
        assert_eq!(status(&s, request("PUT", &uri, None, "9.1.1.1", r#"{"name":"PC"}"#)).await, StatusCode::UNAUTHORIZED);
        let (st, v) = json_of(&s, request("PUT", &uri, Some("code-secret"), "9.1.1.1", r#"{"name":"  PC du salon "}"#)).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["name"], "PC du salon");
        assert_eq!(status(&s, request("PUT", "/api/v1/instances/a%20b", Some("code-secret"), "9.1.1.1", "{}")).await, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn web_app_is_served_under_app() {
        let dir = std::env::temp_dir().join(format!("tdsync-web-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), "<html>web app</html>").unwrap();
        std::fs::write(dir.join("assets/index-abc123.js"), "console.log(1)").unwrap();
        std::fs::write(dir.join("sw.js"), "self").unwrap();
        let mut srv = Server::new("code-secret".into(), ["gifts".to_string()].into(), open_db(":memory:").unwrap(), 1).unwrap();
        srv.web_dir = dir.clone();
        let s = Arc::new(srv);
        let get = |uri: &str| Request::builder().uri(uri).body(Body::empty()).unwrap();
        let read = |res: Response| async move {
            let cache = res.headers().get(header::CACHE_CONTROL).map(|v| v.to_str().unwrap().to_owned());
            (res.status(), cache, String::from_utf8(axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap())
        };

        let (st, cache, body) = read(app(s.clone()).oneshot(get("/app/")).await.unwrap()).await;
        assert_eq!((st, cache.as_deref(), body.as_str()), (StatusCode::OK, Some("no-cache"), "<html>web app</html>"));
        let (st, cache, _) = read(app(s.clone()).oneshot(get("/app/assets/index-abc123.js")).await.unwrap()).await;
        assert_eq!((st, cache.as_deref()), (StatusCode::OK, Some("public, max-age=31536000, immutable")));
        let (st, cache, _) = read(app(s.clone()).oneshot(get("/app/sw.js")).await.unwrap()).await;
        assert_eq!((st, cache.as_deref()), (StatusCode::OK, Some("no-cache")));
        // Navigation dans la web app : index.html
        let (st, _, body) = read(app(s.clone()).oneshot(get("/app/giveaways/42")).await.unwrap()).await;
        assert_eq!((st, body.as_str()), (StatusCode::OK, "<html>web app</html>"));
        // Fichier absent : 404, pas index.html
        assert_eq!(app(s.clone()).oneshot(get("/app/assets/index-old.js")).await.unwrap().status(), StatusCode::NOT_FOUND);
        assert_eq!(app(s.clone()).oneshot(get("/app/icon-9.png")).await.unwrap().status(), StatusCode::NOT_FOUND);
        // Hors de /app : rien de servi, l'API reste protégée
        assert_eq!(app(s.clone()).oneshot(get("/index.html")).await.unwrap().status(), StatusCode::NOT_FOUND);
        assert_eq!(app(s.clone()).oneshot(get("/app/../Cargo.toml")).await.unwrap().status(), StatusCode::NOT_FOUND);

        // Port propre à la web app : /app seulement, la racine y mène
        let web: Router = web::standalone(&dir);
        let res = web.clone().oneshot(get("/")).await.unwrap();
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(res.headers()[header::LOCATION], "/app/");
        assert_eq!(web.clone().oneshot(get("/app/")).await.unwrap().status(), StatusCode::OK);
        assert_eq!(web.clone().oneshot(get("/api/v1/info")).await.unwrap().status(), StatusCode::NOT_FOUND);
        assert_eq!(web.oneshot(get("/health")).await.unwrap().status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(&dir);
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
