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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::app::App;
use crate::giveaways::MonitorCmd;

/// Collections, dans l'ordre où les appliquer (groupes avant cadeaux, cadeaux avant giveaways)
pub const COLLECTIONS: [&str; 6] = ["groups", "gifts", "giveaways", "channels", "streamers", "detection"];
const EVERY: Duration = Duration::from_secs(30);
/// Après une modification locale : petit délai pour regrouper les envois
const DEBOUNCE: Duration = Duration::from_secs(2);
const CODE_KEY: &str = "sync:code";
/// Giveaways envoyés : ouverts, ou détectés depuis moins de 60 jours
const GIVEAWAY_DAYS: i64 = 60;
const PUSH_BATCH_DOCS: usize = 100;
/// Version de `sync_state` (2 : empreinte de ce qui est enregistré après réception)
const STATE_FORMAT: &str = "2";
const PUSH_BATCH_BYTES: usize = 8 * 1024 * 1024;

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
}

#[derive(Default)]
pub struct SyncHandle {
    view: Mutex<SyncView>,
    wake: tokio::sync::Notify,
}

impl SyncHandle {
    pub fn view(&self) -> SyncView {
        self.view.lock().unwrap().clone()
    }

    /// Données modifiées ou réglages changés : synchroniser bientôt
    pub fn poke(&self) {
        self.wake.notify_one();
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

/// JSON aux clés triées : deux contenus identiques ont la même empreinte
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter().map(|(k, v)| (k.clone(), canonical(v))).collect::<BTreeMap<_, _>>().into_iter().collect(),
        ),
        Value::Array(list) => Value::Array(list.iter().map(canonical).collect()),
        v => v.clone(),
    }
}

/// Empreinte FNV-1a 64 bits du JSON canonique (stable d'une version à l'autre)
fn hash(value: &Value) -> String {
    let text = canonical(value).to_string();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
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
    conn.prepare("SELECT kind, value FROM detection_rules")?
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
    let mut docs = export_all(conn, collection, None)?;
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

/// `only` : un seul document (sans limite de date pour les giveaways)
/// Un document tel qu'il est enregistré localement (mêmes règles que `export`)
fn export_one(conn: &Connection, collection: &str, id: &str, enabled: &[&str]) -> rusqlite::Result<Option<Value>> {
    let mut doc = export_all(conn, collection, Some(id))?.into_iter().next().map(|(_, v)| v);
    if let Some(d) = doc.as_mut() {
        strip_links(collection, enabled, d);
    }
    Ok(doc)
}

fn export_all(conn: &Connection, collection: &str, only: Option<&str>) -> rusqlite::Result<Vec<(String, Value)>> {
    match collection {
        "groups" => conn
            .prepare("SELECT sync_id, name, game_id, game_name FROM gift_groups WHERE sync_id IS NOT NULL AND (?1 IS NULL OR sync_id = ?1)")?
            .query_map([only], |r| {
                Ok((r.get(0)?, json!({ "name": r.get::<_, String>(1)?, "gameId": r.get::<_, Option<String>>(2)?, "gameName": r.get::<_, Option<String>>(3)? })))
            })?
            .collect(),
        "gifts" => conn
            .prepare(
                "SELECT g.sync_id, g.title, g.image, gr.sync_id, g.keywords, g.game_id, g.game_name
                 FROM gifts g LEFT JOIN gift_groups gr ON gr.id = g.group_id
                 WHERE g.sync_id IS NOT NULL AND (?1 IS NULL OR g.sync_id = ?1)",
            )?
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
            let since = (chrono::Utc::now() - chrono::Duration::days(GIVEAWAY_DAYS)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let mut gifts = conn.prepare(
                "SELECT gi.sync_id FROM giveaway_gifts gg JOIN gifts gi ON gi.id = gg.gift_id
                 WHERE gg.giveaway_id = ?1 AND gi.sync_id IS NOT NULL ORDER BY gi.sync_id",
            )?;
            let rows: Vec<(i64, String, Value)> = conn
                .prepare(
                    "SELECT id, sync_id, channel, kind, command, url, detected_at, status, manual, winner, winner_chance, drawn_at,
                            ticket_unique, ticket_max, reward_name, reward_cost, draw_at,
                            require_follow, require_presence, require_discord, notes,
                            require_greeting, min_chat_minutes, follow_min_days
                     FROM giveaways WHERE sync_id IS NOT NULL
                         AND (?2 IS NULL OR sync_id = ?2) AND (?2 IS NOT NULL OR status = 'open' OR detected_at >= ?1)",
                )?
                .query_map(params![since, only], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
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
                            "followMinDays": r.get::<_, Option<i64>>(23)?,
                        }),
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            rows.into_iter()
                .map(|(id, sync_id, mut data)| {
                    let list: Vec<String> = gifts.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                    data["gifts"] = json!(list);
                    Ok((sync_id, data))
                })
                .collect()
        }
        "channels" => conn
            .prepare("SELECT login, channel_id FROM giveaway_channels WHERE ?1 IS NULL OR login = ?1")?
            .query_map([only], |r| Ok((r.get(0)?, json!({ "channelId": r.get::<_, String>(1)? }))))?
            .collect(),
        "streamers" => conn
            .prepare(
                "SELECT login, discord_url, notes, ignored_commands, require_follow, follow_min_days, require_presence,
                        require_discord, require_greeting, min_chat_minutes
                 FROM streamers WHERE ?1 IS NULL OR login = ?1",
            )?
            .query_map([only], |r| {
                let ignored: Vec<String> = serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default();
                Ok((
                    r.get(0)?,
                    json!({
                        "discordUrl": r.get::<_, Option<String>>(1)?,
                        "notes": r.get::<_, Option<String>>(2)?,
                        "ignoredCommands": ignored,
                        "conditions": {
                            "requireFollow": r.get::<_, Option<bool>>(4)?,
                            "followMinDays": r.get::<_, Option<i64>>(5)?,
                            "requirePresence": r.get::<_, Option<bool>>(6)?,
                            "requireDiscord": r.get::<_, Option<bool>>(7)?,
                            "requireGreeting": r.get::<_, Option<bool>>(8)?,
                            "minChatMinutes": r.get::<_, Option<i64>>(9)?,
                        },
                    }),
                ))
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
        // Même giveaway ouvert repéré par deux applications : même chaîne, même type, même commande ou lien
        "giveaways" if bool_of(data, "open") == Some(true) => conn
            .query_row(
                "SELECT id, sync_id FROM giveaways WHERE status = 'open' AND channel = ?1 AND kind = ?2
                     AND command IS ?3 AND url IS ?4 AND sync_id != ?5",
                params![str_of(data, "channel"), str_of(data, "kind"), str_of(data, "command"), str_of(data, "url"), id],
                row,
            )
            .optional(),
        _ => Ok(None),
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
            let keywords = serde_json::to_string(&strings_of(data, "keywords")).expect("liste sérialisable");
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
            let open = bool_of(data, "open").unwrap_or(false);
            let status = if open { "open" } else { "closed" };
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
                int_of(data, "followMinDays"),
            ];
            if local_id(conn, "giveaways", id)?.is_some() {
                conn.execute(
                    "UPDATE giveaways SET channel = ?2, kind = ?3, command = ?4, url = ?5, detected_at = ?6, status = ?7, manual = ?8,
                         winner = ?9, winner_chance = ?10, drawn_at = ?11, ticket_unique = ?12, ticket_max = ?13,
                         reward_name = ?14, reward_cost = ?15, draw_at = ?16, require_follow = ?17, require_presence = ?18,
                         require_discord = ?19, notes = ?20, require_greeting = ?21, min_chat_minutes = ?22, follow_min_days = ?23,
                         reminded = CASE WHEN draw_at IS ?16 THEN reminded ELSE 0 END
                     WHERE sync_id = ?1",
                    values,
                )?;
            } else {
                // Un giveaway ouvert reçu arrive dans la file d'alertes ; un giveaway clos n'y va pas
                conn.execute(
                    "INSERT INTO giveaways (sync_id, channel, kind, command, url, detected_at, status, manual, winner, winner_chance,
                         drawn_at, ticket_unique, ticket_max, reward_name, reward_cost, draw_at, require_follow, require_presence,
                         require_discord, notes, require_greeting, min_chat_minutes, follow_min_days, handled)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                         ?21, ?22, ?23, ?7 != 'open')",
                    values,
                )?;
            }
            let giveaway = local_id(conn, "giveaways", id)?.expect("giveaway enregistré");
            // Sans lien envoyé (cadeaux non synchronisés) : les cadeaux locaux restent
            if data.get("gifts").is_none() {
                return Ok(true);
            }
            conn.execute("DELETE FROM giveaway_gifts WHERE giveaway_id = ?1", [giveaway])?;
            for gift in strings_of(data, "gifts") {
                if let Some(gift) = local_id(conn, "gifts", &gift)? {
                    conn.execute("INSERT OR IGNORE INTO giveaway_gifts (giveaway_id, gift_id) VALUES (?1, ?2)", params![giveaway, gift])?;
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
                    "UPDATE streamers SET require_follow = ?2, follow_min_days = ?3, require_presence = ?4, require_discord = ?5,
                         require_greeting = ?6, min_chat_minutes = ?7 WHERE login = ?1",
                    params![
                        id,
                        bool_of(c, "requireFollow"),
                        int_of(c, "followMinDays"),
                        bool_of(c, "requirePresence"),
                        bool_of(c, "requireDiscord"),
                        bool_of(c, "requireGreeting"),
                        int_of(c, "minChatMinutes"),
                    ],
                )?;
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

async fn request<T: serde::de::DeserializeOwned>(req: reqwest::RequestBuilder, code: &str) -> Result<T, String> {
    let res = req
        .bearer_auth(code)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| if e.is_timeout() { "Le serveur ne répond pas".to_owned() } else { format!("Serveur injoignable : {e}") })?;
    match res.status() {
        reqwest::StatusCode::UNAUTHORIZED => return Err("Code refusé par le serveur".into()),
        reqwest::StatusCode::NOT_FOUND => return Err("Ce n'est pas un serveur de synchronisation Twitch Desk".into()),
        s if !s.is_success() => {
            let body: Value = res.json().await.unwrap_or_default();
            return Err(str_of(&body, "error").unwrap_or_else(|| format!("Erreur du serveur ({s})")));
        }
        _ => {}
    }
    res.json::<T>().await.map_err(|_| "Ce n'est pas un serveur de synchronisation Twitch Desk".to_owned())
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

async fn run(app: &App, cfg: &Config) -> Result<Outcome, String> {
    let info = server_info(&app.http, &cfg.url, &cfg.code).await?;
    let skipped = skipped(&app.db.conn());
    let offered: Vec<&str> = COLLECTIONS.iter().copied().filter(|c| info.collections.iter().any(|x| x == c)).collect();
    let enabled: Vec<&str> = offered.iter().copied().filter(|c| !skipped.iter().any(|s| s == c)).collect();

    let mut cursor: i64;
    {
        let conn = app.db.conn();
        assign_ids(&conn).map_err(db_err)?;
        cursor = meta(&conn, "cursor").and_then(|c| c.parse().ok()).unwrap_or(0);
        // Autre serveur, ou serveur remis à zéro : tout est renvoyé et relu
        if meta(&conn, "server").as_deref() != Some(cfg.url.as_str()) || info.seq < cursor {
            conn.execute("DELETE FROM sync_state", []).map_err(db_err)?;
            cursor = 0;
            set_meta(&conn, "server", &cfg.url).map_err(db_err)?;
            set_meta(&conn, "cursor", "0").map_err(db_err)?;
        }
        // État enregistré par une version qui gardait l'empreinte du document reçu (et non de
        // ce qui était enregistré) : certains documents pouvaient passer pour modifiés ici et
        // ne plus recevoir de mises à jour. Une fois : état oublié, tout est relu, le serveur fait foi
        if meta(&conn, "state_format").as_deref() != Some(STATE_FORMAT) {
            conn.execute("DELETE FROM sync_state", []).map_err(db_err)?;
            cursor = 0;
            set_meta(&conn, "cursor", "0").map_err(db_err)?;
            set_meta(&conn, "state_format", STATE_FORMAT).map_err(db_err)?;
        }
        // Collections synchronisées changées : les modifications d'une collection ignorée
        // jusque-là ont été sautées, tout est relu (les documents déjà à jour sont ignorés)
        let scope = enabled.join(",");
        if meta(&conn, "scope").as_deref() != Some(scope.as_str()) {
            cursor = 0;
            set_meta(&conn, "cursor", "0").map_err(db_err)?;
            set_meta(&conn, "scope", &scope).map_err(db_err)?;
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
            let conn = app.db.conn();
            // État local au moment de la page : une modification locale pas encore envoyée l'emporte
            let mut local: HashMap<&str, HashMap<String, String>> = HashMap::new();
            let mut states: HashMap<&str, HashMap<String, String>> = HashMap::new();
            for c in &enabled {
                local.insert(c, export(&conn, c, &enabled).map_err(db_err)?.into_iter().map(|(id, v)| (id, hash(&v))).collect());
                states.insert(c, state(&conn, c).map_err(db_err)?);
            }
            for change in &page.changes {
                let Some(collection) = enabled.iter().copied().find(|c| *c == change.collection) else { continue };
                let data = if change.deleted { None } else { change.data.as_ref() };
                let remote = data.map(hash);
                let known = states[collection].get(&change.id);
                // Déjà dans cet état (notre propre envoi qui revient)
                if remote.as_ref() == known || (remote.is_none() && known.is_none() && !exists(&conn, collection, &change.id).map_err(db_err)?) {
                    continue;
                }
                let mine = local[collection].get(&change.id);
                if known.is_some() && mine.is_some() && mine != known {
                    tracing::info!("synchronisation : {collection}/{} modifié ici, version reçue laissée de côté", change.id);
                    continue;
                }
                let pending = match data {
                    Some(d) => unresolved(&conn, collection, d).map_err(db_err)?,
                    None => false,
                };
                if apply(&conn, collection, &change.id, data).map_err(db_err)? {
                    if pending {
                        deferred.push((collection, change.id.clone(), data.cloned().unwrap_or_default()));
                    } else {
                        // Empreinte de ce qui est réellement enregistré : l'enregistrement peut
                        // différer un peu du document reçu (espaces, image refusée, lien absent)
                        let stored = match data {
                            Some(_) => export_one(&conn, collection, &change.id, &enabled).map_err(db_err)?.map(|v| hash(&v)),
                            None => None,
                        };
                        set_state(&conn, collection, &change.id, stored.as_deref()).map_err(db_err)?;
                    }
                    received += 1;
                    touched.insert(collection);
                }
            }
            // Le curseur n'avance pas tant que des liens restent à résoudre : en cas
            // d'interruption, ces documents seront relus
            cursor = page.seq;
            if deferred.is_empty() {
                set_meta(&conn, "cursor", &cursor.to_string()).map_err(db_err)?;
            }
        }
        if !page.more {
            break;
        }
    }
    if !deferred.is_empty() {
        let conn = app.db.conn();
        for (collection, id, data) in &deferred {
            if apply(&conn, collection, id, Some(data)).map_err(db_err)? {
                let stored = export_one(&conn, collection, id, &enabled).map_err(db_err)?.map(|v| hash(&v));
                set_state(&conn, collection, id, stored.as_deref()).map_err(db_err)?;
            }
        }
        set_meta(&conn, "cursor", &cursor.to_string()).map_err(db_err)?;
    }

    // 2. Nos modifications : contenus changés, puis suppressions
    let author = app.db.accounts().ok().and_then(|a| a.into_iter().next()).map(|a| a.login);
    let mut outgoing: Vec<(OutgoingChange, Option<String>)> = vec![];
    {
        let conn = app.db.conn();
        for c in &enabled {
            let known = state(&conn, c).map_err(db_err)?;
            for (id, data) in export(&conn, c, &enabled).map_err(db_err)? {
                let h = hash(&data);
                if known.get(&id) != Some(&h) {
                    outgoing.push((OutgoingChange { collection: c, id, data: Some(data), deleted: false }, Some(h)));
                }
            }
            for id in known.keys() {
                if !exists(&conn, c, id).map_err(db_err)? {
                    outgoing.push((OutgoingChange { collection: c, id: id.clone(), data: None, deleted: true }, None));
                }
            }
        }
    }
    let sent = outgoing.len() as u64;
    let mut batch: Vec<(OutgoingChange, Option<String>)> = vec![];
    let mut size = 0;
    let mut pending = outgoing.into_iter().peekable();
    while let Some(item) = pending.next() {
        size += item.0.data.as_ref().map_or(64, |d| d.to_string().len());
        batch.push(item);
        if batch.len() >= PUSH_BATCH_DOCS || size >= PUSH_BATCH_BYTES || pending.peek().is_none() {
            let changes: Vec<&OutgoingChange> = batch.iter().map(|(c, _)| c).collect();
            let _: Value =
                request(app.http.post(format!("{}/api/v1/changes", cfg.url)).json(&json!({ "author": author, "changes": changes })), &cfg.code)
                    .await?;
            let conn = app.db.conn();
            for (c, h) in batch.drain(..) {
                set_state(&conn, c.collection, &c.id, h.as_deref()).map_err(db_err)?;
            }
            size = 0;
        }
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
            local_counts.insert(c.to_string(), export(&conn, c, &enabled).map_err(db_err)?.len() as i64);
        }
    }
    let server_counts = server_info(&app.http, &cfg.url, &cfg.code).await.ok().and_then(|i| i.counts);
    Ok(Outcome {
        server_collections: offered.iter().map(|c| c.to_string()).collect(),
        collections: enabled.iter().map(|c| c.to_string()).collect(),
        local_counts,
        server_counts,
        sent,
        received,
    })
}

pub fn spawn(app: Arc<App>) {
    tauri::async_runtime::spawn(async move {
        loop {
            let Some(cfg) = config(&app) else {
                {
                    let mut v = app.sync.view.lock().unwrap();
                    let url = meta(&app.db.conn(), "url").unwrap_or_default();
                    if v.status != "off" || v.url != url {
                        *v = SyncView { status: "off".into(), url, revision: v.revision, ..SyncView::default() };
                        drop(v);
                        app.touch();
                    }
                }
                app.sync.wake.notified().await;
                tokio::time::sleep(DEBOUNCE).await;
                continue;
            };
            {
                let mut v = app.sync.view.lock().unwrap();
                v.enabled = true;
                v.url = cfg.url.clone();
                v.status = "syncing".into();
            }
            app.touch();
            let result = run(&app, &cfg).await;
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
            app.touch();
            tokio::select! {
                _ = tokio::time::sleep(EVERY) => {}
                _ = app.sync.wake.notified() => tokio::time::sleep(DEBOUNCE).await,
            }
        }
    });
}

// --- Commandes ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfigView {
    url: String,
    enabled: bool,
    has_code: bool,
}

#[tauri::command]
pub fn get_sync_config(app: State<'_, Arc<App>>) -> SyncConfigView {
    let conn = app.db.conn();
    SyncConfigView {
        url: meta(&conn, "url").unwrap_or_default(),
        enabled: meta(&conn, "enabled").as_deref() == Some("1"),
        has_code: app.secrets.load_raw(CODE_KEY).is_some_and(|c| !c.is_empty()),
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
    app.touch();
    app.sync.poke();
    Ok(())
}

#[tauri::command]
pub fn sync_now(app: State<'_, Arc<App>>) {
    app.sync.poke();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

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
            details: Default::default(),
        };
        a.insert_manual_giveaway(&input, "2026-11-01T10:00:00Z").unwrap();
        a.add_giveaway_channel("lunarsea", "123").unwrap();
        a.save_streamer(&crate::db::Streamer { login: "lunarsea".into(), discord_url: Some("https://discord.gg/x".into()), notes: None, ignored_commands: vec!["!points".into()], ..Default::default() })
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
}
