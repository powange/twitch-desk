//! Giveaways : surveillance du chat des chaînes choisies, détection (commande
//! reprise par le chat, lien StreamElements), gagnants annoncés par les bots,
//! mentions de nos pseudos, file d'alertes, notifications et participation.
//!
//! Lecture mutualisée : pour chaque chaîne surveillée, un seul lecteur. Si un de
//! nos comptes est déjà dans ce chat (présence StreamElements), ses messages
//! servent ; sinon une connexion anonyme commune (lecture seule) rejoint la chaîne.

pub mod bots;
pub mod detect;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::app::{Alert, App, EventKind};
use crate::db::{GiveawayChannel, GiveawayGame, GiveawayInput, GiveawayRow, NewGiveaway, GIVEAWAY_KINDS};
use crate::twitch::gql::{Gql, StreamInfo};
use crate::twitch::chat::{self, ChatAuth, ChatEvent, TapEvent};
use bots::BotEvent;
use detect::{Detector, Evidence};

/// Source « connexion anonyme » dans les événements du chat
const ANONYMOUS: i64 = 0;
const MENTION_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_MENTIONS: usize = 30;
/// Un giveaway resté ouvert plus longtemps est clos (le tirage a été manqué)
const AUTO_CLOSE_AFTER: chrono::Duration = chrono::Duration::hours(12);
const RECENT_GIVEAWAYS: usize = 60;
// Statut live des chaînes surveillées : relu toutes les 2 min, 3 chaînes au plus toutes les 10 s
const LIVE_CHECK_EVERY: Duration = Duration::from_secs(120);
const LIVE_CHECKS_PER_TICK: usize = 3;
/// Rappel « reste présent » envoyé ce délai avant l'heure du tirage
const PRESENCE_REMINDER: chrono::Duration = chrono::Duration::minutes(5);

pub enum MonitorCmd {
    /// Chaînes, réglages ou comptes modifiés
    Reload,
    /// Retirer un giveaway de la file d'alertes sans participer
    Dismiss(i64),
    Close(i64),
    Delete(i64),
    DismissMention(u64),
    /// Gagnant désigné à la main (journal du chat) : giveaway ouvert de la chaîne, sinon le plus récent
    ManualWinner { channel: String, winner: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayEntryView {
    pub account_id: i64,
    pub login: String,
    pub at: String,
    pub auto: bool,
    /// Giveaway StreamElements : tickets du compte (achetés sur le site, dans le chat ou dans l'app)
    pub tickets: Option<i64>,
}

/// Identifiant StreamElements du giveaway, lu dans son lien (…/giveaway/<id>)
fn se_giveaway_id(url: &str) -> Option<&str> {
    let id = url.trim_end_matches('/').rsplit_once("/giveaway/")?.1;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit())).then_some(id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayView {
    #[serde(flatten)]
    pub row: GiveawayRow,
    pub evidence: Option<Evidence>,
    pub entries_view: Vec<GiveawayEntryView>,
    /// Un de nos comptes a gagné : son pseudo
    pub winner_ours: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionView {
    pub id: u64,
    pub at: String,
    pub channel: String,
    pub account: String,
    pub author: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayChannelView {
    pub id: i64,
    pub login: String,
    pub excluded: Vec<String>,
    /// Qui lit ce chat : « anonyme », le pseudo d'un compte, ou rien (pas encore rejoint)
    pub reader: Option<String>,
    /// Statut live déjà relu au moins une fois
    pub live_checked: bool,
    /// En live (None : hors ligne ou pas encore vérifié)
    pub live: Option<StreamInfo>,
    /// En live dans une des catégories choisies (ou aucune catégorie choisie)
    pub in_category: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GiveawaysView {
    pub channels: Vec<GiveawayChannelView>,
    pub games: Vec<GiveawayGame>,
    pub giveaways: Vec<GiveawayView>,
    pub mentions: Vec<MentionView>,
    pub anonymous_connected: bool,
    /// Réponses des bots aux commandes d'information (!giveaway…), par chaîne
    pub bot_replies: HashMap<String, Vec<crate::db::BotReply>>,
}

pub struct MonitorReceivers {
    cmd: mpsc::Receiver<MonitorCmd>,
    tap: mpsc::Receiver<TapEvent>,
}

/// Point d'accès au moniteur, gardé dans l'état de l'application
pub struct GiveawayHandle {
    cmd: mpsc::Sender<MonitorCmd>,
    tap: mpsc::Sender<TapEvent>,
    view: Arc<RwLock<GiveawaysView>>,
    receivers: Mutex<Option<MonitorReceivers>>,
}

impl Default for GiveawayHandle {
    fn default() -> Self {
        let (cmd, cmd_rx) = mpsc::channel(64);
        let (tap, tap_rx) = mpsc::channel(4096);
        Self {
            cmd,
            tap,
            view: Arc::new(RwLock::new(GiveawaysView::default())),
            receivers: Mutex::new(Some(MonitorReceivers { cmd: cmd_rx, tap: tap_rx })),
        }
    }
}

impl GiveawayHandle {
    pub fn tap(&self) -> mpsc::Sender<TapEvent> {
        self.tap.clone()
    }

    pub fn send(&self, cmd: MonitorCmd) {
        let _ = self.cmd.try_send(cmd);
    }

    pub fn view(&self) -> GiveawaysView {
        self.view.read().unwrap().clone()
    }

    /// Démarre le moniteur (une seule fois)
    pub fn start(&self, app: Arc<App>) {
        let Some(rx) = self.receivers.lock().unwrap().take() else { return };
        let view = self.view.clone();
        let tap = self.tap.clone();
        tauri::async_runtime::spawn(async move { Monitor::new(app, view, tap).run(rx).await });
    }
}

struct LiveState {
    info: Option<StreamInfo>,
    at: Instant,
}

struct Monitor {
    app: Arc<App>,
    games: Vec<GiveawayGame>,
    live: HashMap<String, LiveState>,
    gql: Option<Gql>,
    view: Arc<RwLock<GiveawaysView>>,
    tap: mpsc::Sender<TapEvent>,
    channels: Vec<GiveawayChannel>,
    /// Chaîne → sources (0 : anonyme, sinon compte) présentes dans son chat
    coverage: HashMap<String, BTreeSet<i64>>,
    anon_wanted: watch::Sender<BTreeSet<String>>,
    anon_task: Option<JoinHandle<()>>,
    anon_events: Option<mpsc::Receiver<ChatEvent>>,
    anon_connected: bool,
    detector: Detector,
    /// Nos comptes : identifiant → pseudo
    accounts: Vec<(i64, String)>,
    mentions: VecDeque<MentionView>,
    mention_seq: u64,
    last_mention: HashMap<(String, String), Instant>,
    /// Chaînes d'un giveaway ouvert hors des chaînes surveillées : chat lu pour le journal
    giveaway_only: BTreeSet<String>,
    /// Journal du chat en attente d'écriture (groupée)
    chat_log: Vec<crate::db::ChatLogEntry>,
    last_prune: Option<Instant>,
    /// Commandes des giveaways ouverts par chaîne : un mot-clé sans « ! » tapé est alors journalisé
    open_commands: HashMap<String, Vec<(i64, String)>>,
    /// Commande d'information tapée, en attente de la réponse d'un bot : chaîne → (commande, pseudo, moment)
    pending_info: HashMap<String, (String, String, Instant)>,
}

/// Journal du chat : écrit par lots, gardé 14 jours
const CHAT_LOG_FLUSH: usize = 50;
const CHAT_LOG_DAYS: i64 = 14;
/// Commandes d'information sur le giveaway : la réponse du bot est gardée par chaîne
const INFO_COMMANDS: &[&str] = &["!giveaway", "!giveaways", "!gw", "!concours"];
const INFO_REPLY_WITHIN: Duration = Duration::from_secs(15);

use bots::is_bot_like as is_reply_bot;

async fn next_anon(rx: &mut Option<mpsc::Receiver<ChatEvent>>) -> Option<ChatEvent> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl Monitor {
    fn new(app: Arc<App>, view: Arc<RwLock<GiveawaysView>>, tap: mpsc::Sender<TapEvent>) -> Self {
        Self {
            app,
            games: vec![],
            live: HashMap::new(),
            gql: None,
            view,
            tap,
            channels: vec![],
            coverage: HashMap::new(),
            anon_wanted: watch::channel(BTreeSet::new()).0,
            anon_task: None,
            anon_events: None,
            anon_connected: false,
            detector: Detector::default(),
            accounts: vec![],
            mentions: VecDeque::new(),
            mention_seq: 0,
            last_mention: HashMap::new(),
            giveaway_only: BTreeSet::new(),
            chat_log: vec![],
            last_prune: None,
            pending_info: HashMap::new(),
            open_commands: HashMap::new(),
        }
    }

    async fn run(mut self, mut rx: MonitorReceivers) {
        // Mentions encore en attente au dernier lancement
        self.mentions = self
            .app
            .db
            .mentions()
            .unwrap_or_default()
            .into_iter()
            .map(|m| MentionView { id: m.id as u64, at: m.at, channel: m.channel, account: m.account, author: m.author, message: m.message })
            .collect();
        self.reload();
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        let mut live_tick = tokio::time::interval(Duration::from_secs(10));
        let mut log_tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                Some(event) = rx.tap.recv() => self.on_tap(event),
                Some(cmd) = rx.cmd.recv() => self.on_cmd(cmd),
                event = next_anon(&mut self.anon_events) => match event {
                    Some(ChatEvent::Connected(c)) => {
                        self.anon_connected = c;
                        self.publish();
                    }
                    Some(_) => {}
                    None => self.anon_events = None,
                },
                _ = live_tick.tick() => self.check_lives().await,
                _ = log_tick.tick() => self.flush_chat_log(),
                _ = tick.tick() => {
                    self.update_anonymous();
                    let before = (chrono::Utc::now() - AUTO_CLOSE_AFTER).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    if self.app.db.close_giveaways_before(&before).unwrap_or(0) > 0 {
                        self.publish();
                    }
                    self.presence_reminders();
                }
            }
        }
    }

    fn monitored(&self, channel: &str) -> bool {
        self.channels.iter().any(|c| c.login == channel)
    }

    fn reload(&mut self) {
        let settings = self.app.db.settings();
        self.detector.set_params(detect::DetectParams::from_settings(&settings));
        bots::set_extra_bots(&self.app.db.detection_rules("bot").unwrap_or_default());
        for e in bots::set_custom_patterns(&self.app.db.detection_rules("winner").unwrap_or_default()) {
            tracing::warn!("formulation de gagnant écartée : {e}");
        }
        self.channels = self.app.db.giveaway_channels().unwrap_or_default();
        self.games = self.app.db.giveaway_games().unwrap_or_default();
        self.gql = None;
        self.accounts = self.app.db.accounts().unwrap_or_default().into_iter().map(|a| (a.id, a.login.to_lowercase())).collect();
        let monitored: HashSet<String> = self.channels.iter().map(|c| c.login.clone()).collect();
        let stale: Vec<String> = self.coverage.keys().filter(|c| !monitored.contains(*c)).cloned().collect();
        for c in stale {
            self.detector.forget(&c);
        }
        self.update_anonymous();
        self.publish();
    }

    /// Chaînes dont le chat est lu : surveillées, et celles d'un giveaway ouvert (journal)
    fn logged(&self, channel: &str) -> bool {
        self.monitored(channel) || self.giveaway_only.contains(channel)
    }

    /// Chaînes que la connexion anonyme doit rejoindre : lues et sans compte présent
    fn update_anonymous(&mut self) {
        let open: Vec<GiveawayRow> = self.app.db.giveaways(RECENT_GIVEAWAYS).unwrap_or_default().into_iter().filter(|g| g.open).collect();
        self.open_commands.clear();
        for g in &open {
            if let Some(c) = &g.command {
                self.open_commands.entry(g.channel.clone()).or_default().push((g.id, c.to_lowercase()));
            }
        }
        self.giveaway_only = open.into_iter().filter(|g| !self.monitored(&g.channel)).map(|g| g.channel).collect();
        let wanted: BTreeSet<String> = self
            .channels
            .iter()
            .map(|c| c.login.clone())
            .chain(self.giveaway_only.iter().cloned())
            .filter(|c| !self.coverage.get(c).is_some_and(|s| s.iter().any(|src| *src != ANONYMOUS)))
            .collect();
        if !wanted.is_empty() && self.anon_task.is_none() {
            let (tx, rx) = mpsc::channel(32);
            let task = chat::run(ChatAuth::Anonymous, self.anon_wanted.subscribe(), tx, Some((ANONYMOUS, self.tap.clone())));
            self.anon_task = Some(tokio::spawn(task));
            self.anon_events = Some(rx);
        }
        self.anon_wanted.send_if_modified(|current| {
            let changed = *current != wanted;
            *current = wanted;
            changed
        });
    }

    /// Rappel avant le tirage des giveaways qui exigent d'être présent
    fn presence_reminders(&mut self) {
        let now = chrono::Utc::now();
        let due: Vec<(GiveawayRow, i64)> = self
            .app
            .db
            .giveaways(RECENT_GIVEAWAYS)
            .unwrap_or_default()
            .into_iter()
            .filter(|g| g.open && !g.reminded && g.details.require_presence == Some(true))
            .filter_map(|g| {
                let at = chrono::DateTime::parse_from_rfc3339(g.details.draw_at.as_deref()?).ok()?.with_timezone(&chrono::Utc);
                let left = at - now;
                (left <= PRESENCE_REMINDER && left > -chrono::Duration::minutes(1)).then(|| (g, left.num_minutes().max(0)))
            })
            .collect();
        for (g, minutes) in due {
            let _ = self.app.db.set_giveaway_reminded(g.id);
            let when = if minutes == 0 { "maintenant".to_owned() } else { format!("dans {minutes} min") };
            self.app.log("giveaways", EventKind::Giveaway, format!("Tirage {when} sur {} : présence requise", g.channel));
            self.app.notify(Alert::Reminder, &format!("Tirage {when} sur {}", g.channel), "Présence requise au moment du tirage : reste sur le live");
        }
    }

    /// Client Twitch pour le statut live : celui du premier compte connecté
    fn gql(&mut self) -> Option<Gql> {
        if self.gql.is_none() {
            let account = self.app.db.accounts().ok()?.into_iter().next()?;
            let tokens = self.app.secrets.load(&account.user_id)?;
            self.gql = Some(Gql::new(self.app.http.clone(), tokens.access_token, Some(account.device_id)));
        }
        self.gql.clone()
    }

    async fn check_lives(&mut self) {
        let now = Instant::now();
        let due: Vec<String> = self
            .channels
            .iter()
            .map(|c| c.login.clone())
            .filter(|c| self.live.get(c).is_none_or(|l| now.duration_since(l.at) >= LIVE_CHECK_EVERY))
            .take(LIVE_CHECKS_PER_TICK)
            .collect();
        if due.is_empty() {
            return;
        }
        let Some(gql) = self.gql() else { return };
        let mut changed = false;
        for channel in due {
            let info = match gql.stream_info(&channel).await {
                Ok(info) => info,
                Err(crate::twitch::TwitchError::Unauthorized) => {
                    // Jeton renouvelé entre-temps par le superviseur du compte : on le relira
                    self.gql = None;
                    return;
                }
                Err(e) => {
                    tracing::warn!("giveaways : statut de {channel} : {e}");
                    continue;
                }
            };
            let was_live = self.live.get(&channel).is_some_and(|l| l.info.is_some());
            if was_live && info.is_none() {
                // Fin du live : le tirage a eu lieu ou n'aura pas lieu
                let closed = self.app.db.close_channel_giveaways(&channel).unwrap_or(0);
                if closed > 0 {
                    self.app.log("giveaways", EventKind::Info, format!("{channel} est hors ligne : {closed} giveaway(s) clos"));
                }
            }
            if was_live != info.is_some() || self.live.get(&channel).map(|l| &l.info) != Some(&info) {
                changed = true;
            }
            self.live.insert(channel, LiveState { info, at: Instant::now() });
        }
        if changed {
            self.publish();
        }
    }

    /// En live dans une des catégories choisies (toujours vrai sans catégorie choisie)
    fn in_category(&self, channel: &str) -> bool {
        if self.games.is_empty() {
            return true;
        }
        self.live
            .get(channel)
            .and_then(|l| l.info.as_ref())
            .and_then(|i| i.game_id.as_deref())
            .is_some_and(|game| self.games.iter().any(|g| g.game_id == game))
    }

    /// Détection active : sauf en live hors des catégories choisies
    fn detection_active(&self, channel: &str) -> bool {
        let live_elsewhere = self.live.get(channel).is_some_and(|l| l.info.is_some()) && !self.in_category(channel);
        !live_elsewhere
    }

    /// Lecteur d'une chaîne : le premier de nos comptes présent, sinon la connexion anonyme
    fn reader(&self, channel: &str) -> Option<i64> {
        let sources = self.coverage.get(channel)?;
        sources.iter().find(|s| **s != ANONYMOUS).or_else(|| sources.iter().next()).copied()
    }

    fn account_login(&self, id: i64) -> Option<&str> {
        self.accounts.iter().find(|(a, _)| *a == id).map(|(_, l)| l.as_str())
    }

    fn on_tap(&mut self, event: TapEvent) {
        match event {
            TapEvent::Joined { source, channel } => {
                self.coverage.entry(channel).or_default().insert(source);
                self.update_anonymous();
                self.publish_channels();
            }
            TapEvent::Parted { source, channel } => {
                if let Some(s) = self.coverage.get_mut(&channel) {
                    s.remove(&source);
                }
                self.update_anonymous();
                self.publish_channels();
            }
            TapEvent::Disconnected { source } => {
                for s in self.coverage.values_mut() {
                    s.remove(&source);
                }
                if source == ANONYMOUS {
                    self.anon_connected = false;
                }
                self.update_anonymous();
                self.publish_channels();
            }
            TapEvent::Message { source, channel, user, text, moderator } => {
                // Un seul lecteur par chaîne : sinon chaque message compterait double
                if self.logged(&channel) && self.reader(&channel) == Some(source) {
                    self.log_chat(&channel, &user, &text, moderator);
                    if self.monitored(&channel) {
                        self.on_message(&channel, &user, &text, moderator);
                    }
                }
            }
        }
    }

    fn on_cmd(&mut self, cmd: MonitorCmd) {
        match cmd {
            MonitorCmd::Reload => self.reload(),
            MonitorCmd::Dismiss(id) => {
                let _ = self.app.db.set_giveaway_handled(id);
                self.publish();
            }
            MonitorCmd::Close(id) => {
                let _ = self.app.db.close_giveaway(id);
                self.publish();
            }
            MonitorCmd::Delete(id) => {
                let _ = self.app.db.delete_giveaway(id);
                self.publish();
            }
            MonitorCmd::ManualWinner { channel, winner } => self.on_winner(&channel, "manuel", &winner, None),
            MonitorCmd::DismissMention(id) => {
                self.mentions.retain(|m| m.id != id);
                let _ = self.app.db.delete_mention(id as i64);
                self.publish();
            }
        }
    }

    fn open_giveaways(&self, channel: &str) -> Vec<GiveawayRow> {
        self.app
            .db
            .giveaways(RECENT_GIVEAWAYS)
            .unwrap_or_default()
            .into_iter()
            .filter(|g| g.open && g.channel == channel)
            .collect()
    }

    /// Journal du chat : commandes des spectateurs, tous nos messages et ceux du streamer,
    /// annonces de gagnant, réponses des bots
    fn log_chat(&mut self, channel: &str, user: &str, text: &str, moderator: bool) {
        let author = user.to_lowercase();
        // Commande avec « ! », ou mot-clé d'un giveaway ouvert tapé seul (« bonbons »)
        let mut command = detect::extract_command(text).or_else(|| {
            let word = text.trim().to_lowercase();
            self.open_commands.get(channel).is_some_and(|c| c.iter().any(|(_, cmd)| *cmd == word)).then_some(word)
        });
        // Commande d'information (!giveaway…) : on attend la réponse d'un bot
        if let Some(cmd) = command.as_deref().filter(|c| INFO_COMMANDS.contains(&c.split(' ').next().unwrap_or(c))) {
            let base = cmd.split(' ').next().unwrap_or(cmd).to_owned();
            self.pending_info.insert(channel.to_owned(), (base, author.clone(), Instant::now()));
        }
        let reply = if is_reply_bot(&author) { self.take_info_reply(channel, &author, text) } else { None };
        let kind = if let Some(account) = self.accounts.iter().find(|(_, l)| *l == author).map(|(id, _)| *id) {
            self.own_participation(channel, account, text);
            "own"
        } else if (bots::is_bot_like(&author)
            && bots::parse_bot_message(&author, text).iter().any(|e| matches!(e, BotEvent::Winner { .. })))
            || ((moderator || author == channel) && bots::moderator_winner(&author, text).is_some())
        {
            "winner"
        } else if let Some(cmd) = reply {
            command = Some(cmd);
            "reply"
        } else if author == channel {
            // Tous les messages du streamer (règles, annonces, tirage…)
            "streamer"
        } else if command.is_some() && !is_reply_bot(&author) {
            "command"
        } else if is_reply_bot(&author) {
            // Tous les messages des bots (annonces, règles, minuteurs…)
            "bot"
        } else {
            return;
        };
        self.chat_log.push(crate::db::ChatLogEntry {
            id: 0,
            channel: channel.to_owned(),
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            author,
            kind: kind.to_owned(),
            command,
            message: text.chars().take(500).collect(),
        });
        if self.chat_log.len() >= CHAT_LOG_FLUSH {
            self.flush_chat_log();
        }
    }

    /// Réponse de bot nouvelle ou changée : cadeaux du catalogue repérés dedans ajoutés aux
    /// giveaways ouverts de la chaîne (ceux déjà rattachés restent)
    fn gifts_from_reply(&mut self, channel: &str, message: &str) {
        let gifts = self.app.db.gifts().unwrap_or_default();
        let found = crate::db::gifts_in_text(&gifts, message);
        if found.is_empty() {
            return;
        }
        let mut added = 0;
        for g in self.open_giveaways(channel) {
            added += self.app.db.attach_gifts(g.id, &found).unwrap_or(0);
        }
        if added > 0 {
            let names: Vec<&str> = gifts.iter().filter(|g| found.contains(&g.id)).map(|g| g.title.as_str()).collect();
            self.app.log("giveaways", EventKind::Giveaway, format!("Cadeaux repérés dans la réponse du bot sur {channel} : {}", names.join(", ")));
            self.app.sync.poke();
        }
    }

    /// Nouveau giveaway sur une chaîne : cadeaux des réponses de bot déjà connues
    fn gifts_from_known_replies(&mut self, channel: &str) {
        for r in self.app.db.bot_replies(channel).unwrap_or_default() {
            self.gifts_from_reply(channel, &r.message);
        }
    }

    /// Un de nos comptes tape la commande d'un giveaway ouvert dans le chat (depuis Twitch ou
    /// ailleurs) : comptée comme participation de ce compte
    fn own_participation(&mut self, channel: &str, account: i64, text: &str) {
        let typed = text.trim().to_lowercase();
        let Some(list) = self.open_commands.get(channel) else { return };
        let ids: Vec<i64> = list
            .iter()
            .filter(|(_, cmd)| typed == *cmd || typed.starts_with(&format!("{cmd} ")))
            .map(|(id, _)| *id)
            .collect();
        if ids.is_empty() {
            return;
        }
        let at = now_rfc3339();
        for id in ids {
            if let Err(e) = self.app.db.add_giveaway_entry(id, account, &at, false) {
                tracing::warn!("participation depuis le chat : {e}");
            }
        }
        self.publish();
    }

    /// Message d'un bot juste après une commande d'information : enregistré comme sa réponse
    /// (sauf s'il s'adresse visiblement à quelqu'un d'autre) ; renvoie la commande
    fn take_info_reply(&mut self, channel: &str, bot: &str, text: &str) -> Option<String> {
        let (command, asked_by, at) = self.pending_info.get(channel)?.clone();
        if at.elapsed() > INFO_REPLY_WITHIN {
            self.pending_info.remove(channel);
            return None;
        }
        let lower = text.to_lowercase();
        if lower.contains('@') && !lower.contains(&asked_by) {
            return None;
        }
        self.pending_info.remove(channel);
        let now = now_rfc3339();
        let reply = crate::db::BotReply {
            channel: channel.to_owned(),
            command: command.clone(),
            bot: bot.to_owned(),
            message: text.chars().take(500).collect(),
            asked_by,
            updated_at: now.clone(),
            seen_at: now,
        };
        match self.app.db.save_bot_reply(&reply) {
            Ok(true) => {
                self.app.log("giveaways", EventKind::Info, format!("Réponse de {bot} à {command} sur {channel} enregistrée"));
                self.gifts_from_reply(channel, &reply.message);
                self.publish();
            }
            Ok(false) => {}
            Err(e) => tracing::warn!("réponse de bot : {e}"),
        }
        Some(command)
    }

    fn flush_chat_log(&mut self) {
        if !self.chat_log.is_empty() {
            let entries = std::mem::take(&mut self.chat_log);
            if let Err(e) = self.app.db.add_chat_log(&entries) {
                tracing::warn!("journal du chat : {e}");
            }
        }
        // Ménage une fois par heure
        if self.last_prune.is_none_or(|t| t.elapsed() >= Duration::from_secs(3600)) {
            self.last_prune = Some(Instant::now());
            let before = (chrono::Utc::now() - chrono::Duration::days(CHAT_LOG_DAYS)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let _ = self.app.db.prune_chat_log(&before);
        }
    }

    fn on_message(&mut self, channel: &str, user: &str, text: &str, moderator: bool) {
        let user_l = user.to_lowercase();
        // Annonce du gagnant par un modérateur (bot ou humain) ou le streamer : badges du message,
        // qu'un spectateur ne peut pas imiter
        if (moderator || user_l == channel) && !bots::is_bot_like(&user_l) {
            if let Some(BotEvent::Winner { bot, winner, chance }) = bots::moderator_winner(&user_l, text) {
                self.on_winner(channel, &bot, &winner, chance.as_deref());
                return;
            }
        }
        if bots::is_bot_like(&user_l) {
            for event in bots::parse_bot_message(&user_l, text) {
                match event {
                    BotEvent::Winner { bot, winner, chance } => self.on_winner(channel, &bot, &winner, chance.as_deref()),
                    BotEvent::StreamElementsUrl(url) => self.on_streamelements_url(channel, &url),
                    // « @pseudo got N tickets! (N/MAX) » : le maximum par personne du giveaway en cours
                    BotEvent::TicketInfo { max, .. } => {
                        if let Some(g) = self.open_giveaways(channel).first() {
                            if self.app.db.set_ticket_max_if_unknown(g.id, max).unwrap_or(false) {
                                self.publish();
                            }
                        }
                    }
                    BotEvent::TicketPurchase { .. } => {}
                }
            }
            return;
        }
        self.on_mention(channel, &user_l, text);
        if !self.detection_active(channel) {
            return;
        }

        let open = self.open_giveaways(channel);
        let mut ignored = self.channels.iter().find(|c| c.login == channel).map(|c| c.excluded.clone()).unwrap_or_default();
        if open.iter().any(|g| g.kind == "streamelements") {
            ignored.push(detect::STREAMELEMENTS_TICKET_COMMAND.to_owned());
        }
        let Some((command, evidence)) = self.detector.push(channel, &user_l, text, &ignored) else { return };
        // Même commande déjà ouverte sur la chaîne : pas de doublon
        if open.iter().any(|g| g.command.as_deref() == Some(command.as_str())) {
            return;
        }
        // Giveaway saisi sans commande (pas encore connue) : la commande détectée le complète
        let waiting = open.iter().find(|g| matches!(g.kind.as_str(), "command" | "ticket") && g.command.as_deref().unwrap_or("").is_empty());
        if let Some(g) = waiting {
            if self.app.db.set_giveaway_command_if_unknown(g.id, &command).unwrap_or(false) {
                self.app.log("giveaways", EventKind::Giveaway, format!("Commande trouvée pour le giveaway de {channel} : {command}"));
                self.app.notify(Alert::Giveaway, &format!("Commande trouvée sur {channel}"), &format!("Le giveaway attend {command}"));
                self.publish();
                schedule_auto_participation(self.app.clone(), g.id);
            }
            return;
        }
        let id = match self.app.db.insert_giveaway(&NewGiveaway {
            channel: channel.to_owned(),
            kind: "command".into(),
            command: Some(command.clone()),
            url: None,
            evidence: serde_json::to_string(&evidence).ok(),
            detected_at: now_rfc3339(),
        }) {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("giveaway non enregistré : {e}");
                return;
            }
        };
        // Conditions habituelles du streamer (fiche)
        let _ = self.app.db.apply_streamer_conditions(id);
        let share = evidence.count * 100 / evidence.window.max(1);
        self.app.log("giveaways", EventKind::Giveaway, format!("Giveaway détecté sur {channel} : {command} ({share} % du chat)"));
        self.app.notify(Alert::Giveaway, &format!("Giveaway sur {channel}"), &format!("Commande {command}, reprise par {share} % du chat"));
        self.gifts_from_known_replies(channel);
        self.publish();
        schedule_auto_participation(self.app.clone(), id);
    }

    fn on_streamelements_url(&mut self, channel: &str, url: &str) {
        let open = self.open_giveaways(channel);
        if open.iter().any(|g| g.url.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(url))) {
            return;
        }
        if let Ok(id) = self.app.db.insert_giveaway(&NewGiveaway {
            channel: channel.to_owned(),
            kind: "streamelements".into(),
            command: None,
            url: Some(url.to_owned()),
            evidence: None,
            detected_at: now_rfc3339(),
        }) {
            let _ = self.app.db.apply_streamer_conditions(id);
        }
        self.app.log("giveaways", EventKind::Giveaway, format!("Giveaway StreamElements sur {channel}"));
        let app = self.app.clone();
        let login = channel.to_owned();
        let channel_id = self.channels.iter().find(|c| c.login == channel).map(|c| c.channel_id.clone());
        tauri::async_runtime::spawn(async move { ensure_channel_in_list(&app, &login, channel_id, crate::db::ChannelList::Se).await });
        self.app.notify(Alert::Giveaway, &format!("Giveaway StreamElements sur {channel}"), "Tickets à acheter avec tes points StreamElements");
        self.gifts_from_known_replies(channel);
        self.publish();
    }

    fn on_winner(&mut self, channel: &str, bot: &str, winner: &str, chance: Option<&str>) {
        let open = self.open_giveaways(channel);
        // Giveaway du type tiré par ce bot, sinon le plus récent
        let wanted_kind = if bot == "streamelements" { "streamelements" } else { "command" };
        // Gagnant désigné à la main : à défaut de giveaway ouvert, le plus récent de la chaîne
        let latest = if bot == "manuel" && open.is_empty() {
            self.app.db.giveaways(RECENT_GIVEAWAYS).unwrap_or_default().into_iter().find(|g| g.channel == channel)
        } else {
            None
        };
        let target = open.iter().find(|g| g.kind == wanted_kind).or_else(|| open.first()).or(latest.as_ref());
        if let Some(g) = target {
            let _ = self.app.db.set_giveaway_winner(g.id, winner, chance, &now_rfc3339());
        }
        let ours = self.accounts.iter().find(|(_, l)| l.eq_ignore_ascii_case(winner)).map(|(_, l)| l.clone());
        match ours {
            Some(login) => {
                self.app.log(&login, EventKind::Giveaway, format!("Tu as gagné le giveaway de {channel} !"));
                self.app.notify(Alert::Win, "Tu as gagné !", &format!("{login} a gagné le giveaway de {channel}"));
            }
            None if target.is_some() => {
                self.app.log("giveaways", EventKind::Info, format!("Tirage sur {channel} : {winner}"));
            }
            None => {}
        }
        self.publish();
    }

    fn on_mention(&mut self, channel: &str, author: &str, text: &str) {
        let ours: Vec<String> = self.accounts.iter().map(|(_, l)| l.clone()).collect();
        let now = Instant::now();
        for login in detect::mentions(&ours, author, text) {
            let key = (channel.to_owned(), login.clone());
            if self.last_mention.get(&key).is_some_and(|t| now.duration_since(*t) < MENTION_COOLDOWN) {
                continue;
            }
            self.last_mention.insert(key, now);
            let message: String = text.chars().take(300).collect();
            let at = now_rfc3339();
            // Gardée en base : la file d'alertes survit au redémarrage
            let id = match self.app.db.add_mention(&at, channel, &login, author, &message, MAX_MENTIONS) {
                Ok(id) => id as u64,
                Err(e) => {
                    tracing::warn!("mention non enregistrée : {e}");
                    self.mention_seq += 1;
                    u64::MAX - self.mention_seq
                }
            };
            self.mentions.push_front(MentionView {
                id,
                at,
                channel: channel.to_owned(),
                account: login.clone(),
                author: author.to_owned(),
                message: message.clone(),
            });
            self.mentions.truncate(MAX_MENTIONS);
            self.app.notify(Alert::Mention, &format!("{author} mentionne {login} sur {channel}"), &message);
            self.publish();
        }
    }

    fn channel_views(&self) -> Vec<GiveawayChannelView> {
        self.channels
            .iter()
            .map(|c| GiveawayChannelView {
                id: c.id,
                login: c.login.clone(),
                excluded: c.excluded.clone(),
                reader: self.reader(&c.login).map(|src| match src {
                    ANONYMOUS => "anonyme".to_owned(),
                    id => self.account_login(id).unwrap_or("compte").to_owned(),
                }),
                live_checked: self.live.contains_key(&c.login),
                live: self.live.get(&c.login).and_then(|l| l.info.clone()),
                in_category: self.live.get(&c.login).is_some_and(|l| l.info.is_some()) && self.in_category(&c.login),
            })
            .collect()
    }

    fn publish_channels(&mut self) {
        let channels = self.channel_views();
        self.view.write().unwrap().channels = channels;
        self.view.write().unwrap().anonymous_connected = self.anon_connected;
        self.app.touch();
    }

    fn publish(&mut self) {
        // Giveaways ouverts changés : chats lus et mots-clés journalisés à jour
        self.update_anonymous();
        update_point_priority(&self.app);
        let view = GiveawaysView {
            channels: self.channel_views(),
            games: self.games.clone(),
            giveaways: build_views(&self.app, &self.accounts),
            mentions: self.mentions.iter().cloned().collect(),
            anonymous_connected: self.anon_connected,
            bot_replies: {
                let mut map: HashMap<String, Vec<crate::db::BotReply>> = HashMap::new();
                for r in self.app.db.all_bot_replies().unwrap_or_default() {
                    map.entry(r.channel.clone()).or_default().push(r);
                }
                map
            },
        };
        *self.view.write().unwrap() = view;
        self.app.touch();
    }
}

fn build_views(app: &App, accounts: &[(i64, String)]) -> Vec<GiveawayView> {
    rows_to_views(app, app.db.giveaways(RECENT_GIVEAWAYS).unwrap_or_default(), accounts)
}

fn rows_to_views(app: &App, rows: Vec<GiveawayRow>, accounts: &[(i64, String)]) -> Vec<GiveawayView> {
    rows.into_iter()
        .map(|row| {
            let login_of = |id: i64| accounts.iter().find(|(a, _)| *a == id).map(|(_, l)| l.clone()).unwrap_or_default();
            let mut entries_view: Vec<GiveawayEntryView> = row
                .entries
                .iter()
                .map(|e| GiveawayEntryView {
                    account_id: e.account_id,
                    login: login_of(e.account_id),
                    at: e.at.clone(),
                    auto: e.auto,
                    tickets: None,
                })
                .collect();
            // StreamElements : nos tickets relevés sur ce giveaway valent participation
            if let Some(se_id) = row.url.as_deref().and_then(se_giveaway_id) {
                for (account_id, login) in accounts {
                    let Some(tickets) = app.db.se_tickets(*account_id, se_id).filter(|t| *t > 0) else { continue };
                    match entries_view.iter_mut().find(|e| e.account_id == *account_id) {
                        Some(e) => e.tickets = Some(tickets),
                        None => entries_view.push(GiveawayEntryView {
                            account_id: *account_id,
                            login: login.clone(),
                            at: String::new(),
                            auto: false,
                            tickets: Some(tickets),
                        }),
                    }
                }
            }
            GiveawayView {
                evidence: row.evidence.as_deref().and_then(|e| serde_json::from_str(e).ok()),
                entries_view,
                winner_ours: row
                    .winner
                    .as_deref()
                    .and_then(|w| accounts.iter().find(|(_, l)| l.eq_ignore_ascii_case(w)).map(|(_, l)| l.clone())),
                row,
            }
        })
        .collect()
}

/// Giveaways gagnés par nos comptes (page « Mes gains »)
pub fn won_views(app: &App) -> Vec<GiveawayView> {
    let accounts: Vec<(i64, String)> =
        app.db.accounts().unwrap_or_default().into_iter().map(|a| (a.id, a.login.to_lowercase())).collect();
    let logins: Vec<String> = accounts.iter().map(|(_, l)| l.clone()).collect();
    let rows = app.db.won_giveaways(&logins).unwrap_or_default();
    rows_to_views(app, rows, &accounts)
}

/// Rafraîchit la vue après une participation (hors de la boucle du moniteur)
pub fn refresh_view(app: &App) {
    update_point_priority(app);
    let accounts: Vec<(i64, String)> =
        app.db.accounts().unwrap_or_default().into_iter().map(|a| (a.id, a.login.to_lowercase())).collect();
    let giveaways = build_views(app, &accounts);
    app.giveaways.view.write().unwrap().giveaways = giveaways;
    app.touch();
}

/// Chaînes prioritaires pour les points : un giveaway par points de chaîne y est ouvert
fn update_point_priority(app: &App) {
    let channels: std::collections::HashSet<String> = app.db.open_channelpoints_channels().unwrap_or_default().into_iter().collect();
    if *app.point_priority.read().unwrap() != channels {
        *app.point_priority.write().unwrap() = channels;
        app.broadcast(crate::miner::MinerCommand::Wake);
    }
}

/// Résultat d'une participation, compte par compte
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipationResult {
    pub account_id: i64,
    pub login: String,
    pub ok: bool,
    pub message: String,
}

/// Envoie la commande du giveaway depuis les comptes choisis (une fois par compte)
pub async fn participate(app: &Arc<App>, giveaway_id: i64, account_ids: &[i64], auto: bool) -> Result<Vec<ParticipationResult>, String> {
    let giveaway = app.db.giveaway(giveaway_id).map_err(|e| e.to_string())?.ok_or("giveaway introuvable")?;
    if !giveaway.open {
        return Err("Ce giveaway est terminé".into());
    }
    let command = match (giveaway.kind.as_str(), giveaway.command.clone()) {
        ("streamelements", _) => return Err("Giveaway StreamElements : les tickets s'achètent depuis la page StreamElements".into()),
        ("channelpoints", _) => {
            let reward = giveaway.details.reward_name.as_deref().unwrap_or("la récompense");
            return Err(format!("Giveaway par points de chaîne : achète {reward} dans le chat Twitch"));
        }
        (_, Some(c)) => c,
        (_, None) => return Err("Aucune commande renseignée pour ce giveaway : modifie sa fiche".into()),
    };
    let accounts = app.db.accounts().map_err(|e| e.to_string())?;
    let mut results = vec![];
    for (i, id) in account_ids.iter().enumerate() {
        let Some(account) = accounts.iter().find(|a| a.id == *id) else { continue };
        if giveaway.entries.iter().any(|e| e.account_id == *id) {
            results.push(ParticipationResult { account_id: *id, login: account.login.clone(), ok: true, message: "déjà inscrit".into() });
            continue;
        }
        // Pas plusieurs comptes à la même seconde
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(rand::random_range(1500..4000))).await;
        }
        let result = match app.secrets.load(&account.user_id) {
            None => Err("jeton introuvable : reconnecte ce compte".to_owned()),
            Some(tokens) => chat::send_message(&account.login, &tokens.access_token, &giveaway.channel, &command).await,
        };
        match result {
            Ok(()) => {
                let _ = app.db.add_giveaway_entry(giveaway_id, *id, &now_rfc3339(), auto);
                let how = if auto { "participation automatique" } else { "participation" };
                app.log(&account.login, EventKind::Giveaway, format!("{command} envoyé sur {} ({how})", giveaway.channel));
                results.push(ParticipationResult { account_id: *id, login: account.login.clone(), ok: true, message: "envoyé".into() });
            }
            Err(e) => {
                app.log(&account.login, EventKind::Error, format!("{command} non envoyé sur {} : {e}", giveaway.channel));
                results.push(ParticipationResult { account_id: *id, login: account.login.clone(), ok: false, message: e });
            }
        }
    }
    refresh_view(app);
    Ok(results)
}

/// Participation automatique (si activée) : chaque compte après un délai aléatoire
fn schedule_auto_participation(app: Arc<App>, giveaway_id: i64) {
    let settings = app.db.settings();
    if !settings.auto_participate {
        return;
    }
    let accounts: Vec<i64> = app
        .db
        .accounts()
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.id)
        .filter(|id| settings.auto_accounts.is_empty() || settings.auto_accounts.contains(id))
        .collect();
    let (min, max) = (settings.auto_delay_min.min(settings.auto_delay_max), settings.auto_delay_max.max(settings.auto_delay_min));
    for id in accounts {
        let app = app.clone();
        let delay = rand::random_range(min..=max);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(delay)).await;
            // Mis en pause ou désactivé entre-temps : on n'envoie rien
            if app.is_paused() || !app.db.settings().auto_participate {
                return;
            }
            if let Err(e) = participate(&app, giveaway_id, &[id], true).await {
                tracing::info!("participation automatique annulée : {e}");
            }
        });
    }
}

/// Chaîne d'un giveaway : rejoint une liste de chaque compte. StreamElements : présence
/// dans le chat (points, achat des tickets) ; points de chaîne : récolte, en priorité tant
/// que le giveaway est ouvert. Meilleur effort.
pub async fn ensure_channel_in_list(app: &Arc<App>, login: &str, channel_id: Option<String>, list: crate::db::ChannelList) {
    let accounts = app.db.accounts().unwrap_or_default();
    let Some(first) = accounts.first() else { return };
    let channel_id = match channel_id {
        Some(id) => id,
        None => {
            let Some(tokens) = app.secrets.load(&first.user_id) else { return };
            let gql = Gql::new(app.http.clone(), tokens.access_token, Some(first.device_id.clone()));
            match gql.channel_id(login).await {
                Ok(Some(id)) => id,
                _ => return,
            }
        }
    };
    for account in accounts {
        let (already, page) = match list {
            crate::db::ChannelList::Se => (app.db.channels(account.id).unwrap_or_default().iter().any(|c| c.login == login && c.in_se), "StreamElements"),
            crate::db::ChannelList::Points => {
                (app.db.channels(account.id).unwrap_or_default().iter().any(|c| c.login == login && c.in_points), "Points de chaîne")
            }
        };
        if already {
            continue;
        }
        if app.db.add_channel(account.id, login, &channel_id, list).is_ok() {
            app.log(&account.login, EventKind::Info, format!("{login} ajoutée à {page} (giveaway)"));
            app.send(account.id, crate::miner::MinerCommand::Reload);
        }
    }
}

/// Vérifie et normalise une fiche saisie à la main
pub fn validate_input(mut g: GiveawayInput) -> Result<GiveawayInput, String> {
    g.channel = crate::commands::normalize_login(&g.channel).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    if !GIVEAWAY_KINDS.contains(&g.kind.as_str()) {
        return Err("Type de giveaway inconnu".into());
    }
    g.command = g
        .command
        .map(|c| c.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "))
        // Tel quel : « !join », ou mot-clé sans « ! » (« bonbons »)
        .filter(|c| !c.is_empty());
    if g.command.as_ref().is_some_and(|c| c.chars().count() > 50) {
        return Err("Commande trop longue (50 caractères au plus)".into());
    }
    g.url = g.url.map(|u| u.trim().to_owned()).filter(|u| !u.is_empty());
    if let Some(url) = &g.url {
        if !url.starts_with("https://streamelements.com/") {
            return Err("Le lien StreamElements doit commencer par https://streamelements.com/".into());
        }
    }
    let d = &mut g.details;
    if g.kind != "ticket" {
        d.ticket_unique = None;
    }
    // Maximum par personne : tickets et giveaways StreamElements
    if !matches!(g.kind.as_str(), "ticket" | "streamelements") {
        d.ticket_max = None;
    }
    if d.ticket_unique == Some(true) {
        d.ticket_max = Some(1);
    }
    if d.ticket_max.is_some_and(|m| !(1..=100_000).contains(&m)) {
        return Err("Tickets maximum : entre 1 et 100 000".into());
    }
    if g.kind != "channelpoints" {
        d.reward_name = None;
        d.reward_cost = None;
    }
    d.reward_name = d.reward_name.take().map(|r| r.trim().to_owned()).filter(|r| !r.is_empty());
    if d.reward_cost.is_some_and(|c| !(0..=100_000_000).contains(&c)) {
        return Err("Coût en points invalide".into());
    }
    d.draw_at = match d.draw_at.take().filter(|s| !s.trim().is_empty()) {
        None => None,
        Some(s) => Some(
            chrono::DateTime::parse_from_rfc3339(s.trim())
                .map_err(|_| "Heure du tirage invalide".to_owned())?
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        ),
    };
    d.notes = d.notes.take().map(|n| n.trim().chars().take(1000).collect::<String>()).filter(|n| !n.is_empty());
    if d.min_chat_minutes.is_some_and(|m| !(0..=1440).contains(&m)) {
        return Err("Temps de présence dans le chat invalide (24 h au plus)".into());
    }
    if d.follow_min_days.is_some_and(|n| !(1..=3650).contains(&n)) {
        return Err("Ancienneté du follow invalide (1 à 3650 jours)".into());
    }
    // Ancienneté du follow : n'a de sens qu'avec le follow requis
    if d.require_follow != Some(true) {
        d.follow_min_days = None;
    }
    Ok(g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::GiveawayDetails;

    fn input(kind: &str) -> GiveawayInput {
        GiveawayInput {
            channel: "https://www.twitch.tv/LunarSea".into(),
            kind: kind.into(),
            command: Some(" Coffre ".into()),
            url: None,
            gift_ids: vec![],
            details: GiveawayDetails::default(),
        }
    }

    #[test]
    fn streamelements_ids() {
        assert_eq!(se_giveaway_id("https://streamelements.com/levraidoffy/giveaway/6ae6992ca2537beab7d66474"), Some("6ae6992ca2537beab7d66474"));
        assert_eq!(se_giveaway_id("https://streamelements.com/x/giveaway/abc123/"), Some("abc123"));
        assert_eq!(se_giveaway_id("https://streamelements.com/x"), None);
    }

    #[test]
    fn validation() {
        let g = validate_input(input("command")).unwrap();
        // Commande gardée telle quelle : un mot-clé sans « ! » est valide
        assert_eq!((g.channel.as_str(), g.command.as_deref()), ("lunarsea", Some("coffre")));

        let mut t = input("ticket");
        t.details.ticket_unique = Some(true);
        t.details.ticket_max = Some(9);
        t.details.reward_name = Some("ignoré".into());
        t.details.draw_at = Some("2026-11-01T22:30:00+01:00".into());
        let t = validate_input(t).unwrap();
        assert_eq!(t.details.ticket_max, Some(1));
        assert_eq!(t.details.reward_name, None);
        assert_eq!(t.details.draw_at.as_deref(), Some("2026-11-01T21:30:00Z"));

        // Ancienneté du follow gardée seulement avec le follow requis
        let mut f = input("command");
        f.details.follow_min_days = Some(30);
        assert_eq!(validate_input(f.clone()).unwrap().details.follow_min_days, None);
        f.details.require_follow = Some(true);
        assert_eq!(validate_input(f).unwrap().details.follow_min_days, Some(30));

        // Commande pas encore connue : acceptée, la détection la complétera
        let mut c = input("command");
        c.command = None;
        assert_eq!(validate_input(c).unwrap().command, None);
        let mut se = input("streamelements");
        se.command = None;
        se.url = Some("https://evil.example/x".into());
        assert!(validate_input(se).is_err());
        let mut bad = input("other");
        bad.command = None;
        assert!(validate_input(bad).is_err());
        let mut late = input("command");
        late.details.draw_at = Some("demain".into());
        assert!(validate_input(late).is_err());
    }
}
