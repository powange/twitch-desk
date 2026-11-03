//! Base locale SQLite : comptes, chaînes suivies, gains de points, réglages.

use std::collections::HashMap;
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
"#, r#"
    -- Catalogue de cadeaux (comme le site Giveaway Twitch) et suivi de nos gains
    CREATE TABLE gift_groups (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL UNIQUE
    );
    CREATE TABLE gifts (
        id INTEGER PRIMARY KEY,
        title TEXT NOT NULL,
        -- Image en data URL, réduite à l'ajout
        image TEXT,
        group_id INTEGER REFERENCES gift_groups(id) ON DELETE SET NULL,
        keywords TEXT NOT NULL DEFAULT '[]'
    );
    CREATE TABLE giveaway_gifts (
        giveaway_id INTEGER NOT NULL REFERENCES giveaways(id) ON DELETE CASCADE,
        gift_id INTEGER NOT NULL REFERENCES gifts(id) ON DELETE CASCADE,
        PRIMARY KEY (giveaway_id, gift_id)
    );
    -- Lot d'un giveaway gagné : reçu ou non, et note (clé, code…)
    ALTER TABLE giveaways ADD COLUMN prize_received INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE giveaways ADD COLUMN prize_note TEXT;
"#, r#"
    -- Cadeau rattaché à une catégorie Twitch (jeu) ; groupes propres à une catégorie.
    -- Table des groupes reconstruite (nom unique par catégorie) sans toucher aux cadeaux.
    PRAGMA foreign_keys = OFF;
    ALTER TABLE gifts ADD COLUMN game_id TEXT;
    ALTER TABLE gifts ADD COLUMN game_name TEXT;
    CREATE TABLE gift_groups_new (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        game_id TEXT
    );
    INSERT INTO gift_groups_new (id, name, game_id) SELECT id, name, NULL FROM gift_groups;
    DROP TABLE gift_groups;
    ALTER TABLE gift_groups_new RENAME TO gift_groups;
    CREATE UNIQUE INDEX gift_groups_name ON gift_groups (IFNULL(game_id, ''), name);
    PRAGMA foreign_keys = ON;
"#, r#"
    -- Informations propres au streamer (et non à un giveaway), retrouvées à chaque giveaway
    CREATE TABLE streamers (
        login TEXT PRIMARY KEY,
        discord_url TEXT,
        notes TEXT
    );
"#, r#"
    -- Nom de la catégorie d'un groupe, pour l'afficher même sans cadeau
    ALTER TABLE gift_groups ADD COLUMN game_name TEXT;
    UPDATE gift_groups SET game_name = (SELECT game_name FROM gifts WHERE gifts.game_id = gift_groups.game_id AND game_name IS NOT NULL LIMIT 1)
        WHERE game_id IS NOT NULL;
"#, r#"
    -- Synchronisation : identifiant partagé entre les applications, dernier état envoyé ou reçu
    ALTER TABLE gift_groups ADD COLUMN sync_id TEXT;
    ALTER TABLE gifts ADD COLUMN sync_id TEXT;
    ALTER TABLE giveaways ADD COLUMN sync_id TEXT;
    CREATE UNIQUE INDEX gift_groups_sync ON gift_groups (sync_id);
    CREATE UNIQUE INDEX gifts_sync ON gifts (sync_id);
    CREATE UNIQUE INDEX giveaways_sync ON giveaways (sync_id);
    CREATE TABLE sync_state (
        collection TEXT NOT NULL,
        id TEXT NOT NULL,
        hash TEXT NOT NULL,
        PRIMARY KEY (collection, id)
    );
    CREATE TABLE sync_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
"#, r#"
    -- Fil d'événements du tableau de bord, gardé d'un lancement à l'autre
    CREATE TABLE events (
        id INTEGER PRIMARY KEY,
        at TEXT NOT NULL,
        account TEXT NOT NULL,
        kind TEXT NOT NULL,
        message TEXT NOT NULL
    );
"#, r#"
    -- Points de chaîne : solde à atteindre, puis place à la chaîne suivante
    ALTER TABLE channels ADD COLUMN max_balance INTEGER;
"#, r#"
    -- Commandes ignorées : sur la fiche du streamer (et non plus sur la chaîne surveillée)
    ALTER TABLE streamers ADD COLUMN ignored_commands TEXT NOT NULL DEFAULT '[]';
    INSERT INTO streamers (login, ignored_commands)
        SELECT login, excluded FROM giveaway_channels WHERE excluded != '[]'
        ON CONFLICT(login) DO UPDATE SET ignored_commands = excluded.ignored_commands;
"#, r#"
    -- Cadeaux recherchés (favoris), propres à ce poste
    ALTER TABLE gifts ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
"#, r#"
    -- Journal du chat des chaînes suivies : commandes, nos messages, annonces de gagnant
    CREATE TABLE chat_log (
        id INTEGER PRIMARY KEY,
        channel TEXT NOT NULL,
        at TEXT NOT NULL,
        author TEXT NOT NULL,
        kind TEXT NOT NULL,
        command TEXT,
        message TEXT NOT NULL
    );
    CREATE INDEX chat_log_channel ON chat_log (channel, id);
    CREATE INDEX chat_log_at ON chat_log (at);
    -- Réponse d'un bot à une commande d'information (!giveaway…), par chaîne
    CREATE TABLE bot_replies (
        channel TEXT NOT NULL,
        command TEXT NOT NULL,
        bot TEXT NOT NULL,
        message TEXT NOT NULL,
        asked_by TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        seen_at TEXT NOT NULL,
        PRIMARY KEY (channel, command)
    );
"#, r#"
    -- Conditions : avoir dit bonjour, temps de présence dans le chat, ancienneté du follow
    ALTER TABLE giveaways ADD COLUMN require_greeting INTEGER;
    ALTER TABLE giveaways ADD COLUMN min_chat_minutes INTEGER;
    ALTER TABLE giveaways ADD COLUMN follow_min_days INTEGER;
"#, r#"
    -- État de l'interface gardé d'un lancement à l'autre (fenêtre des streams…)
    CREATE TABLE app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
"#, r#"
    -- Mentions de nos pseudos en attente dans la file d'alertes (gardées d'un lancement à l'autre)
    CREATE TABLE mentions (
        id INTEGER PRIMARY KEY,
        at TEXT NOT NULL,
        channel TEXT NOT NULL,
        account TEXT NOT NULL,
        author TEXT NOT NULL,
        message TEXT NOT NULL
    );
"#, r#"
    -- Conditions habituelles du streamer : pré-remplissent ses giveaways
    ALTER TABLE streamers ADD COLUMN require_follow INTEGER;
    ALTER TABLE streamers ADD COLUMN follow_min_days INTEGER;
    ALTER TABLE streamers ADD COLUMN require_presence INTEGER;
    ALTER TABLE streamers ADD COLUMN require_discord INTEGER;
    ALTER TABLE streamers ADD COLUMN require_greeting INTEGER;
    ALTER TABLE streamers ADD COLUMN min_chat_minutes INTEGER;
"#, r#"
    -- Détection des gagnants ajoutée par l'utilisateur : formulations ('winner') et bots ('bot').
    -- À part des réglages : synchronisée élément par élément, jamais écrasée par une mise à jour
    CREATE TABLE detection_rules (
        kind TEXT NOT NULL,
        value TEXT NOT NULL,
        PRIMARY KEY (kind, value)
    );
"#, r#"
    -- Synchronisation : documents modifiés ici depuis le dernier passage, notés par des
    -- déclencheurs ; seuls ceux-là sont relus et comparés à `sync_state`. Numéro croissant
    -- (jamais réutilisé) : une entrée n'est effacée que si elle n'a pas changé entre-temps.
    -- Règles de détection : identifiant calculé côté Rust, toute la collection est notée ('*')
    CREATE TABLE sync_dirty (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        collection TEXT NOT NULL,
        id TEXT NOT NULL
    );
    CREATE INDEX sync_dirty_doc ON sync_dirty (collection, id);
    -- Noter un document : INSERT INTO sync_touch (sans conflit possible, quelle que soit la
    -- politique ON CONFLICT de l'instruction qui déclenche)
    CREATE VIEW sync_touch AS SELECT collection, id FROM sync_dirty;
    CREATE TRIGGER sync_touch_insert INSTEAD OF INSERT ON sync_touch WHEN NEW.id IS NOT NULL BEGIN
        DELETE FROM sync_dirty WHERE collection = NEW.collection AND id = NEW.id;
        INSERT INTO sync_dirty (collection, id) VALUES (NEW.collection, NEW.id);
    END;

    CREATE TRIGGER sync_groups_insert AFTER INSERT ON gift_groups BEGIN
        INSERT INTO sync_touch VALUES ('groups', NEW.sync_id);
    END;
    CREATE TRIGGER sync_groups_update AFTER UPDATE ON gift_groups BEGIN
        INSERT INTO sync_touch VALUES ('groups', OLD.sync_id);
        INSERT INTO sync_touch VALUES ('groups', NEW.sync_id);
    END;
    -- Les cadeaux citent l'identifiant partagé de leur groupe
    CREATE TRIGGER sync_groups_relink AFTER UPDATE OF sync_id ON gift_groups WHEN OLD.sync_id IS NOT NEW.sync_id BEGIN
        INSERT INTO sync_touch SELECT 'gifts', sync_id FROM gifts WHERE group_id = NEW.id;
    END;
    CREATE TRIGGER sync_groups_unlink BEFORE DELETE ON gift_groups BEGIN
        INSERT INTO sync_touch SELECT 'gifts', sync_id FROM gifts WHERE group_id = OLD.id;
    END;
    CREATE TRIGGER sync_groups_delete AFTER DELETE ON gift_groups BEGIN
        INSERT INTO sync_touch VALUES ('groups', OLD.sync_id);
    END;

    CREATE TRIGGER sync_gifts_insert AFTER INSERT ON gifts BEGIN
        INSERT INTO sync_touch VALUES ('gifts', NEW.sync_id);
    END;
    CREATE TRIGGER sync_gifts_update AFTER UPDATE ON gifts BEGIN
        INSERT INTO sync_touch VALUES ('gifts', OLD.sync_id);
        INSERT INTO sync_touch VALUES ('gifts', NEW.sync_id);
    END;
    -- Les giveaways citent l'identifiant partagé de leurs cadeaux
    CREATE TRIGGER sync_gifts_relink AFTER UPDATE OF sync_id ON gifts WHEN OLD.sync_id IS NOT NEW.sync_id BEGIN
        INSERT INTO sync_touch SELECT 'giveaways', g.sync_id FROM giveaways g JOIN giveaway_gifts gg ON gg.giveaway_id = g.id
            WHERE gg.gift_id = NEW.id;
    END;
    CREATE TRIGGER sync_gifts_unlink BEFORE DELETE ON gifts BEGIN
        INSERT INTO sync_touch SELECT 'giveaways', g.sync_id FROM giveaways g JOIN giveaway_gifts gg ON gg.giveaway_id = g.id
            WHERE gg.gift_id = OLD.id;
    END;
    CREATE TRIGGER sync_gifts_delete AFTER DELETE ON gifts BEGIN
        INSERT INTO sync_touch VALUES ('gifts', OLD.sync_id);
    END;

    CREATE TRIGGER sync_giveaways_insert AFTER INSERT ON giveaways BEGIN
        INSERT INTO sync_touch VALUES ('giveaways', NEW.sync_id);
    END;
    CREATE TRIGGER sync_giveaways_update AFTER UPDATE ON giveaways BEGIN
        INSERT INTO sync_touch VALUES ('giveaways', OLD.sync_id);
        INSERT INTO sync_touch VALUES ('giveaways', NEW.sync_id);
    END;
    CREATE TRIGGER sync_giveaways_delete AFTER DELETE ON giveaways BEGIN
        INSERT INTO sync_touch VALUES ('giveaways', OLD.sync_id);
    END;
    -- Cadeaux d'un giveaway : partie du document du giveaway
    CREATE TRIGGER sync_giveaway_gifts_insert AFTER INSERT ON giveaway_gifts BEGIN
        INSERT INTO sync_touch SELECT 'giveaways', sync_id FROM giveaways WHERE id = NEW.giveaway_id;
    END;
    CREATE TRIGGER sync_giveaway_gifts_update AFTER UPDATE ON giveaway_gifts BEGIN
        INSERT INTO sync_touch SELECT 'giveaways', sync_id FROM giveaways WHERE id IN (OLD.giveaway_id, NEW.giveaway_id);
    END;
    CREATE TRIGGER sync_giveaway_gifts_delete AFTER DELETE ON giveaway_gifts BEGIN
        INSERT INTO sync_touch SELECT 'giveaways', sync_id FROM giveaways WHERE id = OLD.giveaway_id;
    END;

    CREATE TRIGGER sync_channels_insert AFTER INSERT ON giveaway_channels BEGIN
        INSERT INTO sync_touch VALUES ('channels', NEW.login);
    END;
    CREATE TRIGGER sync_channels_update AFTER UPDATE ON giveaway_channels BEGIN
        INSERT INTO sync_touch VALUES ('channels', OLD.login);
        INSERT INTO sync_touch VALUES ('channels', NEW.login);
    END;
    CREATE TRIGGER sync_channels_delete AFTER DELETE ON giveaway_channels BEGIN
        INSERT INTO sync_touch VALUES ('channels', OLD.login);
    END;

    CREATE TRIGGER sync_streamers_insert AFTER INSERT ON streamers BEGIN
        INSERT INTO sync_touch VALUES ('streamers', NEW.login);
    END;
    CREATE TRIGGER sync_streamers_update AFTER UPDATE ON streamers BEGIN
        INSERT INTO sync_touch VALUES ('streamers', OLD.login);
        INSERT INTO sync_touch VALUES ('streamers', NEW.login);
    END;
    CREATE TRIGGER sync_streamers_delete AFTER DELETE ON streamers BEGIN
        INSERT INTO sync_touch VALUES ('streamers', OLD.login);
    END;

    CREATE TRIGGER sync_detection_insert AFTER INSERT ON detection_rules BEGIN
        INSERT INTO sync_touch VALUES ('detection', '*');
    END;
    CREATE TRIGGER sync_detection_update AFTER UPDATE ON detection_rules BEGIN
        INSERT INTO sync_touch VALUES ('detection', '*');
    END;
    CREATE TRIGGER sync_detection_delete AFTER DELETE ON detection_rules BEGIN
        INSERT INTO sync_touch VALUES ('detection', '*');
    END;

    -- Modifications d'avant ces déclencheurs inconnues : au premier passage, tout est comparé
    INSERT INTO sync_meta (key, value) VALUES ('dirty_all', '1')
        ON CONFLICT(key) DO UPDATE SET value = excluded.value;
"#, r#"
    -- Réponse automatique quand un de nos comptes gagne un giveaway avec présence requise :
    -- phrase écrite dans le chat et délai en secondes (propres à ce PC, non synchronisés)
    ALTER TABLE streamers ADD COLUMN win_reply TEXT;
    ALTER TABLE streamers ADD COLUMN win_reply_delay INTEGER;
"#, r#"
    -- Plusieurs gagnants par giveaway (liste JSON, dans l'ordre du tirage) ; winner garde le premier
    ALTER TABLE giveaways ADD COLUMN winners TEXT NOT NULL DEFAULT '[]';
    UPDATE giveaways SET winners = json_array(winner) WHERE winner IS NOT NULL;
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

/// Informations propres à un streamer
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Streamer {
    pub login: String,
    /// Lien d'invitation au Discord du streamer
    pub discord_url: Option<String>,
    pub notes: Option<String>,
    /// Commandes du chat que la détection de giveaway ignore sur sa chaîne
    pub ignored_commands: Vec<String>,
    /// Conditions habituelles de ses giveaways (None : on ne sait pas) ; pré-remplissent ses
    /// nouveaux giveaways, chacun gardant ensuite les siennes
    pub require_follow: Option<bool>,
    pub follow_min_days: Option<i64>,
    pub require_presence: Option<bool>,
    pub require_discord: Option<bool>,
    pub require_greeting: Option<bool>,
    pub min_chat_minutes: Option<i64>,
    /// Phrase écrite dans le chat par notre compte quand il gagne un giveaway avec présence
    /// requise (None : pas de réponse) ; propre à ce PC
    pub win_reply: Option<String>,
    /// Délai avant cette réponse, en secondes (None : WIN_REPLY_DELAY)
    pub win_reply_delay: Option<i64>,
}

/// Délai par défaut avant la réponse au gain, en secondes
pub const WIN_REPLY_DELAY: i64 = 10;

const STREAMER_SELECT: &str = "SELECT login, discord_url, notes, ignored_commands, require_follow, follow_min_days,
     require_presence, require_discord, require_greeting, min_chat_minutes, win_reply, win_reply_delay FROM streamers";

fn streamer_row(r: &rusqlite::Row) -> rusqlite::Result<Streamer> {
    Ok(Streamer {
        login: r.get(0)?,
        discord_url: r.get(1)?,
        notes: r.get(2)?,
        ignored_commands: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
        require_follow: r.get(4)?,
        follow_min_days: r.get(5)?,
        require_presence: r.get(6)?,
        require_discord: r.get(7)?,
        require_greeting: r.get(8)?,
        min_chat_minutes: r.get(9)?,
        win_reply: r.get(10)?,
        win_reply_delay: r.get(11)?,
    })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GiftGroup {
    pub id: i64,
    pub name: String,
    /// Catégorie (jeu Twitch) du groupe ; None : groupe sans catégorie
    pub game_id: Option<String>,
    pub game_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Gift {
    #[serde(default)]
    pub id: i64,
    pub title: String,
    pub image: Option<String>,
    pub group_id: Option<i64>,
    /// Mots qui, dans le titre d'un live, suggèrent ce cadeau
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Catégorie Twitch (jeu) du cadeau
    #[serde(default)]
    pub game_id: Option<String>,
    #[serde(default)]
    pub game_name: Option<String>,
    /// Cadeau recherché : propre à ce poste (jamais synchronisé ni modifié par le formulaire)
    #[serde(default)]
    pub favorite: bool,
}

/// Texte sans accents ni majuscules (repérage des cadeaux)
fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' | 'õ' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// `term` présent au début d'un mot de `text` (mots séparés par n'importe quels espaces)
pub(crate) fn starts_word(text: &str, term: &str) -> bool {
    let words: Vec<&str> = term.split_whitespace().collect();
    let Some(first) = words.first() else { return false };
    let mut from = 0;
    while let Some(pos) = text[from..].find(first) {
        let start = from + pos;
        let before_ok = text[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        if before_ok {
            // Mots suivants, séparés par des espaces
            let mut rest = &text[start + first.len()..];
            let mut ok = true;
            for w in &words[1..] {
                let trimmed = rest.trim_start();
                if trimmed.len() == rest.len() || !trimmed.starts_with(w) {
                    ok = false;
                    break;
                }
                rest = &trimmed[w.len()..];
            }
            if ok {
                return true;
            }
        }
        from = start + first.len().max(1);
    }
    false
}

/// Cadeaux du catalogue dont le titre ou un mot-clé apparaît dans le texte (même règle que
/// l'interface : début de mot, majuscules et accents ignorés)
pub fn gifts_in_text(gifts: &[Gift], text: &str) -> Vec<i64> {
    let t = fold(text);
    gifts
        .iter()
        .filter(|g| std::iter::once(&g.title).chain(g.keywords.iter()).any(|term| starts_word(&t, &fold(term.trim()))))
        .map(|g| g.id)
        .collect()
}

/// Mention de nos pseudos en attente dans la file d'alertes
pub struct StoredMention {
    pub id: i64,
    pub at: String,
    pub channel: String,
    pub account: String,
    pub author: String,
    pub message: String,
}

/// Dernière réponse d'un bot à une commande d'information sur une chaîne
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BotReply {
    pub channel: String,
    pub command: String,
    pub bot: String,
    pub message: String,
    /// Spectateur dont la commande a obtenu cette réponse
    pub asked_by: String,
    /// Dernier changement du message, et dernière fois qu'il a été vu
    pub updated_at: String,
    pub seen_at: String,
}

/// Message du journal du chat
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatLogEntry {
    #[serde(default)]
    pub id: i64,
    pub channel: String,
    pub at: String,
    pub author: String,
    /// « command » (commande d'un spectateur), « own » (un de nos comptes), « streamer » (message
    /// du streamer), « winner » (annonce de gagnant), « reply » (réponse d'un bot)
    pub kind: String,
    pub command: Option<String>,
    pub message: String,
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
    /// Statut : `open`, `paused` (live terminé, tirage pas encore fait) ou `closed`
    pub status: String,
    /// Statut `open` (gardé pour l'interface)
    pub open: bool,
    /// Participé ou ignoré : sort de la file d'alertes
    pub handled: bool,
    /// Premier gagnant (voir `winners`)
    pub winner: Option<String>,
    pub winner_chance: Option<String>,
    pub drawn_at: Option<String>,
    /// Tous les gagnants, dans l'ordre du tirage
    pub winners: Vec<String>,
    pub entries: Vec<GiveawayEntry>,
    /// Cadeaux du catalogue rattachés au giveaway
    pub gift_ids: Vec<i64>,
    pub prize_received: bool,
    pub prize_note: Option<String>,
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
    /// Avoir dit bonjour dans le chat
    pub require_greeting: Option<bool>,
    /// Temps de présence minimum dans le chat, en minutes (0 : non requis, None : on ne sait pas)
    pub min_chat_minutes: Option<i64>,
    /// Follow depuis au moins tant de jours (avec le follow requis)
    pub follow_min_days: Option<i64>,
}

/// Fiche saisie à la main (création ou modification)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayInput {
    pub channel: String,
    pub kind: String,
    pub command: Option<String>,
    pub url: Option<String>,
    #[serde(default)]
    pub gift_ids: Vec<i64>,
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
    /// Solde de points de chaîne à atteindre : au-delà, la chaîne n'est plus regardée
    pub max_balance: Option<i64>,
}

impl Channel {
    /// Solde voulu atteint : la place revient à la chaîne suivante
    pub fn balance_reached(&self) -> bool {
        self.max_balance.is_some_and(|max| self.balance.is_some_and(|b| b >= max))
    }

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
    /// Scanner : mots-clés cherchés dans les titres des lives
    pub scanner_keywords: Vec<String>,
    /// Scanner : mots-clés de la liste décochés, gardés pour être recochés plus tard
    pub scanner_keywords_off: Vec<String>,
    /// Scanner : catégories décochées (identifiants Twitch des jeux)
    pub scanner_games_off: Vec<String>,
    /// Scanner : langues retenues (vide : toutes)
    pub scanner_languages: Vec<String>,
    pub scanner_min_viewers: u64,
    /// Scanner automatique toutes les 3 min, avec notification des nouveaux streams
    pub scanner_auto: bool,
    /// Alertes sonores (jouées par la fenêtre, même cachée)
    pub sound_alerts: bool,
    /// Volume des alertes, de 0 à 100
    pub sound_volume: u64,
    /// Alertes avec un son : giveaway, win, mention, reminder, scanner
    pub sound_events: Vec<String>,
    /// Son de chaque alerte : son prédéfini, ou « custom » pour le fichier choisi (absent : son
    /// par défaut de l'alerte)
    pub sound_choices: std::collections::BTreeMap<String, String>,
    /// Nom du fichier choisi pour chaque alerte (affichage)
    pub sound_files: std::collections::BTreeMap<String, String>,
    /// Détection d'une commande de giveaway : derniers messages analysés
    pub detect_window: u64,
    /// Pourcentage de ces messages avec la même commande pour déclencher l'alerte
    pub detect_percent: u64,
    /// Messages d'un même spectateur comptés pour une commande
    pub detect_max_per_user: u64,
    /// Délai minimum entre deux alertes pour la même commande sur une chaîne, en secondes
    pub detect_cooldown_secs: u64,
    /// Compter aussi les mots-clés tapés seuls, sans « ! »
    pub detect_keywords: bool,
    /// Phrases pour dire bonjour dans le chat, par langue (code ISO 639-1)
    pub greetings: std::collections::BTreeMap<String, Vec<String>>,
    /// Langue des phrases de bonjour quand celle de la chaîne est inconnue ou sans phrases
    pub greeting_default_lang: String,
    /// Fenêtre « Streams » : bloqueur de publicités Twitch (vaft) dans les lecteurs
    pub viewer_adblock: bool,
    /// Processeur, mémoire et trafic de l'application affichés en bas à gauche (relevé arrêté sinon)
    pub show_usage: bool,
}

pub const DEFAULT_SCANNER_KEYWORDS: &[&str] =
    &["giveaway", "concours", "cadeau", "tirage", "cle", "key", "drop", "gift", "gw"];

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
            scanner_keywords: DEFAULT_SCANNER_KEYWORDS.iter().map(|k| k.to_string()).collect(),
            scanner_keywords_off: vec![],
            scanner_games_off: vec![],
            scanner_languages: vec![],
            scanner_min_viewers: 0,
            scanner_auto: false,
            sound_alerts: true,
            sound_volume: 70,
            sound_events: crate::app::Alert::ALL.iter().map(|a| a.key().to_owned()).collect(),
            sound_choices: Default::default(),
            sound_files: Default::default(),
            detect_window: 20,
            detect_percent: 35,
            detect_max_per_user: 2,
            detect_cooldown_secs: 600,
            detect_keywords: true,
            greetings: [
                ("fr", &["Bonjour tout le monde !", "Salut le chat !", "Coucou à tous !", "Hello tout le monde !"][..]),
                ("en", &["Hello everyone!", "Hi chat!", "Hey everyone!", "Hello chat!"][..]),
                ("de", &["Hallo zusammen!", "Hi Chat!", "Servus zusammen!"][..]),
                ("es", &["¡Hola a todos!", "¡Hola chat!", "¡Buenas a todos!"][..]),
            ]
            .iter()
            .map(|(lang, phrases)| (lang.to_string(), phrases.iter().map(|p| p.to_string()).collect()))
            .collect(),
            greeting_default_lang: "en".into(),
            viewer_adblock: false,
            show_usage: false,
        }
    }
}

/// Heure du tirage absente, illisible ou passée : le giveaway peut être clos automatiquement
/// (`draw_at` en RFC 3339, lu par SQLite)
const DRAW_NOT_AHEAD: &str = "COALESCE(julianday(draw_at) > julianday('now'), 0) = 0";
/// Fin du live : giveaway mis en pause (tirage plus tard ou au prochain live) plutôt que clos :
/// heure de tirage à venir, saisi à la main ou StreamElements (giveaways sur plusieurs lives)
const PAUSE_ON_OFFLINE: &str =
    "winner IS NULL AND (COALESCE(julianday(draw_at) > julianday('now'), 0) = 1 OR manual = 1 OR kind = 'streamelements')";

/// Fin du live d'une chaîne : giveaways mis en pause et clos
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OfflineOutcome {
    pub paused: usize,
    pub closed: usize,
}

pub struct Db(Mutex<Connection>);

type DbResult<T> = rusqlite::Result<T>;

impl Db {
    pub fn open(path: &Path) -> DbResult<Self> {
        let existed = path.exists();
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        // Mise à jour du schéma : copie de la base d'avant (WAL compris), au cas où
        let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if existed && version > 0 && version < MIGRATIONS.len() {
            let backup = path.with_file_name(format!(
                "{}.bak-v{version}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("twitch-desk.db")
            ));
            let _ = std::fs::remove_file(&backup);
            match conn.execute("VACUUM INTO ?1", [backup.to_string_lossy()]) {
                Ok(_) => tracing::info!("base sauvegardée avant mise à jour du schéma : {}", backup.display()),
                Err(e) => tracing::warn!("sauvegarde de la base avant mise à jour du schéma : {e}"),
            }
        }
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn memory() -> DbResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> DbResult<Self> {
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")?;
        let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            // Chaque migration et son numéro dans une transaction : tout ou rien. PRAGMA
            // foreign_keys est sans effet dans une transaction : appliqué autour
            let fk_off = sql.contains("PRAGMA foreign_keys = OFF;");
            let body = sql.replace("PRAGMA foreign_keys = OFF;", "").replace("PRAGMA foreign_keys = ON;", "");
            if fk_off {
                conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
            }
            let result = (|| {
                let tx = conn.transaction()?;
                tx.execute_batch(&body)?;
                tx.pragma_update(None, "user_version", i + 1)?;
                tx.commit()
            })();
            if fk_off {
                conn.execute_batch("PRAGMA foreign_keys = ON;")?;
            }
            result?;
        }
        Ok(Self(Mutex::new(conn)))
    }

    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // Verrou empoisonné (panique d'un autre fil) : la connexion reste utilisable
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
                    chat_presence, se_channel_id, se_points, in_points, in_se, se_refresh, max_balance
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
                max_balance: r.get(14)?,
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

    pub fn set_max_balance(&self, id: i64, max: Option<i64>) -> DbResult<()> {
        self.conn().execute("UPDATE channels SET max_balance = ?2 WHERE id = ?1", params![id, max])?;
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
        // Commandes ignorées : celles de la fiche du streamer
        let mut stmt = conn.prepare(
            "SELECT c.id, c.login, c.channel_id, COALESCE(s.ignored_commands, '[]')
             FROM giveaway_channels c LEFT JOIN streamers s ON s.login = c.login ORDER BY c.position, c.id",
        )?;
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

    /// Commandes ignorées d'un streamer (fiche créée au besoin)
    pub fn set_ignored_commands(&self, login: &str, commands: &[String]) -> DbResult<()> {
        let value = serde_json::to_string(commands).expect("liste sérialisable");
        self.conn().execute(
            "INSERT INTO streamers (login, ignored_commands) VALUES (?1, ?2)
             ON CONFLICT(login) DO UPDATE SET ignored_commands = excluded.ignored_commands",
            params![login, value],
        )?;
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

    /// Fin du live d'une chaîne : ses giveaways ouverts sans gagnant passent en pause (tirage à
    /// venir, saisis à la main, StreamElements), les autres sont clos
    pub fn close_channel_giveaways(&self, channel: &str) -> DbResult<OfflineOutcome> {
        let conn = self.conn();
        let paused = conn.execute(
            &format!("UPDATE giveaways SET status = 'paused' WHERE status = 'open' AND channel = ?1 AND {PAUSE_ON_OFFLINE}"),
            [channel],
        )?;
        let closed = conn.execute("UPDATE giveaways SET status = 'closed' WHERE status = 'open' AND channel = ?1", [channel])?;
        Ok(OfflineOutcome { paused, closed })
    }

    /// Retour en live d'une chaîne : ses giveaways en pause rouverts (redétectés à `at` pour ne
    /// pas être clos aussitôt par l'ancienneté) ; file d'alertes et participations inchangées.
    /// Renvoie les giveaways rouverts
    pub fn resume_channel_giveaways(&self, channel: &str, at: &str) -> DbResult<Vec<GiveawayRow>> {
        let paused = self.paused_giveaways(Some(channel))?;
        if !paused.is_empty() {
            self.conn().execute("UPDATE giveaways SET status = 'open', detected_at = ?2 WHERE status = 'paused' AND channel = ?1", [channel, at])?;
        }
        Ok(paused)
    }

    /// Giveaways en pause expirés, clos : tirage passé avant `draw_before`, ou détectés avant
    /// `detected_before` (dates RFC 3339) ; renvoie le nombre clos
    pub fn expire_paused_giveaways(&self, draw_before: &str, detected_before: &str) -> DbResult<usize> {
        self.conn().execute(
            "UPDATE giveaways SET status = 'closed' WHERE status = 'paused'
                 AND ((winner IS NULL AND COALESCE(julianday(draw_at) < julianday(?1), 0) = 1) OR detected_at < ?2)",
            [draw_before, detected_before],
        )
    }

    /// Giveaway détecté : conditions reprises de la fiche du streamer
    pub fn insert_giveaway(&self, g: &NewGiveaway) -> DbResult<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO giveaways (channel, kind, command, url, evidence, detected_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![g.channel, g.kind, g.command, g.url, g.evidence, g.detected_at],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE giveaways SET require_follow = s.require_follow, follow_min_days = s.follow_min_days,
                 require_presence = s.require_presence, require_discord = s.require_discord,
                 require_greeting = s.require_greeting, min_chat_minutes = s.min_chat_minutes
             FROM (SELECT * FROM streamers WHERE login = ?2) AS s WHERE giveaways.id = ?1",
            params![id, g.channel],
        )?;
        Ok(id)
    }

    /// Conditions changées entre `before` et `after` (None : inconnue avant, à la création)
    /// recopiées dans la fiche du streamer, créée au besoin ; renvoie si la fiche a changé
    pub fn save_streamer_conditions(&self, channel: &str, before: Option<&GiveawayDetails>, after: &GiveawayDetails) -> DbResult<bool> {
        let b = before.cloned().unwrap_or_default();
        let mut sets: Vec<&str> = vec![];
        let mut values: Vec<rusqlite::types::Value> = vec![channel.to_owned().into()];
        let mut set = |column: &'static str, changed: bool, value: rusqlite::types::Value| {
            if changed {
                values.push(value);
                sets.push(column);
            }
        };
        let flag = |v: Option<bool>| v.map_or(rusqlite::types::Value::Null, |v| (v as i64).into());
        let num = |v: Option<i64>| v.map_or(rusqlite::types::Value::Null, Into::into);
        set("require_follow", b.require_follow != after.require_follow, flag(after.require_follow));
        set("follow_min_days", b.follow_min_days != after.follow_min_days, num(after.follow_min_days));
        set("require_presence", b.require_presence != after.require_presence, flag(after.require_presence));
        set("require_discord", b.require_discord != after.require_discord, flag(after.require_discord));
        set("require_greeting", b.require_greeting != after.require_greeting, flag(after.require_greeting));
        set("min_chat_minutes", b.min_chat_minutes != after.min_chat_minutes, num(after.min_chat_minutes));
        if sets.is_empty() {
            return Ok(false);
        }
        let assignments: Vec<String> = sets.iter().enumerate().map(|(i, c)| format!("{c} = ?{}", i + 2)).collect();
        let conn = self.conn();
        conn.execute("INSERT INTO streamers (login) VALUES (?1) ON CONFLICT(login) DO NOTHING", [channel])?;
        conn.execute(&format!("UPDATE streamers SET {} WHERE login = ?1", assignments.join(", ")), rusqlite::params_from_iter(values))?;
        Ok(true)
    }

    /// Giveaways récents (ouverts d'abord), avec leurs participations
    pub fn giveaways(&self, limit: usize) -> DbResult<Vec<GiveawayRow>> {
        self.select_giveaways("ORDER BY status = 'open' DESC, status = 'paused' DESC, detected_at DESC LIMIT ?1", [limit as i64])
    }

    /// Giveaways ouverts (d'une chaîne, ou de toutes), les plus récents d'abord
    pub fn open_giveaways(&self, channel: Option<&str>) -> DbResult<Vec<GiveawayRow>> {
        match channel {
            Some(c) => self.select_giveaways("WHERE status = 'open' AND channel = ?1 ORDER BY detected_at DESC", [c]),
            None => self.select_giveaways("WHERE status = 'open' ORDER BY detected_at DESC", []),
        }
    }

    /// Giveaways en pause (d'une chaîne, ou de toutes), les plus récents d'abord
    /// Giveaways StreamElements ouverts ou en pause avec leur lien (identifiant du giveaway
    /// StreamElements), les plus récents d'abord
    pub fn se_giveaways_to_check(&self) -> DbResult<Vec<GiveawayRow>> {
        self.select_giveaways(
            "WHERE kind = 'streamelements' AND url IS NOT NULL AND status IN ('open', 'paused') ORDER BY detected_at DESC LIMIT 50",
            [],
        )
    }

    pub fn paused_giveaways(&self, channel: Option<&str>) -> DbResult<Vec<GiveawayRow>> {
        match channel {
            Some(c) => self.select_giveaways("WHERE status = 'paused' AND channel = ?1 ORDER BY detected_at DESC", [c]),
            None => self.select_giveaways("WHERE status = 'paused' ORDER BY detected_at DESC", []),
        }
    }

    /// Giveaway par commande d'une chaîne en pause ou clos sans gagnant, même commande (sans
    /// tenir compte de la casse), détecté depuis `since` : à rouvrir plutôt que dupliquer
    pub fn reopenable_command_giveaway(&self, channel: &str, command: &str, since: &str) -> DbResult<Option<GiveawayRow>> {
        Ok(self
            .select_giveaways(
                "WHERE channel = ?1 AND kind = 'command' AND status IN ('paused', 'closed') AND winner IS NULL
                     AND command = ?2 COLLATE NOCASE AND detected_at >= ?3 ORDER BY detected_at DESC LIMIT 1",
                [channel, command, since],
            )?
            .pop())
    }

    /// Giveaways d'une chaîne clos sans gagnant, détectés depuis `since` (RFC 3339), les plus récents d'abord
    pub fn recent_undrawn_giveaways(&self, channel: &str, since: &str) -> DbResult<Vec<GiveawayRow>> {
        self.select_giveaways(
            "WHERE channel = ?1 AND status = 'closed' AND winner IS NULL AND detected_at >= ?2 ORDER BY detected_at DESC",
            [channel, since],
        )
    }

    /// Giveaway tiré depuis `since` (date RFC 3339) sur la chaîne, sans giveaway ajouté depuis sur
    /// cette chaîne : une nouvelle annonce de gagnant s'y ajoute (plusieurs gagnants)
    pub fn recent_drawn_giveaway(&self, channel: &str, since: &str) -> DbResult<Option<GiveawayRow>> {
        Ok(self
            .select_giveaways(
                "WHERE channel = ?1 AND status = 'closed' AND winner IS NOT NULL AND drawn_at >= ?2
                   AND NOT EXISTS (SELECT 1 FROM giveaways n WHERE n.channel = giveaways.channel AND n.id <> giveaways.id
                                   AND n.detected_at > giveaways.drawn_at)
                 ORDER BY drawn_at DESC LIMIT 1",
                [channel, since],
            )?
            .into_iter()
            .next())
    }

    /// Dernier giveaway d'une chaîne ayant ce lien (ouvert ou clos)
    pub fn giveaway_by_url(&self, channel: &str, url: &str) -> DbResult<Option<GiveawayRow>> {
        Ok(self
            .select_giveaways("WHERE channel = ?1 AND url = ?2 COLLATE NOCASE ORDER BY detected_at DESC LIMIT 1", [channel, url])?
            .pop())
    }

    /// Rouvre un giveaway (lien StreamElements reposté) : redétecté maintenant, pour ne pas
    /// être clos aussitôt par l'ancienneté
    pub fn reopen_giveaway(&self, id: i64, at: &str) -> DbResult<()> {
        self.conn().execute("UPDATE giveaways SET status = 'open', detected_at = ?2 WHERE id = ?1", params![id, at])?;
        Ok(())
    }

    /// Giveaways choisis par `filter` (WHERE / ORDER BY / LIMIT), avec participations et cadeaux
    /// lus en deux requêtes groupées
    fn select_giveaways<P: rusqlite::Params>(&self, filter: &str, params: P) -> DbResult<Vec<GiveawayRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, channel, kind, command, url, evidence, detected_at, status, handled, winner, winner_chance, drawn_at,
                    ticket_unique, ticket_max, reward_name, reward_cost, draw_at,
                    require_follow, require_presence, require_discord, notes, manual, reminded,
                    prize_received, prize_note, require_greeting, min_chat_minutes, follow_min_days, winners
             FROM giveaways {filter}"
        ))?;
        let mut rows: Vec<GiveawayRow> = stmt
            .query_map(params, |r| {
                let status: String = r.get(7)?;
                Ok(GiveawayRow {
                    id: r.get(0)?,
                    channel: r.get(1)?,
                    kind: r.get(2)?,
                    command: r.get(3)?,
                    url: r.get(4)?,
                    evidence: r.get(5)?,
                    detected_at: r.get(6)?,
                    open: status == "open",
                    status,
                    handled: r.get(8)?,
                    winner: r.get(9)?,
                    winner_chance: r.get(10)?,
                    drawn_at: r.get(11)?,
                    winners: serde_json::from_str(&r.get::<_, String>(28)?).unwrap_or_default(),
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
                        require_greeting: r.get(25)?,
                        min_chat_minutes: r.get(26)?,
                        follow_min_days: r.get(27)?,
                    },
                    manual: r.get(21)?,
                    reminded: r.get(22)?,
                    gift_ids: vec![],
                    prize_received: r.get(23)?,
                    prize_note: r.get(24)?,
                })
            })?
            .collect::<DbResult<_>>()?;
        if rows.is_empty() {
            return Ok(rows);
        }
        // Identifiants entiers : liste insérée telle quelle dans la requête
        let ids = rows.iter().map(|g| g.id.to_string()).collect::<Vec<_>>().join(",");
        let mut entries: HashMap<i64, Vec<GiveawayEntry>> = HashMap::new();
        let mut stmt =
            conn.prepare(&format!("SELECT giveaway_id, account_id, at, auto FROM giveaway_entries WHERE giveaway_id IN ({ids}) ORDER BY at"))?;
        for e in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, GiveawayEntry { account_id: r.get(1)?, at: r.get(2)?, auto: r.get(3)? })))? {
            let (id, e) = e?;
            entries.entry(id).or_default().push(e);
        }
        let mut gifts: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut stmt = conn.prepare(&format!("SELECT giveaway_id, gift_id FROM giveaway_gifts WHERE giveaway_id IN ({ids}) ORDER BY gift_id"))?;
        for g in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (id, gift) = g?;
            gifts.entry(id).or_default().push(gift);
        }
        for g in &mut rows {
            g.entries = entries.remove(&g.id).unwrap_or_default();
            g.gift_ids = gifts.remove(&g.id).unwrap_or_default();
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
        self.set_giveaway_gifts(id, &g.gift_ids)?;
        self.conn().execute(
            "UPDATE giveaways SET channel = ?2, kind = ?3, command = ?4, url = ?5, ticket_unique = ?6, ticket_max = ?7,
                 reward_name = ?8, reward_cost = ?9, draw_at = ?10, require_follow = ?11, require_presence = ?12,
                 require_discord = ?13, notes = ?14, require_greeting = ?15, min_chat_minutes = ?16, follow_min_days = ?17,
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
                d.notes,
                d.require_greeting,
                d.min_chat_minutes,
                d.follow_min_days
            ],
        )?;
        Ok(())
    }

    pub fn set_giveaway_gifts(&self, id: i64, gift_ids: &[i64]) -> DbResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM giveaway_gifts WHERE giveaway_id = ?1", [id])?;
        for gift in gift_ids {
            tx.execute("INSERT OR IGNORE INTO giveaway_gifts (giveaway_id, gift_id) VALUES (?1, ?2)", params![id, gift])?;
        }
        tx.commit()
    }

    pub fn set_prize(&self, id: i64, received: bool, note: Option<&str>) -> DbResult<()> {
        self.conn().execute(
            "UPDATE giveaways SET prize_received = ?2, prize_note = ?3 WHERE id = ?1",
            params![id, received, note],
        )?;
        Ok(())
    }

    /// Giveaways gagnés par un de nos comptes (pseudos en minuscules), les plus récents d'abord
    pub fn won_giveaways(&self, logins: &[String]) -> DbResult<Vec<GiveawayRow>> {
        if logins.is_empty() {
            return Ok(vec![]);
        }
        let marks = vec!["?"; logins.len()].join(",");
        let lower: Vec<String> = logins.iter().map(|l| l.to_lowercase()).collect();
        self.select_giveaways(
            &format!(
                "WHERE winner IS NOT NULL AND EXISTS (SELECT 1 FROM json_each(giveaways.winners) w WHERE lower(w.value) IN ({marks}))
                 ORDER BY status = 'open' DESC, detected_at DESC"
            ),
            rusqlite::params_from_iter(lower),
        )
    }

    // --- Streamers ---

    pub fn streamer(&self, login: &str) -> DbResult<Option<Streamer>> {
        self.conn().query_row(&format!("{STREAMER_SELECT} WHERE login = ?1"), [login], streamer_row).optional()
    }

    pub fn streamers(&self) -> DbResult<Vec<Streamer>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(STREAMER_SELECT)?;
        let rows = stmt.query_map([], streamer_row)?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn save_streamer(&self, s: &Streamer) -> DbResult<()> {
        self.conn().execute(
            "INSERT INTO streamers (login, discord_url, notes, ignored_commands, require_follow, follow_min_days,
                 require_presence, require_discord, require_greeting, min_chat_minutes, win_reply, win_reply_delay)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(login) DO UPDATE SET discord_url = excluded.discord_url, notes = excluded.notes,
                 ignored_commands = excluded.ignored_commands, require_follow = excluded.require_follow,
                 follow_min_days = excluded.follow_min_days, require_presence = excluded.require_presence,
                 require_discord = excluded.require_discord, require_greeting = excluded.require_greeting,
                 min_chat_minutes = excluded.min_chat_minutes, win_reply = excluded.win_reply,
                 win_reply_delay = excluded.win_reply_delay",
            params![
                s.login,
                s.discord_url,
                s.notes,
                serde_json::to_string(&s.ignored_commands).expect("liste sérialisable"),
                s.require_follow,
                s.follow_min_days,
                s.require_presence,
                s.require_discord,
                s.require_greeting,
                s.min_chat_minutes,
                s.win_reply,
                s.win_reply_delay
            ],
        )?;
        Ok(())
    }

    /// Nouveau giveaway : conditions encore inconnues reprises de la fiche du streamer
    pub fn apply_streamer_conditions(&self, giveaway_id: i64) -> DbResult<()> {
        self.conn().execute(
            "UPDATE giveaways SET
                 require_follow = COALESCE(require_follow, (SELECT require_follow FROM streamers WHERE login = giveaways.channel)),
                 follow_min_days = COALESCE(follow_min_days, (SELECT follow_min_days FROM streamers WHERE login = giveaways.channel)),
                 require_presence = COALESCE(require_presence, (SELECT require_presence FROM streamers WHERE login = giveaways.channel)),
                 require_discord = COALESCE(require_discord, (SELECT require_discord FROM streamers WHERE login = giveaways.channel)),
                 require_greeting = COALESCE(require_greeting, (SELECT require_greeting FROM streamers WHERE login = giveaways.channel)),
                 min_chat_minutes = COALESCE(min_chat_minutes, (SELECT min_chat_minutes FROM streamers WHERE login = giveaways.channel))
             WHERE id = ?1",
            [giveaway_id],
        )?;
        Ok(())
    }

    /// Chaînes ayant un giveaway par points de chaîne ouvert (prioritaires pour le visionnage)
    pub fn open_channelpoints_channels(&self) -> DbResult<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT channel FROM giveaways WHERE status = 'open' AND kind = 'channelpoints'")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    // --- Catalogue de cadeaux ---

    pub fn gift_groups(&self) -> DbResult<Vec<GiftGroup>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, name, game_id, game_name FROM gift_groups ORDER BY name COLLATE NOCASE")?;
        let rows = stmt.query_map([], |r| {
            Ok(GiftGroup { id: r.get(0)?, name: r.get(1)?, game_id: r.get(2)?, game_name: r.get(3)? })
        })?;
        rows.collect()
    }

    /// Groupe d'après son nom dans une catégorie (créé s'il n'existe pas)
    pub fn ensure_gift_group(&self, name: &str, game_id: Option<&str>, game_name: Option<&str>) -> DbResult<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO gift_groups (name, game_id, game_name) VALUES (?1, ?2, ?3)",
            params![name, game_id, game_name],
        )?;
        conn.query_row(
            "SELECT id FROM gift_groups WHERE name = ?1 AND game_id IS ?2",
            params![name, game_id],
            |r| r.get(0),
        )
    }

    /// Nouveau groupe ; None si ce nom existe déjà dans la catégorie
    pub fn add_gift_group(&self, name: &str, game_id: Option<&str>, game_name: Option<&str>) -> DbResult<Option<i64>> {
        let conn = self.conn();
        let added = conn.execute(
            "INSERT OR IGNORE INTO gift_groups (name, game_id, game_name) VALUES (?1, ?2, ?3)",
            params![name, game_id, game_name],
        )?;
        Ok((added == 1).then(|| conn.last_insert_rowid()))
    }

    /// Renomme un groupe (sa catégorie ne change pas) ; faux si le nom est déjà pris dans la catégorie
    pub fn rename_gift_group(&self, id: i64, name: &str) -> DbResult<bool> {
        Ok(self.conn().execute("UPDATE OR IGNORE gift_groups SET name = ?2 WHERE id = ?1", params![id, name])? == 1)
    }

    pub fn delete_gift_group(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM gift_groups WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn gifts(&self) -> DbResult<Vec<Gift>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, title, image, group_id, keywords, game_id, game_name, favorite FROM gifts ORDER BY title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            let keywords: String = r.get(4)?;
            Ok(Gift {
                id: r.get(0)?,
                title: r.get(1)?,
                image: r.get(2)?,
                group_id: r.get(3)?,
                keywords: serde_json::from_str(&keywords).unwrap_or_default(),
                game_id: r.get(5)?,
                game_name: r.get(6)?,
                favorite: r.get(7)?,
            })
        })?;
        rows.collect()
    }

    pub fn set_gift_favorite(&self, id: i64, favorite: bool) -> DbResult<()> {
        self.conn().execute("UPDATE gifts SET favorite = ?2 WHERE id = ?1", params![id, favorite])?;
        Ok(())
    }

    /// Ajoute (id 0) ou modifie un cadeau ; renvoie son identifiant
    pub fn save_gift(&self, g: &Gift) -> DbResult<i64> {
        let conn = self.conn();
        let keywords = serde_json::to_string(&g.keywords).expect("liste sérialisable");
        if g.id == 0 {
            conn.execute(
                "INSERT INTO gifts (title, image, group_id, keywords, game_id, game_name) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![g.title, g.image, g.group_id, keywords, g.game_id, g.game_name],
            )?;
            Ok(conn.last_insert_rowid())
        } else {
            conn.execute(
                "UPDATE gifts SET title = ?2, image = ?3, group_id = ?4, keywords = ?5, game_id = ?6, game_name = ?7 WHERE id = ?1",
                params![g.id, g.title, g.image, g.group_id, keywords, g.game_id, g.game_name],
            )?;
            Ok(g.id)
        }
    }

    pub fn delete_gift(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM gifts WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Nombre de giveaways par cadeau
    pub fn gift_usage(&self) -> DbResult<HashMap<i64, i64>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT gift_id, COUNT(*) FROM giveaway_gifts GROUP BY gift_id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    pub fn set_giveaway_command(&self, id: i64, command: Option<&str>) -> DbResult<()> {
        self.conn().execute("UPDATE giveaways SET command = ?2 WHERE id = ?1", params![id, command])?;
        Ok(())
    }

    /// Commande d'un giveaway saisi sans commande, trouvée ensuite par la détection
    pub fn set_giveaway_command_if_unknown(&self, id: i64, command: &str) -> DbResult<bool> {
        let n = self.conn().execute(
            // Remis dans la file d'alertes : il est maintenant possible d'y participer
            "UPDATE giveaways SET command = ?2, handled = 0 WHERE id = ?1 AND (command IS NULL OR command = '')",
            params![id, command],
        )?;
        Ok(n > 0)
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
        Ok(self.select_giveaways("WHERE id = ?1", [id])?.pop())
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

    /// Gagnant ajouté aux gagnants du giveaway, qui est clos ; `false` : déjà gagnant (ou giveaway
    /// inconnu). Le premier gagnant reste dans `winner`, avec sa chance et l'heure du tirage
    pub fn set_giveaway_winner(&self, id: i64, winner: &str, chance: Option<&str>, at: &str) -> DbResult<bool> {
        let conn = self.conn();
        let Some(text) = conn.query_row("SELECT winners FROM giveaways WHERE id = ?1", [id], |r| r.get::<_, String>(0)).optional()? else {
            return Ok(false);
        };
        let mut winners: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
        if winners.iter().any(|w| w.eq_ignore_ascii_case(winner)) {
            return Ok(false);
        }
        winners.push(winner.to_owned());
        conn.execute(
            "UPDATE giveaways SET winners = ?2, winner = COALESCE(winner, ?3), winner_chance = CASE WHEN winner IS NULL THEN ?4 ELSE winner_chance END,
                 drawn_at = COALESCE(drawn_at, ?5), status = 'closed' WHERE id = ?1",
            params![id, serde_json::to_string(&winners).expect("liste"), winner, chance, at],
        )?;
        Ok(true)
    }

    /// Gagnants choisis à la main (formulaire) : remplacent la liste ; aucun : giveaway rouvert
    pub fn set_giveaway_winners(&self, id: i64, winners: &[String], at: &str) -> DbResult<bool> {
        let Some(first) = winners.first() else { return self.clear_giveaway_winner(id) };
        let n = self.conn().execute(
            "UPDATE giveaways SET winners = ?2, winner_chance = CASE WHEN winner IS ?3 THEN winner_chance ELSE NULL END, winner = ?3,
                 drawn_at = COALESCE(drawn_at, ?4), status = 'closed' WHERE id = ?1",
            params![id, serde_json::to_string(winners).expect("liste"), first, at],
        )?;
        Ok(n > 0)
    }

    /// Gagnants retirés (erreur de détection ou de saisie) : le giveaway est rouvert, les règles de
    /// clôture habituelles s'appliquent ensuite
    pub fn clear_giveaway_winner(&self, id: i64) -> DbResult<bool> {
        let n = self.conn().execute(
            "UPDATE giveaways SET winner = NULL, winners = '[]', winner_chance = NULL, drawn_at = NULL, status = 'open' WHERE id = ?1",
            [id],
        )?;
        Ok(n > 0)
    }

    /// Clôt les giveaways ouverts détectés avant `before` (date RFC 3339), sauf tirage annoncé plus
    /// tard ; ceux sur plusieurs lives (saisis à la main, StreamElements) seulement s'ils ont été
    /// détectés avant `long_before`
    pub fn close_giveaways_before(&self, before: &str, long_before: &str) -> DbResult<usize> {
        self.conn().execute(
            &format!(
                "UPDATE giveaways SET status = 'closed' WHERE status = 'open' AND {DRAW_NOT_AHEAD}
                 AND ((detected_at < ?1 AND manual = 0 AND kind <> 'streamelements') OR detected_at < ?2)"
            ),
            [before, long_before],
        )
    }

    /// Chaîne hors ligne alors que des giveaways sur plusieurs lives sont ouverts (fin du live
    /// manquée : application fermée à ce moment) : mis en pause comme à la fin d'un live
    pub fn pause_offline_giveaways(&self, channel: &str) -> DbResult<usize> {
        self.conn().execute(
            &format!("UPDATE giveaways SET status = 'paused' WHERE status = 'open' AND channel = ?1 AND {PAUSE_ON_OFFLINE}"),
            [channel],
        )
    }

    /// Giveaway terminé rouvert à la main : gagnant retiré, ouvert (`live`) ou en pause jusqu'au
    /// prochain live, ancienneté repartie de `at`
    pub fn reopen_closed_giveaway(&self, id: i64, live: bool, at: &str) -> DbResult<bool> {
        let n = self.conn().execute(
            "UPDATE giveaways SET status = ?2, winner = NULL, winners = '[]', winner_chance = NULL, drawn_at = NULL, detected_at = ?3
             WHERE id = ?1 AND status = 'closed'",
            params![id, if live { "open" } else { "paused" }, at],
        )?;
        Ok(n > 0)
    }

    pub fn delete_giveaway(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM giveaways WHERE id = ?1", [id])?;
        Ok(())
    }

    // --- Réglages ---

    // --- Journal du chat ---

    pub fn add_chat_log(&self, entries: &[ChatLogEntry]) -> DbResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO chat_log (channel, at, author, kind, command, message) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for e in entries {
                stmt.execute(params![e.channel, e.at, e.author, e.kind, e.command, e.message])?;
            }
        }
        tx.commit()
    }

    /// Enregistre la réponse d'un bot ; vrai si elle est nouvelle ou a changé
    pub fn save_bot_reply(&self, r: &BotReply) -> DbResult<bool> {
        let conn = self.conn();
        let current: Option<(String, String)> = conn
            .query_row(
                "SELECT bot, message FROM bot_replies WHERE channel = ?1 AND command = ?2",
                params![r.channel, r.command],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let changed = current.as_ref().is_none_or(|(bot, message)| *bot != r.bot || *message != r.message);
        if changed {
            conn.execute(
                "INSERT INTO bot_replies (channel, command, bot, message, asked_by, updated_at, seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT(channel, command) DO UPDATE SET bot = excluded.bot, message = excluded.message,
                     asked_by = excluded.asked_by, updated_at = excluded.updated_at, seen_at = excluded.seen_at",
                params![r.channel, r.command, r.bot, r.message, r.asked_by, r.seen_at],
            )?;
        } else {
            conn.execute(
                "UPDATE bot_replies SET seen_at = ?3 WHERE channel = ?1 AND command = ?2",
                params![r.channel, r.command, r.seen_at],
            )?;
        }
        Ok(changed)
    }

    /// Réponses des bots de toutes les chaînes (affichage dans les giveaways et les alertes)
    pub fn all_bot_replies(&self) -> DbResult<Vec<BotReply>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT channel, command, bot, message, asked_by, updated_at, seen_at FROM bot_replies
             ORDER BY updated_at DESC LIMIT 500",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(BotReply {
                channel: r.get(0)?,
                command: r.get(1)?,
                bot: r.get(2)?,
                message: r.get(3)?,
                asked_by: r.get(4)?,
                updated_at: r.get(5)?,
                seen_at: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    /// Ajoute des cadeaux à un giveaway (ceux déjà rattachés restent) ; renvoie le nombre ajouté
    pub fn attach_gifts(&self, giveaway_id: i64, gift_ids: &[i64]) -> DbResult<usize> {
        let conn = self.conn();
        let mut added = 0;
        for gift in gift_ids {
            added += conn.execute(
                "INSERT OR IGNORE INTO giveaway_gifts (giveaway_id, gift_id) VALUES (?1, ?2)",
                params![giveaway_id, gift],
            )?;
        }
        Ok(added)
    }

    pub fn bot_replies(&self, channel: &str) -> DbResult<Vec<BotReply>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT channel, command, bot, message, asked_by, updated_at, seen_at FROM bot_replies
             WHERE channel = ?1 ORDER BY command",
        )?;
        let rows = stmt.query_map([channel], |r| {
            Ok(BotReply {
                channel: r.get(0)?,
                command: r.get(1)?,
                bot: r.get(2)?,
                message: r.get(3)?,
                asked_by: r.get(4)?,
                updated_at: r.get(5)?,
                seen_at: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    /// Messages d'un auteur sur une chaîne depuis une date (bonjour déjà dit ?)
    pub fn messages_by(&self, channel: &str, author: &str, since: &str) -> DbResult<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT message FROM chat_log WHERE channel = ?1 AND author = ?2 AND at >= ?3")?;
        let rows = stmt.query_map(params![channel, author, since], |r| r.get(0))?;
        rows.collect()
    }

    pub fn prune_chat_log(&self, before: &str) -> DbResult<usize> {
        self.conn().execute("DELETE FROM chat_log WHERE at < ?1", [before])
    }

    /// Messages d'une chaîne, les plus récents d'abord ; filtres facultatifs : types, commande,
    /// messages plus anciens que `before_id` (page suivante)
    pub fn chat_log(
        &self,
        channel: &str,
        kinds: &[String],
        commands: &[String],
        author: Option<&str>,
        before_id: Option<i64>,
        limit: usize,
    ) -> DbResult<Vec<ChatLogEntry>> {
        let conn = self.conn();
        let kinds_json = serde_json::to_string(kinds).expect("liste sérialisable");
        let mut stmt = conn.prepare(
            "SELECT id, channel, at, author, kind, command, message FROM chat_log
             WHERE channel = ?1
               AND (?2 = '[]' OR kind IN (SELECT value FROM json_each(?2)))
               AND (?3 = '[]' OR command IN (SELECT value FROM json_each(?3)))
               AND (?4 IS NULL OR id < ?4)
               AND (?6 IS NULL OR instr(author, ?6) > 0)
             ORDER BY id DESC LIMIT ?5",
        )?;
        let author = author.map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty());
        let commands_json = serde_json::to_string(commands).expect("liste sérialisable");
        let rows = stmt.query_map(params![channel, kinds_json, commands_json, before_id, limit as i64, author], |r| {
            Ok(ChatLogEntry {
                id: r.get(0)?,
                channel: r.get(1)?,
                at: r.get(2)?,
                author: r.get(3)?,
                kind: r.get(4)?,
                command: r.get(5)?,
                message: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    /// Commandes vues sur une chaîne, les plus tapées d'abord
    pub fn chat_log_commands(&self, channel: &str) -> DbResult<Vec<(String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT command, COUNT(*) FROM chat_log WHERE channel = ?1 AND command IS NOT NULL
             GROUP BY command ORDER BY COUNT(*) DESC, command LIMIT 200",
        )?;
        let rows = stmt.query_map([channel], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    // --- Mentions (file d'alertes) ---

    /// Ajoute une mention et ne garde que les `keep` plus récentes ; renvoie son identifiant
    pub fn add_mention(&self, at: &str, channel: &str, account: &str, author: &str, message: &str, keep: usize) -> DbResult<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO mentions (at, channel, account, author, message) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![at, channel, account, author, message],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute("DELETE FROM mentions WHERE id <= ?1 - ?2", params![id, keep as i64])?;
        Ok(id)
    }

    /// Mentions en attente, les plus récentes d'abord
    pub fn mentions(&self) -> DbResult<Vec<StoredMention>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, at, channel, account, author, message FROM mentions ORDER BY id DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok(StoredMention { id: r.get(0)?, at: r.get(1)?, channel: r.get(2)?, account: r.get(3)?, author: r.get(4)?, message: r.get(5)? })
        })?;
        rows.collect()
    }

    pub fn delete_mention(&self, id: i64) -> DbResult<()> {
        self.conn().execute("DELETE FROM mentions WHERE id = ?1", [id])?;
        Ok(())
    }

    // --- État de l'interface ---

    pub fn get_state(&self, key: &str) -> Option<String> {
        self.conn().query_row("SELECT value FROM app_state WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_state(&self, key: &str, value: &str) -> DbResult<()> {
        self.conn().execute(
            "INSERT INTO app_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // --- Détection des gagnants ajoutée par l'utilisateur ---

    /// Formulations (`winner`, « Le gagnant est {pseudo} ») ou bots (`bot`), dans l'ordre alphabétique
    pub fn detection_rules(&self, kind: &str) -> DbResult<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT value FROM detection_rules WHERE kind = ?1 ORDER BY value COLLATE NOCASE")?;
        let rows = stmt.query_map([kind], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Remplace toute la liste d'un type
    pub fn set_detection_rules(&self, kind: &str, values: &[String]) -> DbResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM detection_rules WHERE kind = ?1", [kind])?;
        for v in values {
            tx.execute("INSERT OR IGNORE INTO detection_rules (kind, value) VALUES (?1, ?2)", params![kind, v])?;
        }
        tx.commit()?;
        Ok(())
    }

    // --- Fil d'événements ---

    /// Ajoute un événement et ne garde que les `keep` plus récents
    pub fn add_event(&self, e: &crate::app::LogEvent, keep: usize) -> DbResult<()> {
        let kind = serde_json::to_value(e.kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO events (at, account, kind, message) VALUES (?1, ?2, ?3, ?4)",
            params![e.at, e.account, kind, e.message],
        )?;
        conn.execute("DELETE FROM events WHERE id <= ?1 - ?2", params![conn.last_insert_rowid(), keep as i64])?;
        Ok(())
    }

    /// Événements les plus récents d'abord
    pub fn recent_events(&self, limit: usize) -> DbResult<Vec<crate::app::LogEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT at, account, kind, message FROM events ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| {
            let kind: String = r.get(2)?;
            Ok(crate::app::LogEvent {
                at: r.get(0)?,
                account: r.get(1)?,
                kind: serde_json::from_value(serde_json::Value::String(kind)).unwrap_or(crate::app::EventKind::Info),
                message: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    pub fn settings(&self) -> Settings {
        let conn = self.conn();
        let Some(value) = conn.query_row("SELECT value FROM settings WHERE key = 'app'", [], |r| r.get::<_, String>(0)).ok() else {
            return Settings::default();
        };
        match serde_json::from_str(&value) {
            Ok(settings) => settings,
            Err(e) => {
                // Réglages illisibles : valeurs par défaut, mais l'original est gardé de côté
                // (« app.bak ») au lieu d'être écrasé au prochain enregistrement ; signalé une fois
                let saved = conn
                    .execute(
                        "INSERT INTO settings (key, value) VALUES ('app.bak', ?1)
                         ON CONFLICT(key) DO UPDATE SET value = excluded.value WHERE value <> excluded.value",
                        [&value],
                    )
                    .unwrap_or(0);
                if saved > 0 {
                    tracing::error!("réglages illisibles, valeurs par défaut (original gardé sous « app.bak ») : {e}");
                }
                Settings::default()
            }
        }
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
        db.set_ignored_commands("lunarsea", &["!points".into()]).unwrap();
        assert_eq!(db.giveaway_channels().unwrap()[0].excluded, vec!["!points"]);
        assert_eq!(db.streamer("lunarsea").unwrap().unwrap().ignored_commands, vec!["!points"]);

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

        assert_eq!(db.close_giveaways_before("2026-11-01T00:00:00Z", "2026-10-01T00:00:00Z").unwrap(), 1);
        assert!(!db.giveaway(old).unwrap().unwrap().open);
        let input = GiveawayInput {
            channel: "lunarsea".into(),
            kind: "ticket".into(),
            command: Some("!ticket".into()),
            url: None,
            gift_ids: vec![],
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
        let group = db.ensure_gift_group("Navires", Some("490377"), Some("Sea of Thieves")).unwrap();
        assert_eq!(db.ensure_gift_group("Navires", Some("490377"), None).unwrap(), group);
        // Même nom dans une autre catégorie, ou sans catégorie : autres groupes
        assert_ne!(db.ensure_gift_group("Navires", Some("263490"), None).unwrap(), group);
        let loose = db.ensure_gift_group("Navires", None, None).unwrap();
        assert_eq!(db.ensure_gift_group("Navires", None, None).unwrap(), loose);
        assert_eq!(db.add_gift_group("Navires", None, None).unwrap(), None);
        let sails = db.add_gift_group("Voiles", Some("490377"), Some("Sea of Thieves")).unwrap().unwrap();
        assert!(!db.rename_gift_group(sails, "Navires").unwrap());
        assert!(db.rename_gift_group(sails, "Voiles d'obsidienne").unwrap());
        let groups = db.gift_groups().unwrap();
        let renamed = groups.iter().find(|g| g.id == sails).unwrap();
        assert_eq!((renamed.name.as_str(), renamed.game_name.as_deref()), ("Voiles d'obsidienne", Some("Sea of Thieves")));
        let gift = db
            .save_gift(&Gift {
                id: 0,
                title: "Navire Obsidienne".into(),
                image: Some("https://example.com/navire.png".into()),
                group_id: Some(group),
                keywords: vec!["obsidienne".into()],
                game_id: Some("490377".into()),
                game_name: Some("Sea of Thieves".into()),
                favorite: false,
            })
            .unwrap();
        assert_eq!(db.gifts().unwrap()[0].game_name.as_deref(), Some("Sea of Thieves"));
        let mut input = input.clone();
        input.gift_ids = vec![gift];
        let manual = db.insert_manual_giveaway(&input, "2026-11-01T12:00:00Z").unwrap();
        assert_eq!(db.giveaway(manual).unwrap().unwrap().gift_ids, vec![gift]);
        assert_eq!(db.gift_usage().unwrap().get(&gift), Some(&1));
        assert_eq!(db.gifts().unwrap()[0].keywords, vec!["obsidienne"]);
        let row = db.giveaway(manual).unwrap().unwrap();
        assert!(row.manual && row.handled && !row.reminded);
        assert_eq!(row.details.draw_at.as_deref(), Some("2026-11-01T21:00:00Z"));
        db.set_giveaway_winner(g, "powange", Some("3%"), "2026-11-01T11:00:00Z").unwrap();
        let row = db.giveaway(g).unwrap().unwrap();
        assert_eq!((row.winner.as_deref(), row.open), (Some("powange"), false));
        assert_eq!(db.won_giveaways(&["POWANGE".into()]).unwrap().len(), 1);
        db.set_prize(g, true, Some("XXXX-YYYY")).unwrap();
        let row = db.giveaway(g).unwrap().unwrap();
        assert!(row.prize_received);
        assert_eq!(row.prize_note.as_deref(), Some("XXXX-YYYY"));
        db.delete_gift(gift).unwrap();
        assert!(db.giveaway(manual).unwrap().unwrap().gift_ids.is_empty());
    }

    #[test]
    fn giveaway_queries_and_draw_ahead() {
        let db = Db::memory().unwrap();
        let a = db.upsert_account("33", "powange", "dev").unwrap();
        let new = |channel: &str, url: Option<&str>| NewGiveaway {
            channel: channel.into(),
            kind: if url.is_some() { "streamelements" } else { "command" }.into(),
            command: url.is_none().then(|| "!join".into()),
            url: url.map(Into::into),
            evidence: None,
            detected_at: "2026-10-01T10:00:00Z".into(),
        };
        let past = db.insert_giveaway(&new("lunarsea", None)).unwrap();
        let ahead = db.insert_giveaway(&new("lunarsea", None)).unwrap();
        let other = db.insert_giveaway(&new("kraken_tv", Some("https://streamelements.com/k/giveaway/abc1"))).unwrap();
        db.add_giveaway_entry(past, a, "2026-10-01T10:01:00Z", false).unwrap();
        let mut input = GiveawayInput {
            channel: "lunarsea".into(),
            kind: "command".into(),
            command: Some("!join".into()),
            url: None,
            gift_ids: vec![],
            details: GiveawayDetails { draw_at: Some("2099-01-01T20:00:00Z".into()), ..Default::default() },
        };
        db.update_giveaway(ahead, &input).unwrap();
        input.details.draw_at = Some("2020-01-01T20:00:00Z".into());
        db.update_giveaway(past, &input).unwrap();

        assert_eq!(db.open_giveaways(Some("lunarsea")).unwrap().len(), 2);
        assert_eq!(db.open_giveaways(None).unwrap().len(), 3);
        assert_eq!(db.giveaway(past).unwrap().unwrap().entries.len(), 1);
        assert!(db.giveaway(ahead).unwrap().unwrap().entries.is_empty());

        // Fin du live : le tirage annoncé plus tard passe en pause ; l'ancienneté ne le clôt pas
        assert_eq!(db.close_channel_giveaways("lunarsea").unwrap(), OfflineOutcome { paused: 1, closed: 1 });
        assert_eq!(db.giveaway(past).unwrap().unwrap().status, "closed");
        let row = db.giveaway(ahead).unwrap().unwrap();
        assert!(row.status == "paused" && !row.open);
        // StreamElements (plusieurs lives) : pas clos par l'ancienneté de 12 h, seulement par la longue
        assert_eq!(db.close_giveaways_before("2026-11-01T00:00:00Z", "2026-10-01T00:00:00Z").unwrap(), 0);
        assert!(db.giveaway(other).unwrap().unwrap().open);
        assert_eq!(db.close_giveaways_before("2026-11-01T00:00:00Z", "2026-11-01T00:00:00Z").unwrap(), 1);
        assert_eq!(db.giveaway(ahead).unwrap().unwrap().status, "paused");
        assert!(!db.giveaway(other).unwrap().unwrap().open);

        // Lien StreamElements retrouvé même clos, puis rouvert
        let found = db.giveaway_by_url("kraken_tv", "https://streamelements.com/K/giveaway/ABC1").unwrap().unwrap();
        assert_eq!(found.id, other);
        assert!(db.giveaway_by_url("lunarsea", "https://streamelements.com/k/giveaway/abc1").unwrap().is_none());
        db.reopen_giveaway(other, "2026-11-01T10:00:00Z").unwrap();
        let row = db.giveaway(other).unwrap().unwrap();
        assert!(row.open && row.detected_at == "2026-11-01T10:00:00Z");
        assert!(db.won_giveaways(&[]).unwrap().is_empty());
    }

    #[test]
    fn giveaways_across_lives() {
        let db = Db::memory().unwrap();
        let a = db.upsert_account("33", "powange", "dev").unwrap();
        let new = |kind: &str, command: Option<&str>, at: &str| NewGiveaway {
            channel: "lunarsea".into(),
            kind: kind.into(),
            command: command.map(Into::into),
            url: (kind == "streamelements").then(|| "https://streamelements.com/l/giveaway/abc1".into()),
            evidence: None,
            detected_at: at.into(),
        };
        let input = |kind: &str, draw_at: Option<&str>| GiveawayInput {
            channel: "lunarsea".into(),
            kind: kind.into(),
            command: Some(if kind == "ticket" { "!join" } else { "!coffre" }.into()),
            url: None,
            gift_ids: vec![],
            details: GiveawayDetails { draw_at: draw_at.map(Into::into), ..Default::default() },
        };
        let now = "2026-11-01T10:00:00Z";
        let plain = db.insert_giveaway(&new("command", Some("!join"), now)).unwrap();
        let se = db.insert_giveaway(&new("streamelements", None, now)).unwrap();
        let manual = db.insert_manual_giveaway(&input("ticket", None), now).unwrap();
        let ahead = db.insert_giveaway(&new("command", Some("!coffre"), now)).unwrap();
        db.update_giveaway(ahead, &input("command", Some("2099-01-01T20:00:00Z"))).unwrap();
        let drawn = db.insert_giveaway(&new("streamelements", None, now)).unwrap();
        db.set_giveaway_winner(drawn, "x", None, now).unwrap();
        db.conn().execute("UPDATE giveaways SET status = 'open' WHERE id = ?1", [drawn]).unwrap();
        db.add_giveaway_entry(se, a, now, true).unwrap();
        db.set_giveaway_handled(se).unwrap();
        let other = db.insert_giveaway(&NewGiveaway { channel: "kraken_tv".into(), ..new("streamelements", None, now) }).unwrap();

        // Fin du live : en pause sans gagnant (tirage à venir, à la main, StreamElements), clos sinon
        assert_eq!(db.close_channel_giveaways("lunarsea").unwrap(), OfflineOutcome { paused: 3, closed: 2 });
        let status = |id: i64| db.giveaway(id).unwrap().unwrap().status;
        assert_eq!([status(plain), status(se), status(manual), status(ahead), status(drawn)], ["closed", "paused", "paused", "paused", "closed"]);
        assert_eq!(status(other), "open");
        assert_eq!(db.paused_giveaways(Some("lunarsea")).unwrap().len(), 3);
        assert!(db.open_giveaways(Some("lunarsea")).unwrap().is_empty());
        // L'ancienneté ne touche que les ouverts ; StreamElements : seulement au bout de la longue durée
        assert_eq!(db.close_giveaways_before("2099-01-01T00:00:00Z", "2026-10-01T00:00:00Z").unwrap(), 0);
        assert_eq!(status(other), "open");
        // Chaîne vue hors ligne sans fin de live (application fermée) : en pause
        assert_eq!(db.pause_offline_giveaways("kraken_tv").unwrap(), 1);
        assert_eq!(status(other), "paused");
        db.conn().execute("UPDATE giveaways SET status = 'open' WHERE id = ?1", [other]).unwrap();
        assert_eq!(db.close_giveaways_before("2099-01-01T00:00:00Z", "2099-01-01T00:00:00Z").unwrap(), 1);
        assert_eq!(status(se), "paused");
        // Réouverture à la main : gagnant retiré, ouvert ou en pause selon le live
        assert!(db.reopen_closed_giveaway(drawn, false, "2026-11-02T08:00:00Z").unwrap());
        let row = db.giveaway(drawn).unwrap().unwrap();
        assert!(row.status == "paused" && row.winner.is_none() && row.detected_at == "2026-11-02T08:00:00Z");
        assert!(!db.reopen_closed_giveaway(drawn, true, now).unwrap());
        db.close_giveaway(drawn).unwrap();
        assert!(db.reopen_closed_giveaway(drawn, true, now).unwrap());
        assert_eq!(status(drawn), "open");
        db.close_giveaway(drawn).unwrap();

        // Retour en live : rouverts, file d'alertes et participations inchangées
        let resumed = db.resume_channel_giveaways("lunarsea", "2026-11-02T18:00:00Z").unwrap();
        assert_eq!(resumed.len(), 3);
        let row = db.giveaway(se).unwrap().unwrap();
        assert!(row.open && row.handled && row.entries.len() == 1 && row.detected_at == "2026-11-02T18:00:00Z");
        assert!(db.resume_channel_giveaways("lunarsea", now).unwrap().is_empty());

        // Expiration des giveaways en pause : tirage passé de plus de 24 h, ou détecté il y a plus de 14 jours
        db.close_channel_giveaways("lunarsea").unwrap();
        db.update_giveaway(ahead, &input("command", Some("2026-11-03T20:00:00Z"))).unwrap();
        assert_eq!(db.expire_paused_giveaways("2026-11-03T19:00:00Z", "2026-10-20T00:00:00Z").unwrap(), 0);
        assert_eq!(db.expire_paused_giveaways("2026-11-03T21:00:00Z", "2026-10-20T00:00:00Z").unwrap(), 1);
        assert_eq!(status(ahead), "closed");
        assert_eq!(db.expire_paused_giveaways("2026-11-03T21:00:00Z", "2026-11-03T00:00:00Z").unwrap(), 2);
        assert!(db.paused_giveaways(None).unwrap().is_empty());

        // Même commande en pause ou close sans gagnant, détectée depuis peu : à rouvrir
        assert_eq!(db.reopenable_command_giveaway("lunarsea", "!JOIN", "2026-10-25T00:00:00Z").unwrap().map(|g| g.id), Some(plain));
        assert!(db.reopenable_command_giveaway("lunarsea", "!join", "2026-11-01T11:00:00Z").unwrap().is_none());
        assert!(db.reopenable_command_giveaway("kraken_tv", "!join", "2026-10-25T00:00:00Z").unwrap().is_none());
        db.set_giveaway_winner(plain, "x", None, now).unwrap();
        assert!(db.reopenable_command_giveaway("lunarsea", "!join", "2026-10-25T00:00:00Z").unwrap().is_none());
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
    fn streamers() {
        let db = Db::memory().unwrap();
        assert_eq!(db.streamer("levraidoffy").unwrap(), None);
        let s = Streamer { login: "levraidoffy".into(), discord_url: Some("https://discord.gg/abc".into()), notes: None, ignored_commands: vec![], ..Default::default() };
        db.save_streamer(&s).unwrap();
        db.save_streamer(&Streamer { notes: Some("Tirage à 22 h".into()), ..s.clone() }).unwrap();
        let back = db.streamer("levraidoffy").unwrap().unwrap();
        assert_eq!((back.discord_url.as_deref(), back.notes.as_deref()), (Some("https://discord.gg/abc"), Some("Tirage à 22 h")));
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
    fn streamer_conditions_prefill_giveaways() {
        let db = Db::memory().unwrap();
        db.save_streamer(&Streamer {
            login: "lunarsea".into(),
            require_follow: Some(true),
            follow_min_days: Some(30),
            require_greeting: Some(true),
            ..Default::default()
        })
        .unwrap();
        let id = db
            .insert_giveaway(&NewGiveaway {
                channel: "lunarsea".into(),
                kind: "command".into(),
                command: Some("!join".into()),
                url: None,
                evidence: None,
                detected_at: "2026-11-01T10:00:00Z".into(),
            })
            .unwrap();
        // Déjà renseignée sur le giveaway : gardée
        db.conn().execute("UPDATE giveaways SET require_greeting = 0 WHERE id = ?1", [id]).unwrap();
        db.apply_streamer_conditions(id).unwrap();
        let d = db.giveaway(id).unwrap().unwrap().details;
        assert_eq!((d.require_follow, d.follow_min_days, d.require_greeting, d.require_presence), (Some(true), Some(30), Some(false), None));
    }

    #[test]
    fn gifts_found_in_text() {
        let gift = |id: i64, title: &str, keywords: &[&str]| Gift {
            id,
            title: title.into(),
            image: None,
            group_id: None,
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            game_id: None,
            game_name: None,
            favorite: false,
        };
        let gifts = vec![gift(1, "Obsidian Sails", &[]), gift(2, "Coffre", &["épée"]), gift(3, "Key", &[])];
        assert_eq!(gifts_in_text(&gifts, "Giveaway : OBSIDIAN   sails + une Epee"), vec![1, 2]);
        assert!(gifts_in_text(&gifts, "monkey business").is_empty());
        assert_eq!(gifts_in_text(&gifts, "3 keys"), vec![3]);
    }

    #[test]
    fn chat_log_filters() {
        let db = Db::memory().unwrap();
        let e = |kind: &str, author: &str, command: Option<&str>, message: &str, at: &str| ChatLogEntry {
            id: 0,
            channel: "lunarsea".into(),
            at: at.into(),
            author: author.into(),
            kind: kind.into(),
            command: command.map(str::to_owned),
            message: message.into(),
        };
        db.add_chat_log(&[
            e("command", "bob", Some("!join"), "!join", "2026-11-01T10:00:00Z"),
            e("command", "eve", Some("!join"), "!join", "2026-11-01T10:00:01Z"),
            e("own", "powange", None, "bonne chance", "2026-11-01T10:00:02Z"),
            e("winner", "streamelements", None, "Le gagnant est bob", "2026-11-01T10:00:03Z"),
        ])
        .unwrap();
        assert_eq!(db.chat_log("lunarsea", &[], &[], None, None, 10).unwrap().len(), 4);
        assert_eq!(db.chat_log("lunarsea", &["own".into(), "winner".into()], &[], None, None, 10).unwrap().len(), 2);
        let joins = db.chat_log("lunarsea", &[], &["!join".into()], None, None, 10).unwrap();
        assert_eq!(joins.iter().map(|x| x.author.as_str()).collect::<Vec<_>>(), vec!["eve", "bob"]);
        assert_eq!(db.chat_log("lunarsea", &[], &[], None, Some(joins[0].id), 10).unwrap().len(), 1);
        assert_eq!(db.chat_log("lunarsea", &[], &[], Some(" EV "), None, 10).unwrap().len(), 1);
        assert_eq!(db.chat_log_commands("lunarsea").unwrap(), vec![("!join".to_owned(), 2)]);
        assert_eq!(db.prune_chat_log("2026-11-01T10:00:02Z").unwrap(), 2);

        // Réponse de bot : gardée, mise à jour seulement quand elle change
        let reply = |message: &str, at: &str| BotReply {
            channel: "lunarsea".into(),
            command: "!giveaway".into(),
            bot: "streamelements".into(),
            message: message.into(),
            asked_by: "bob".into(),
            updated_at: at.into(),
            seen_at: at.into(),
        };
        assert!(db.save_bot_reply(&reply("Tapez !join pour gagner", "2026-11-01T10:00:00Z")).unwrap());
        assert!(!db.save_bot_reply(&reply("Tapez !join pour gagner", "2026-11-01T11:00:00Z")).unwrap());
        let saved = &db.bot_replies("lunarsea").unwrap()[0];
        assert_eq!((saved.updated_at.as_str(), saved.seen_at.as_str()), ("2026-11-01T10:00:00Z", "2026-11-01T11:00:00Z"));
        assert!(db.save_bot_reply(&reply("Tapez !coffre", "2026-11-01T12:00:00Z")).unwrap());
        assert_eq!(db.bot_replies("lunarsea").unwrap()[0].message, "Tapez !coffre");
    }

    #[test]
    fn events_survive_restart_and_are_capped() {
        let db = Db::memory().unwrap();
        for i in 0..5 {
            let e = crate::app::LogEvent {
                at: format!("2026-11-01T10:0{i}:00Z"),
                account: "powange".into(),
                kind: crate::app::EventKind::Gain,
                message: format!("+{i}"),
            };
            db.add_event(&e, 3).unwrap();
        }
        let events = db.recent_events(10).unwrap();
        assert_eq!(events.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(), vec!["+4", "+3", "+2"]);
        assert!(matches!(events[0].kind, crate::app::EventKind::Gain));
    }

    #[test]
    fn settings_default_and_roundtrip() {
        let db = Db::memory().unwrap();
        assert!(db.settings().close_to_tray);
        db.save_settings(&Settings { close_to_tray: false, ..Settings::default() }).unwrap();
        assert!(!db.settings().close_to_tray);
        // Réglages illisibles : valeurs par défaut, original gardé de côté
        db.conn().execute("UPDATE settings SET value = '{abîmé' WHERE key = 'app'", []).unwrap();
        assert!(db.settings().close_to_tray);
        let bak: String = db.conn().query_row("SELECT value FROM settings WHERE key = 'app.bak'", [], |r| r.get(0)).unwrap();
        assert_eq!(bak, "{abîmé");
    }

    #[test]
    fn streamer_sheet_conditions_follow_giveaways() {
        let db = Db::memory().unwrap();
        let new = || NewGiveaway {
            channel: "lunarsea".into(),
            kind: "command".into(),
            command: Some("!coffre".into()),
            url: None,
            evidence: None,
            detected_at: "2026-11-03T10:00:00Z".into(),
        };
        // Sans fiche : conditions inconnues
        let g = db.insert_giveaway(&new()).unwrap();
        assert_eq!(db.giveaway(g).unwrap().unwrap().details.require_presence, None);
        // Conditions changées sur un giveaway : recopiées dans la fiche (créée), seulement celles-là
        let before = db.giveaway(g).unwrap().unwrap().details;
        let after = GiveawayDetails { require_presence: Some(true), min_chat_minutes: Some(30), notes: Some("x".into()), ..before.clone() };
        assert!(db.save_streamer_conditions("lunarsea", Some(&before), &after).unwrap());
        assert!(!db.save_streamer_conditions("lunarsea", Some(&after), &after).unwrap());
        let sheet = db.streamer("lunarsea").unwrap().unwrap();
        assert_eq!((sheet.require_presence, sheet.min_chat_minutes, sheet.require_follow), (Some(true), Some(30), None));
        db.save_streamer(&Streamer { require_follow: Some(false), win_reply: Some("Merci !".into()), ..sheet }).unwrap();
        let only_discord = GiveawayDetails { require_discord: Some(true), ..after.clone() };
        db.save_streamer_conditions("lunarsea", Some(&after), &only_discord).unwrap();
        let sheet = db.streamer("lunarsea").unwrap().unwrap();
        assert_eq!((sheet.require_follow, sheet.require_discord, sheet.win_reply.as_deref()), (Some(false), Some(true), Some("Merci !")));
        // Giveaway détecté ensuite : toutes les conditions de la fiche
        let d = db.giveaway(db.insert_giveaway(&new()).unwrap()).unwrap().unwrap().details;
        assert_eq!(
            (d.require_follow, d.require_presence, d.require_discord, d.min_chat_minutes),
            (Some(false), Some(true), Some(true), Some(30))
        );
    }

    #[test]
    fn several_winners() {
        let db = Db::memory().unwrap();
        let new = |at: &str| NewGiveaway {
            channel: "lunarsea".into(),
            kind: "command".into(),
            command: Some("!coffre".into()),
            url: None,
            evidence: None,
            detected_at: at.into(),
        };
        let g = db.insert_giveaway(&new("2026-11-03T10:00:00Z")).unwrap();
        assert!(db.set_giveaway_winner(g, "alice", Some("2 %"), "2026-11-03T10:30:00Z").unwrap());
        assert!(!db.set_giveaway_winner(g, "Alice", None, "2026-11-03T10:31:00Z").unwrap());
        assert!(db.set_giveaway_winner(g, "bob", None, "2026-11-03T10:32:00Z").unwrap());
        let row = db.giveaway(g).unwrap().unwrap();
        assert_eq!(row.winners, ["alice", "bob"]);
        assert_eq!((row.winner.as_deref(), row.winner_chance.as_deref(), row.drawn_at.as_deref(), row.status.as_str()), (Some("alice"), Some("2 %"), Some("2026-11-03T10:30:00Z"), "closed"));
        assert_eq!(db.won_giveaways(&["bob".into()]).unwrap().len(), 1);
        // Tiré il y a peu, sans giveaway ajouté depuis : une nouvelle annonce s'y ajoute
        assert_eq!(db.recent_drawn_giveaway("lunarsea", "2026-11-03T10:20:00Z").unwrap().map(|r| r.id), Some(g));
        assert!(db.recent_drawn_giveaway("lunarsea", "2026-11-03T10:45:00Z").unwrap().is_none());
        let later = db.insert_giveaway(&new("2026-11-03T10:40:00Z")).unwrap();
        assert!(db.recent_drawn_giveaway("lunarsea", "2026-11-03T10:20:00Z").unwrap().is_none());
        db.delete_giveaway(later).unwrap();
        // Formulaire : liste remplacée (premier gagnant changé : sa chance oubliée), vide : rouvert
        assert!(db.set_giveaway_winners(g, &["bob".into(), "carol".into()], "2026-11-03T11:00:00Z").unwrap());
        let row = db.giveaway(g).unwrap().unwrap();
        assert_eq!((row.winner.as_deref(), row.winner_chance, row.winners.len()), (Some("bob"), None, 2));
        assert!(db.set_giveaway_winners(g, &[], "2026-11-03T11:00:00Z").unwrap());
        let row = db.giveaway(g).unwrap().unwrap();
        assert!(row.winners.is_empty() && row.winner.is_none() && row.status == "open");
    }

    #[test]
    fn migrations_are_transactional_and_backed_up() {
        let dir = std::env::temp_dir().join(format!("twitch-desk-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("twitch-desk.db");
        drop(Db::open(&path).unwrap());
        // Base d'une version antérieure : sauvegardée avant la reprise des migrations
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!("PRAGMA user_version = {}", MIGRATIONS.len() - 1)).unwrap();
        // Dernière migration rejouée : elle échoue si elle ajoute une colonne déjà là ; la base
        // reste alors à la version d'avant, sinon elle passe à la dernière
        drop(conn);
        let reopened = Db::open(&path);
        assert!(dir.join(format!("twitch-desk.db.bak-v{}", MIGRATIONS.len() - 1)).exists());
        let conn = Connection::open(&path).unwrap();
        let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        match reopened {
            Ok(_) => assert_eq!(version, MIGRATIONS.len()),
            Err(_) => assert_eq!(version, MIGRATIONS.len() - 1),
        }
        // Clés étrangères toujours actives après les migrations
        let db = Db::memory().unwrap();
        let fk: i64 = db.conn().query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert_eq!(fk, 1);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
