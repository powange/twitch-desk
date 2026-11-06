//! Synchronisation avec un serveur partagé (dossier `server/` du dépôt) : cadeaux, groupes,
//! giveaways, chaînes surveillées et fiches des streamers, selon ce que le serveur synchronise.
//!
//! Chaque passage récupère d'abord les modifications des autres depuis le dernier curseur,
//! puis envoie les nôtres : tout document dont le contenu diffère du dernier état envoyé ou
//! reçu (`sync_state`), et les suppressions. La dernière modification reçue par le serveur
//! l'emporte ; une modification locale pas encore envoyée n'est pas écrasée.
//!
//! Deux applications qui créent le même élément (même cadeau, même groupe, même giveaway
//! ouvert) le fusionnent : chacune garde le plus petit identifiant, l'autre est supprimé.
//!
//! Temps réel (serveur 0.5 et plus) : un flux Server-Sent Events (`/api/v1/events`) annonce
//! chaque modification enregistrée ; un passage est lancé tout de suite et le passage régulier
//! s'espace tant que le flux est ouvert. Sans flux (serveur plus ancien, proxy qui le bloque),
//! le passage reste toutes les 30 s.
//!
//! Web app mobile (serveur 0.6 et plus) : l'application s'enregistre sous un identifiant tiré
//! une fois pour toutes et un nom choisi, associe des téléphones (code à 6 chiffres) et publie
//! un petit état (participations de ses comptes, statut live des chaînes) que les téléphones
//! lisent.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::app::App;
use crate::db::Db;
use crate::giveaways::MonitorCmd;

/// Collections, dans l'ordre où les appliquer (groupes avant cadeaux, cadeaux avant giveaways)
pub const COLLECTIONS: [&str; 6] = ["groups", "gifts", "giveaways", "channels", "streamers", "detection"];
const EVERY: Duration = Duration::from_secs(30);
/// Passage régulier quand le serveur annonce ses modifications en temps réel
const EVERY_LIVE: Duration = Duration::from_secs(3 * 60);
/// Flux temps réel : attente avant de se reconnecter, doublée à chaque échec
const LIVE_RETRY_MIN: Duration = Duration::from_secs(2);
const LIVE_RETRY_MAX: Duration = Duration::from_secs(60);
/// Serveur sans flux (avant la 0.5) ou code refusé : nouvel essai bien plus tard
const LIVE_PAUSE: Duration = Duration::from_secs(30 * 60);
/// Code de synchronisation refusé : nouvel essai automatique avec le même code après ce délai
const CODE_REFUSED_PAUSE: Duration = Duration::from_secs(30 * 60);
/// Rien reçu pendant ce délai (le serveur envoie un commentaire toutes les 20 s) : connexion perdue
const LIVE_IDLE: Duration = Duration::from_secs(60);
/// Connexion renouvelée au plus tard après ce délai
const LIVE_MAX: Duration = Duration::from_secs(3600);
/// Flux fermé plus tôt que ça après l'ouverture : compté comme un échec (proxy qui le coupe)
const LIVE_SHORT: Duration = Duration::from_secs(30);
/// Après une modification locale : petit délai pour regrouper les envois
const DEBOUNCE: Duration = Duration::from_secs(2);
const CODE_KEY: &str = "sync:code";
/// Giveaways envoyés : ouverts, ou détectés depuis moins de 60 jours
const GIVEAWAY_DAYS: i64 = 60;
const PUSH_BATCH_DOCS: usize = 100;
/// Version de `sync_state` (2 : empreinte de ce qui est enregistré après réception)
const STATE_FORMAT: &str = "2";
const PUSH_BATCH_BYTES: usize = 8 * 1024 * 1024;
/// État publié pour la web app : regroupé sur ce délai, republié au moins à ce rythme
const STATE_DEBOUNCE: Duration = Duration::from_secs(3);
const STATE_REFRESH: Duration = Duration::from_secs(30 * 60);
/// Serveur d'avant la 0.6 (pas de web app) : nouvel essai bien plus tard
const STATE_UNSUPPORTED_PAUSE: Duration = Duration::from_secs(30 * 60);
/// Alertes pour les téléphones en attente d'envoi, au plus (serveur injoignable : les plus
/// anciennes sont oubliées)
const MAX_PHONE_ALERTS: usize = 20;
const NOT_SYNC_SERVER: &str = "Ce n'est pas un serveur de synchronisation Twitch Desk";
const OLD_SERVER: &str = "Mets à jour le serveur de synchronisation (version 0.6 ou plus) pour connecter un téléphone";

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncView {
    pub enabled: bool,
    pub url: String,
    /// « off », « syncing », « ok » ou « error »
    pub status: String,
    pub last_sync: Option<String>,
    pub error: Option<String>,
    /// Collections proposées par le serveur
    pub server_collections: Vec<String>,
    /// Collections synchronisées : celles du serveur que l'utilisateur n'a pas décochées
    pub collections: Vec<String>,
    /// Éléments par collection, ici et sur le serveur (diagnostic)
    pub local_counts: BTreeMap<String, i64>,
    pub server_counts: Option<BTreeMap<String, i64>>,
    /// Depuis le démarrage : modifications envoyées et reçues
    pub sent: u64,
    pub received: u64,
    /// Augmente à chaque modification reçue : l'interface relit ses données
    pub revision: u64,
    /// Flux temps réel : « off », « connecting », « connected », « reconnecting »,
    /// « unavailable » (serveur d'avant la 0.5) ou « refused » (code refusé)
    pub live: String,
}

#[derive(Default)]
pub struct SyncHandle {
    view: Mutex<SyncView>,
    wake: tokio::sync::Notify,
    /// Modification annoncée par le serveur : passage tout de suite, sans DEBOUNCE
    pull: tokio::sync::Notify,
    /// « Synchroniser » demandé : passage fait même avec un code refusé juste avant
    forced: std::sync::atomic::AtomicBool,
    /// Code refusé par le serveur : état, alertes et demandes du téléphone en attente aussi
    code_refused: std::sync::atomic::AtomicBool,
    /// Réglages changés : le flux temps réel est rouvert
    reconfigure: tokio::sync::Notify,
    /// État du flux temps réel (voir `SyncView::live`), vide : « off »
    live: Mutex<&'static str>,
    /// Giveaways, comptes ou statut live changés : état de l'application à republier
    state_changed: tokio::sync::Notify,
    /// Application enregistrée sur le serveur : « adresse|nom » (réenregistrée s'il change)
    registered: Mutex<Option<String>>,
    /// Alertes à transmettre aux téléphones (notifications push)
    phone_alerts: Mutex<VecDeque<PhoneAlert>>,
    phone_alert: tokio::sync::Notify,
    /// Demande d'un téléphone annoncée par le serveur : à lire tout de suite
    phone_action: tokio::sync::Notify,
    /// Dernière vérification des mises à jour demandée par un téléphone (`{available, checkedAt}`)
    pc_update: Mutex<Value>,
}

#[derive(Debug, Serialize)]
struct PhoneAlert {
    alert: &'static str,
    title: String,
    body: String,
    /// Cible de l'alerte, ouverte au clic sur le téléphone : la chaîne (giveaway, rappel, gain,
    /// mention, stream repéré) ou le compte (drop)
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    /// Mise en file : une alerte trop ancienne n'est plus renvoyée après une erreur
    #[serde(skip)]
    queued: std::time::Instant,
}

/// Alerte non transmise (réseau, serveur redémarré) : renvoyée à ce rythme, pendant au plus
/// PHONE_ALERT_TTL (un « Tu as gagné » plus vieux n'a plus d'intérêt)
const PHONE_ALERT_RETRY: Duration = Duration::from_secs(30);
const PHONE_ALERT_TTL: Duration = Duration::from_secs(15 * 60);

impl SyncHandle {
    pub fn view(&self) -> SyncView {
        let mut v = self.view.lock().unwrap().clone();
        v.live = self.live_state().to_owned();
        v
    }

    fn live_state(&self) -> &'static str {
        match *self.live.lock().unwrap() {
            "" => "off",
            s => s,
        }
    }

    /// Données modifiées ou réglages changés : synchroniser bientôt
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    /// Giveaways, comptes ou statut live changés : état publié pour la web app mobile
    /// (regroupé, STATE_DEBOUNCE)
    pub fn state_changed(&self) {
        self.state_changed.notify_one();
    }

    /// Alerte transmise au serveur, qui l'envoie aux téléphones qui l'ont choisie
    pub fn phone_alert(&self, alert: &'static str, title: &str, body: &str, target: Option<&str>) {
        let mut queue = self.phone_alerts.lock().unwrap();
        if queue.len() >= MAX_PHONE_ALERTS {
            queue.pop_front();
        }
        queue.push_back(PhoneAlert {
            alert,
            title: title.to_owned(),
            body: body.to_owned(),
            target: target.map(str::to_lowercase),
            queued: std::time::Instant::now(),
        });
        drop(queue);
        self.phone_alert.notify_one();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub version: String,
    pub collections: Vec<String>,
    pub seq: i64,
    /// Documents par collection (serveur 0.2 et plus)
    #[serde(default)]
    pub counts: Option<BTreeMap<String, i64>>,
    /// Adresse de la web app réglée sur le serveur (`SYNC_WEB_URL`, serveur 0.8.8 et plus)
    #[serde(default, rename = "webUrl")]
    pub web_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteChange {
    collection: String,
    id: String,
    data: Option<Value>,
    deleted: bool,
}

#[derive(Debug, Deserialize)]
struct ChangesPage {
    changes: Vec<RemoteChange>,
    seq: i64,
    more: bool,
}

#[derive(Debug, Serialize)]
struct OutgoingChange<'a> {
    collection: &'a str,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    deleted: bool,
}

// --- Configuration ---

struct Config {
    url: String,
    code: String,
}

fn meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sync_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Collections décochées par l'utilisateur (celles que le serveur ajoutera plus tard sont cochées).
/// Tant qu'il n'a rien choisi, les chaînes surveillées ne sont pas partagées : chacun garde
/// sa liste, sauf s'il coche la case
fn skipped(conn: &Connection) -> Vec<String> {
    meta(conn, "skipped").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| vec!["channels".to_owned()])
}

fn config(app: &App) -> Option<Config> {
    let conn = app.db.conn();
    if meta(&conn, "enabled").as_deref() != Some("1") {
        return None;
    }
    let url = meta(&conn, "url").filter(|u| !u.is_empty())?;
    drop(conn);
    let code = app.secrets.load_raw(CODE_KEY).filter(|c| !c.is_empty())?;
    Some(Config { url, code })
}

/// Adresse du serveur sans « / » final ; http:// ou https:// obligatoire
fn normalize_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    let parsed = url::Url::parse(url).map_err(|_| "Adresse du serveur invalide".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("L'adresse du serveur doit commencer par http:// ou https://".into());
    }
    Ok(url.to_owned())
}

// --- Contenu des documents ---

/// Empreinte FNV-1a 64 bits du JSON canonique, aux clés triées (stable d'une version à
/// l'autre). Les objets de serde_json gardent leurs clés triées (pas de « preserve_order ») :
/// le texte est écrit directement dans l'empreinte, sans copie ni chaîne intermédiaire, même
/// pour une image de plusieurs Mo
fn hash(value: &Value) -> String {
    struct Fnv(u64);
    impl std::io::Write for Fnv {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            for b in buf {
                self.0 ^= *b as u64;
                self.0 = self.0.wrapping_mul(0x100000001b3);
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut h = Fnv(0xcbf29ce484222325);
    serde_json::to_writer(&mut h, value).expect("JSON sérialisable");
    format!("{:016x}", h.0)
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn bool_of(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

fn int_of(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

/// Ancienneté du follow en secondes ; document d'une version avant la 0.48.2 : en jours
fn follow_secs_of(v: &Value) -> Option<i64> {
    int_of(v, "followMinSecs").or_else(|| int_of(v, "followMinDays").and_then(|d| d.checked_mul(86_400)))
}

fn secs_to_days(secs: i64) -> i64 {
    (secs + 86_399) / 86_400
}

fn strings_of(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

pub fn valid_login(login: &str) -> bool {
    !login.is_empty() && login.len() <= 25 && login.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Règle de détection (formulation de gagnant ou bot) : identifiant tiré de son contenu,
/// la même règle ajoutée sur deux postes n'est qu'un seul document
fn rule_id(kind: &str, value: &str) -> String {
    match kind {
        "bot" => format!("bot:{value}"),
        _ => format!("{kind}:{}", hash(&json!(value.to_lowercase()))),
    }
}

fn rules(conn: &Connection) -> rusqlite::Result<Vec<(String, Value)>> {
    conn.prepare_cached("SELECT kind, value FROM detection_rules")?
        .query_map([], |r| {
            let (kind, value): (String, String) = (r.get(0)?, r.get(1)?);
            Ok((rule_id(&kind, &value), json!({ "kind": kind, "value": value })))
        })?
        .collect()
}

/// Image d'un cadeau reçue : data URL d'image ou adresse https:// seulement
fn safe_image(image: Option<String>) -> Option<String> {
    image.filter(|i| {
        (i.starts_with("data:image/") && i.len() <= 3 * 1024 * 1024)
            || (i.starts_with("https://") && i.len() <= 2048 && !i.contains(char::is_whitespace))
    })
}

/// Donne un identifiant partagé aux groupes, cadeaux et giveaways qui n'en ont pas
fn assign_ids(conn: &Connection) -> rusqlite::Result<()> {
    for table in ["gift_groups", "gifts", "giveaways"] {
        let ids: Vec<i64> = conn
            .prepare(&format!("SELECT id FROM {table} WHERE sync_id IS NULL"))?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in ids {
            conn.execute(&format!("UPDATE {table} SET sync_id = ?2 WHERE id = ?1"), params![id, uuid::Uuid::new_v4().to_string()])?;
        }
    }
    Ok(())
}

/// Documents locaux d'une collection : identifiant partagé → contenu. Les liens vers une
/// collection que le serveur ne synchronise pas (groupe d'un cadeau, cadeaux d'un giveaway)
/// ne sont pas envoyés : chacun garde les siens
fn export(conn: &Connection, collection: &str, enabled: &[&str]) -> rusqlite::Result<Vec<(String, Value)>> {
    let mut docs = export_all(conn, collection, None, true)?;
    for (_, data) in &mut docs {
        strip_links(collection, enabled, data);
    }
    Ok(docs)
}

fn strip_links(collection: &str, enabled: &[&str], data: &mut Value) {
    let unlinked = match collection {
        "gifts" if !enabled.contains(&"groups") => "group",
        "giveaways" if !enabled.contains(&"gifts") => "gifts",
        _ => return,
    };
    data.as_object_mut().map(|o| o.remove(unlinked));
}

/// Un document tel qu'il est enregistré localement (mêmes règles que `export`, sans limite
/// de date pour les giveaways)
fn export_one(conn: &Connection, collection: &str, id: &str, enabled: &[&str]) -> rusqlite::Result<Option<Value>> {
    let mut doc = export_all(conn, collection, Some(id), false)?.into_iter().next().map(|(_, v)| v);
    if let Some(d) = doc.as_mut() {
        strip_links(collection, enabled, d);
    }
    Ok(doc)
}

/// Un document tel que `export` l'enverrait (absent : supprimé, ou giveaway trop ancien).
/// Règle de détection en double à la casse près : la dernière, comme `export` rangé en table
fn export_sent(conn: &Connection, collection: &str, id: &str, enabled: &[&str]) -> rusqlite::Result<Option<Value>> {
    let mut doc = export_all(conn, collection, Some(id), true)?.pop().map(|(_, v)| v);
    if let Some(d) = doc.as_mut() {
        strip_links(collection, enabled, d);
    }
    Ok(doc)
}

/// `only` : un seul document ; `windowed` : giveaways ouverts ou récents seulement (ceux qu'on envoie)
fn export_all(conn: &Connection, collection: &str, only: Option<&str>, windowed: bool) -> rusqlite::Result<Vec<(String, Value)>> {
    // Un seul document : condition écrite pour l'index de l'identifiant (« ?n IS NULL OR … »
    // ferait relire toute la table à chaque document)
    let one = |column: &str, n: u8| if only.is_some() { format!("{column} = ?{n}") } else { format!("?{n} IS NULL") };
    match collection {
        "groups" => conn
            .prepare_cached(&format!("SELECT sync_id, name, game_id, game_name FROM gift_groups WHERE sync_id IS NOT NULL AND {}", one("sync_id", 1)))?
            .query_map([only], |r| {
                Ok((r.get(0)?, json!({ "name": r.get::<_, String>(1)?, "gameId": r.get::<_, Option<String>>(2)?, "gameName": r.get::<_, Option<String>>(3)? })))
            })?
            .collect(),
        "gifts" => conn
            .prepare_cached(&format!(
                "SELECT g.sync_id, g.title, g.image, gr.sync_id, g.keywords, g.game_id, g.game_name
                 FROM gifts g LEFT JOIN gift_groups gr ON gr.id = g.group_id
                 WHERE g.sync_id IS NOT NULL AND {}",
                one("g.sync_id", 1)
            ))?
            .query_map([only], |r| {
                let keywords: Vec<String> = serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default();
                Ok((
                    r.get(0)?,
                    json!({
                        "title": r.get::<_, String>(1)?,
                        "image": r.get::<_, Option<String>>(2)?,
                        "group": r.get::<_, Option<String>>(3)?,
                        "keywords": keywords,
                        "gameId": r.get::<_, Option<String>>(5)?,
                        "gameName": r.get::<_, Option<String>>(6)?,
                    }),
                ))
            })?
            .collect(),
        "giveaways" => {
            let since = giveaway_since();
            let mut gifts = conn.prepare_cached(
                "SELECT gi.sync_id FROM giveaway_gifts gg JOIN gifts gi ON gi.id = gg.gift_id
                 WHERE gg.giveaway_id = ?1 AND gi.sync_id IS NOT NULL ORDER BY gi.sync_id",
            )?;
            let mut removed = conn.prepare_cached(
                "SELECT gi.sync_id FROM giveaway_gifts_rejected gr JOIN gifts gi ON gi.id = gr.gift_id
                 WHERE gr.giveaway_id = ?1 AND gi.sync_id IS NOT NULL ORDER BY gi.sync_id",
            )?;
            let rows: Vec<(i64, String, bool, Value, String, String)> = conn
                .prepare_cached(&format!(
                    "SELECT id, sync_id, channel, kind, command, url, detected_at, status, manual, winner, winner_chance, drawn_at,
                            ticket_unique, ticket_max, reward_name, reward_cost, draw_at,
                            require_follow, require_presence, require_discord, notes,
                            require_greeting, min_chat_minutes, follow_min_secs, winners, winner_chances, can_gift
                     FROM giveaways WHERE sync_id IS NOT NULL
                         AND {} AND (?3 = 0 OR status IN ('open', 'paused') OR detected_at >= ?1 OR manual = 1)",
                    one("sync_id", 2)
                ))?
                .query_map(params![since, only, windowed], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get::<_, String>(7)? == "paused",
                        json!({
                            "channel": r.get::<_, String>(2)?,
                            "kind": r.get::<_, String>(3)?,
                            "command": r.get::<_, Option<String>>(4)?,
                            "url": r.get::<_, Option<String>>(5)?,
                            "detectedAt": r.get::<_, String>(6)?,
                            "open": r.get::<_, String>(7)? == "open",
                            "manual": r.get::<_, bool>(8)?,
                            "winner": r.get::<_, Option<String>>(9)?,
                            "winnerChance": r.get::<_, Option<String>>(10)?,
                            "drawnAt": r.get::<_, Option<String>>(11)?,
                            "ticketUnique": r.get::<_, Option<bool>>(12)?,
                            "ticketMax": r.get::<_, Option<i64>>(13)?,
                            "rewardName": r.get::<_, Option<String>>(14)?,
                            "rewardCost": r.get::<_, Option<i64>>(15)?,
                            "drawAt": r.get::<_, Option<String>>(16)?,
                            "requireFollow": r.get::<_, Option<bool>>(17)?,
                            "requirePresence": r.get::<_, Option<bool>>(18)?,
                            "requireDiscord": r.get::<_, Option<bool>>(19)?,
                            "notes": r.get::<_, Option<String>>(20)?,
                            "requireGreeting": r.get::<_, Option<bool>>(21)?,
                            "minChatMinutes": r.get::<_, Option<i64>>(22)?,
                            "followMinSecs": r.get::<_, Option<i64>>(23)?,
                            // Versions avant la 0.48.2 : en jours (arrondi au-dessus)
                            "followMinDays": r.get::<_, Option<i64>>(23)?.map(secs_to_days),
                        }),
                        r.get::<_, String>(24)?,
                        r.get::<_, String>(25)?,
                    ))
                    .map(|(id, sync_id, paused, mut data, winners, chances)| {
                        // Gain offrable ou non : champ ajouté seulement s'il est connu (empreinte
                        // des autres inchangée)
                        if let Ok(Some(can)) = r.get::<_, Option<bool>>(26) {
                            data["canGift"] = json!(can);
                        }
                        (id, sync_id, paused, data, winners, chances)
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            rows.into_iter()
                .map(|(id, sync_id, paused, mut data, winners, chances)| {
                    // Plusieurs gagnants : liste ajoutée seulement là (empreinte des autres
                    // inchangée) ; une version plus ancienne n'y voit que le premier
                    let winners: Vec<String> = serde_json::from_str(&winners).unwrap_or_default();
                    if winners.len() > 1 {
                        data["winners"] = json!(winners);
                        data["winnerChances"] = serde_json::from_str::<Value>(&chances).unwrap_or_else(|_| json!({}));
                    }
                    let list: Vec<String> = gifts.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                    data["gifts"] = json!(list);
                    // Cadeaux retirés à la main : champ ajouté seulement s'il y en a (empreinte
                    // des autres inchangée)
                    let gone: Vec<String> = removed.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                    if !gone.is_empty() {
                        data["giftsRemoved"] = json!(gone);
                    }
                    // En pause : champ ajouté seulement là (empreinte des autres inchangée) ;
                    // une version plus ancienne n'y voit qu'un giveaway clos
                    if paused {
                        data["status"] = json!("paused");
                    }
                    Ok((sync_id, data))
                })
                .collect()
        }
        "channels" => conn
            .prepare_cached(&format!("SELECT login, channel_id FROM giveaway_channels WHERE {}", one("login", 1)))?
            .query_map([only], |r| Ok((r.get(0)?, json!({ "channelId": r.get::<_, String>(1)? }))))?
            .collect(),
        "streamers" => conn
            .prepare_cached(&format!(
                "SELECT login, discord_url, notes, ignored_commands, require_follow, follow_min_secs, require_presence,
                        require_discord, require_greeting, min_chat_minutes, se_name, can_gift, auto_commands
                 FROM streamers WHERE {}",
                one("login", 1)
            ))?
            .query_map([only], |r| {
                let ignored: Vec<String> = serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default();
                let mut doc = json!({
                        "discordUrl": r.get::<_, Option<String>>(1)?,
                        "notes": r.get::<_, Option<String>>(2)?,
                        "ignoredCommands": ignored,
                        "seName": r.get::<_, Option<String>>(10)?,
                        "conditions": {
                            "requireFollow": r.get::<_, Option<bool>>(4)?,
                            "followMinSecs": r.get::<_, Option<i64>>(5)?,
                            "followMinDays": r.get::<_, Option<i64>>(5)?.map(secs_to_days),
                            "requirePresence": r.get::<_, Option<bool>>(6)?,
                            "requireDiscord": r.get::<_, Option<bool>>(7)?,
                            "requireGreeting": r.get::<_, Option<bool>>(8)?,
                            "minChatMinutes": r.get::<_, Option<i64>>(9)?,
                        },
                    });
                // Gain offrable : seulement s'il est connu (empreinte des autres fiches inchangée)
                if let Some(can) = r.get::<_, Option<bool>>(11)? {
                    doc["conditions"]["canGift"] = json!(can);
                }
                // Commandes en participation automatique : la liste seulement (y répondre reste le
                // choix de chaque PC) ; ajoutée seulement s'il y en a (empreinte des autres inchangée)
                let auto: Vec<String> = serde_json::from_str(&r.get::<_, String>(12)?).unwrap_or_default();
                if !auto.is_empty() {
                    doc["autoCommands"] = json!(auto);
                }
                Ok((r.get(0)?, doc))
            })?
            .collect(),
        "detection" => Ok(rules(conn)?.into_iter().filter(|(id, _)| only.is_none_or(|o| o == id)).collect()),
        _ => Ok(vec![]),
    }
}

/// Le document existe-t-il encore localement (quelle que soit sa date) ?
fn exists(conn: &Connection, collection: &str, id: &str) -> rusqlite::Result<bool> {
    let sql = match collection {
        "groups" => "SELECT 1 FROM gift_groups WHERE sync_id = ?1",
        "gifts" => "SELECT 1 FROM gifts WHERE sync_id = ?1",
        "giveaways" => "SELECT 1 FROM giveaways WHERE sync_id = ?1",
        "channels" => "SELECT 1 FROM giveaway_channels WHERE login = ?1",
        "streamers" => "SELECT 1 FROM streamers WHERE login = ?1",
        "detection" => return Ok(rules(conn)?.iter().any(|(r, _)| r == id)),
        _ => return Ok(false),
    };
    Ok(conn.query_row(sql, [id], |_| Ok(())).optional()?.is_some())
}

fn state(conn: &Connection, collection: &str) -> rusqlite::Result<HashMap<String, String>> {
    conn.prepare("SELECT id, hash FROM sync_state WHERE collection = ?1")?
        .query_map([collection], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

fn set_state(conn: &Connection, collection: &str, id: &str, hash: Option<&str>) -> rusqlite::Result<()> {
    match hash {
        Some(h) => conn.execute(
            "INSERT INTO sync_state (collection, id, hash) VALUES (?1, ?2, ?3)
             ON CONFLICT(collection, id) DO UPDATE SET hash = excluded.hash",
            params![collection, id, h],
        )?,
        None => conn.execute("DELETE FROM sync_state WHERE collection = ?1 AND id = ?2", params![collection, id])?,
    };
    Ok(())
}

fn state_of(conn: &Connection, collection: &str, id: &str) -> rusqlite::Result<Option<String>> {
    conn.prepare_cached("SELECT hash FROM sync_state WHERE collection = ?1 AND id = ?2")?
        .query_row(params![collection, id], |r| r.get(0))
        .optional()
}

// --- Documents modifiés ici (table `sync_dirty`, remplie par des déclencheurs) ---

/// Identifiant noté pour toute une collection (règles de détection : identifiant calculé en Rust)
const WHOLE: &str = "*";

/// Documents notés modifiés d'une collection, avec leur dernier numéro, dans l'ordre des modifications
fn dirty(conn: &Connection, collection: &str) -> rusqlite::Result<Vec<(String, i64)>> {
    conn.prepare_cached("SELECT id, MAX(seq) FROM sync_dirty WHERE collection = ?1 GROUP BY id ORDER BY MAX(seq)")?
        .query_map([collection], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

/// Oublie un document noté, sauf s'il a encore été modifié depuis la lecture (numéro plus grand)
fn clear_dirty(conn: &Connection, collection: &str, id: &str, seq: i64) -> rusqlite::Result<()> {
    conn.prepare_cached("DELETE FROM sync_dirty WHERE collection = ?1 AND id = ?2 AND seq <= ?3")?.execute(params![collection, id, seq])?;
    Ok(())
}

/// Note tous les documents, y compris ceux supprimés que le serveur connaît : le passage
/// suivant compare tout, comme avant le suivi des modifications
fn mark_all_dirty(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "INSERT INTO sync_touch SELECT 'groups', sync_id FROM gift_groups;
         INSERT INTO sync_touch SELECT 'gifts', sync_id FROM gifts;
         INSERT INTO sync_touch SELECT 'giveaways', sync_id FROM giveaways;
         INSERT INTO sync_touch SELECT 'channels', login FROM giveaway_channels;
         INSERT INTO sync_touch SELECT 'streamers', login FROM streamers;
         INSERT INTO sync_touch VALUES ('detection', '*');
         INSERT INTO sync_touch SELECT collection, id FROM sync_state WHERE collection != 'detection';
         DELETE FROM sync_meta WHERE key = 'dirty_all';",
    )
}

fn giveaway_since() -> String {
    (chrono::Utc::now() - chrono::Duration::days(GIVEAWAY_DAYS)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Nombre de documents que `export` donnerait, sans les relire (diagnostic)
fn count(conn: &Connection, collection: &str) -> rusqlite::Result<i64> {
    let sql = match collection {
        "groups" => "SELECT COUNT(*) FROM gift_groups WHERE sync_id IS NOT NULL",
        "gifts" => "SELECT COUNT(*) FROM gifts WHERE sync_id IS NOT NULL",
        "giveaways" => {
            return conn.query_row(
                "SELECT COUNT(*) FROM giveaways WHERE sync_id IS NOT NULL AND (status IN ('open', 'paused') OR detected_at >= ?1 OR manual = 1)",
                [giveaway_since()],
                |r| r.get(0),
            )
        }
        "channels" => "SELECT COUNT(*) FROM giveaway_channels",
        "streamers" => "SELECT COUNT(*) FROM streamers",
        "detection" => "SELECT COUNT(*) FROM detection_rules",
        _ => return Ok(0),
    };
    conn.query_row(sql, [], |r| r.get(0))
}

/// Envoi à faire, avec l'empreinte à garder une fois reçu par le serveur et le numéro de
/// la modification notée à oublier alors
struct Pending<'a> {
    change: OutgoingChange<'a>,
    hash: Option<String>,
    seq: Option<i64>,
}

/// Envoi pour un document noté modifié : contenu différent du dernier état envoyé ou reçu,
/// ou suppression d'un document que le serveur connaît
fn diff_one<'a>(conn: &Connection, collection: &'a str, id: &str, enabled: &[&str]) -> rusqlite::Result<Option<(OutgoingChange<'a>, Option<String>)>> {
    let known = state_of(conn, collection, id)?;
    // Hors de la fenêtre d'envoi (giveaway ancien) mais connu du serveur : sa modification part
    // quand même (gagnants corrigés), sinon les autres PC garderaient l'ancienne version
    let data = match export_sent(conn, collection, id, enabled)? {
        Some(d) => Some(d),
        None if known.is_some() => export_one(conn, collection, id, enabled)?,
        None => None,
    };
    Ok(match data {
        Some(data) => {
            let h = hash(&data);
            (known.as_ref() != Some(&h)).then(|| (OutgoingChange { collection, id: id.to_owned(), data: Some(data), deleted: false }, Some(h)))
        }
        None if known.is_some() && !exists(conn, collection, id)? => {
            Some((OutgoingChange { collection, id: id.to_owned(), data: None, deleted: true }, None))
        }
        None => None,
    })
}

/// Comparaison d'une collection entière : documents comparés par verrou de la base
const WHOLE_BATCH: usize = 25;

/// Identifiants des documents locaux d'une collection, quelle que soit leur date (None : règles
/// de détection, dont l'identifiant est calculé)
fn local_ids(conn: &Connection, collection: &str) -> rusqlite::Result<Option<Vec<String>>> {
    let sql = match collection {
        "groups" => "SELECT sync_id FROM gift_groups WHERE sync_id IS NOT NULL",
        "gifts" => "SELECT sync_id FROM gifts WHERE sync_id IS NOT NULL",
        "giveaways" => "SELECT sync_id FROM giveaways WHERE sync_id IS NOT NULL",
        "channels" => "SELECT login FROM giveaway_channels",
        "streamers" => "SELECT login FROM streamers",
        _ => return Ok(None),
    };
    conn.prepare_cached(sql)?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>().map(Some)
}

/// Collection notée en entier, et dernier numéro couvert
type Whole<'a> = (&'a str, i64);

/// Nos modifications : parmi les documents notés modifiés, ceux qui diffèrent du dernier état
/// envoyé ou reçu, puis les suppressions (collection notée en entier : tout est comparé).
/// Verrou de la base repris document par document. Rend aussi les collections notées en
/// entier, à oublier une fois tout envoyé
fn outgoing<'a>(db: &Db, enabled: &[&'a str]) -> rusqlite::Result<(Vec<Pending<'a>>, Vec<Whole<'a>>)> {
    let mut out = vec![];
    let mut whole = vec![];
    for &c in enabled {
        let ids = dirty(&db.conn(), c)?;
        if let Some(seq) = ids.iter().filter(|(id, _)| id == WHOLE).map(|(_, s)| *s).next() {
            // Tout ce qui est noté pour la collection jusqu'ici est couvert
            let covered = ids.iter().map(|(_, s)| *s).max().unwrap_or(seq);
            // Identifiants locaux et connus du serveur comparés un par un (diff_one), verrou de
            // la base rendu tous les WHOLE_BATCH documents : le chat, les drops et les points ne
            // l'attendent pas pendant toute la collection
            let local = local_ids(&db.conn(), c)?;
            if let Some(local) = local {
                let mut every: BTreeSet<String> = local.into_iter().collect();
                every.extend(state(&db.conn(), c)?.into_keys());
                let every: Vec<String> = every.into_iter().collect();
                for chunk in every.chunks(WHOLE_BATCH) {
                    let conn = db.conn();
                    for id in chunk {
                        if let Some((change, hash)) = diff_one(&conn, c, id, enabled)? {
                            out.push(Pending { change, hash, seq: None });
                        }
                    }
                }
                whole.push((c, covered));
                continue;
            }
            // Règles de détection (identifiant calculé, peu nombreuses) : comparées d'un coup
            let conn = db.conn();
            let known = state(&conn, c)?;
            let mut exported = HashSet::new();
            for (id, data) in export(&conn, c, enabled)? {
                let h = hash(&data);
                exported.insert(id.clone());
                if known.get(&id) != Some(&h) {
                    out.push(Pending { change: OutgoingChange { collection: c, id, data: Some(data), deleted: false }, hash: Some(h), seq: None });
                }
            }
            for (id, h) in &known {
                if !exists(&conn, c, id)? {
                    out.push(Pending { change: OutgoingChange { collection: c, id: id.clone(), data: None, deleted: true }, hash: None, seq: None });
                } else if !exported.contains(id) {
                    // Hors de la fenêtre d'envoi mais connu du serveur : modifié, il part (diff_one)
                    if let Some(data) = export_one(&conn, c, id, enabled)? {
                        let new = hash(&data);
                        if &new != h {
                            out.push(Pending { change: OutgoingChange { collection: c, id: id.clone(), data: Some(data), deleted: false }, hash: Some(new), seq: None });
                        }
                    }
                }
            }
            whole.push((c, covered));
            continue;
        }
        for (id, seq) in ids {
            let conn = db.conn();
            match diff_one(&conn, c, &id, enabled)? {
                Some((change, hash)) => out.push(Pending { change, hash, seq: Some(seq) }),
                // Inchangé (modification reçue, ou revenue à l'état connu), giveaway trop ancien
                None => clear_dirty(&conn, c, &id, seq)?,
            }
        }
    }
    Ok((out, whole))
}

/// Envois reçus par le serveur : état gardé, modifications notées oubliées
fn acknowledge(conn: &Connection, sent: Vec<Pending>) -> rusqlite::Result<()> {
    for p in sent {
        set_state(conn, p.change.collection, &p.change.id, p.hash.as_deref())?;
        if let Some(seq) = p.seq {
            clear_dirty(conn, p.change.collection, &p.change.id, seq)?;
        }
    }
    Ok(())
}

fn clear_whole(conn: &Connection, whole: &[Whole]) -> rusqlite::Result<()> {
    for (c, seq) in whole {
        conn.execute("DELETE FROM sync_dirty WHERE collection = ?1 AND seq <= ?2", params![c, seq])?;
    }
    Ok(())
}

/// Ligne locale qui représente le même élément sous un autre identifiant (doublon à fusionner)
fn twin(conn: &Connection, collection: &str, id: &str, data: &Value) -> rusqlite::Result<Option<(i64, String)>> {
    let row = |r: &rusqlite::Row| Ok((r.get(0)?, r.get(1)?));
    match collection {
        "groups" => conn
            .query_row(
                "SELECT id, sync_id FROM gift_groups WHERE name = ?1 COLLATE NOCASE AND game_id IS ?2 AND sync_id != ?3",
                params![str_of(data, "name"), str_of(data, "gameId"), id],
                row,
            )
            .optional(),
        "gifts" => conn
            .query_row(
                "SELECT id, sync_id FROM gifts WHERE title = ?1 COLLATE NOCASE AND game_id IS ?2 AND sync_id != ?3",
                params![str_of(data, "title"), str_of(data, "gameId"), id],
                row,
            )
            .optional(),
        // Même giveaway ouvert (ou en pause) repéré par deux applications : même chaîne, même type,
        // même commande ou lien
        "giveaways" if giveaway_status(data) != "closed" => conn
            .query_row(
                "SELECT id, sync_id FROM giveaways WHERE status IN ('open', 'paused') AND channel = ?1 AND kind = ?2
                     AND command IS ?3 AND url IS ?4 AND sync_id != ?5",
                params![str_of(data, "channel"), str_of(data, "kind"), str_of(data, "command"), str_of(data, "url"), id],
                row,
            )
            .optional(),
        _ => Ok(None),
    }
}

/// Statut d'un giveaway reçu : `status` envoyé seulement en pause, sinon `open`
fn giveaway_status(data: &Value) -> &'static str {
    if str_of(data, "status").as_deref() == Some("paused") {
        "paused"
    } else if bool_of(data, "open") == Some(true) {
        "open"
    } else {
        "closed"
    }
}

fn local_id(conn: &Connection, table: &str, sync_id: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(&format!("SELECT id FROM {table} WHERE sync_id = ?1"), [sync_id], |r| r.get(0)).optional()
}

/// Le document pointe-t-il vers un groupe ou un cadeau pas encore reçu ?
fn unresolved(conn: &Connection, collection: &str, data: &Value) -> rusqlite::Result<bool> {
    match collection {
        "gifts" => match str_of(data, "group") {
            Some(g) => Ok(local_id(conn, "gift_groups", &g)?.is_none()),
            None => Ok(false),
        },
        "giveaways" => {
            for gift in strings_of(data, "gifts") {
                if local_id(conn, "gifts", &gift)?.is_none() {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// Applique un document reçu ; faux s'il a été ignoré (doublon gardé sous notre identifiant)
/// Streamers dont le pseudo StreamElements a changé par la synchronisation : identifiant
/// StreamElements oublié partout une fois les modifications enregistrées
static SE_RENAMED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn apply(conn: &Connection, collection: &str, id: &str, data: Option<&Value>) -> rusqlite::Result<bool> {
    let Some(data) = data else {
        let sql = match collection {
            "groups" => "DELETE FROM gift_groups WHERE sync_id = ?1",
            "gifts" => "DELETE FROM gifts WHERE sync_id = ?1",
            "giveaways" => "DELETE FROM giveaways WHERE sync_id = ?1",
            "channels" => "DELETE FROM giveaway_channels WHERE login = ?1",
            "streamers" => "DELETE FROM streamers WHERE login = ?1",
            "detection" => {
                for (rid, d) in rules(conn)? {
                    if rid == id {
                        conn.execute(
                            "DELETE FROM detection_rules WHERE kind = ?1 AND value = ?2",
                            params![str_of(&d, "kind"), str_of(&d, "value")],
                        )?;
                    }
                }
                return Ok(true);
            }
            _ => return Ok(false),
        };
        conn.execute(sql, [id])?;
        return Ok(true);
    };
    let table = match collection {
        "groups" => Some("gift_groups"),
        "gifts" => Some("gifts"),
        "giveaways" => Some("giveaways"),
        _ => None,
    };
    if let Some(table) = table {
        if local_id(conn, table, id)?.is_none() {
            if let Some((row, other)) = twin(conn, collection, id, data)? {
                // Le plus petit identifiant l'emporte, partout
                if id > other.as_str() {
                    return Ok(false);
                }
                conn.execute(&format!("UPDATE {table} SET sync_id = ?2 WHERE id = ?1"), params![row, id])?;
            }
        }
    }
    match collection {
        "groups" => {
            let Some(name) = str_of(data, "name") else { return Ok(false) };
            let game_id = str_of(data, "gameId");
            let game_name = str_of(data, "gameName");
            if local_id(conn, "gift_groups", id)?.is_some() {
                conn.execute(
                    "UPDATE OR IGNORE gift_groups SET name = ?2, game_id = ?3, game_name = ?4 WHERE sync_id = ?1",
                    params![id, name, game_id, game_name],
                )?;
            } else {
                conn.execute(
                    "INSERT OR IGNORE INTO gift_groups (name, game_id, game_name, sync_id) VALUES (?1, ?2, ?3, ?4)",
                    params![name, game_id, game_name, id],
                )?;
            }
        }
        "gifts" => {
            let Some(title) = str_of(data, "title") else { return Ok(false) };
            let current = conn
                .query_row("SELECT group_id FROM gifts WHERE sync_id = ?1", [id], |r| r.get::<_, Option<i64>>(0))
                .optional()?
                .flatten();
            // Sans lien envoyé (groupes non synchronisés) : le groupe local reste
            let group = match data.get("group") {
                None => current,
                Some(_) => match str_of(data, "group") {
                    Some(g) => local_id(conn, "gift_groups", &g)?,
                    None => None,
                },
            };
            let keywords = serde_json::to_string(&crate::db::clean_keywords(&title, &strings_of(data, "keywords"))).expect("liste sérialisable");
            let image = safe_image(str_of(data, "image"));
            let game_id = str_of(data, "gameId");
            let game_name = str_of(data, "gameName");
            if local_id(conn, "gifts", id)?.is_some() {
                conn.execute(
                    "UPDATE gifts SET title = ?2, image = ?3, group_id = ?4, keywords = ?5, game_id = ?6, game_name = ?7 WHERE sync_id = ?1",
                    params![id, title, image, group, keywords, game_id, game_name],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO gifts (title, image, group_id, keywords, game_id, game_name, sync_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![title, image, group, keywords, game_id, game_name, id],
                )?;
            }
        }
        "giveaways" => {
            let (Some(channel), Some(kind), Some(detected_at)) = (str_of(data, "channel"), str_of(data, "kind"), str_of(data, "detectedAt"))
            else {
                return Ok(false);
            };
            if !valid_login(&channel) || !["command", "ticket", "streamelements", "channelpoints"].contains(&kind.as_str()) {
                return Ok(false);
            }
            let status = giveaway_status(data);
            let values = params![
                id,
                channel,
                kind,
                str_of(data, "command"),
                str_of(data, "url"),
                detected_at,
                status,
                bool_of(data, "manual").unwrap_or(false),
                str_of(data, "winner"),
                str_of(data, "winnerChance"),
                str_of(data, "drawnAt"),
                bool_of(data, "ticketUnique"),
                int_of(data, "ticketMax"),
                str_of(data, "rewardName"),
                int_of(data, "rewardCost"),
                str_of(data, "drawAt"),
                bool_of(data, "requireFollow"),
                bool_of(data, "requirePresence"),
                bool_of(data, "requireDiscord"),
                str_of(data, "notes"),
                bool_of(data, "requireGreeting"),
                int_of(data, "minChatMinutes"),
                follow_secs_of(data),
                bool_of(data, "canGift"),
            ];
            if local_id(conn, "giveaways", id)?.is_some() {
                conn.execute(
                    "UPDATE giveaways SET channel = ?2, kind = ?3, command = ?4, url = ?5, detected_at = ?6, status = ?7, manual = ?8,
                         winner = ?9, winner_chance = ?10, drawn_at = ?11, ticket_unique = ?12, ticket_max = ?13,
                         reward_name = ?14, reward_cost = ?15, draw_at = ?16, require_follow = ?17, require_presence = ?18,
                         require_discord = ?19, notes = ?20, require_greeting = ?21, min_chat_minutes = ?22, follow_min_secs = ?23,
                         can_gift = ?24, reminded = CASE WHEN draw_at IS ?16 THEN reminded ELSE 0 END
                     WHERE sync_id = ?1",
                    values,
                )?;
            } else {
                // Un giveaway ouvert (ou en pause, à son retour) reçu arrive dans la file d'alertes ; un
                // giveaway clos n'y va pas
                conn.execute(
                    "INSERT INTO giveaways (sync_id, channel, kind, command, url, detected_at, status, manual, winner, winner_chance,
                         drawn_at, ticket_unique, ticket_max, reward_name, reward_cost, draw_at, require_follow, require_presence,
                         require_discord, notes, require_greeting, min_chat_minutes, follow_min_secs, can_gift, handled)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                         ?21, ?22, ?23, ?24, ?7 = 'closed')",
                    values,
                )?;
            }
            let giveaway = local_id(conn, "giveaways", id)?.expect("giveaway enregistré");
            // Gagnants : la liste (plusieurs), sinon le seul gagnant (versions plus anciennes)
            let winners: Vec<String> = match data.get("winners").and_then(Value::as_array) {
                Some(list) => list.iter().filter_map(Value::as_str).filter(|w| valid_login(w)).map(str::to_owned).collect(),
                None => str_of(data, "winner").into_iter().collect(),
            };
            // Chances : par gagnant, sinon celle du seul gagnant
            let mut chances: BTreeMap<String, String> = data
                .get("winnerChances")
                .and_then(Value::as_object)
                .map(|m| m.iter().filter_map(|(k, v)| Some((k.to_lowercase(), v.as_str()?.chars().take(20).collect()))).collect())
                .unwrap_or_default();
            if let (true, Some(w), Some(c)) = (chances.is_empty(), str_of(data, "winner"), str_of(data, "winnerChance")) {
                chances.insert(w.to_lowercase(), c);
            }
            conn.execute(
                "UPDATE giveaways SET winners = ?2, winner_chances = ?3 WHERE id = ?1",
                params![giveaway, serde_json::to_string(&winners).expect("liste"), serde_json::to_string(&chances).expect("objet")],
            )?;
            // Sans lien envoyé (cadeaux non synchronisés) : les cadeaux locaux restent
            if data.get("gifts").is_none() {
                return Ok(true);
            }
            // Fusion plutôt que remplacement : un cadeau rattaché ici (live vu par ce PC) reste ;
            // ceux retirés à la main sur l'autre PC sont retirés ici aussi et plus remis, ceux
            // retirés ici ne reviennent pas
            for gift in strings_of(data, "giftsRemoved") {
                if let Some(gift) = local_id(conn, "gifts", &gift)? {
                    conn.execute("INSERT OR IGNORE INTO giveaway_gifts_rejected (giveaway_id, gift_id) VALUES (?1, ?2)", params![giveaway, gift])?;
                    conn.execute("DELETE FROM giveaway_gifts WHERE giveaway_id = ?1 AND gift_id = ?2", params![giveaway, gift])?;
                }
            }
            for gift in strings_of(data, "gifts") {
                if let Some(gift) = local_id(conn, "gifts", &gift)? {
                    conn.execute(
                        "INSERT OR IGNORE INTO giveaway_gifts (giveaway_id, gift_id) SELECT ?1, ?2
                         WHERE NOT EXISTS (SELECT 1 FROM giveaway_gifts_rejected WHERE giveaway_id = ?1 AND gift_id = ?2)",
                        params![giveaway, gift],
                    )?;
                }
            }
        }
        "channels" => {
            if !valid_login(id) {
                return Ok(false);
            }
            let Some(channel_id) = str_of(data, "channelId") else { return Ok(false) };
            // Les commandes ignorées voyagent avec la fiche du streamer
            conn.execute(
                "INSERT INTO giveaway_channels (login, channel_id, position)
                 VALUES (?1, ?2, (SELECT COALESCE(MAX(position), -1) + 1 FROM giveaway_channels))
                 ON CONFLICT(login) DO UPDATE SET channel_id = excluded.channel_id",
                params![id, channel_id],
            )?;
        }
        "streamers" => {
            if !valid_login(id) {
                return Ok(false);
            }
            conn.execute(
                "INSERT INTO streamers (login, discord_url, notes) VALUES (?1, ?2, ?3)
                 ON CONFLICT(login) DO UPDATE SET discord_url = excluded.discord_url, notes = excluded.notes",
                params![id, str_of(data, "discordUrl"), str_of(data, "notes")],
            )?;
            // Absent (version plus ancienne) : les commandes ignorées locales restent
            // Conditions habituelles (absentes : version plus ancienne, celles d'ici restent)
            if let Some(c) = data.get("conditions") {
                conn.execute(
                    "UPDATE streamers SET require_follow = ?2, follow_min_secs = ?3, require_presence = ?4, require_discord = ?5,
                         require_greeting = ?6, min_chat_minutes = ?7, can_gift = ?8 WHERE login = ?1",
                    params![
                        id,
                        bool_of(c, "requireFollow"),
                        follow_secs_of(c),
                        bool_of(c, "requirePresence"),
                        bool_of(c, "requireDiscord"),
                        bool_of(c, "requireGreeting"),
                        int_of(c, "minChatMinutes"),
                        bool_of(c, "canGift"),
                    ],
                )?;
            }
            // Commandes en participation automatique (absentes : aucune) ; celles activées sur ce PC
            // restent activées si elles sont toujours dans la liste
            {
                let list = crate::giveaways::detect::normalize_excluded(&strings_of(data, "autoCommands"));
                let on: String = conn
                    .query_row("SELECT auto_commands_on FROM streamers WHERE login = ?1", [id], |r| r.get(0))
                    .unwrap_or_else(|_| "[]".into());
                let on: Vec<String> = serde_json::from_str::<Vec<String>>(&on).unwrap_or_default().into_iter().filter(|c| list.contains(c)).collect();
                conn.execute(
                    "UPDATE streamers SET auto_commands = ?2, auto_commands_on = ?3 WHERE login = ?1",
                    params![id, serde_json::to_string(&list).expect("liste"), serde_json::to_string(&on).expect("liste")],
                )?;
            }
            // Pseudo StreamElements (absent : version plus ancienne, celui d'ici reste) ; changé :
            // identifiant StreamElements de ses chaînes à retrouver
            if data.get("seName").is_some() {
                let name = str_of(data, "seName").map(|n| n.to_lowercase()).filter(|n| crate::streamelements::valid_page_name(n));
                let changed = conn.execute(
                    "UPDATE streamers SET se_name = ?2 WHERE login = ?1 AND se_name IS NOT ?2",
                    params![id, name],
                )?;
                if changed > 0 {
                    conn.execute("UPDATE channels SET se_channel_id = NULL WHERE login = ?1", [id])?;
                    // Identifiant à oublier aussi par les comptes, les analyses et le scanner
                    // (après l'enregistrement de la synchronisation)
                    SE_RENAMED.lock().unwrap().push(id.to_owned());
                }
            }
            if data.get("ignoredCommands").is_some() {
                let ignored = serde_json::to_string(&strings_of(data, "ignoredCommands")).expect("liste sérialisable");
                conn.execute("UPDATE streamers SET ignored_commands = ?2 WHERE login = ?1", params![id, ignored])?;
            }
        }
        "detection" => {
            let (Some(kind), Some(value)) = (str_of(data, "kind"), str_of(data, "value")) else { return Ok(false) };
            let valid = match kind.as_str() {
                "winner" => crate::giveaways::bots::compile_template(&value).is_ok(),
                "bot" => valid_login(&value),
                _ => false,
            };
            // Identifiant qui ne correspond pas au contenu : document refusé
            if !valid || rule_id(&kind, &value) != id {
                return Ok(false);
            }
            // Déjà là, éventuellement avec d'autres majuscules
            if !exists(conn, "detection", id)? {
                conn.execute("INSERT OR IGNORE INTO detection_rules (kind, value) VALUES (?1, ?2)", params![kind, value])?;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

// --- Passage de synchronisation ---

fn db_err(e: rusqlite::Error) -> String {
    format!("base locale : {e}")
}

/// Requête avec le code ; réponse en erreur : message du serveur
async fn send(req: reqwest::RequestBuilder, code: &str) -> Result<reqwest::Response, String> {
    let res = req
        .bearer_auth(code)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| if e.is_timeout() { "Le serveur ne répond pas".to_owned() } else { format!("Serveur injoignable : {e}") })?;
    match res.status() {
        reqwest::StatusCode::UNAUTHORIZED => Err("Code refusé par le serveur".into()),
        s if !s.is_success() => {
            let body: Value = res.json().await.unwrap_or_default();
            // 404 sans message : route inconnue (autre serveur, ou serveur plus ancien)
            Err(str_of(&body, "error").unwrap_or_else(|| {
                if matches!(s, reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED) {
                    NOT_SYNC_SERVER.to_owned()
                } else {
                    format!("Erreur du serveur ({s})")
                }
            }))
        }
        _ => Ok(res),
    }
}

async fn request<T: serde::de::DeserializeOwned>(req: reqwest::RequestBuilder, code: &str) -> Result<T, String> {
    send(req, code).await?.json::<T>().await.map_err(|_| NOT_SYNC_SERVER.to_owned())
}

/// Route de la web app (serveur 0.6 et plus) : route inconnue, serveur à mettre à jour
fn web_app_err(e: String) -> String {
    if e == NOT_SYNC_SERVER {
        OLD_SERVER.to_owned()
    } else {
        e
    }
}

async fn server_info(http: &reqwest::Client, url: &str, code: &str) -> Result<ServerInfo, String> {
    request(http.get(format!("{url}/api/v1/info")), code).await
}

struct Outcome {
    server_collections: Vec<String>,
    collections: Vec<String>,
    local_counts: BTreeMap<String, i64>,
    server_counts: Option<BTreeMap<String, i64>>,
    sent: u64,
    received: u64,
}

/// Document reçu enregistré : empreinte de ce qui est réellement enregistré (l'enregistrement
/// peut différer un peu du document reçu : espaces, image refusée, lien absent), et modification
/// notée par les déclencheurs oubliée (rien à renvoyer). Règles de détection : notées en
/// entier, comparées au prochain envoi
fn received_state(conn: &Connection, collection: &str, id: &str, data: Option<&Value>, enabled: &[&str]) -> rusqlite::Result<()> {
    let stored = match data {
        Some(_) => export_one(conn, collection, id, enabled)?.map(|v| hash(&v)),
        None => None,
    };
    set_state(conn, collection, id, stored.as_deref())?;
    if collection != "detection" {
        conn.prepare_cached("DELETE FROM sync_dirty WHERE collection = ?1 AND id = ?2")?.execute(params![collection, id])?;
    }
    Ok(())
}

/// Applique une page de modifications reçues ; rend le nombre de documents appliqués
fn receive<'a>(
    conn: &Connection,
    changes: &[RemoteChange],
    enabled: &[&'a str],
    deferred: &mut Vec<(&'a str, String, Value)>,
    touched: &mut HashSet<&'a str>,
) -> rusqlite::Result<u64> {
    let mut received = 0;
    // État local au moment de la page, documents de la page seulement : une modification
    // locale pas encore envoyée l'emporte
    let mut local: HashMap<(&str, &str), Option<String>> = HashMap::new();
    let mut states: HashMap<&str, HashMap<String, String>> = HashMap::new();
    for change in changes {
        let Some(c) = enabled.iter().copied().find(|c| *c == change.collection) else { continue };
        if !states.contains_key(c) {
            states.insert(c, state(conn, c)?);
        }
        if let std::collections::hash_map::Entry::Vacant(e) = local.entry((c, change.id.as_str())) {
            e.insert(export_sent(conn, c, &change.id, enabled)?.map(|v| hash(&v)));
        }
    }
    for change in changes {
        let Some(collection) = enabled.iter().copied().find(|c| *c == change.collection) else { continue };
        let data = if change.deleted { None } else { change.data.as_ref() };
        let remote = data.map(hash);
        let known = states[collection].get(&change.id);
        // Déjà dans cet état (notre propre envoi qui revient)
        if remote.as_ref() == known || (remote.is_none() && known.is_none() && !exists(conn, collection, &change.id)?) {
            continue;
        }
        let mine = local[&(collection, change.id.as_str())].as_ref();
        if known.is_some() && mine.is_some() && mine != known {
            tracing::info!("synchronisation : {collection}/{} modifié ici, version reçue laissée de côté", change.id);
            continue;
        }
        // Supprimé ici, pas encore envoyé : la suppression l'emporte sur la modification reçue
        if known.is_some() && mine.is_none() && remote.is_some() && !exists(conn, collection, &change.id)? {
            tracing::info!("synchronisation : {collection}/{} supprimé ici, version reçue laissée de côté", change.id);
            continue;
        }
        let pending = match data {
            Some(d) => unresolved(conn, collection, d)?,
            None => false,
        };
        if apply(conn, collection, &change.id, data)? {
            if pending {
                deferred.push((collection, change.id.clone(), data.cloned().unwrap_or_default()));
            } else {
                received_state(conn, collection, &change.id, data, enabled)?;
            }
            received += 1;
            touched.insert(collection);
        }
    }
    Ok(received)
}

/// Documents reçus avant le groupe ou les cadeaux qu'ils citent : réappliqués
fn reapply(conn: &Connection, deferred: &[(&str, String, Value)], enabled: &[&str]) -> rusqlite::Result<()> {
    for (collection, id, data) in deferred {
        if apply(conn, collection, id, Some(data))? {
            received_state(conn, collection, id, Some(data), enabled)?;
        }
    }
    Ok(())
}

async fn run(app: &App, cfg: &Config) -> Result<Outcome, String> {
    let info = server_info(&app.http, &cfg.url, &cfg.code).await?;
    // Adresse de la web app donnée par le serveur : gardée (paramètres, QR code de connexion)
    let server_web = info.web_url.as_deref().and_then(|u| normalize_url(u).ok()).unwrap_or_default();
    {
        let conn = app.db.conn();
        if meta(&conn, "server_web_url").unwrap_or_default() != server_web {
            set_meta(&conn, "server_web_url", &server_web).map_err(db_err)?;
        }
    }
    let skipped = skipped(&app.db.conn());
    let offered: Vec<&str> = COLLECTIONS.iter().copied().filter(|c| info.collections.iter().any(|x| x == c)).collect();
    let enabled: Vec<&str> = offered.iter().copied().filter(|c| !skipped.iter().any(|s| s == c)).collect();

    let mut cursor: i64;
    let mut reset = false;
    {
        let conn = app.db.conn();
        assign_ids(&conn).map_err(db_err)?;
        cursor = meta(&conn, "cursor").and_then(|c| c.parse().ok()).unwrap_or(0);
        // Autre serveur, ou serveur remis à zéro : tout est renvoyé et relu
        if meta(&conn, "server").as_deref() != Some(cfg.url.as_str()) || info.seq < cursor {
            conn.execute("DELETE FROM sync_state", []).map_err(db_err)?;
            cursor = 0;
            reset = true;
            set_meta(&conn, "server", &cfg.url).map_err(db_err)?;
            set_meta(&conn, "cursor", "0").map_err(db_err)?;
        }
        // État enregistré par une version qui gardait l'empreinte du document reçu (et non de
        // ce qui était enregistré) : certains documents pouvaient passer pour modifiés ici et
        // ne plus recevoir de mises à jour. Une fois : état oublié, tout est relu, le serveur fait foi
        if meta(&conn, "state_format").as_deref() != Some(STATE_FORMAT) {
            conn.execute("DELETE FROM sync_state", []).map_err(db_err)?;
            cursor = 0;
            reset = true;
            set_meta(&conn, "cursor", "0").map_err(db_err)?;
            set_meta(&conn, "state_format", STATE_FORMAT).map_err(db_err)?;
        }
        // Collections synchronisées changées : les modifications d'une collection ignorée
        // jusque-là ont été sautées, tout est relu (les documents déjà à jour sont ignorés)
        let scope = enabled.join(",");
        if meta(&conn, "scope").as_deref() != Some(scope.as_str()) {
            cursor = 0;
            reset = true;
            set_meta(&conn, "cursor", "0").map_err(db_err)?;
            set_meta(&conn, "scope", &scope).map_err(db_err)?;
        }
        // État oublié, collections changées (les liens envoyés en dépendent) ou première fois
        // depuis le suivi des modifications : tout est comparé une fois
        if reset || meta(&conn, "dirty_all").is_some() {
            mark_all_dirty(&conn).map_err(db_err)?;
        }
    }

    // 1. Modifications des autres
    let mut received = 0u64;
    let mut touched: HashSet<&str> = HashSet::new();
    // Reçus avant le groupe ou les cadeaux qu'ils citent : réappliqués à la fin de la lecture
    let mut deferred: Vec<(&str, String, Value)> = vec![];
    loop {
        let page: ChangesPage =
            request(app.http.get(format!("{}/api/v1/changes", cfg.url)).query(&[("since", cursor)]), &cfg.code).await?;
        {
            let guard = app.db.conn();
            // Page appliquée d'un bloc (curseur compris) : interrompue, elle sera relue en entier
            let conn = guard.unchecked_transaction().map_err(db_err)?;
            received += receive(&conn, &page.changes, &enabled, &mut deferred, &mut touched).map_err(db_err)?;
            // Le curseur n'avance pas tant que des liens restent à résoudre : en cas
            // d'interruption, ces documents seront relus
            cursor = page.seq;
            if deferred.is_empty() {
                set_meta(&conn, "cursor", &cursor.to_string()).map_err(db_err)?;
            }
            conn.commit().map_err(db_err)?;
        }
        if !page.more {
            break;
        }
    }
    if !deferred.is_empty() {
        let conn = app.db.conn();
        reapply(&conn, &deferred, &enabled).map_err(db_err)?;
        set_meta(&conn, "cursor", &cursor.to_string()).map_err(db_err)?;
    }

    // 2. Nos modifications : documents notés modifiés dont le contenu a changé, et suppressions
    let author = app.db.accounts().ok().and_then(|a| a.into_iter().next()).map(|a| a.login);
    let (outgoing, whole) = outgoing(&app.db, &enabled).map_err(db_err)?;
    let sent = outgoing.len() as u64;
    let mut batch: Vec<Pending> = vec![];
    let mut size = 0;
    let mut pending = outgoing.into_iter().peekable();
    while let Some(item) = pending.next() {
        size += item.change.data.as_ref().map_or(64, |d| d.to_string().len());
        batch.push(item);
        if batch.len() >= PUSH_BATCH_DOCS || size >= PUSH_BATCH_BYTES || pending.peek().is_none() {
            let changes: Vec<&OutgoingChange> = batch.iter().map(|p| &p.change).collect();
            let _: Value =
                request(app.http.post(format!("{}/api/v1/changes", cfg.url)).json(&json!({ "author": author, "changes": changes })), &cfg.code)
                    .await?;
            acknowledge(&app.db.conn(), std::mem::take(&mut batch)).map_err(db_err)?;
            size = 0;
        }
    }
    // Collections notées en entier : oubliées une fois tous les envois reçus
    clear_whole(&app.db.conn(), &whole).map_err(db_err)?;

    for login in std::mem::take(&mut *SE_RENAMED.lock().unwrap()) {
        crate::commands::refresh_se_names(app, &login);
    }
    if !touched.is_empty() {
        app.giveaways.send(MonitorCmd::Reload);
    }
    if received > 0 || sent > 0 {
        tracing::info!("synchronisation : {received} modification(s) reçue(s), {sent} envoyée(s)");
    }
    // Comptes pour le diagnostic : ici, et sur le serveur une fois nos envois reçus
    let mut local_counts = BTreeMap::new();
    {
        let conn = app.db.conn();
        for c in &offered {
            local_counts.insert(c.to_string(), count(&conn, c).map_err(db_err)?);
        }
    }
    let server_counts = server_info(&app.http, &cfg.url, &cfg.code).await.ok().and_then(|i| i.counts);
    register(app, cfg).await;
    Ok(Outcome {
        server_collections: offered.iter().map(|c| c.to_string()).collect(),
        collections: enabled.iter().map(|c| c.to_string()).collect(),
        local_counts,
        server_counts,
        sent,
        received,
    })
}

// --- Temps réel ---

/// Lecture d'un flux Server-Sent Events, morceau par morceau
#[derive(Default)]
struct EventParser {
    line: Vec<u8>,
    event: String,
    data: String,
    /// Applications visées par les événements `action` reçus (demandes des téléphones)
    actions: Vec<String>,
}

impl EventParser {
    /// Ligne plus longue : ignorée (rien de tel n'est attendu)
    const MAX_LINE: usize = 64 * 1024;

    /// Morceau reçu : numéros annoncés par les événements `seq` complets
    fn feed(&mut self, bytes: &[u8]) -> Vec<i64> {
        let mut seqs = vec![];
        for &b in bytes {
            if b != b'\n' {
                if self.line.len() < Self::MAX_LINE {
                    self.line.push(b);
                }
                continue;
            }
            let raw = std::mem::take(&mut self.line);
            let text = String::from_utf8_lossy(&raw);
            let line = text.strip_suffix('\r').unwrap_or(&text);
            if line.is_empty() {
                // Ligne vide : fin de l'événement
                let event = std::mem::take(&mut self.event);
                let data = std::mem::take(&mut self.data);
                if matches!(event.as_str(), "" | "seq") {
                    if let Some(seq) = parse_seq(&data) {
                        seqs.push(seq);
                    }
                } else if event == "action" {
                    if let Some(instance) = serde_json::from_str::<Value>(data.trim()).ok().as_ref().and_then(|v| str_of(v, "instance")) {
                        self.actions.push(instance);
                    }
                }
            } else if !line.starts_with(':') {
                // « : » au début : commentaire (maintien de la connexion)
                let (field, value) = line.split_once(':').map_or((line, ""), |(f, v)| (f, v.strip_prefix(' ').unwrap_or(v)));
                match field {
                    "event" => self.event = value.to_owned(),
                    "data" => {
                        if !self.data.is_empty() {
                            self.data.push('\n');
                        }
                        self.data.push_str(value);
                    }
                    _ => {}
                }
            }
        }
        seqs
    }
}

/// `{"seq": 42}` (ou `42`)
fn parse_seq(data: &str) -> Option<i64> {
    let v: Value = serde_json::from_str(data.trim()).ok()?;
    v.as_i64().or_else(|| int_of(&v, "seq"))
}

fn set_live(app: &App, state: &'static str) {
    let previous = std::mem::replace(&mut *app.sync.live.lock().unwrap(), state);
    if previous != state {
        app.touch_domain(crate::app::Domain::Sync);
    }
}

enum LiveEnd {
    /// Flux ouvert puis fermé (serveur redémarré, connexion renouvelée ou perdue)
    Closed,
    /// Serveur injoignable ou erreur
    Failed,
    /// Pas de flux sur ce serveur (avant la 0.5, ou proxy qui ne le laisse pas passer)
    Unavailable,
    /// Code refusé, ou adresse bloquée après trop de codes refusés
    Refused,
}

/// Écoute le flux temps réel jusqu'à sa fermeture : chaque numéro annoncé différent du
/// curseur lance un passage
async fn listen(app: &App, cfg: &Config) -> LiveEnd {
    let instance = instance_id(&app.db.conn()).unwrap_or_default();
    let res = app
        .http
        // Identifiant de l'application : le serveur la sait ouverte tant que ce flux l'est
        .get(format!("{}/api/v1/events", cfg.url))
        .query(&[("instance", instance)])
        .bearer_auth(&cfg.code)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        // Pas de compression : chaque événement arrive aussitôt
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .timeout(LIVE_MAX)
        .send()
        .await;
    let mut res = match res {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("synchronisation en temps réel : {e}");
            return LiveEnd::Failed;
        }
    };
    match res.status() {
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED => return LiveEnd::Unavailable,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::TOO_MANY_REQUESTS => return LiveEnd::Refused,
        s if !s.is_success() => {
            tracing::debug!("synchronisation en temps réel : réponse {s}");
            return LiveEnd::Failed;
        }
        _ => {}
    }
    let is_stream = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"));
    if !is_stream {
        return LiveEnd::Unavailable;
    }
    set_live(app, "connected");
    let mut parser = EventParser::default();
    loop {
        let chunk = match tokio::time::timeout(LIVE_IDLE, res.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            _ => return LiveEnd::Closed,
        };
        for seq in parser.feed(&chunk) {
            // Numéro différent du curseur : modifications à lire (celles des autres, ou notre
            // propre envoi, relu sans effet) ; serveur remis à zéro : le passage s'en charge
            let cursor = meta(&app.db.conn(), "cursor").and_then(|c| c.parse::<i64>().ok()).unwrap_or(0);
            if seq != cursor {
                app.sync.pull.notify_one();
            }
        }
        // Demande d'un de nos téléphones : lue tout de suite
        if !parser.actions.is_empty() {
            let mine = instance_id(&app.db.conn()).ok();
            if parser.actions.drain(..).any(|i| Some(&i) == mine.as_ref()) {
                app.sync.phone_action.notify_one();
            }
        }
    }
}

/// Flux temps réel tant que la synchronisation est activée ; rouvert quand les réglages changent
fn spawn_live(app: Arc<App>) {
    tauri::async_runtime::spawn(async move {
        let mut retry = LIVE_RETRY_MIN;
        loop {
            let Some(cfg) = config(&app) else {
                set_live(&app, "off");
                retry = LIVE_RETRY_MIN;
                app.sync.reconfigure.notified().await;
                continue;
            };
            if app.sync.live_state() != "reconnecting" {
                set_live(&app, "connecting");
            }
            let opened = std::time::Instant::now();
            let end = tokio::select! {
                end = listen(&app, &cfg) => end,
                _ = app.sync.reconfigure.notified() => {
                    retry = LIVE_RETRY_MIN;
                    continue;
                }
            };
            let (state, wait) = match end {
                LiveEnd::Closed if opened.elapsed() >= LIVE_SHORT => {
                    retry = LIVE_RETRY_MIN;
                    ("reconnecting", LIVE_RETRY_MIN)
                }
                LiveEnd::Closed | LiveEnd::Failed => {
                    let wait = retry;
                    retry = (retry * 2).min(LIVE_RETRY_MAX);
                    ("reconnecting", wait)
                }
                LiveEnd::Unavailable => ("unavailable", LIVE_PAUSE),
                // Pas d'essais répétés : ils finiraient par bloquer l'adresse sur le serveur
                LiveEnd::Refused => ("refused", LIVE_PAUSE),
            };
            set_live(&app, state);
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = app.sync.reconfigure.notified() => retry = LIVE_RETRY_MIN,
            }
        }
    });
}

pub fn spawn(app: Arc<App>) {
    spawn_live(app.clone());
    spawn_state(app.clone());
    spawn_phone_alerts(app.clone());
    spawn_phone_actions(app.clone());
    tauri::async_runtime::spawn(async move {
        // Code refusé (ou adresse bloquée pour trop de codes refusés) : plus d'essai avec ce
        // même code pendant CODE_REFUSED_PAUSE, sauf « Synchroniser » ; sinon le serveur
        // bloquerait l'adresse en boucle (et les téléphones de la maison avec)
        let mut refused: Option<(u64, std::time::Instant)> = None;
        loop {
            let Some(cfg) = config(&app) else {
                {
                    let mut v = app.sync.view.lock().unwrap();
                    let url = meta(&app.db.conn(), "url").unwrap_or_default();
                    if v.status != "off" || v.url != url {
                        *v = SyncView { status: "off".into(), url, revision: v.revision, ..SyncView::default() };
                        drop(v);
                        app.touch_domain(crate::app::Domain::Sync);
                    }
                }
                app.sync.wake.notified().await;
                tokio::time::sleep(DEBOUNCE).await;
                continue;
            };
            let key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (&cfg.url, &cfg.code).hash(&mut h);
                h.finish()
            };
            let forced = app.sync.forced.swap(false, std::sync::atomic::Ordering::Relaxed);
            if !forced && refused.is_some_and(|(k, at)| k == key && at.elapsed() < CODE_REFUSED_PAUSE) {
                // Même code refusé : rien envoyé, l'erreur reste affichée
                let _ = tokio::time::timeout(EVERY, app.sync.wake.notified()).await;
                continue;
            }
            {
                let mut v = app.sync.view.lock().unwrap();
                v.enabled = true;
                v.url = cfg.url.clone();
                v.status = "syncing".into();
            }
            app.touch_domain(crate::app::Domain::Sync);
            let result = run(&app, &cfg).await;
            refused = match &result {
                Err(e) if e == "Code refusé par le serveur" || e.starts_with("Trop de codes incorrects") => Some((key, std::time::Instant::now())),
                _ => None,
            };
            app.sync.code_refused.store(refused.is_some(), std::sync::atomic::Ordering::Relaxed);
            {
                let mut v = app.sync.view.lock().unwrap();
                match result {
                    Ok(o) => {
                        v.status = "ok".into();
                        v.error = None;
                        v.server_collections = o.server_collections;
                        v.local_counts = o.local_counts;
                        v.server_counts = o.server_counts;
                        v.collections = o.collections;
                        v.last_sync = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
                        v.sent += o.sent;
                        v.received += o.received;
                        if o.received > 0 {
                            v.revision += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("synchronisation : {e}");
                        v.status = "error".into();
                        v.error = Some(e);
                    }
                }
            }
            app.touch_domain(crate::app::Domain::Sync);
            // Prochain passage : toutes les 30 s, ou EVERY_LIVE tant que le flux temps réel est
            // ouvert (vérifié toutes les 30 s : flux perdu, le rythme normal reprend)
            let since = std::time::Instant::now();
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(EVERY) => {
                        if app.sync.live_state() == "connected" && since.elapsed() < EVERY_LIVE {
                            continue;
                        }
                    }
                    _ = app.sync.wake.notified() => tokio::time::sleep(DEBOUNCE).await,
                    _ = app.sync.pull.notified() => {}
                }
                break;
            }
        }
    });
}

// --- Web app mobile ---

/// Identifiant de cette application, tiré une fois pour toutes
fn instance_id(conn: &Connection) -> rusqlite::Result<String> {
    if let Some(id) = meta(conn, "instance_id").filter(|id| !id.is_empty()) {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    set_meta(conn, "instance_id", &id)?;
    Ok(id)
}

/// Nom de cette application choisi par l'utilisateur, sinon « PC <premier compte> », sinon
/// le nom de la machine
fn instance_name(app: &App) -> String {
    if let Some(name) = meta(&app.db.conn(), "instance_name").filter(|n| !n.trim().is_empty()) {
        return name;
    }
    default_instance_name(app.db.accounts().ok().and_then(|a| a.into_iter().next()).map(|a| a.login))
}

fn default_instance_name(first_login: Option<String>) -> String {
    if let Some(login) = first_login {
        return format!("PC {login}");
    }
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty());
    host.unwrap_or_else(|| "Twitch Desk".to_owned())
}

/// Adresse de la web app : celle choisie sur ce PC, sinon celle réglée sur le serveur
/// (`SYNC_WEB_URL`), sinon l'adresse du serveur suivie de /app
fn web_app_url(conn: &Connection, server_url: &str) -> String {
    meta(conn, "web_url")
        .filter(|u| !u.is_empty())
        .or_else(|| meta(conn, "server_web_url").filter(|u| !u.is_empty()))
        .unwrap_or_else(|| format!("{}/app", server_url.trim_end_matches('/')))
}

/// Enregistre l'application sur le serveur (nom compris) quand l'adresse ou le nom change ;
/// serveur d'avant la 0.6 : sans effet
async fn register(app: &App, cfg: &Config) {
    let name = instance_name(app);
    let key = format!("{}|{name}", cfg.url);
    if app.sync.registered.lock().unwrap().as_deref() == Some(key.as_str()) {
        return;
    }
    let Ok(id) = instance_id(&app.db.conn()) else { return };
    let req = app.http.put(format!("{}/api/v1/instances/{id}", cfg.url)).json(&json!({ "name": name }));
    match send(req, &cfg.code).await {
        Ok(_) => *app.sync.registered.lock().unwrap() = Some(key),
        Err(e) if e == NOT_SYNC_SERVER => *app.sync.registered.lock().unwrap() = Some(key),
        Err(e) => tracing::debug!("enregistrement de l'application : {e}"),
    }
}

/// État publié pour les téléphones : pour chaque giveaway ouvert ou en pause (par identifiant
/// partagé), nos comptes qui ont participé et s'il est traité ; statut live de ses chaînes ;
/// nos comptes
fn instance_state(view: &crate::giveaways::GiveawaysView, sync_ids: &HashMap<i64, String>, accounts: &[String]) -> Value {
    let mut giveaways = serde_json::Map::new();
    let mut channels: Vec<&str> = vec![];
    for g in &view.giveaways {
        if !matches!(g.row.status.as_str(), "open" | "paused") {
            continue;
        }
        let Some(id) = sync_ids.get(&g.row.id) else { continue };
        let mut participated: Vec<&str> = g.entries_view.iter().map(|e| e.login.as_str()).filter(|l| !l.is_empty()).collect();
        participated.sort_unstable();
        participated.dedup();
        giveaways.insert(id.clone(), json!({ "participated": participated, "handled": g.row.handled, "hidden": g.row.hidden }));
        if !channels.contains(&g.row.channel.as_str()) {
            channels.push(&g.row.channel);
        }
    }
    let mut live = serde_json::Map::new();
    for login in channels {
        let Some(c) = view.channels.iter().find(|c| c.login.eq_ignore_ascii_case(login)) else { continue };
        if !c.live_checked {
            continue;
        }
        let info = match &c.live {
            Some(s) => json!({ "live": true, "viewers": round_viewers(s.viewers), "category": s.game_name, "title": s.title }),
            None => json!({ "live": false }),
        };
        live.insert(login.to_owned(), info);
    }
    json!({ "accounts": accounts, "giveaways": giveaways, "live": live })
}

/// Spectateurs arrondis (2 chiffres significatifs) : l'état n'est pas republié à chaque petite
/// variation
fn round_viewers(n: u64) -> u64 {
    if n < 100 {
        return n;
    }
    let step = 10u64.pow((n as f64).log10().floor() as u32 - 1);
    (n + step / 2) / step * step
}

fn phone_live(live: &Option<crate::twitch::gql::StreamInfo>) -> Value {
    match live {
        Some(s) => json!({ "viewers": round_viewers(s.viewers), "category": s.game_name, "title": s.title }),
        None => Value::Null,
    }
}

/// Chaînes des pages Points de chaîne et StreamElements, par compte, pour la web app (lecture
/// seule) : état, soldes, gains du jour, live et giveaway StreamElements en cours
fn phone_channels(views: &[crate::app::AccountView]) -> (Value, Value) {
    let mut points = vec![];
    let mut se = vec![];
    for a in views {
        let p: Vec<Value> = a
            .channels
            .iter()
            .filter(|c| c.in_points)
            .map(|c| {
                json!({
                    "login": c.login,
                    "avatar": c.profile_image,
                    "state": c.state,
                    "balance": c.balance,
                    "earnedToday": c.earned_today,
                    "maxBalance": c.max_balance,
                    "priority": c.priority,
                    "harvest": c.enabled,
                    "autoClaim": c.auto_claim,
                    "live": phone_live(&c.live),
                })
            })
            .collect();
        points.push(json!({ "login": a.login, "enabled": a.points_enabled, "channels": p }));
        let s: Vec<Value> = a
            .channels
            .iter()
            .filter(|c| c.in_se)
            .map(|c| {
                json!({
                    "login": c.login,
                    "avatar": c.profile_image,
                    "enabled": c.se_enabled,
                    "points": c.se_points,
                    "earnedToday": c.se_earned_today,
                    "inChat": c.in_chat,
                    "chatPresence": c.chat_presence,
                    "autoBuy": c.se_auto_buy,
                    "live": phone_live(&c.live),
                    "myTickets": c.se_my_tickets,
                    "giveaway": c.se_giveaway.as_ref().map(|g| json!({
                        "id": g.id,
                        "title": g.title,
                        "description": g.description,
                        "command": g.command,
                        "cost": g.cost,
                        "maxTickets": g.max_tickets,
                        "totalTickets": g.total_tickets,
                        "totalUsers": g.total_users,
                    })),
                })
            })
            .collect();
        se.push(json!({ "login": a.login, "connected": a.se_account == crate::app::SeAccount::Connected, "channels": s }));
    }
    (Value::Array(points), Value::Array(se))
}

/// Streams repérés publiés, au plus
const PHONE_SCANS: usize = 50;

/// Page Scanner de la web app (lecture seule) : streams repérés, hors streams ignorés
fn phone_scanner(view: &crate::scanner::ScannerView) -> Value {
    let matches: Vec<Value> = view
        .matches
        .iter()
        .filter(|m| !m.ignored)
        .take(PHONE_SCANS)
        .map(|m| {
            json!({
                "login": m.stream.login,
                "displayName": m.stream.display_name,
                "avatar": m.stream.profile_image,
                "title": m.stream.title,
                "game": m.stream.game_name,
                "viewers": round_viewers(m.stream.viewers.max(0) as u64),
                "startedAt": m.stream.started_at,
                "keywords": m.keywords,
                "monitored": m.monitored,
                "giveawayKind": m.giveaway_kind,
                "giveawayCommand": m.giveaway_command,
            })
        })
        .collect();
    json!({ "scannedAt": view.scanned_at, "matches": matches })
}

/// Lien du Discord de chaque streamer d'un giveaway en cours (fiche du streamer)
fn phone_discord(app: &App, view: &crate::giveaways::GiveawaysView) -> Value {
    let channels: BTreeSet<String> =
        view.giveaways.iter().filter(|g| matches!(g.row.status.as_str(), "open" | "paused")).map(|g| g.row.channel.to_lowercase()).collect();
    let mut out = serde_json::Map::new();
    for s in app.db.streamers().unwrap_or_default() {
        if let Some(url) = s.discord_url.filter(|u| u.starts_with("https://")) {
            if channels.contains(&s.login.to_lowercase()) {
                out.insert(s.login.to_lowercase(), json!(url));
            }
        }
    }
    Value::Object(out)
}

/// Langue de diffusion (code ISO 639-1) des chaînes de l'état publié (giveaways, Points,
/// StreamElements, scanner, mentions, drops), et photo de profil de celles qui n'en ont pas déjà
/// une dans l'état (giveaways, mentions, drops) ; inconnues : absentes
fn phone_channel_info(app: &App, state: &Value, scanner: &crate::scanner::ScannerView) -> (Value, Value) {
    let mut logins: BTreeSet<String> = BTreeSet::new();
    if let Some(live) = state["live"].as_object() {
        logins.extend(live.keys().cloned());
    }
    for key in ["points", "streamelements"] {
        for account in state[key].as_array().into_iter().flatten() {
            for c in account["channels"].as_array().into_iter().flatten() {
                if let Some(l) = c["login"].as_str() {
                    logins.insert(l.to_lowercase());
                }
            }
        }
    }
    for m in state["mentions"].as_array().into_iter().flatten() {
        if let Some(c) = m["channel"].as_str() {
            logins.insert(c.to_lowercase());
        }
    }
    // Sans photo dans l'état : giveaways, mentions, chaîne regardée pour les drops
    let mut need_avatar: BTreeSet<String> = BTreeSet::new();
    for g in &app.giveaways.view().giveaways {
        if matches!(g.row.status.as_str(), "open" | "paused") {
            logins.insert(g.row.channel.to_lowercase());
            need_avatar.insert(g.row.channel.to_lowercase());
        }
    }
    for m in state["mentions"].as_array().into_iter().flatten() {
        if let Some(c) = m["channel"].as_str() {
            need_avatar.insert(c.to_lowercase());
        }
    }
    for account in state["drops"].as_array().into_iter().flatten() {
        if let Some(l) = account["target"]["login"].as_str() {
            logins.insert(l.to_lowercase());
            need_avatar.insert(l.to_lowercase());
        }
    }
    let mut avatars = serde_json::Map::new();
    for login in need_avatar {
        if let Some(url) = app.lives.avatar(&login) {
            avatars.insert(login, json!(url));
        }
    }
    let mut out = serde_json::Map::new();
    for m in &scanner.matches {
        if let Some(lang) = m.stream.language.as_deref().filter(|l| !l.is_empty()) {
            out.insert(m.stream.login.to_lowercase(), json!(lang.to_lowercase()));
        }
    }
    for login in logins {
        if out.contains_key(&login) {
            continue;
        }
        if let Some(lang) = app.channel_language(&login) {
            out.insert(login, json!(lang));
        }
    }
    (Value::Object(out), Value::Object(avatars))
}

/// Campagnes publiées par compte, au plus (la regardée et les inachevées d'abord)
const PHONE_CAMPAIGNS: usize = 20;

/// Jaquette en 80×112 (gabarit `{width}x{height}` ou taille déjà choisie), comme l'interface
fn phone_box_art(game: &crate::drops::Game) -> String {
    static SIZE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"-\d+x\d+\.").unwrap());
    match &game.box_art {
        Some(url) => SIZE.replace(&url.replace("{width}", "80").replace("{height}", "112"), "-80x112.").into_owned(),
        None => format!("https://static-cdn.jtvnw.net/ttv-boxart/{}-80x112.jpg", game.id),
    }
}

/// Page Drops de la web app (lecture seule) : chaîne regardée et campagnes en cours de chaque
/// compte, avec la progression de chaque drop
fn phone_drops(views: &[crate::app::AccountView]) -> Value {
    let list: Vec<Value> = views
        .iter()
        .map(|a| {
            let d = &a.drops;
            let watched = d.target.as_ref().and_then(|t| t.campaign_id.as_deref());
            let mut campaigns: Vec<&crate::drops::Campaign> = d.campaigns.iter().filter(|c| c.active() && !d.ignored_campaigns.contains(&c.id)).collect();
            campaigns.sort_by_key(|c| (Some(c.id.as_str()) != watched, !c.unfinished()));
            let campaigns: Vec<Value> = campaigns
                .into_iter()
                .take(PHONE_CAMPAIGNS)
                .map(|c| {
                    json!({
                        "id": c.id,
                        "name": c.name,
                        "game": c.game.name,
                        "boxArt": phone_box_art(&c.game),
                        "endsAt": c.ends_at,
                        "accountLinked": c.account_linked,
                        "watched": Some(c.id.as_str()) == watched,
                        "drops": c.drops.iter().map(|t| {
                            let names: Vec<&str> = t.rewards.iter().map(|r| r.name.as_str()).collect();
                            json!({
                                "id": t.id,
                                "name": if names.is_empty() { t.name.clone() } else { names.join(", ") },
                                "image": t.rewards.first().and_then(|r| r.image.clone()),
                                "requiredMinutes": t.required_minutes,
                                "currentMinutes": t.current_minutes.min(t.required_minutes),
                                "claimed": t.claimed,
                                "claimable": t.claimable(),
                            })
                        }).collect::<Vec<_>>(),
                    })
                })
                .collect();
            // Campagnes ignorées (actives) : à rétablir depuis le téléphone
            let ignored: Vec<Value> = d
                .campaigns
                .iter()
                .filter(|c| c.active() && d.ignored_campaigns.contains(&c.id))
                .take(PHONE_CAMPAIGNS)
                .map(|c| json!({ "id": c.id, "name": c.name, "game": c.game.name }))
                .collect();
            json!({
                "login": a.login,
                "enabled": d.enabled,
                "target": d.target.as_ref().map(|t| json!({ "game": t.game_name, "login": t.login, "title": t.title })),
                "campaigns": campaigns,
                "ignored": ignored,
            })
        })
        .collect();
    Value::Array(list)
}

/// Giveaways ouverts ou en pause : identifiant local → identifiant partagé
fn open_sync_ids(conn: &Connection) -> rusqlite::Result<HashMap<i64, String>> {
    conn.prepare_cached("SELECT id, sync_id FROM giveaways WHERE sync_id IS NOT NULL AND status IN ('open', 'paused')")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

/// Publie l'état de l'application quand il change (regroupé sur STATE_DEBOUNCE), et au moins
/// toutes les STATE_REFRESH
fn spawn_state(app: Arc<App>) {
    tauri::async_runtime::spawn(async move {
        // (adresse, empreinte) du dernier état publié, et quand
        let mut last: Option<(String, String, std::time::Instant)> = None;
        let mut unsupported: Option<(String, std::time::Instant)> = None;
        app.sync.state_changed();
        loop {
            tokio::select! {
                _ = app.sync.state_changed.notified() => {}
                _ = tokio::time::sleep(STATE_REFRESH) => {}
            }
            tokio::time::sleep(STATE_DEBOUNCE).await;
            let Some(cfg) = config(&app).filter(|_| !app.sync.code_refused.load(std::sync::atomic::Ordering::Relaxed)) else { continue };
            if unsupported.as_ref().is_some_and(|(url, at)| *url == cfg.url && at.elapsed() < STATE_UNSUPPORTED_PAUSE) {
                continue;
            }
            let accounts: Vec<String> = app.db.accounts().unwrap_or_default().into_iter().map(|a| a.login).collect();
            let (id, sync_ids) = {
                let conn = app.db.conn();
                match (instance_id(&conn), open_sync_ids(&conn)) {
                    (Ok(id), Ok(ids)) => (id, ids),
                    _ => continue,
                }
            };
            let giveaways = app.giveaways.view();
            let mut state = instance_state(&giveaways, &sync_ids, &accounts);
            // Mentions de nos pseudos (les plus récentes d'abord, 30 au plus)
            state["mentions"] = json!(giveaways.mentions);
            // Pages Points de chaîne et StreamElements de la web app
            let views = app.account_views();
            let (points, se) = phone_channels(&views);
            state["points"] = points;
            state["streamelements"] = se;
            state["drops"] = phone_drops(&views);
            let scanner = app.scanner.lock().unwrap().view();
            state["scanner"] = phone_scanner(&scanner);
            // Langue de diffusion (drapeaux) et photo de profil des chaînes affichées, 0.48.55
            let (languages, avatars) = phone_channel_info(&app, &state, &scanner);
            state["languages"] = languages;
            state["avatars"] = avatars;
            state["actionsAllowed"] = json!(phone_actions_allowed(&app));
            // Application en pause, réponses au gain en attente (annulables), Discord des
            // streamers des giveaways en cours (application 0.48.56)
            state["paused"] = json!(app.is_paused());
            state["winReplies"] = json!(crate::win_reply::pending_view(&app));
            state["discord"] = phone_discord(&app, &giveaways);
            state["pc"] = json!({
                "version": app.handle().package_info().version.to_string(),
                "update": app.sync.pc_update.lock().unwrap().clone(),
            });
            let digest = hash(&state);
            if last.as_ref().is_some_and(|(url, h, at)| *url == cfg.url && *h == digest && at.elapsed() < STATE_REFRESH) {
                continue;
            }
            register(&app, &cfg).await;
            let req = app.http.put(format!("{}/api/v1/instances/{id}/state", cfg.url)).json(&state);
            match send(req, &cfg.code).await {
                Ok(_) => {
                    last = Some((cfg.url.clone(), digest, std::time::Instant::now()));
                    unsupported = None;
                }
                Err(e) if e == NOT_SYNC_SERVER => unsupported = Some((cfg.url.clone(), std::time::Instant::now())),
                Err(e) => {
                    tracing::debug!("état pour la web app : {e}");
                    // Nouvel essai un peu plus tard
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        handle.sync.state_changed();
                    });
                }
            }
        }
    });
}

/// Achats depuis le téléphone autorisés sur ce PC (désactivé par défaut)
fn phone_actions_allowed(app: &App) -> bool {
    meta(&app.db.conn(), "phone_actions").as_deref() == Some("1")
}

#[derive(Debug, Deserialize)]
struct PhoneAction {
    id: String,
    kind: String,
    payload: Value,
}

/// Mise à jour vérifiée pour un téléphone : résultat gardé pour l'état publié
async fn check_update(app: &Arc<App>) -> Result<Option<tauri_plugin_updater::Update>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let found = match app.handle().updater() {
        Ok(updater) => updater.check().await.map_err(|e| format!("Vérification impossible : {e}")),
        Err(e) => Err(format!("Vérification impossible : {e}")),
    };
    let available = found.as_ref().ok().and_then(|u| u.as_ref().map(|u| u.version.clone()));
    *app.sync.pc_update.lock().unwrap() =
        json!({ "available": available, "checkedAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true) });
    app.sync.state_changed();
    found
}

/// Installe la mise à jour puis redémarre l'application (l'installateur Windows la ferme lui-même)
async fn install_update(app: &Arc<App>, update: tauri_plugin_updater::Update) {
    crate::window_state_for_update(app.handle().clone(), Some(true));
    match update.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => app.handle().restart(),
        Err(e) => {
            crate::window_state_for_update(app.handle().clone(), Some(false));
            app.log("téléphone", crate::app::EventKind::Error, format!("Mise à jour demandée par le téléphone : {e}"));
        }
    }
}

/// Exécute une demande d'un téléphone ; renvoie le message du résultat, et la mise à jour à
/// installer une fois le résultat rendu (l'application redémarre ensuite)
async fn run_phone_action(app: &Arc<App>, action: &PhoneAction) -> Result<(String, Option<tauri_plugin_updater::Update>), String> {
    let current = app.handle().package_info().version.to_string();
    match action.kind.as_str() {
        "update_check" => {
            return Ok(match check_update(app).await? {
                Some(u) => (format!("Version {} disponible (installée : {current})", u.version), None),
                None => (format!("L'application du PC est à jour (version {current})"), None),
            });
        }
        "update_install" => {
            return match check_update(app).await? {
                Some(u) => Ok((format!("Installation de la version {} : l'application du PC va redémarrer", u.version), Some(u))),
                None => Err(format!("Aucune mise à jour à installer : l'application du PC est à jour (version {current})")),
            };
        }
        "se_buy" => {}
        _ => return Err("Action inconnue de cette version de l'application".into()),
    }
    if !phone_actions_allowed(app) {
        return Err("Achats depuis le téléphone désactivés sur le PC (Paramètres → Synchronisation)".into());
    }
    let p = &action.payload;
    let (Some(login), Some(channel), Some(giveaway), Some(tickets)) =
        (str_of(p, "account"), str_of(p, "channel"), str_of(p, "giveaway"), int_of(p, "tickets"))
    else {
        return Err("Demande incomplète".into());
    };
    let account = app
        .db
        .accounts()
        .map_err(db_err)?
        .into_iter()
        .find(|a| a.login.eq_ignore_ascii_case(&login))
        .ok_or_else(|| format!("Compte {login} inconnu sur ce PC"))?;
    let ch = app
        .db
        .channels(account.id)
        .map_err(db_err)?
        .into_iter()
        .find(|c| c.in_se && c.login.eq_ignore_ascii_case(&channel))
        .ok_or_else(|| format!("{channel} n'est pas dans la page StreamElements de {login}"))?;
    let total = crate::commands::buy_tickets(app, account.id, ch.id, &giveaway, tickets).await?;
    Ok((format!("{tickets} ticket(s) acheté(s) sur {channel} pour {login} ({total} au total)"), None))
}

/// Demandes du téléphone sur un giveaway ou le scanner (serveur 0.8.10) : masquer, journal du
/// chat, conditions, surveiller une chaîne ; None : autre demande. Pas d'achat : toujours permises
async fn run_giveaway_action(app: &Arc<App>, action: &PhoneAction) -> Option<Result<(String, Option<Value>), String>> {
    if !matches!(action.kind.as_str(), "giveaway_hide" | "giveaway_chat_log" | "giveaway_conditions" | "channel_monitor" | "points_order" | "se_option" | "giveaway_participate" | "giveaway_follow" | "giveaway_hello"
            | "win_reply_cancel" | "points_option" | "points_max" | "drop_campaign_ignore" | "drops_refresh" | "app_pause"
    ) {
        return None;
    }
    let p = &action.payload;
    if action.kind == "points_order" {
        return Some(phone_points_order(app, p));
    }
    if action.kind == "se_option" {
        return Some(phone_se_option(app, p));
    }
    match action.kind.as_str() {
        "win_reply_cancel" => {
            let id = str_of(p, "item").and_then(|i| i.parse::<u64>().ok());
            return Some(match id {
                Some(id) if crate::win_reply::cancel(app, id) => Ok(("Réponse au gain annulée".into(), None)),
                _ => Err("Réponse déjà partie ou inconnue".into()),
            });
        }
        "app_pause" => {
            let paused = bool_of(p, "enabled").unwrap_or(true);
            app.set_paused(paused);
            crate::tray::refresh(app.handle(), paused);
            return Some(Ok((if paused { "Application du PC en pause".into() } else { "Application du PC relancée".into() }, None)));
        }
        "points_option" | "points_max" | "drop_campaign_ignore" | "drops_refresh" => return Some(phone_account_action(app, &action.kind, p)),
        _ => {}
    }
    if action.kind == "channel_monitor" {
        let Some(channel) = str_of(p, "channel") else { return Some(Err("Demande incomplète".into())) };
        return Some(crate::commands::monitor_channel(app, &channel).await.map(|login| (format!("{login} ajoutée aux chaînes surveillées"), None)));
    }
    let local = str_of(p, "item").and_then(|item| local_id(&app.db.conn(), "giveaways", &item).ok().flatten());
    let Some(id) = local else { return Some(Err("Ce giveaway n'existe pas sur le PC".into())) };
    let row = match app.db.giveaway(id) {
        Ok(Some(row)) => row,
        Ok(None) => return Some(Err("Ce giveaway n'existe pas sur le PC".into())),
        Err(e) => return Some(Err(db_err(e))),
    };
    Some(match action.kind.as_str() {
        "giveaway_participate" => {
            // Points de chaîne : des points sont dépensés, même autorisation que les achats
            if row.kind == "channelpoints" && !phone_actions_allowed(app) {
                return Some(Err("Achats depuis le téléphone désactivés sur le PC (Paramètres → Synchronisation)".into()));
            }
            let entered: Vec<i64> = row.entries.iter().map(|e| e.account_id).collect();
            let ids: Vec<i64> = app.db.accounts().unwrap_or_default().into_iter().map(|a| a.id).filter(|a| !entered.contains(a)).collect();
            if ids.is_empty() {
                return Some(Ok(("Tous tes comptes participent déjà".into(), None)));
            }
            match crate::giveaways::participate(app, id, &ids, false).await {
                Ok(results) if results.iter().all(|r| r.ok) => Ok((format!("Participation envoyée sur {} : {}", row.channel, results.iter().map(|r| r.login.as_str()).collect::<Vec<_>>().join(", ")), None)),
                Ok(results) => Err(results.iter().filter(|r| !r.ok).map(|r| format!("{} : {}", r.login, r.message)).collect::<Vec<_>>().join(" · ")),
                Err(e) => Err(e),
            }
        }
        "giveaway_follow" => {
            let ids: Vec<i64> = app.db.accounts().unwrap_or_default().into_iter().map(|a| a.id).collect();
            crate::commands::follow_accounts(app, &row.channel, &ids).await.map(|_| (format!("{} suivie par tes comptes", row.channel), None))
        }
        "giveaway_hello" => crate::commands::hello_accounts(app, &row.channel, None).await.map(|n| {
            (if n == 0 { format!("Bonjour déjà dit sur {} par tous tes comptes", row.channel) } else { format!("Bonjour dit sur {} ({n} compte(s))", row.channel) }, None)
        }),
        "giveaway_hide" => {
            let hidden = bool_of(p, "hidden").unwrap_or(true);
            crate::commands::hide_giveaway(app, id, hidden)
                .map(|_| (format!("Giveaway de {} {}", row.channel, if hidden { "masqué" } else { "affiché de nouveau" }), None))
        }
        "giveaway_chat_log" => {
            // Messages depuis la détection (200 au plus, les plus récents), dans l'ordre
            let since = chrono::DateTime::parse_from_rfc3339(&row.detected_at).ok().map(|t| t - chrono::Duration::minutes(5));
            match app.db.chat_log(&row.channel, &[], &[], None, None, None, None, 200) {
                Ok(mut list) => {
                    list.retain(|e| since.is_none_or(|t| chrono::DateTime::parse_from_rfc3339(&e.at).is_ok_and(|at| at >= t)));
                    list.reverse();
                    let entries: Vec<Value> = list
                        .iter()
                        .map(|e| json!({ "at": e.at, "author": e.author, "kind": e.kind, "command": e.command, "message": e.message }))
                        .collect();
                    Ok((format!("{} message(s) du journal de {}", entries.len(), row.channel), Some(json!({ "channel": row.channel, "entries": entries }))))
                }
                Err(e) => Err(db_err(e)),
            }
        }
        _ => {
            // Conditions : seules celles envoyées changent ; le reste du giveaway est gardé
            let Some(c) = p.get("conditions").and_then(Value::as_object) else { return Some(Err("Demande incomplète".into())) };
            let mut d = row.details.clone();
            let flag = |key: &str, current: Option<bool>| match c.get(key) {
                Some(v) => v.as_bool(),
                None => current,
            };
            let num = |key: &str, current: Option<i64>| match c.get(key) {
                Some(v) => v.as_i64(),
                None => current,
            };
            d.require_follow = flag("requireFollow", d.require_follow);
            d.require_presence = flag("requirePresence", d.require_presence);
            d.require_discord = flag("requireDiscord", d.require_discord);
            d.require_greeting = flag("requireGreeting", d.require_greeting);
            d.can_gift = flag("canGift", d.can_gift);
            d.follow_min_secs = num("followMinSecs", d.follow_min_secs).filter(|_| d.require_follow == Some(true));
            d.min_chat_minutes = num("minChatMinutes", d.min_chat_minutes);
            let input = crate::db::GiveawayInput {
                channel: row.channel.clone(),
                kind: row.kind.clone(),
                command: row.command.clone(),
                url: row.url.clone(),
                gift_ids: row.gift_ids.clone(),
                shown_gift_ids: None,
                details: d,
            };
            crate::commands::apply_giveaway_update(app, id, input).await.map(|_| (format!("Conditions du giveaway de {} enregistrées", row.channel), None))
        }
    })
}

/// Demandes du téléphone sur un compte : récolte ou bonus auto d'une chaîne Points, solde
/// maximum, campagne de drops ignorée ou rétablie, drops relus (et réclamés) tout de suite
fn phone_account_action(app: &Arc<App>, kind: &str, p: &Value) -> Result<(String, Option<Value>), String> {
    let Some(login) = str_of(p, "account") else { return Err("Demande incomplète".into()) };
    let account = app
        .db
        .accounts()
        .map_err(db_err)?
        .into_iter()
        .find(|a| a.login.eq_ignore_ascii_case(&login))
        .ok_or_else(|| format!("Compte {login} inconnu sur ce PC"))?;
    let points_channel = || -> Result<crate::db::Channel, String> {
        let channel = str_of(p, "channel").ok_or("Demande incomplète")?;
        app.db
            .channels(account.id)
            .map_err(db_err)?
            .into_iter()
            .find(|c| c.in_points && c.login.eq_ignore_ascii_case(&channel))
            .ok_or_else(|| format!("{channel} n'est pas dans la page Points de chaîne de {login}"))
    };
    let message = match kind {
        "points_option" => {
            let ch = points_channel()?;
            let enabled = bool_of(p, "enabled").unwrap_or(false);
            match str_of(p, "option").as_deref() {
                Some("harvest") => app.db.set_channel_options(ch.id, enabled, ch.auto_claim, ch.chat_presence).map_err(db_err)?,
                Some("bonus") => app.db.set_channel_options(ch.id, ch.enabled, enabled, ch.chat_presence).map_err(db_err)?,
                _ => return Err("Réglage inconnu".into()),
            }
            format!("{} : réglage enregistré pour {}", ch.login, account.login)
        }
        "points_max" => {
            let ch = points_channel()?;
            let max = int_of(p, "max").filter(|m| *m > 0);
            if max.is_some_and(|m| m > 2_000_000_000) {
                return Err("Solde maximum invalide".into());
            }
            app.db.set_max_balance(ch.id, max).map_err(db_err)?;
            match max {
                Some(m) => format!("{} : solde maximum {m}", ch.login),
                None => format!("{} : plus de solde maximum", ch.login),
            }
        }
        "drop_campaign_ignore" => {
            let campaign = str_of(p, "item").ok_or("Demande incomplète")?;
            let ignored = bool_of(p, "enabled").unwrap_or(true);
            app.db.set_drop_campaign_ignored(account.id, &campaign, ignored).map_err(db_err)?;
            (if ignored { "Campagne ignorée" } else { "Campagne rétablie" }).to_owned()
        }
        "drops_refresh" => {
            app.send(account.id, crate::miner::MinerCommand::RefreshDrops);
            return Ok((format!("Drops de {} relus : ceux terminés sont réclamés", account.login), None));
        }
        _ => return Err("Action inconnue".into()),
    };
    app.send(account.id, crate::miner::MinerCommand::Reload);
    Ok((message, None))
}

/// Présence dans le chat ou achat automatique d'une chaîne StreamElements, depuis le téléphone
fn phone_se_option(app: &Arc<App>, p: &Value) -> Result<(String, Option<Value>), String> {
    let (Some(login), Some(channel), Some(option)) = (str_of(p, "account"), str_of(p, "channel"), str_of(p, "option")) else {
        return Err("Demande incomplète".into());
    };
    let enabled = bool_of(p, "enabled").unwrap_or(false);
    let account = app
        .db
        .accounts()
        .map_err(db_err)?
        .into_iter()
        .find(|a| a.login.eq_ignore_ascii_case(&login))
        .ok_or_else(|| format!("Compte {login} inconnu sur ce PC"))?;
    let ch = app
        .db
        .channels(account.id)
        .map_err(db_err)?
        .into_iter()
        .find(|c| c.in_se && c.login.eq_ignore_ascii_case(&channel))
        .ok_or_else(|| format!("{channel} n'est pas dans la page StreamElements de {login}"))?;
    let what = match option.as_str() {
        "chat" => {
            app.db.set_channel_options(ch.id, ch.enabled, ch.auto_claim, enabled).map_err(db_err)?;
            "présence dans le chat"
        }
        "autoBuy" => {
            // Des points seront dépensés : même autorisation que les achats depuis le téléphone
            if enabled && !phone_actions_allowed(app) {
                return Err("Achats depuis le téléphone désactivés sur le PC (Paramètres → Synchronisation)".into());
            }
            app.db.set_se_auto_buy(ch.id, enabled).map_err(db_err)?;
            "achat automatique"
        }
        _ => return Err("Réglage inconnu".into()),
    };
    app.send(account.id, crate::miner::MinerCommand::Reload);
    Ok((format!("{} : {what} {} pour {}", ch.login, if enabled { "activé(e)" } else { "désactivé(e)" }, account.login), None))
}

/// Ordre des chaînes de la page Points de chaîne d'un compte, choisi sur le téléphone (pseudos) ;
/// chaînes absentes de la liste reçue : après, dans leur ordre actuel
fn phone_points_order(app: &Arc<App>, p: &Value) -> Result<(String, Option<Value>), String> {
    let Some(login) = str_of(p, "account") else { return Err("Demande incomplète".into()) };
    let order = strings_of(p, "order");
    let account = app
        .db
        .accounts()
        .map_err(db_err)?
        .into_iter()
        .find(|a| a.login.eq_ignore_ascii_case(&login))
        .ok_or_else(|| format!("Compte {login} inconnu sur ce PC"))?;
    let channels: Vec<crate::db::Channel> = app.db.channels(account.id).map_err(db_err)?.into_iter().filter(|c| c.in_points).collect();
    let mut ids: Vec<i64> = order.iter().filter_map(|l| channels.iter().find(|c| c.login.eq_ignore_ascii_case(l)).map(|c| c.id)).collect();
    ids.extend(channels.iter().map(|c| c.id).filter(|id| !ids.contains(id)).collect::<Vec<_>>());
    app.db.reorder_channels(account.id, &ids).map_err(db_err)?;
    app.send(account.id, crate::miner::MinerCommand::Reload);
    Ok((format!("Ordre des chaînes de {} enregistré", account.login), None))
}

/// Demandes des téléphones (serveur 0.8 et plus) : lues à chaque annonce du serveur et toutes les
/// minutes, exécutées une par une, résultat rendu au serveur
fn spawn_phone_actions(app: Arc<App>) {
    tauri::async_runtime::spawn(async move {
        let mut unsupported: Option<(String, std::time::Instant)> = None;
        loop {
            tokio::select! {
                _ = app.sync.phone_action.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
            let Some(cfg) = config(&app).filter(|_| !app.sync.code_refused.load(std::sync::atomic::Ordering::Relaxed)) else { continue };
            if unsupported.as_ref().is_some_and(|(url, at)| *url == cfg.url && at.elapsed() < STATE_UNSUPPORTED_PAUSE) {
                continue;
            }
            let Ok(id) = instance_id(&app.db.conn()) else { continue };
            let list: Vec<PhoneAction> = match request(app.http.get(format!("{}/api/v1/instances/{id}/actions", cfg.url)), &cfg.code).await {
                Ok(list) => list,
                Err(e) if e == NOT_SYNC_SERVER => {
                    unsupported = Some((cfg.url.clone(), std::time::Instant::now()));
                    continue;
                }
                Err(e) => {
                    tracing::debug!("demandes des téléphones : {e}");
                    continue;
                }
            };
            for action in list {
                let (ok, message, update, data) = if let Some(result) = run_giveaway_action(&app, &action).await {
                    match result {
                        Ok((m, data)) => (true, m, None, data),
                        Err(e) => (false, e, None, None),
                    }
                } else {
                    match run_phone_action(&app, &action).await {
                        Ok((m, update)) => (true, m, update, None),
                        Err(e) => (false, e, None, None),
                    }
                };
                let kind = if ok { crate::app::EventKind::Info } else { crate::app::EventKind::Error };
                app.log("téléphone", kind, format!("Demande du téléphone : {message}"));
                // État du téléphone republié aussitôt (scanner, masqués…), sans attendre
                app.sync.state_changed();
                let req = app.http.put(format!("{}/api/v1/instances/{id}/actions/{}", cfg.url, action.id)).json(&json!({ "ok": ok, "message": message, "data": data }));
                if let Err(e) = send(req, &cfg.code).await {
                    tracing::debug!("résultat d'une demande du téléphone : {e}");
                }
                // Résultat rendu : installation, puis redémarrage
                if let Some(update) = update {
                    install_update(&app, update).await;
                }
            }
        }
    });
}

/// Achats depuis le téléphone (pages de la web app) autorisés ou non
#[tauri::command]
pub fn set_phone_actions(app: State<'_, Arc<App>>, enabled: bool) -> Result<(), String> {
    set_meta(&app.db.conn(), "phone_actions", if enabled { "1" } else { "0" }).map_err(db_err)?;
    app.sync.state_changed();
    Ok(())
}

/// Envoie les alertes aux téléphones (serveur 0.7 et plus ; plus ancien : alertes oubliées,
/// nouvel essai STATE_UNSUPPORTED_PAUSE plus tard)
fn spawn_phone_alerts(app: Arc<App>) {
    tauri::async_runtime::spawn(async move {
        let mut unsupported: Option<(String, std::time::Instant)> = None;
        let mut retry = false;
        loop {
            if retry {
                // Nouvel essai après une erreur (ou plus tôt si une nouvelle alerte arrive)
                let _ = tokio::time::timeout(PHONE_ALERT_RETRY, app.sync.phone_alert.notified()).await;
            } else {
                app.sync.phone_alert.notified().await;
            }
            retry = false;
            let alerts: Vec<PhoneAlert> = app.sync.phone_alerts.lock().unwrap().drain(..).filter(|a| a.queued.elapsed() < PHONE_ALERT_TTL).collect();
            if alerts.is_empty() {
                continue;
            }
            let Some(cfg) = config(&app).filter(|_| !app.sync.code_refused.load(std::sync::atomic::Ordering::Relaxed)) else { continue };
            if unsupported.as_ref().is_some_and(|(url, at)| *url == cfg.url && at.elapsed() < STATE_UNSUPPORTED_PAUSE) {
                continue;
            }
            let Ok(id) = instance_id(&app.db.conn()) else { continue };
            register(&app, &cfg).await;
            let mut rest = alerts.into_iter();
            while let Some(alert) = rest.next() {
                let req = app.http.post(format!("{}/api/v1/instances/{id}/notify", cfg.url)).json(&alert);
                match send(req, &cfg.code).await {
                    Ok(_) => unsupported = None,
                    Err(e) if e == NOT_SYNC_SERVER => {
                        unsupported = Some((cfg.url.clone(), std::time::Instant::now()));
                        break;
                    }
                    Err(e) => {
                        // Serveur injoignable : cette alerte et les suivantes remises en tête de
                        // file (sans attendre l'échec de chacune), nouvel essai plus tard
                        tracing::debug!("alerte pour les téléphones : {e} ; nouvel essai dans 30 s");
                        let mut queue = app.sync.phone_alerts.lock().unwrap();
                        for a in std::iter::once(alert).chain(rest).collect::<Vec<_>>().into_iter().rev() {
                            queue.push_front(a);
                        }
                        while queue.len() > MAX_PHONE_ALERTS {
                            queue.pop_front();
                        }
                        retry = true;
                        break;
                    }
                }
            }
        }
    });
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingView {
    code: String,
    expires_at: String,
    /// Lien à ouvrir sur le téléphone (QR code)
    #[serde(default)]
    link: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    id: String,
    name: String,
    created_at: String,
    last_seen: Option<String>,
}

/// Adresse et code du serveur enregistrés (synchronisation activée ou non)
fn saved_server(app: &App) -> Result<(String, String, String), String> {
    let conn = app.db.conn();
    let url = meta(&conn, "url").filter(|u| !u.is_empty()).ok_or("Indique d'abord l'adresse du serveur de synchronisation")?;
    let id = instance_id(&conn).map_err(db_err)?;
    drop(conn);
    let code = app.secrets.load_raw(CODE_KEY).filter(|c| !c.is_empty()).ok_or("Indique d'abord le code du serveur")?;
    Ok((url, code, id))
}

/// Code d'association à 6 chiffres (valable 5 min) et lien de la web app qui le contient
#[tauri::command]
pub async fn sync_pair_start(app: State<'_, Arc<App>>) -> Result<PairingView, String> {
    let (url, code, id) = saved_server(&app)?;
    let req = app.http.post(format!("{url}/api/v1/pair/start")).json(&json!({ "instance": id, "name": instance_name(&app) }));
    let mut pairing: PairingView = request(req, &code).await.map_err(web_app_err)?;
    pairing.link = format!("{}/#pair={}", web_app_url(&app.db.conn(), &url), pairing.code);
    Ok(pairing)
}

/// Téléphones associés à cette application
#[tauri::command]
pub async fn sync_devices(app: State<'_, Arc<App>>) -> Result<Vec<DeviceView>, String> {
    let (url, code, id) = saved_server(&app)?;
    request(app.http.get(format!("{url}/api/v1/instances/{id}/devices")), &code).await.map_err(web_app_err)
}

/// Applications connues du serveur (diagnostic) : ouvertes ou non, comptes Twitch, appareils
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceOverview {
    id: String,
    name: String,
    online: bool,
    last_seen: String,
    accounts: Vec<String>,
    /// Version de l'application (serveur 0.8.9 et plus)
    #[serde(default)]
    version: Option<String>,
    devices: i64,
    devices_online: i64,
    /// Cette application
    #[serde(default)]
    current: bool,
}

#[tauri::command]
pub async fn sync_instances(app: State<'_, Arc<App>>) -> Result<Vec<InstanceOverview>, String> {
    let (url, code, id) = saved_server(&app)?;
    let mut list: Vec<InstanceOverview> = request(app.http.get(format!("{url}/api/v1/admin/instances")), &code)
        .await
        .map_err(|e| if e == NOT_SYNC_SERVER { "Mets à jour le serveur de synchronisation (version 0.8.8 ou plus)".to_owned() } else { e })?;
    for i in &mut list {
        i.current = i.id == id;
    }
    Ok(list)
}

/// Révoque un téléphone : son accès est refusé aussitôt
#[tauri::command]
pub async fn sync_revoke_device(app: State<'_, Arc<App>>, device_id: String) -> Result<(), String> {
    let (url, code, id) = saved_server(&app)?;
    if device_id.is_empty() || !device_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Appareil inconnu".into());
    }
    send(app.http.delete(format!("{url}/api/v1/instances/{id}/devices/{device_id}")), &code).await.map_err(web_app_err)?;
    Ok(())
}

/// Nom de cette application (vide : nom par défaut) et adresse de la web app (vide : celle du
/// serveur suivie de /app)
#[tauri::command]
pub fn save_sync_instance(app: State<'_, Arc<App>>, name: String, web_url: String) -> Result<(), String> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(60).collect();
    let web_url = if web_url.trim().is_empty() { String::new() } else { normalize_url(&web_url).map_err(|_| "Adresse de la web app invalide (http:// ou https://)".to_owned())? };
    let conn = app.db.conn();
    set_meta(&conn, "instance_name", &name).map_err(db_err)?;
    set_meta(&conn, "web_url", &web_url).map_err(db_err)?;
    drop(conn);
    // Nouveau nom : réenregistrée au prochain passage
    *app.sync.registered.lock().unwrap() = None;
    app.sync.poke();
    Ok(())
}

// --- Commandes ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfigView {
    url: String,
    enabled: bool,
    has_code: bool,
    /// Nom de cette application (vide : `default_name`)
    instance_name: String,
    default_name: String,
    /// Adresse de la web app choisie (vide : celle du serveur)
    web_url: String,
    /// Adresse réglée sur le serveur (`SYNC_WEB_URL`, vide : aucune)
    server_web_url: String,
    /// Adresse de la web app utilisée (choisie, sinon serveur, sinon serveur suivi de /app)
    app_url: String,
    /// Achats de tickets StreamElements depuis le téléphone autorisés
    phone_actions: bool,
}

#[tauri::command]
pub fn get_sync_config(app: State<'_, Arc<App>>) -> SyncConfigView {
    let default_name = default_instance_name(app.db.accounts().ok().and_then(|a| a.into_iter().next()).map(|a| a.login));
    let conn = app.db.conn();
    SyncConfigView {
        url: meta(&conn, "url").unwrap_or_default(),
        enabled: meta(&conn, "enabled").as_deref() == Some("1"),
        has_code: app.secrets.load_raw(CODE_KEY).is_some_and(|c| !c.is_empty()),
        instance_name: meta(&conn, "instance_name").unwrap_or_default(),
        default_name,
        web_url: meta(&conn, "web_url").unwrap_or_default(),
        server_web_url: meta(&conn, "server_web_url").unwrap_or_default(),
        app_url: meta(&conn, "url").filter(|u| !u.is_empty()).map(|u| web_app_url(&conn, &u)).unwrap_or_default(),
        phone_actions: meta(&conn, "phone_actions").as_deref() == Some("1"),
    }
}

/// Enregistre l'adresse, le code (laissé vide : inchangé) et l'activation
#[tauri::command]
pub fn save_sync_config(app: State<'_, Arc<App>>, url: String, code: Option<String>, enabled: bool) -> Result<(), String> {
    let url = if url.trim().is_empty() { String::new() } else { normalize_url(&url)? };
    if let Some(code) = code.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()) {
        app.secrets.save_raw(CODE_KEY, &code)?;
    }
    if enabled && url.is_empty() {
        return Err("Indique l'adresse du serveur".into());
    }
    if enabled && !app.secrets.load_raw(CODE_KEY).is_some_and(|c| !c.is_empty()) {
        return Err("Indique le code du serveur".into());
    }
    let conn = app.db.conn();
    set_meta(&conn, "url", &url).map_err(db_err)?;
    set_meta(&conn, "enabled", if enabled { "1" } else { "0" }).map_err(db_err)?;
    drop(conn);
    app.sync.reconfigure.notify_one();
    app.sync.poke();
    Ok(())
}

/// Vérifie l'adresse et le code (code vide : celui enregistré) sans rien enregistrer
#[tauri::command]
pub async fn test_sync(app: State<'_, Arc<App>>, url: String, code: Option<String>) -> Result<ServerInfo, String> {
    let url = normalize_url(&url)?;
    let code = match code.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()) {
        Some(c) => c,
        None => app.secrets.load_raw(CODE_KEY).filter(|c| !c.is_empty()).ok_or("Indique le code du serveur")?,
    };
    server_info(&app.http, &url, &code).await
}

/// Tout relire depuis le serveur : état de synchronisation oublié, le serveur fait foi pour
/// ce qu'il contient ; ce qui n'existe qu'ici est renvoyé
#[tauri::command]
pub fn sync_resync(app: State<'_, Arc<App>>) -> Result<(), String> {
    let conn = app.db.conn();
    conn.execute("DELETE FROM sync_state", []).map_err(db_err)?;
    set_meta(&conn, "cursor", "0").map_err(db_err)?;
    mark_all_dirty(&conn).map_err(db_err)?;
    drop(conn);
    tracing::info!("synchronisation : relecture complète demandée");
    app.sync.poke();
    Ok(())
}

/// Collections à ne pas synchroniser sur ce poste
#[tauri::command]
pub fn set_sync_skipped(app: State<'_, Arc<App>>, skipped: Vec<String>) -> Result<(), String> {
    let list: Vec<&str> = COLLECTIONS.iter().copied().filter(|c| skipped.iter().any(|s| s == c)).collect();
    set_meta(&app.db.conn(), "skipped", &serde_json::to_string(&list).expect("liste sérialisable")).map_err(db_err)?;
    // Affiché tout de suite, sans attendre le prochain passage
    {
        let mut v = app.sync.view.lock().unwrap();
        v.collections = v.server_collections.iter().filter(|c| !list.contains(&c.as_str())).cloned().collect();
    }
    app.touch_domain(crate::app::Domain::Sync);
    app.sync.poke();
    Ok(())
}

#[tauri::command]
pub fn sync_now(app: State<'_, Arc<App>>) {
    // Flux temps réel en attente (serveur mis à jour depuis ?) : nouvel essai tout de suite
    if matches!(app.sync.live_state(), "reconnecting" | "unavailable" | "refused") {
        app.sync.reconfigure.notify_one();
    }
    app.sync.forced.store(true, std::sync::atomic::Ordering::Relaxed);
    app.sync.poke();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use std::collections::BTreeSet;

    fn docs(db: &Db, c: &str) -> HashMap<String, Value> {
        export(&db.conn(), c, &COLLECTIONS).unwrap().into_iter().collect()
    }

    /// Simule le serveur : ce qu'une application envoie, l'autre l'applique
    fn transfer(from: &Db, to: &Db, c: &str) {
        for (id, data) in docs(from, c) {
            let conn = to.conn();
            if apply(&conn, c, &id, Some(&data)).unwrap() {
                set_state(&conn, c, &id, Some(&hash(&data))).unwrap();
            }
        }
    }

    #[test]
    fn detection_rules_merge() {
        let a = Db::memory().unwrap();
        let b = Db::memory().unwrap();
        a.set_detection_rules("winner", &["Bravo {pseudo} !".into()]).unwrap();
        a.set_detection_rules("bot", &["mwetton".into()]).unwrap();
        b.set_detection_rules("winner", &["bravo {PSEUDO} !".into(), "Gagné par {pseudo}".into()]).unwrap();
        transfer(&a, &b, "detection");
        // Même formulation à la casse près : un seul document, gardé sous sa forme locale
        assert_eq!(b.detection_rules("winner").unwrap().len(), 2);
        assert_eq!(b.detection_rules("bot").unwrap(), vec!["mwetton"]);
        // Suppression reçue
        let id = rule_id("bot", "mwetton");
        apply(&b.conn(), "detection", &id, None).unwrap();
        assert!(b.detection_rules("bot").unwrap().is_empty());
        // Contenu qui ne correspond pas à l'identifiant, ou invalide : refusé
        assert!(!apply(&b.conn(), "detection", "bot:autre", Some(&json!({ "kind": "bot", "value": "pirate" }))).unwrap());
        assert!(!apply(&b.conn(), "detection", &rule_id("winner", "sans pseudo"), Some(&json!({ "kind": "winner", "value": "sans pseudo" }))).unwrap());
    }

    #[test]
    fn server_events_are_read_piece_by_piece() {
        let mut p = EventParser::default();
        assert!(p.feed(b": keep-alive\n\nevent: seq\nda").is_empty());
        assert_eq!(p.feed(b"ta: {\"seq\":42}\n\n"), vec![42]);
        assert_eq!(p.feed(b"event: seq\r\ndata: {\"seq\":43}\r\n\r\nevent: seq\ndata:44\n\n"), vec![43, 44]);
        // Autres événements et données illisibles : ignorés
        assert!(p.feed(b"event: autre\ndata: {\"seq\":1}\n\ndata: pas du json\n\n").is_empty());
        assert_eq!(p.feed(b"data: {\"seq\":45}\n\n"), vec![45]);
        // Demande d'un téléphone : application visée retenue
        assert!(p.feed(b"event: action\ndata: {\"instance\":\"abc-123\"}\n\n").is_empty());
        assert_eq!(p.actions, vec!["abc-123".to_owned()]);
    }

    #[test]
    fn hash_ignores_key_order() {
        assert_eq!(hash(&json!({"a": 1, "b": [1, {"y": 2, "x": 1}]})), hash(&json!({"b": [1, {"x": 1, "y": 2}], "a": 1})));
        assert_ne!(hash(&json!({"a": 1})), hash(&json!({"a": 2})));
    }

    #[test]
    fn url_rules() {
        assert_eq!(normalize_url(" https://sync.example.com/ ").unwrap(), "https://sync.example.com");
        assert!(normalize_url("ftp://x").is_err());
        assert!(normalize_url("sync.example.com").is_err());
    }

    #[test]
    fn gifts_groups_and_giveaways_travel_with_their_links() {
        let a = Db::memory().unwrap();
        let b = Db::memory().unwrap();
        let group = a.ensure_gift_group("Voiles", Some("490377"), Some("Sea of Thieves")).unwrap();
        let gift = a
            .save_gift(&crate::db::Gift {
                id: 0,
                title: "Obsidian Sails".into(),
                image: Some("https://example.com/sails.png".into()),
                group_id: Some(group),
                keywords: vec!["obsidian".into()],
                game_id: Some("490377".into()),
                game_name: Some("Sea of Thieves".into()),
                favorite: false,
            })
            .unwrap();
        let input = crate::db::GiveawayInput {
            channel: "lunarsea".into(),
            kind: "command".into(),
            command: Some("!join".into()),
            url: None,
            gift_ids: vec![gift],
            shown_gift_ids: None,
            details: crate::db::GiveawayDetails { can_gift: Some(true), ..Default::default() },
        };
        a.insert_manual_giveaway(&input, "2026-11-01T10:00:00Z").unwrap();
        a.add_giveaway_channel("lunarsea", "123").unwrap();
        a.save_streamer(&crate::db::Streamer {
            login: "lunarsea".into(),
            discord_url: Some("https://discord.gg/x".into()),
            notes: None,
            ignored_commands: vec!["!points".into()],
            can_gift: Some(false),
            auto_commands: vec!["!points".into(), "!bonus".into()],
            auto_commands_on: vec!["!points".into()],
            ..Default::default()
        })
            .unwrap();
        assign_ids(&a.conn()).unwrap();
        for c in COLLECTIONS {
            transfer(&a, &b, c);
        }
        let gifts = b.gifts().unwrap();
        assert_eq!(gifts.len(), 1);
        let groups = b.gift_groups().unwrap();
        assert_eq!(gifts[0].group_id, Some(groups[0].id));
        let giveaways = b.giveaways(10).unwrap();
        assert_eq!((giveaways[0].command.as_deref(), giveaways[0].gift_ids.clone()), (Some("!join"), vec![gifts[0].id]));
        assert!(giveaways[0].open && !giveaways[0].handled);
        assert_eq!(giveaways[0].details.can_gift, Some(true));
        let sheet = b.streamer("lunarsea").unwrap().unwrap();
        assert_eq!(sheet.can_gift, Some(false));
        // Liste des commandes automatiques partagée, mais y répondre reste le choix de chaque PC
        assert_eq!(sheet.auto_commands, vec!["!points".to_owned(), "!bonus".to_owned()]);
        assert!(sheet.auto_commands_on.is_empty());
        assert_eq!(b.giveaway_channels().unwrap()[0].channel_id, "123");
        assert_eq!(b.streamer("lunarsea").unwrap().unwrap().discord_url.as_deref(), Some("https://discord.gg/x"));
        // Même contenu des deux côtés : rien à renvoyer
        for c in COLLECTIONS {
            let known = state(&b.conn(), c).unwrap();
            assert!(docs(&b, c).iter().all(|(id, v)| known.get(id) == Some(&hash(v))), "{c}");
        }
        // Suppression reçue
        let id = docs(&a, "gifts").into_keys().next().unwrap();
        apply(&b.conn(), "gifts", &id, None).unwrap();
        assert!(b.gifts().unwrap().is_empty());
    }

    #[test]
    fn duplicates_merge_on_the_smallest_id() {
        let a = Db::memory().unwrap();
        let b = Db::memory().unwrap();
        for db in [&a, &b] {
            db.ensure_gift_group("Voiles", None, None).unwrap();
            assign_ids(&db.conn()).unwrap();
        }
        let id_a = docs(&a, "groups").into_keys().next().unwrap();
        let id_b = docs(&b, "groups").into_keys().next().unwrap();
        let (small, big) = if id_a < id_b { (&a, &b) } else { (&b, &a) };
        // Le plus grand reçu par celui qui a le plus petit : ignoré
        transfer(big, small, "groups");
        assert_eq!(small.gift_groups().unwrap().len(), 1);
        // Le plus petit reçu par l'autre : adopté, sans doublon
        transfer(small, big, "groups");
        assert_eq!(big.gift_groups().unwrap().len(), 1);
        assert_eq!(docs(big, "groups").into_keys().collect::<Vec<_>>(), docs(small, "groups").into_keys().collect::<Vec<_>>());
    }

    #[test]
    fn gift_received_before_its_group_is_linked_on_reapply() {
        let a = Db::memory().unwrap();
        let b = Db::memory().unwrap();
        let group = a.ensure_gift_group("Voiles", None, None).unwrap();
        a.save_gift(&crate::db::Gift {
            id: 0,
            title: "Obsidian Sails".into(),
            image: None,
            group_id: Some(group),
            keywords: vec![],
            game_id: None,
            game_name: None,
            favorite: false,
        })
        .unwrap();
        assign_ids(&a.conn()).unwrap();
        let (gift_id, gift) = docs(&a, "gifts").into_iter().next().unwrap();
        let conn = b.conn();
        assert!(unresolved(&conn, "gifts", &gift).unwrap());
        apply(&conn, "gifts", &gift_id, Some(&gift)).unwrap();
        drop(conn);
        transfer(&a, &b, "groups");
        let conn = b.conn();
        assert!(!unresolved(&conn, "gifts", &gift).unwrap());
        apply(&conn, "gifts", &gift_id, Some(&gift)).unwrap();
        drop(conn);
        assert_eq!(b.gifts().unwrap()[0].group_id, Some(b.gift_groups().unwrap()[0].id));
        // Groupes non synchronisés : le lien n'est pas envoyé, le groupe local reste
        let without = export(&b.conn(), "gifts", &["gifts"]).unwrap();
        assert!(without[0].1.get("group").is_none());
        apply(&b.conn(), "gifts", &gift_id, Some(&without[0].1)).unwrap();
        assert!(b.gifts().unwrap()[0].group_id.is_some());
    }

    #[test]
    fn channels_are_not_shared_until_chosen() {
        let db = Db::memory().unwrap();
        assert_eq!(skipped(&db.conn()), vec!["channels"]);
        set_meta(&db.conn(), "skipped", "[]").unwrap();
        assert!(skipped(&db.conn()).is_empty());
    }

    #[test]
    fn stored_version_may_differ_from_received_one() {
        let db = Db::memory().unwrap();
        let received = json!({"title": "  Obsidian Sails ", "image": "http://insecure.example/a.png", "keywords": ["voiles"]});
        apply(&db.conn(), "gifts", "g1", Some(&received)).unwrap();
        let stored = export_one(&db.conn(), "gifts", "g1", &COLLECTIONS).unwrap().unwrap();
        assert_eq!(stored["title"], "Obsidian Sails");
        assert!(stored["image"].is_null());
        // C'est l'empreinte de la version enregistrée qui est gardée : sinon le cadeau passerait
        // pour modifié ici et les mises à jour suivantes seraient ignorées
        assert_ne!(hash(&stored), hash(&received));
        let all: HashMap<String, Value> = docs(&db, "gifts");
        assert_eq!(hash(&all["g1"]), hash(&stored));
    }

    #[test]
    fn invalid_documents_are_ignored() {
        let db = Db::memory().unwrap();
        let conn = db.conn();
        assert!(!apply(&conn, "channels", "Not A Login!", Some(&json!({"channelId": "1"}))).unwrap());
        assert!(!apply(&conn, "giveaways", "x", Some(&json!({"channel": "a", "kind": "evil", "detectedAt": "2026"}))).unwrap());
        assert!(apply(&conn, "gifts", "g", Some(&json!({"title": "T", "image": "javascript:alert(1)"}))).unwrap());
        drop(conn);
        assert_eq!(db.gifts().unwrap()[0].image, None);
    }

    // --- Envois à partir des modifications notées ---

    type Sent = BTreeSet<(String, String, bool)>;

    /// Début de passage (comme `run`) : identifiants donnés, tout noté la première fois
    fn start(db: &Db) {
        let conn = db.conn();
        assign_ids(&conn).unwrap();
        if meta(&conn, "dirty_all").is_some() {
            mark_all_dirty(&conn).unwrap();
        }
    }

    /// Ce qu'envoyait l'ancienne version : tout relu et comparé à `sync_state`
    fn full_outgoing(db: &Db, enabled: &[&str]) -> Sent {
        let conn = db.conn();
        let mut out = Sent::new();
        for c in enabled {
            let known = state(&conn, c).unwrap();
            let mut exported = HashSet::new();
            for (id, data) in export(&conn, c, enabled).unwrap() {
                exported.insert(id.clone());
                if known.get(&id) != Some(&hash(&data)) {
                    out.insert((c.to_string(), id, false));
                }
            }
            for (id, h) in &known {
                if !exists(&conn, c, id).unwrap() {
                    out.insert((c.to_string(), id.clone(), true));
                } else if !exported.contains(id) && export_one(&conn, c, id, enabled).unwrap().is_some_and(|d| &hash(&d) != h) {
                    out.insert((c.to_string(), id.clone(), false));
                }
            }
        }
        out
    }

    /// Envoi réussi : rend ce qui est parti, sous forme de modifications reçues par les autres
    fn push(db: &Db, enabled: &[&str]) -> Vec<RemoteChange> {
        start(db);
        let (out, whole) = outgoing(db, enabled).unwrap();
        let changes = out
            .iter()
            .map(|p| RemoteChange { collection: p.change.collection.to_owned(), id: p.change.id.clone(), data: p.change.data.clone(), deleted: p.change.deleted })
            .collect();
        let conn = db.conn();
        acknowledge(&conn, out).unwrap();
        clear_whole(&conn, &whole).unwrap();
        changes
    }

    fn sent(changes: &[RemoteChange]) -> Sent {
        let set: Sent = changes.iter().map(|c| (c.collection.clone(), c.id.clone(), c.deleted)).collect();
        assert_eq!(set.len(), changes.len(), "document envoyé deux fois");
        set
    }

    /// Envoi comparé à l'ancienne version, qui relisait tout
    fn push_checked(db: &Db, enabled: &[&str]) -> Vec<RemoteChange> {
        start(db);
        let expected = full_outgoing(db, enabled);
        let changes = push(db, enabled);
        assert_eq!(sent(&changes), expected);
        assert!(full_outgoing(db, enabled).is_empty());
        changes
    }

    fn receive_all(db: &Db, changes: &[RemoteChange], enabled: &[&str]) -> u64 {
        start(db);
        let conn = db.conn();
        let mut deferred = vec![];
        let mut touched = HashSet::new();
        let n = receive(&conn, changes, enabled, &mut deferred, &mut touched).unwrap();
        reapply(&conn, &deferred, enabled).unwrap();
        n
    }

    fn gift(title: &str, group_id: Option<i64>) -> crate::db::Gift {
        crate::db::Gift {
            id: 0,
            title: title.into(),
            image: Some(format!("data:image/png;base64,{}", "A".repeat(4096))),
            group_id,
            keywords: vec![],
            game_id: None,
            game_name: None,
            favorite: false,
        }
    }

    fn giveaway_input(gift_ids: Vec<i64>) -> crate::db::GiveawayInput {
        crate::db::GiveawayInput {
            channel: "lunarsea".into(),
            kind: "command".into(),
            command: Some("!join".into()),
            url: None,
            gift_ids,
            shown_gift_ids: None,
            details: Default::default(),
        }
    }

    /// Un exemplaire de chaque collection ; rend (groupe, cadeau, giveaway)
    fn populate(db: &Db) -> (i64, i64, i64) {
        let group = db.ensure_gift_group("Voiles", None, None).unwrap();
        let gift = db.save_gift(&gift("Obsidian Sails", Some(group))).unwrap();
        let giveaway = db.insert_manual_giveaway(&giveaway_input(vec![gift]), &chrono::Utc::now().to_rfc3339()).unwrap();
        db.add_giveaway_channel("lunarsea", "123").unwrap();
        db.save_streamer(&crate::db::Streamer { login: "lunarsea".into(), notes: Some("note".into()), ..Default::default() }).unwrap();
        db.set_detection_rules("bot", &["mwetton".into()]).unwrap();
        (group, gift, giveaway)
    }

    fn sync_id(db: &Db, table: &str, id: i64) -> String {
        db.conn().query_row(&format!("SELECT sync_id FROM {table} WHERE id = ?1"), [id], |r| r.get(0)).unwrap()
    }

    #[test]
    fn hash_matches_previous_format() {
        // Empreinte d'avant (JSON canonique copié puis converti en texte) : les états déjà
        // enregistrés restent valables
        fn canonical(value: &Value) -> Value {
            match value {
                Value::Object(map) => Value::Object(
                    map.iter().map(|(k, v)| (k.clone(), canonical(v))).collect::<BTreeMap<_, _>>().into_iter().collect(),
                ),
                Value::Array(list) => Value::Array(list.iter().map(canonical).collect()),
                v => v.clone(),
            }
        }
        fn previous(value: &Value) -> String {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in canonical(value).to_string().bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            format!("{h:016x}")
        }
        let v = json!({"z": "é\"\n", "a": [1, 2.5, null, {"y": true, "b": "x"}], "m": {"k": -3}});
        assert_eq!(hash(&v), previous(&v));
        assert_eq!(hash(&json!("bravo")), previous(&json!("bravo")));
    }

    #[test]
    fn unchanged_pass_sends_nothing_and_modified_row_is_sent_once() {
        let db = Db::memory().unwrap();
        let (_, gift_id, _) = populate(&db);
        assert_eq!(push_checked(&db, &COLLECTIONS).len(), 6);
        assert!(push_checked(&db, &COLLECTIONS).is_empty());
        // Rien noté une fois tout envoyé
        let left: i64 = db.conn().query_row("SELECT COUNT(*) FROM sync_dirty", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
        // Cadeau modifié : lui seul, une fois
        let mut g = gift("Obsidian Sails 2", None);
        g.id = gift_id;
        g.group_id = db.gifts().unwrap()[0].group_id;
        db.save_gift(&g).unwrap();
        let changes = push_checked(&db, &COLLECTIONS);
        assert_eq!(sent(&changes), Sent::from([("gifts".into(), sync_id(&db, "gifts", gift_id), false)]));
        assert!(push_checked(&db, &COLLECTIONS).is_empty());
        // Favori (pas envoyé) : noté, comparé, rien ne part
        db.set_gift_favorite(gift_id, true).unwrap();
        assert!(push_checked(&db, &COLLECTIONS).is_empty());
        // Modification revenue à l'état connu : rien ne part
        db.add_giveaway_channel("other", "9").unwrap();
        db.conn().execute("DELETE FROM giveaway_channels WHERE login = 'other'", []).unwrap();
        assert!(push_checked(&db, &COLLECTIONS).is_empty());
    }

    #[test]
    fn old_giveaway_known_to_the_server_still_sends_its_changes() {
        let db = Db::memory().unwrap();
        let (_, _, giveaway_id) = populate(&db);
        push_checked(&db, &COLLECTIONS);
        // Giveaway détecté automatiquement il y a 100 jours, terminé : hors de la fenêtre d'envoi
        db.conn()
            .execute("UPDATE giveaways SET manual = 0, status = 'closed', detected_at = '2020-01-01T00:00:00Z' WHERE id = ?1", [giveaway_id])
            .unwrap();
        start(&db);
        // Gagnants corrigés : la modification part (le serveur le connaît)
        db.set_giveaway_winners(giveaway_id, &[("bob".to_owned(), None)], "2020-01-02T00:00:00Z").unwrap();
        start(&db);
        let changes = push(&db, &COLLECTIONS);
        assert!(changes.iter().any(|c| c.collection == "giveaways" && c.data.as_ref().is_some_and(|d| d["winner"] == json!("bob"))));
    }

    #[test]
    fn received_changes_are_not_sent_back() {
        let a = Db::memory().unwrap();
        let b = Db::memory().unwrap();
        let (_, gift_id, giveaway_id) = populate(&a);
        let changes = push_checked(&a, &COLLECTIONS);
        assert_eq!(receive_all(&b, &changes, &COLLECTIONS), 6);
        assert!(push_checked(&b, &COLLECTIONS).is_empty());
        // Modification reçue plus tard, giveaway reçu avant le cadeau qu'il cite
        let other = a.save_gift(&gift("Golden Hull", None)).unwrap();
        a.update_giveaway(giveaway_id, &giveaway_input(vec![gift_id, other])).unwrap();
        let mut changes = push_checked(&a, &COLLECTIONS);
        changes.sort_by_key(|c| c.collection != "giveaways");
        assert_eq!(changes.len(), 2);
        assert_eq!(receive_all(&b, &changes, &COLLECTIONS), 2);
        assert_eq!(b.giveaways(10).unwrap()[0].gift_ids.len(), 2);
        assert!(push_checked(&b, &COLLECTIONS).is_empty());
        // Notre envoi qui revient : ignoré
        assert_eq!(receive_all(&a, &changes, &COLLECTIONS), 0);
        assert!(push_checked(&a, &COLLECTIONS).is_empty());
        // Suppression reçue : pas renvoyée
        a.delete_gift(other).unwrap();
        let changes = push_checked(&a, &COLLECTIONS);
        receive_all(&b, &changes, &COLLECTIONS);
        assert_eq!(b.gifts().unwrap().len(), 1);
        assert!(push_checked(&b, &COLLECTIONS).is_empty());
        // Modifié ici et là-bas : la version locale part, la reçue est laissée de côté
        b.save_streamer(&crate::db::Streamer { login: "lunarsea".into(), notes: Some("b".into()), ..Default::default() }).unwrap();
        a.save_streamer(&crate::db::Streamer { login: "lunarsea".into(), notes: Some("a".into()), ..Default::default() }).unwrap();
        let changes = push_checked(&a, &COLLECTIONS);
        assert_eq!(receive_all(&b, &changes, &COLLECTIONS), 0);
        assert_eq!(sent(&push_checked(&b, &COLLECTIONS)), Sent::from([("streamers".into(), "lunarsea".into(), false)]));
    }

    #[test]
    fn deletions_and_linked_documents_are_sent() {
        let db = Db::memory().unwrap();
        let (group, gift_id, giveaway_id) = populate(&db);
        push_checked(&db, &COLLECTIONS);
        let (gift_sync, giveaway_sync) = (sync_id(&db, "gifts", gift_id), sync_id(&db, "giveaways", giveaway_id));
        // Cadeau supprimé : sa suppression, et le giveaway qui le citait
        db.delete_gift(gift_id).unwrap();
        assert_eq!(
            sent(&push_checked(&db, &COLLECTIONS)),
            Sent::from([("gifts".into(), gift_sync, true), ("giveaways".into(), giveaway_sync.clone(), false)])
        );
        // Groupe supprimé : les cadeaux qui le citaient changent aussi
        let other = db.save_gift(&gift("Golden Hull", Some(group))).unwrap();
        push_checked(&db, &COLLECTIONS);
        db.delete_gift_group(group).unwrap();
        assert_eq!(sent(&push_checked(&db, &COLLECTIONS)).len(), 2);
        // Nouvel identifiant partagé d'un cadeau (doublon fusionné) : les giveaways qui le citent
        db.update_giveaway(giveaway_id, &giveaway_input(vec![other])).unwrap();
        push_checked(&db, &COLLECTIONS);
        db.conn().execute("UPDATE gifts SET sync_id = '0' WHERE id = ?1", [other]).unwrap();
        assert_eq!(sent(&push_checked(&db, &COLLECTIONS)).len(), 3);
        // Règle de détection retirée, chaîne retirée, giveaway supprimé
        db.set_detection_rules("bot", &[]).unwrap();
        db.conn().execute("DELETE FROM giveaway_channels", []).unwrap();
        db.delete_giveaway(giveaway_id).unwrap();
        let changes = push_checked(&db, &COLLECTIONS);
        assert_eq!(sent(&changes).len(), 3);
        assert!(changes.iter().all(|c| c.deleted));
    }

    #[test]
    fn first_pass_after_upgrade_sends_what_a_full_comparison_would() {
        for enabled in [&COLLECTIONS[..], &["gifts", "giveaways", "streamers"][..]] {
            let db = Db::memory().unwrap();
            let (_, gift_id, giveaway_id) = populate(&db);
            // Anciens giveaways (hors fenêtre) déjà connus du serveur : l'un clos, l'autre ouvert
            // puis clos ; leur état diffère de celui connu, ils partent
            for (status, sid) in [("closed", "old-closed"), ("open", "old-open")] {
                db.conn()
                    .execute(
                        "INSERT INTO giveaways (channel, kind, detected_at, status, sync_id) VALUES ('lunarsea', 'command', '2020-01-01T00:00:00Z', ?1, ?2)",
                        params![status, sid],
                    )
                    .unwrap();
            }
            start(&db);
            // Synchronisé autrefois, puis modifié et supprimé sans suivi des modifications
            {
                let conn = db.conn();
                for c in enabled {
                    for (id, data) in export(&conn, c, enabled).unwrap() {
                        set_state(&conn, c, &id, Some(&hash(&data))).unwrap();
                    }
                }
                set_state(&conn, "gifts", "ghost", Some("0")).unwrap();
                set_state(&conn, "giveaways", "old-closed", Some("0")).unwrap();
                conn.execute("UPDATE gifts SET title = 'Changé' WHERE id = ?1", [gift_id]).unwrap();
                conn.execute("UPDATE giveaways SET notes = 'n' WHERE id = ?1", [giveaway_id]).unwrap();
                conn.execute("UPDATE giveaways SET status = 'closed' WHERE sync_id = 'old-open'", []).unwrap();
                conn.execute("DELETE FROM sync_dirty", []).unwrap();
                set_meta(&conn, "dirty_all", "1").unwrap();
            }
            let expected = full_outgoing(&db, enabled);
            assert!(!expected.is_empty());
            assert_eq!(sent(&push_checked(&db, enabled)), expected);
            assert!(push_checked(&db, enabled).is_empty());
        }
    }

    #[test]
    fn paused_giveaway_status_travels_without_changing_other_documents() {
        let a = Db::memory().unwrap();
        let b = Db::memory().unwrap();
        let (_, _, giveaway_id) = populate(&a);
        assert_eq!(push_checked(&a, &COLLECTIONS).len(), 6);
        for c in COLLECTIONS {
            transfer(&a, &b, c);
        }
        let id = sync_id(&a, "giveaways", giveaway_id);
        // Ouvert : pas de champ « status » (empreinte d'avant inchangée)
        assert!(docs(&a, "giveaways")[&id].get("status").is_none());
        // Fin du live : giveaway saisi à la main en pause, noté par les déclencheurs et envoyé seul
        assert_eq!(a.close_channel_giveaways("lunarsea").unwrap().paused, 1);
        let changes = push_checked(&a, &COLLECTIONS);
        assert_eq!(sent(&changes), Sent::from([("giveaways".into(), id.clone(), false)]));
        let data = changes[0].data.clone().unwrap();
        assert_eq!((data["status"].as_str(), data["open"].as_bool()), (Some("paused"), Some(false)));
        // Reçu : en pause de l'autre côté, sans aller dans la file d'alertes avant son retour
        apply(&b.conn(), "giveaways", &id, Some(&data)).unwrap();
        let row = &b.giveaways(10).unwrap()[0];
        assert!(row.status == "paused" && !row.handled);
        assert_eq!(docs(&b, "giveaways")[&id], data);
        // Repris : le champ disparaît
        a.resume_channel_giveaways("lunarsea", &chrono::Utc::now().to_rfc3339()).unwrap();
        let changes = push_checked(&a, &COLLECTIONS);
        let data = changes[0].data.clone().unwrap();
        assert!(data.get("status").is_none() && data["open"].as_bool() == Some(true));
        apply(&b.conn(), "giveaways", &id, Some(&data)).unwrap();
        assert_eq!(b.giveaways(10).unwrap()[0].status, "open");
        // Version plus ancienne (sans « status ») : giveaway clos
        let mut old = data.clone();
        old["open"] = json!(false);
        apply(&b.conn(), "giveaways", &id, Some(&old)).unwrap();
        assert_eq!(b.giveaways(10).unwrap()[0].status, "closed");
    }

    #[test]
    fn local_counts_match_export() {
        let db = Db::memory().unwrap();
        populate(&db);
        db.conn()
            .execute("INSERT INTO giveaways (channel, kind, detected_at, status, sync_id) VALUES ('a', 'command', '2020-01-01T00:00:00Z', 'closed', 'x')", [])
            .unwrap();
        // Ancien giveaway en pause : toujours d'actualité, envoyé et compté
        db.conn()
            .execute("INSERT INTO giveaways (channel, kind, detected_at, status, sync_id) VALUES ('a', 'command', '2020-01-01T00:00:00Z', 'paused', 'y')", [])
            .unwrap();
        start(&db);
        let conn = db.conn();
        for c in COLLECTIONS {
            assert_eq!(count(&conn, c).unwrap(), export(&conn, c, &COLLECTIONS).unwrap().len() as i64, "{c}");
        }
    }

    #[test]
    fn instance_id_is_drawn_once() {
        let db = Db::memory().unwrap();
        let id = instance_id(&db.conn()).unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(instance_id(&db.conn()).unwrap(), id);
        // Changement de serveur ou relecture complète : l'identifiant reste
        set_meta(&db.conn(), "server", "https://autre.exemple").unwrap();
        db.conn().execute("DELETE FROM sync_state", []).unwrap();
        assert_eq!(instance_id(&db.conn()).unwrap(), id);
        assert_ne!(instance_id(&Db::memory().unwrap().conn()).unwrap(), id);
        assert_eq!(default_instance_name(Some("alice".into())), "PC alice");
        assert!(!default_instance_name(None).is_empty());
    }

    #[test]
    fn box_art_resized_for_the_phone() {
        let game = |art: Option<&str>| crate::drops::Game { id: "263490".into(), name: "Rust".into(), slug: None, box_art: art.map(Into::into) };
        assert_eq!(phone_box_art(&game(Some("https://x/rust-40x56.jpg"))), "https://x/rust-80x112.jpg");
        assert_eq!(phone_box_art(&game(Some("https://x/rust-{width}x{height}.jpg"))), "https://x/rust-80x112.jpg");
        assert_eq!(phone_box_art(&game(None)), "https://static-cdn.jtvnw.net/ttv-boxart/263490-80x112.jpg");
    }

    #[test]
    fn viewers_rounded_for_the_phone() {
        assert_eq!([round_viewers(7), round_viewers(99), round_viewers(123), round_viewers(1_287), round_viewers(45_678)], [7, 99, 120, 1_300, 46_000]);
    }

    #[test]
    fn instance_state_lists_our_participations_and_live_channels() {
        use crate::giveaways::{GiveawayChannelView, GiveawayEntryView, GiveawayView, GiveawaysView};
        let db = Db::memory().unwrap();
        let (_, _, giveaway) = populate(&db);
        assign_ids(&db.conn()).unwrap();
        let ids = open_sync_ids(&db.conn()).unwrap();
        let shared = sync_id(&db, "giveaways", giveaway);
        assert_eq!(ids.get(&giveaway), Some(&shared));
        let row = db.giveaways(10).unwrap().into_iter().find(|g| g.id == giveaway).unwrap();
        let entry = |login: &str| GiveawayEntryView { account_id: 1, login: login.into(), at: String::new(), auto: false, tickets: None };
        let view = GiveawaysView {
            giveaways: vec![GiveawayView { row, evidence: None, entries_view: vec![entry("bob"), entry("alice"), entry("bob")], winner_ours: None, win_reply: None, win_reply_delay: None }],
            channels: vec![GiveawayChannelView {
                id: 1,
                login: "lunarsea".into(),
                reader: None,
                live_checked: true,
                live: Some(crate::twitch::gql::StreamInfo {
                    broadcast_id: "1".into(),
                    title: "Tirage ce soir".into(),
                    game_id: None,
                    game_name: Some("Sea of Thieves".into()),
                    viewers: 42,
                }),
                in_category: true,
                profile_image: None,
            }],
            ..Default::default()
        };
        let state = instance_state(&view, &ids, &["alice".into(), "bob".into()]);
        assert_eq!(state["accounts"], json!(["alice", "bob"]));
        assert_eq!(state["giveaways"][&shared], json!({ "participated": ["alice", "bob"], "handled": true, "hidden": false }));
        assert_eq!(state["live"]["lunarsea"], json!({ "live": true, "viewers": 42, "category": "Sea of Thieves", "title": "Tirage ce soir" }));
        // Giveaway sans identifiant partagé (pas encore synchronisé) : absent
        let state = instance_state(&view, &HashMap::new(), &[]);
        assert_eq!(state["giveaways"], json!({}));
        assert_eq!(state["live"], json!({}));
    }
}
