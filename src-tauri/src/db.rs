//! Base locale SQLite : comptes, chaînes suivies, gains de points, réglages.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const MIGRATIONS: &[&str] = &[r#"
    CREATE TABLE accounts (
        id INTEGER PRIMARY KEY,
        user_id TEXT NOT NULL UNIQUE,
        login TEXT NOT NULL,
        device_id TEXT NOT NULL,
        points_enabled INTEGER NOT NULL DEFAULT 1,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
    );
    CREATE TABLE channels (
        id INTEGER PRIMARY KEY,
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        login TEXT NOT NULL,
        channel_id TEXT NOT NULL,
        position INTEGER NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 1,
        auto_claim INTEGER NOT NULL DEFAULT 1,
        balance INTEGER,
        UNIQUE (account_id, login)
    );
    CREATE TABLE points_events (
        id INTEGER PRIMARY KEY,
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        channel_login TEXT NOT NULL,
        at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        amount INTEGER NOT NULL,
        reason TEXT NOT NULL
    );
    CREATE INDEX points_events_at ON points_events (account_id, at);
    CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
"#, r#"
    -- v0.2 : présence dans le chat et points StreamElements
    ALTER TABLE channels ADD COLUMN chat_presence INTEGER NOT NULL DEFAULT 1;
    -- NULL : pas encore cherché ; '' : la chaîne n'utilise pas StreamElements
    ALTER TABLE channels ADD COLUMN se_channel_id TEXT;
    ALTER TABLE channels ADD COLUMN se_points INTEGER;
    ALTER TABLE points_events ADD COLUMN currency TEXT NOT NULL DEFAULT 'twitch';
"#, r#"
    -- Deux listes par compte : points de chaîne Twitch et StreamElements.
    -- Une chaîne peut être dans l'une, l'autre ou les deux.
    ALTER TABLE channels ADD COLUMN in_points INTEGER NOT NULL DEFAULT 1;
    ALTER TABLE channels ADD COLUMN in_se INTEGER NOT NULL DEFAULT 0;
    UPDATE channels SET in_se = 1 WHERE se_channel_id IS NOT NULL AND se_channel_id != '';
"#, r#"
    -- Nos tickets par giveaway StreamElements, lus dans les réponses du bot
    CREATE TABLE se_tickets (
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        giveaway_id TEXT NOT NULL,
        tickets INTEGER NOT NULL,
        PRIMARY KEY (account_id, giveaway_id)
    );
"#, r#"
    -- Relevé du solde StreamElements, en minutes (NULL : automatique, d'après le rythme de la chaîne)
    ALTER TABLE channels ADD COLUMN se_refresh INTEGER;
"#];

/// Liste d'une chaîne : une page de l'interface
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelList {
    Points,
    Se,
}

impl ChannelList {
    fn column(self) -> &'static str {
        match self {
            ChannelList::Points => "in_points",
            ChannelList::Se => "in_se",
        }
    }
}

pub const TWITCH: &str = "twitch";
pub const STREAMELEMENTS: &str = "se";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: i64,
    pub user_id: String,
    pub login: String,
    #[serde(skip)]
    pub device_id: String,
    pub points_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: i64,
    pub account_id: i64,
    pub login: String,
    pub channel_id: String,
    pub position: i64,
    pub enabled: bool,
    pub auto_claim: bool,
    pub balance: Option<i64>,
    pub chat_presence: bool,
    pub se_channel_id: Option<String>,
    pub se_points: Option<i64>,
    pub in_points: bool,
    pub in_se: bool,
    /// Relevé du solde StreamElements en minutes (None : automatique)
    pub se_refresh: Option<i64>,
}

impl Channel {
    /// Temps de visionnage à simuler (liste Points, récolte activée)
    pub fn farms_points(&self) -> bool {
        self.in_points && self.enabled
    }

    /// Présence voulue dans le chat (liste StreamElements, chat activé)
    pub fn wants_chat(&self) -> bool {
        self.in_se && self.chat_presence
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Fermer la fenêtre la cache dans la zone de notification au lieu de quitter
    pub close_to_tray: bool,
    /// Démarrer avec la fenêtre cachée
    pub start_minimized: bool,
    /// Chercher et proposer les mises à jour automatiquement
    pub auto_update: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { close_to_tray: true, start_minimized: false, auto_update: true }
    }
}

pub struct Db(Mutex<Connection>);

type DbResult<T> = rusqlite::Result<T>;

impl Db {
    pub fn open(path: &Path) -> DbResult<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn memory() -> DbResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> DbResult<Self> {
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            conn.execute_batch(sql)?;
            conn.execute_batch(&format!("PRAGMA user_version = {}", i + 1))?;
        }
        Ok(Self(Mutex::new(conn)))
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().unwrap()
    }

    // --- Comptes ---

    pub fn accounts(&self) -> DbResult<Vec<Account>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, user_id, login, device_id, points_enabled FROM accounts ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Account {
                id: r.get(0)?,
                user_id: r.get(1)?,
                login: r.get(2)?,
                device_id: r.get(3)?,
                points_enabled: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Ajoute le compte, ou met à jour son login s'il est déjà connu (reconnexion)
    pub fn upsert_account(&self, user_id: &str, login: &str, device_id: &str) -> DbResult<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO accounts (user_id, login, device_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(user_id) DO UPDATE SET login = excluded.login",
            params![user_id, login, device_id],
        )?;
        conn.query_row("SELECT id FROM accounts WHERE user_id = ?1", [user_id], |r| r.get(0))
    }

    pub fn delete_account(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_points_enabled(&self, id: i64, enabled: bool) -> DbResult<()> {
        self.conn().execute("UPDATE accounts SET points_enabled = ?2 WHERE id = ?1", params![id, enabled])?;
        Ok(())
    }

    // --- Chaînes ---

    pub fn channels(&self, account_id: i64) -> DbResult<Vec<Channel>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, login, channel_id, position, enabled, auto_claim, balance,
                    chat_presence, se_channel_id, se_points, in_points, in_se, se_refresh
             FROM channels WHERE account_id = ?1 ORDER BY position, id",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            Ok(Channel {
                id: r.get(0)?,
                account_id: r.get(1)?,
                login: r.get(2)?,
                channel_id: r.get(3)?,
                position: r.get(4)?,
                enabled: r.get(5)?,
                auto_claim: r.get(6)?,
                balance: r.get(7)?,
                chat_presence: r.get(8)?,
                se_channel_id: r.get(9)?,
                se_points: r.get(10)?,
                in_points: r.get(11)?,
                in_se: r.get(12)?,
                se_refresh: r.get(13)?,
            })
        })?;
        rows.collect()
    }

    /// Ajoute la chaîne à une liste (la ligne existe peut-être déjà pour l'autre liste)
    pub fn add_channel(&self, account_id: i64, login: &str, channel_id: &str, list: ChannelList) -> DbResult<()> {
        let conn = self.conn();
        let next: i64 = conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM channels WHERE account_id = ?1",
            [account_id],
            |r| r.get(0),
        )?;
        let (in_points, in_se) = (list == ChannelList::Points, list == ChannelList::Se);
        let column = list.column();
        conn.execute(
            &format!(
                "INSERT INTO channels (account_id, login, channel_id, position, in_points, in_se)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, login) DO UPDATE SET {column} = 1"
            ),
            params![account_id, login, channel_id, next, in_points, in_se],
        )?;
        Ok(())
    }

    /// Retire la chaîne d'une liste ; la ligne disparaît quand elle n'est plus dans aucune
    pub fn remove_channel(&self, id: i64, list: ChannelList) -> DbResult<()> {
        let conn = self.conn();
        conn.execute(&format!("UPDATE channels SET {} = 0 WHERE id = ?1", list.column()), [id])?;
        conn.execute("DELETE FROM channels WHERE id = ?1 AND in_points = 0 AND in_se = 0", [id])?;
        Ok(())
    }

    pub fn channel_account(&self, id: i64) -> DbResult<Option<i64>> {
        self.conn()
            .query_row("SELECT account_id FROM channels WHERE id = ?1", [id], |r| r.get(0))
            .optional()
    }

    /// Nouvel ordre des chaînes d'un compte (identifiants dans l'ordre voulu)
    pub fn reorder_channels(&self, account_id: i64, ids: &[i64]) -> DbResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (position, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE channels SET position = ?1 WHERE id = ?2 AND account_id = ?3",
                params![position as i64, id, account_id],
            )?;
        }
        tx.commit()
    }

    pub fn set_channel_options(&self, id: i64, enabled: bool, auto_claim: bool, chat_presence: bool) -> DbResult<()> {
        self.conn().execute(
            "UPDATE channels SET enabled = ?2, auto_claim = ?3, chat_presence = ?4 WHERE id = ?1",
            params![id, enabled, auto_claim, chat_presence],
        )?;
        Ok(())
    }

    pub fn set_se_refresh(&self, id: i64, minutes: Option<i64>) -> DbResult<()> {
        self.conn().execute("UPDATE channels SET se_refresh = ?2 WHERE id = ?1", params![id, minutes])?;
        Ok(())
    }

    pub fn set_se_channel(&self, id: i64, se_channel_id: &str) -> DbResult<()> {
        self.conn().execute("UPDATE channels SET se_channel_id = ?2 WHERE id = ?1", params![id, se_channel_id])?;
        Ok(())
    }

    pub fn set_se_points(&self, id: i64, points: i64) -> DbResult<()> {
        self.conn().execute("UPDATE channels SET se_points = ?2 WHERE id = ?1", params![id, points])?;
        Ok(())
    }

    pub fn set_balance(&self, account_id: i64, login: &str, balance: i64) -> DbResult<()> {
        self.conn().execute(
            "UPDATE channels SET balance = ?3 WHERE account_id = ?1 AND login = ?2",
            params![account_id, login, balance],
        )?;
        Ok(())
    }

    // --- Tickets StreamElements ---

    pub fn se_tickets(&self, account_id: i64, giveaway_id: &str) -> Option<i64> {
        self.conn()
            .query_row(
                "SELECT tickets FROM se_tickets WHERE account_id = ?1 AND giveaway_id = ?2",
                params![account_id, giveaway_id],
                |r| r.get(0),
            )
            .ok()
    }

    pub fn set_se_tickets(&self, account_id: i64, giveaway_id: &str, tickets: i64) -> DbResult<()> {
        self.conn().execute(
            "INSERT INTO se_tickets (account_id, giveaway_id, tickets) VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id, giveaway_id) DO UPDATE SET tickets = excluded.tickets",
            params![account_id, giveaway_id, tickets],
        )?;
        Ok(())
    }

    // --- Gains ---

    pub fn record_gain(&self, account_id: i64, login: &str, currency: &str, amount: i64, reason: &str) -> DbResult<()> {
        self.conn().execute(
            "INSERT INTO points_events (account_id, channel_login, currency, amount, reason) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![account_id, login, currency, amount, reason],
        )?;
        Ok(())
    }

    /// Gains par chaîne dans une monnaie depuis `since` (date UTC au format RFC 3339)
    pub fn gains_since(&self, account_id: i64, currency: &str, since: &str) -> DbResult<Vec<(String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT channel_login, SUM(amount) FROM points_events
             WHERE account_id = ?1 AND currency = ?2 AND at >= ?3 GROUP BY channel_login",
        )?;
        let rows = stmt.query_map(params![account_id, currency, since], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    // --- Réglages ---

    pub fn settings(&self) -> Settings {
        self.conn()
            .query_row("SELECT value FROM settings WHERE key = 'app'", [], |r| r.get::<_, String>(0))
            .ok()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, settings: &Settings) -> DbResult<()> {
        let value = serde_json::to_string(settings).expect("réglages sérialisables");
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES ('app', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_and_channels() {
        let db = Db::memory().unwrap();
        let a = db.upsert_account("33", "powange", "dev").unwrap();
        assert_eq!(db.upsert_account("33", "powange_renamed", "autre").unwrap(), a);
        assert_eq!(db.accounts().unwrap()[0].login, "powange_renamed");

        db.add_channel(a, "lunarsea", "11", ChannelList::Points).unwrap();
        db.add_channel(a, "kraken_tv", "12", ChannelList::Points).unwrap();
        db.add_channel(a, "lunarsea", "11", ChannelList::Points).unwrap(); // doublon ignoré
        let channels = db.channels(a).unwrap();
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].login, "lunarsea");
        assert!(channels[0].in_points && !channels[0].in_se);

        db.reorder_channels(a, &[channels[1].id, channels[0].id]).unwrap();
        assert_eq!(db.channels(a).unwrap()[0].login, "kraken_tv");

        db.record_gain(a, "kraken_tv", TWITCH, 50, "CLAIM").unwrap();
        db.record_gain(a, "kraken_tv", TWITCH, 10, "WATCH").unwrap();
        db.record_gain(a, "kraken_tv", STREAMELEMENTS, 100, "WATCH").unwrap();
        assert_eq!(db.gains_since(a, TWITCH, "2000-01-01T00:00:00Z").unwrap(), vec![("kraken_tv".into(), 60)]);
        assert_eq!(db.gains_since(a, STREAMELEMENTS, "2000-01-01T00:00:00Z").unwrap(), vec![("kraken_tv".into(), 100)]);

        let ch = db.channels(a).unwrap()[0].clone();
        assert!(!ch.wants_chat());
        assert_eq!(ch.se_channel_id, None);
        db.set_se_channel(ch.id, "600abc").unwrap();
        db.set_se_points(ch.id, 1500).unwrap();
        let ch = db.channels(a).unwrap()[0].clone();
        assert_eq!((ch.se_channel_id.as_deref(), ch.se_points), (Some("600abc"), Some(1500)));

        db.delete_account(a).unwrap();
        assert!(db.channels(a).unwrap().is_empty());
    }

    #[test]
    fn se_tickets() {
        let db = Db::memory().unwrap();
        let a = db.upsert_account("33", "powange", "dev").unwrap();
        assert_eq!(db.se_tickets(a, "g1"), None);
        db.set_se_tickets(a, "g1", 3).unwrap();
        db.set_se_tickets(a, "g1", 8).unwrap();
        assert_eq!(db.se_tickets(a, "g1"), Some(8));
    }

    #[test]
    fn two_lists() {
        let db = Db::memory().unwrap();
        let a = db.upsert_account("33", "powange", "dev").unwrap();
        db.add_channel(a, "spar", "1", ChannelList::Se).unwrap();
        let ch = db.channels(a).unwrap()[0].clone();
        assert!(ch.wants_chat() && !ch.farms_points());

        // Ajoutée aussi aux points : même ligne, deux listes
        db.add_channel(a, "spar", "1", ChannelList::Points).unwrap();
        let ch = db.channels(a).unwrap()[0].clone();
        assert!(ch.wants_chat() && ch.farms_points());

        // Retirée des points : reste dans StreamElements
        db.remove_channel(ch.id, ChannelList::Points).unwrap();
        let ch = db.channels(a).unwrap()[0].clone();
        assert!(ch.in_se && !ch.in_points);

        db.remove_channel(ch.id, ChannelList::Se).unwrap();
        assert!(db.channels(a).unwrap().is_empty());
    }

    #[test]
    fn settings_default_and_roundtrip() {
        let db = Db::memory().unwrap();
        assert!(db.settings().close_to_tray);
        db.save_settings(&Settings { close_to_tray: false, ..Settings::default() }).unwrap();
        assert!(!db.settings().close_to_tray);
    }
}
