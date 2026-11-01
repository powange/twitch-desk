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
"#];

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
            "SELECT id, account_id, login, channel_id, position, enabled, auto_claim, balance
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
            })
        })?;
        rows.collect()
    }

    pub fn add_channel(&self, account_id: i64, login: &str, channel_id: &str) -> DbResult<()> {
        let conn = self.conn();
        let next: i64 = conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM channels WHERE account_id = ?1",
            [account_id],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO channels (account_id, login, channel_id, position) VALUES (?1, ?2, ?3, ?4)",
            params![account_id, login, channel_id, next],
        )?;
        Ok(())
    }

    pub fn remove_channel(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM channels WHERE id = ?1", [id])?;
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

    pub fn set_channel_options(&self, id: i64, enabled: bool, auto_claim: bool) -> DbResult<()> {
        self.conn().execute(
            "UPDATE channels SET enabled = ?2, auto_claim = ?3 WHERE id = ?1",
            params![id, enabled, auto_claim],
        )?;
        Ok(())
    }

    pub fn set_balance(&self, account_id: i64, login: &str, balance: i64) -> DbResult<()> {
        self.conn().execute(
            "UPDATE channels SET balance = ?3 WHERE account_id = ?1 AND login = ?2",
            params![account_id, login, balance],
        )?;
        Ok(())
    }

    // --- Gains ---

    pub fn record_gain(&self, account_id: i64, login: &str, amount: i64, reason: &str) -> DbResult<()> {
        self.conn().execute(
            "INSERT INTO points_events (account_id, channel_login, amount, reason) VALUES (?1, ?2, ?3, ?4)",
            params![account_id, login, amount, reason],
        )?;
        Ok(())
    }

    /// Gains par chaîne depuis `since` (date UTC au format RFC 3339)
    pub fn gains_since(&self, account_id: i64, since: &str) -> DbResult<Vec<(String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT channel_login, SUM(amount) FROM points_events
             WHERE account_id = ?1 AND at >= ?2 GROUP BY channel_login",
        )?;
        let rows = stmt.query_map(params![account_id, since], |r| Ok((r.get(0)?, r.get(1)?)))?;
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

        db.add_channel(a, "lunarsea", "11").unwrap();
        db.add_channel(a, "kraken_tv", "12").unwrap();
        db.add_channel(a, "lunarsea", "11").unwrap(); // doublon ignoré
        let channels = db.channels(a).unwrap();
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].login, "lunarsea");

        db.reorder_channels(a, &[channels[1].id, channels[0].id]).unwrap();
        assert_eq!(db.channels(a).unwrap()[0].login, "kraken_tv");

        db.record_gain(a, "kraken_tv", 50, "CLAIM").unwrap();
        db.record_gain(a, "kraken_tv", 10, "WATCH").unwrap();
        assert_eq!(db.gains_since(a, "2000-01-01T00:00:00Z").unwrap(), vec![("kraken_tv".into(), 60)]);

        db.delete_account(a).unwrap();
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
