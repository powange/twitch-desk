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
"#, r#"
    -- Drops : module activable par compte, et jeux souhaités par ordre de priorité.
    -- Un jeu peut être ajouté sans campagne en cours : il est surveillé jusqu'à la suivante.
    ALTER TABLE accounts ADD COLUMN drops_enabled INTEGER NOT NULL DEFAULT 1;
    CREATE TABLE drop_games (
        id INTEGER PRIMARY KEY,
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        game_id TEXT NOT NULL,
        name TEXT NOT NULL,
        slug TEXT NOT NULL,
        position INTEGER NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 1,
        UNIQUE (account_id, game_id)
    );
"#, r#"
    -- Jaquette de la catégorie Twitch
    ALTER TABLE drop_games ADD COLUMN box_art TEXT;
"#, r#"
    -- Giveaways : chaînes surveillées (liste globale), giveaways détectés, participations
    CREATE TABLE giveaway_channels (
        id INTEGER PRIMARY KEY,
        login TEXT NOT NULL UNIQUE,
        channel_id TEXT NOT NULL,
        position INTEGER NOT NULL,
        excluded TEXT NOT NULL DEFAULT '[]'
    );
    CREATE TABLE giveaways (
        id INTEGER PRIMARY KEY,
        channel TEXT NOT NULL,
        kind TEXT NOT NULL,
        command TEXT,
        url TEXT,
        evidence TEXT,
        detected_at TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'open',
        handled INTEGER NOT NULL DEFAULT 0,
        winner TEXT,
        winner_chance TEXT,
        drawn_at TEXT
    );
    CREATE INDEX giveaways_channel ON giveaways (channel, status);
    CREATE TABLE giveaway_entries (
        giveaway_id INTEGER NOT NULL REFERENCES giveaways(id) ON DELETE CASCADE,
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        at TEXT NOT NULL,
        auto INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (giveaway_id, account_id)
    );
"#, r#"
    -- Catégories où surveiller les giveaways (vide : toutes)
    CREATE TABLE giveaway_games (
        id INTEGER PRIMARY KEY,
        game_id TEXT NOT NULL UNIQUE,
        name TEXT NOT NULL,
        slug TEXT NOT NULL,
        box_art TEXT,
        position INTEGER NOT NULL
    );
"#, r#"
    -- Fiche giveaway complète (comme le site Giveaway Twitch) ; conditions : NULL = on ne sait pas
    ALTER TABLE giveaways ADD COLUMN ticket_unique INTEGER;
    ALTER TABLE giveaways ADD COLUMN ticket_max INTEGER;
    ALTER TABLE giveaways ADD COLUMN reward_name TEXT;
    ALTER TABLE giveaways ADD COLUMN reward_cost INTEGER;
    ALTER TABLE giveaways ADD COLUMN draw_at TEXT;
    ALTER TABLE giveaways ADD COLUMN require_follow INTEGER;
    ALTER TABLE giveaways ADD COLUMN require_presence INTEGER;
    ALTER TABLE giveaways ADD COLUMN require_discord INTEGER;
    ALTER TABLE giveaways ADD COLUMN notes TEXT;
    ALTER TABLE giveaways ADD COLUMN manual INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE giveaways ADD COLUMN reminded INTEGER NOT NULL DEFAULT 0;
"#];

/// Types de giveaway
pub const GIVEAWAY_KINDS: &[&str] = &["command", "ticket", "streamelements", "channelpoints"];

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
    pub drops_enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayGame {
    pub id: i64,
    pub game_id: String,
    pub name: String,
    pub slug: String,
    pub box_art: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayChannel {
    pub id: i64,
    pub login: String,
    pub channel_id: String,
    /// Commandes ignorées par la détection sur cette chaîne
    pub excluded: Vec<String>,
}

pub struct NewGiveaway {
    pub channel: String,
    /// « command » (détecté dans le chat) ou « streamelements » (lien du bot)
    pub kind: String,
    pub command: Option<String>,
    pub url: Option<String>,
    pub evidence: Option<String>,
    pub detected_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayEntry {
    pub account_id: i64,
    pub at: String,
    pub auto: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayRow {
    pub id: i64,
    pub channel: String,
    pub kind: String,
    pub command: Option<String>,
    pub url: Option<String>,
    /// Ce qui a déclenché l'alerte (JSON)
    #[serde(skip)]
    pub evidence: Option<String>,
    pub detected_at: String,
    pub open: bool,
    /// Participé ou ignoré : sort de la file d'alertes
    pub handled: bool,
    pub winner: Option<String>,
    pub winner_chance: Option<String>,
    pub drawn_at: Option<String>,
    pub entries: Vec<GiveawayEntry>,
    #[serde(flatten)]
    pub details: GiveawayDetails,
    /// Créé à la main (pas détecté dans le chat)
    pub manual: bool,
    /// Rappel « présence au tirage » déjà envoyé
    #[serde(skip)]
    pub reminded: bool,
}

/// Informations d'un giveaway au-delà de sa détection (renseignées à la main ou par les bots)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GiveawayDetails {
    /// Type ticket : commande à usage unique (1 ticket par personne) ou multi-tickets
    pub ticket_unique: Option<bool>,
    /// Type ticket multi : tickets maximum par personne
    pub ticket_max: Option<i64>,
    /// Type points de chaîne : récompense à acheter et son coût
    pub reward_name: Option<String>,
    pub reward_cost: Option<i64>,
    /// Heure du tirage (RFC 3339)
    pub draw_at: Option<String>,
    pub require_follow: Option<bool>,
    /// Présent dans le chat au moment du tirage
    pub require_presence: Option<bool>,
    pub require_discord: Option<bool>,
    pub notes: Option<String>,
}

/// Fiche saisie à la main (création ou modification)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayInput {
    pub channel: String,
    pub kind: String,
    pub command: Option<String>,
    pub url: Option<String>,
    #[serde(flatten)]
    pub details: GiveawayDetails,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DropGame {
    pub id: i64,
    pub game_id: String,
    pub name: String,
    pub slug: String,
    pub enabled: bool,
    pub box_art: Option<String>,
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
    /// Notifications Windows (giveaway détecté, mention, victoire)
    pub notifications: bool,
    /// Participation automatique aux giveaways détectés (désactivée par défaut)
    pub auto_participate: bool,
    /// Délai aléatoire avant la participation automatique, en secondes
    pub auto_delay_min: u64,
    pub auto_delay_max: u64,
    /// Comptes qui participent automatiquement (vide : tous)
    pub auto_accounts: Vec<i64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            start_minimized: false,
            auto_update: true,
            notifications: true,
            auto_participate: false,
            auto_delay_min: 8,
            auto_delay_max: 45,
            auto_accounts: vec![],
        }
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
        let mut stmt = conn.prepare("SELECT id, user_id, login, device_id, points_enabled, drops_enabled FROM accounts ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Account {
                id: r.get(0)?,
                user_id: r.get(1)?,
                login: r.get(2)?,
                device_id: r.get(3)?,
                points_enabled: r.get(4)?,
                drops_enabled: r.get(5)?,
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

    pub fn set_drops_enabled(&self, id: i64, enabled: bool) -> DbResult<()> {
        self.conn().execute("UPDATE accounts SET drops_enabled = ?2 WHERE id = ?1", params![id, enabled])?;
        Ok(())
    }

    // --- Jeux souhaités pour les drops ---

    pub fn drop_games(&self, account_id: i64) -> DbResult<Vec<DropGame>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, game_id, name, slug, enabled, box_art FROM drop_games WHERE account_id = ?1 ORDER BY position, id",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            Ok(DropGame {
                id: r.get(0)?,
                game_id: r.get(1)?,
                name: r.get(2)?,
                slug: r.get(3)?,
                enabled: r.get(4)?,
                box_art: r.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_drop_game(
        &self,
        account_id: i64,
        game_id: &str,
        name: &str,
        slug: &str,
        box_art: Option<&str>,
    ) -> DbResult<()> {
        let conn = self.conn();
        let next: i64 = conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM drop_games WHERE account_id = ?1",
            [account_id],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO drop_games (account_id, game_id, name, slug, position, box_art)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![account_id, game_id, name, slug, next, box_art],
        )?;
        Ok(())
    }

    /// Jaquette d'un jeu (tous comptes confondus), pour les jeux ajoutés sans
    pub fn set_drop_game_box_art(&self, game_id: &str, box_art: &str) -> DbResult<()> {
        self.conn().execute(
            "UPDATE drop_games SET box_art = ?2 WHERE game_id = ?1 AND box_art IS NULL",
            params![game_id, box_art],
        )?;
        Ok(())
    }

    pub fn remove_drop_game(&self, id: i64) -> DbResult<Option<i64>> {
        let conn = self.conn();
        let account: Option<i64> =
            conn.query_row("SELECT account_id FROM drop_games WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        conn.execute("DELETE FROM drop_games WHERE id = ?1", [id])?;
        Ok(account)
    }

    pub fn set_drop_game_enabled(&self, id: i64, enabled: bool) -> DbResult<Option<i64>> {
        let conn = self.conn();
        conn.execute("UPDATE drop_games SET enabled = ?2 WHERE id = ?1", params![id, enabled])?;
        conn.query_row("SELECT account_id FROM drop_games WHERE id = ?1", [id], |r| r.get(0)).optional()
    }

    pub fn reorder_drop_games(&self, account_id: i64, ids: &[i64]) -> DbResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (position, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE drop_games SET position = ?1 WHERE id = ?2 AND account_id = ?3",
                params![position as i64, id, account_id],
            )?;
        }
        tx.commit()
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

    // --- Giveaways ---

    pub fn giveaway_channels(&self) -> DbResult<Vec<GiveawayChannel>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, login, channel_id, excluded FROM giveaway_channels ORDER BY position, id")?;
        let rows = stmt.query_map([], |r| {
            let excluded: String = r.get(3)?;
            Ok(GiveawayChannel {
                id: r.get(0)?,
                login: r.get(1)?,
                channel_id: r.get(2)?,
                excluded: serde_json::from_str(&excluded).unwrap_or_default(),
            })
        })?;
        rows.collect()
    }

    pub fn add_giveaway_channel(&self, login: &str, channel_id: &str) -> DbResult<()> {
        let conn = self.conn();
        let next: i64 =
            conn.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM giveaway_channels", [], |r| r.get(0))?;
        conn.execute(
            "INSERT OR IGNORE INTO giveaway_channels (login, channel_id, position) VALUES (?1, ?2, ?3)",
            params![login, channel_id, next],
        )?;
        Ok(())
    }

    pub fn remove_giveaway_channel(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM giveaway_channels WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_giveaway_excluded(&self, id: i64, excluded: &[String]) -> DbResult<()> {
        let value = serde_json::to_string(excluded).expect("liste sérialisable");
        self.conn().execute("UPDATE giveaway_channels SET excluded = ?2 WHERE id = ?1", params![id, value])?;
        Ok(())
    }

    pub fn giveaway_games(&self) -> DbResult<Vec<GiveawayGame>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, game_id, name, slug, box_art FROM giveaway_games ORDER BY position, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(GiveawayGame { id: r.get(0)?, game_id: r.get(1)?, name: r.get(2)?, slug: r.get(3)?, box_art: r.get(4)? })
        })?;
        rows.collect()
    }

    pub fn add_giveaway_game(&self, game_id: &str, name: &str, slug: &str, box_art: Option<&str>) -> DbResult<()> {
        let conn = self.conn();
        let next: i64 = conn.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM giveaway_games", [], |r| r.get(0))?;
        conn.execute(
            "INSERT OR IGNORE INTO giveaway_games (game_id, name, slug, box_art, position) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![game_id, name, slug, box_art, next],
        )?;
        Ok(())
    }

    pub fn remove_giveaway_game(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM giveaway_games WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Clôt les giveaways ouverts d'une chaîne ; renvoie le nombre clos
    pub fn close_channel_giveaways(&self, channel: &str) -> DbResult<usize> {
        self.conn().execute("UPDATE giveaways SET status = 'closed' WHERE status = 'open' AND channel = ?1", [channel])
    }

    pub fn insert_giveaway(&self, g: &NewGiveaway) -> DbResult<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO giveaways (channel, kind, command, url, evidence, detected_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![g.channel, g.kind, g.command, g.url, g.evidence, g.detected_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Giveaways récents (ouverts d'abord), avec leurs participations
    pub fn giveaways(&self, limit: usize) -> DbResult<Vec<GiveawayRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, channel, kind, command, url, evidence, detected_at, status, handled, winner, winner_chance, drawn_at,
                    ticket_unique, ticket_max, reward_name, reward_cost, draw_at,
                    require_follow, require_presence, require_discord, notes, manual, reminded
             FROM giveaways ORDER BY status = 'open' DESC, detected_at DESC LIMIT ?1",
        )?;
        let mut rows: Vec<GiveawayRow> = stmt
            .query_map([limit as i64], |r| {
                Ok(GiveawayRow {
                    id: r.get(0)?,
                    channel: r.get(1)?,
                    kind: r.get(2)?,
                    command: r.get(3)?,
                    url: r.get(4)?,
                    evidence: r.get(5)?,
                    detected_at: r.get(6)?,
                    open: r.get::<_, String>(7)? == "open",
                    handled: r.get(8)?,
                    winner: r.get(9)?,
                    winner_chance: r.get(10)?,
                    drawn_at: r.get(11)?,
                    entries: vec![],
                    details: GiveawayDetails {
                        ticket_unique: r.get(12)?,
                        ticket_max: r.get(13)?,
                        reward_name: r.get(14)?,
                        reward_cost: r.get(15)?,
                        draw_at: r.get(16)?,
                        require_follow: r.get(17)?,
                        require_presence: r.get(18)?,
                        require_discord: r.get(19)?,
                        notes: r.get(20)?,
                    },
                    manual: r.get(21)?,
                    reminded: r.get(22)?,
                })
            })?
            .collect::<DbResult<_>>()?;
        let mut stmt = conn.prepare("SELECT account_id, at, auto FROM giveaway_entries WHERE giveaway_id = ?1 ORDER BY at")?;
        for g in &mut rows {
            g.entries = stmt
                .query_map([g.id], |r| Ok(GiveawayEntry { account_id: r.get(0)?, at: r.get(1)?, auto: r.get(2)? }))?
                .collect::<DbResult<_>>()?;
        }
        Ok(rows)
    }

    /// Giveaway créé à la main ; renvoie son identifiant
    pub fn insert_manual_giveaway(&self, g: &GiveawayInput, at: &str) -> DbResult<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO giveaways (channel, kind, command, url, detected_at, manual, handled) VALUES (?1, ?2, ?3, ?4, ?5, 1, 1)",
            params![g.channel, g.kind, g.command, g.url, at],
        )?;
        let id = conn.last_insert_rowid();
        drop(conn);
        self.update_giveaway(id, g)?;
        Ok(id)
    }

    pub fn update_giveaway(&self, id: i64, g: &GiveawayInput) -> DbResult<()> {
        let d = &g.details;
        self.conn().execute(
            "UPDATE giveaways SET channel = ?2, kind = ?3, command = ?4, url = ?5, ticket_unique = ?6, ticket_max = ?7,
                 reward_name = ?8, reward_cost = ?9, draw_at = ?10, require_follow = ?11, require_presence = ?12,
                 require_discord = ?13, notes = ?14,
                 reminded = CASE WHEN draw_at IS ?10 THEN reminded ELSE 0 END
             WHERE id = ?1",
            params![
                id,
                g.channel,
                g.kind,
                g.command,
                g.url,
                d.ticket_unique,
                d.ticket_max,
                d.reward_name,
                d.reward_cost,
                d.draw_at,
                d.require_follow,
                d.require_presence,
                d.require_discord,
                d.notes
            ],
        )?;
        Ok(())
    }

    /// Maximum de tickets annoncé par le bot, s'il n'est pas déjà renseigné
    pub fn set_ticket_max_if_unknown(&self, id: i64, max: i64) -> DbResult<bool> {
        let n = self.conn().execute(
            "UPDATE giveaways SET ticket_max = ?2, ticket_unique = CASE WHEN ?2 = 1 THEN 1 ELSE 0 END
             WHERE id = ?1 AND ticket_max IS NULL",
            params![id, max],
        )?;
        Ok(n > 0)
    }

    pub fn set_giveaway_reminded(&self, id: i64) -> DbResult<()> {
        self.conn().execute("UPDATE giveaways SET reminded = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn giveaway(&self, id: i64) -> DbResult<Option<GiveawayRow>> {
        Ok(self.giveaways(500)?.into_iter().find(|g| g.id == id))
    }

    pub fn add_giveaway_entry(&self, giveaway_id: i64, account_id: i64, at: &str, auto: bool) -> DbResult<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO giveaway_entries (giveaway_id, account_id, at, auto) VALUES (?1, ?2, ?3, ?4)",
            params![giveaway_id, account_id, at, auto],
        )?;
        conn.execute("UPDATE giveaways SET handled = 1 WHERE id = ?1", [giveaway_id])?;
        Ok(())
    }

    pub fn set_giveaway_handled(&self, id: i64) -> DbResult<()> {
        self.conn().execute("UPDATE giveaways SET handled = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn close_giveaway(&self, id: i64) -> DbResult<()> {
        self.conn().execute("UPDATE giveaways SET status = 'closed', handled = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_giveaway_winner(&self, id: i64, winner: &str, chance: Option<&str>, at: &str) -> DbResult<()> {
        self.conn().execute(
            "UPDATE giveaways SET winner = ?2, winner_chance = ?3, drawn_at = ?4, status = 'closed' WHERE id = ?1",
            params![id, winner, chance, at],
        )?;
        Ok(())
    }

    /// Clôt les giveaways ouverts détectés avant `before` (date RFC 3339)
    pub fn close_giveaways_before(&self, before: &str) -> DbResult<usize> {
        self.conn().execute("UPDATE giveaways SET status = 'closed' WHERE status = 'open' AND detected_at < ?1", [before])
    }

    pub fn delete_giveaway(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM giveaways WHERE id = ?1", [id])?;
        Ok(())
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
    fn giveaways() {
        let db = Db::memory().unwrap();
        let a = db.upsert_account("33", "powange", "dev").unwrap();
        db.add_giveaway_channel("lunarsea", "11").unwrap();
        db.add_giveaway_channel("lunarsea", "11").unwrap();
        let ch = db.giveaway_channels().unwrap();
        assert_eq!(ch.len(), 1);
        db.set_giveaway_excluded(ch[0].id, &["!points".into()]).unwrap();
        assert_eq!(db.giveaway_channels().unwrap()[0].excluded, vec!["!points"]);

        let new = |at: &str| NewGiveaway {
            channel: "lunarsea".into(),
            kind: "command".into(),
            command: Some("!coffre".into()),
            url: None,
            evidence: None,
            detected_at: at.into(),
        };
        let old = db.insert_giveaway(&new("2026-10-30T10:00:00Z")).unwrap();
        let g = db.insert_giveaway(&new("2026-11-01T10:00:00Z")).unwrap();
        db.add_giveaway_entry(g, a, "2026-11-01T10:00:30Z", true).unwrap();
        db.add_giveaway_entry(g, a, "2026-11-01T10:00:40Z", false).unwrap(); // une seule participation par compte
        let row = db.giveaway(g).unwrap().unwrap();
        assert!(row.handled && row.open);
        assert_eq!(row.entries.len(), 1);
        assert!(row.entries[0].auto);

        assert_eq!(db.close_giveaways_before("2026-11-01T00:00:00Z").unwrap(), 1);
        assert!(!db.giveaway(old).unwrap().unwrap().open);
        let input = GiveawayInput {
            channel: "lunarsea".into(),
            kind: "ticket".into(),
            command: Some("!ticket".into()),
            url: None,
            details: GiveawayDetails {
                ticket_unique: Some(false),
                draw_at: Some("2026-11-01T21:00:00Z".into()),
                require_presence: Some(true),
                ..Default::default()
            },
        };
        db.update_giveaway(g, &input).unwrap();
        assert!(db.set_ticket_max_if_unknown(g, 5).unwrap());
        assert!(!db.set_ticket_max_if_unknown(g, 9).unwrap());
        let row = db.giveaway(g).unwrap().unwrap();
        assert_eq!((row.kind.as_str(), row.details.ticket_max, row.details.require_presence), ("ticket", Some(5), Some(true)));
        db.set_giveaway_reminded(g).unwrap();
        assert!(db.giveaway(g).unwrap().unwrap().reminded);
        let manual = db.insert_manual_giveaway(&input, "2026-11-01T12:00:00Z").unwrap();
        let row = db.giveaway(manual).unwrap().unwrap();
        assert!(row.manual && row.handled && !row.reminded);
        assert_eq!(row.details.draw_at.as_deref(), Some("2026-11-01T21:00:00Z"));
        db.set_giveaway_winner(g, "powange", Some("3%"), "2026-11-01T11:00:00Z").unwrap();
        let row = db.giveaway(g).unwrap().unwrap();
        assert_eq!((row.winner.as_deref(), row.open), (Some("powange"), false));
    }

    #[test]
    fn drop_games() {
        let db = Db::memory().unwrap();
        let a = db.upsert_account("33", "powange", "dev").unwrap();
        assert!(db.accounts().unwrap()[0].drops_enabled);
        db.add_drop_game(a, "490377", "Sea of Thieves", "sea-of-thieves", None).unwrap();
        db.add_drop_game(a, "464339927", "ARC Raiders", "arc-raiders", Some("https://x/arc.jpg")).unwrap();
        db.add_drop_game(a, "490377", "Sea of Thieves", "sea-of-thieves", None).unwrap(); // doublon ignoré
        let games = db.drop_games(a).unwrap();
        assert_eq!(games.len(), 2);
        db.reorder_drop_games(a, &[games[1].id, games[0].id]).unwrap();
        assert_eq!(db.drop_games(a).unwrap()[0].name, "ARC Raiders");
        db.set_drop_game_box_art("490377", "https://x/sot.jpg").unwrap();
        assert_eq!(db.drop_games(a).unwrap()[1].box_art.as_deref(), Some("https://x/sot.jpg"));
        assert_eq!(db.set_drop_game_enabled(games[0].id, false).unwrap(), Some(a));
        assert_eq!(db.remove_drop_game(games[0].id).unwrap(), Some(a));
        assert_eq!(db.drop_games(a).unwrap().len(), 1);
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
