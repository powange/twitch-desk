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
use crate::twitch::chat::{self, ChatAuth, ChatEvent, TapEvent, TapSender};
use bots::BotEvent;
use detect::{Detector, Evidence};

/// Source « connexion anonyme » dans les événements du chat
const ANONYMOUS: i64 = 0;
const MENTION_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_MENTIONS: usize = 30;
/// Un giveaway resté ouvert plus longtemps est clos (le tirage a été manqué)
const AUTO_CLOSE_AFTER: chrono::Duration = chrono::Duration::hours(12);
/// Giveaway en pause clos : heure de tirage dépassée de tant, ou détecté depuis tant
const PAUSED_DRAW_GRACE: chrono::Duration = chrono::Duration::hours(24);
const PAUSED_MAX_AGE: chrono::Duration = chrono::Duration::days(14);
/// Commande redétectée : giveaway en pause ou clos sans gagnant de cet âge rouvert, pas dupliqué
const REOPEN_WITHIN: chrono::Duration = chrono::Duration::days(7);
const RECENT_GIVEAWAYS: usize = 60;
// Statut live des chaînes surveillées : relu toutes les 2 min, 3 chaînes au plus toutes les 10 s
const LIVE_CHECK_EVERY: Duration = Duration::from_secs(120);
/// Rappel « reste présent » envoyé ce délai avant l'heure du tirage
const PRESENCE_REMINDER: chrono::Duration = chrono::Duration::minutes(5);

pub enum MonitorCmd {
    /// Chaînes, réglages ou comptes modifiés
    Reload,
    /// Giveaways seuls changés (gagnants, lot reçu, ancien giveaway…) : vue republiée sans relire
    /// réglages, règles, chaînes ni comptes
    Republish,
    /// Retirer un giveaway de la file d'alertes sans participer
    Dismiss(i64),
    Close(i64),
    /// Giveaway terminé rouvert à la main (gagnant retiré)
    Reopen(i64),
    /// Gagnants retirés : ouvert ou en pause selon le live, ancienneté repartie de maintenant
    WinnersCleared(i64),
    /// Bouton « Actualiser » : statut live et giveaways StreamElements suivis relus tout de suite
    Refresh,
    Delete(i64),
    DismissMention(u64),
    /// Gagnant désigné à la main (journal du chat) : giveaway choisi, sinon ouvert de la chaîne,
    /// sinon le plus récent
    ManualWinner { channel: String, winner: String, giveaway_id: Option<i64> },
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
    /// Réponse écrite automatiquement si un de nos comptes gagne (présence requise), et son
    /// délai en secondes (fiche du streamer)
    pub win_reply: Option<String>,
    pub win_reply_delay: Option<i64>,
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
    /// Qui lit ce chat : « anonyme », le pseudo d'un compte, ou rien (pas encore rejoint)
    pub reader: Option<String>,
    /// Statut live déjà relu au moins une fois
    pub live_checked: bool,
    /// En live (None : hors ligne ou pas encore vérifié)
    pub live: Option<StreamInfo>,
    /// En live dans une des catégories choisies (ou aucune catégorie choisie)
    pub in_category: bool,
    /// Photo de profil (connue après la première relecture du statut live)
    pub profile_image: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GiveawaysView {
    pub channels: Vec<GiveawayChannelView>,
    pub games: Vec<GiveawayGame>,
    pub giveaways: Vec<GiveawayView>,
    pub mentions: Vec<MentionView>,
    #[serde(skip_serializing)]
    pub anonymous_connected: bool,
    /// Réponses des bots aux commandes d'information (!giveaway…), par chaîne
    pub bot_replies: HashMap<String, Vec<crate::db::BotReply>>,
}

pub struct MonitorReceivers {
    cmd: mpsc::UnboundedReceiver<MonitorCmd>,
    tap: mpsc::Receiver<TapEvent>,
    control: mpsc::UnboundedReceiver<TapEvent>,
}

/// Point d'accès au moniteur, gardé dans l'état de l'application
pub struct GiveawayHandle {
    /// Commandes (rechargement, participation…) : file jamais pleine, aucune ne se perd
    cmd: mpsc::UnboundedSender<MonitorCmd>,
    tap: TapSender,
    view: Arc<RwLock<GiveawaysView>>,
    receivers: Mutex<Option<MonitorReceivers>>,
    /// Participations en cours d'envoi (giveaway, compte) : manuelle et automatique ne
    /// partent pas en même temps
    sending: Mutex<HashSet<(i64, i64)>>,
}

impl Default for GiveawayHandle {
    fn default() -> Self {
        let (cmd, cmd_rx) = mpsc::unbounded_channel();
        let (tap, tap_rx) = mpsc::channel(4096);
        let (control, control_rx) = mpsc::unbounded_channel();
        Self {
            cmd,
            tap: TapSender::new(tap, control),
            view: Arc::new(RwLock::new(GiveawaysView::default())),
            receivers: Mutex::new(Some(MonitorReceivers { cmd: cmd_rx, tap: tap_rx, control: control_rx })),
            sending: Mutex::new(HashSet::new()),
        }
    }
}

impl GiveawayHandle {
    pub fn tap(&self) -> TapSender {
        self.tap.clone()
    }

    pub fn send(&self, cmd: MonitorCmd) {
        let _ = self.cmd.send(cmd);
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

/// Statut live relu hors de la boucle du moniteur
/// Relecture des giveaways StreamElements suivis : (giveaway, état ; Ok(None) : inconnu ou chaîne
/// sans StreamElements ; Err : lecture échouée, à refaire)
struct SeReport {
    results: Vec<(i64, Result<Option<crate::streamelements::GiveawayOutcome>, ()>)>,
    channel_ids: Vec<(String, Option<String>)>,
}

/// Giveaway StreamElements suivi : relu à ce rythme (ou tout de suite après une annonce du bot)
const SE_CHECK_EVERY: Duration = Duration::from_secs(3 * 60);
/// … en pause ou chaîne hors ligne
const SE_CHECK_IDLE: Duration = Duration::from_secs(45 * 60);
/// Chaîne surveillée en live dans une catégorie suivie : giveaway StreamElements en cours
/// cherché à ce rythme (créé s'il n'est pas encore connu)
const SE_ACTIVE_EVERY: Duration = Duration::from_secs(3 * 60);

/// Giveaways StreamElements en cours des chaînes surveillées : (chaîne, giveaway en cours)
struct SeActiveReport {
    results: Vec<(String, crate::streamelements::Giveaway)>,
    channel_ids: Vec<(String, Option<String>)>,
}

struct LiveReport {
    results: Vec<(String, Option<StreamInfo>)>,
    /// Un jeton a été refusé : clients à refaire (jeton renouvelé entre-temps)
    unauthorized: bool,
}

struct Monitor {
    app: Arc<App>,
    games: Vec<GiveawayGame>,
    live: HashMap<String, LiveState>,
    /// Clients Twitch pour le statut live : un par compte, essayés dans l'ordre
    gqls: Vec<Gql>,
    live_tx: mpsc::Sender<LiveReport>,
    live_rx: Option<mpsc::Receiver<LiveReport>>,
    /// Giveaways StreamElements suivis par leur identifiant : relecture en tâche à part
    se_tx: mpsc::Sender<SeReport>,
    se_rx: Option<mpsc::Receiver<SeReport>>,
    /// Relecture en cours depuis ; giveaway → dernière relecture
    se_busy: Option<Instant>,
    se_checked: HashMap<i64, Instant>,
    /// Recherche du giveaway StreamElements en cours des chaînes surveillées
    se_active_tx: mpsc::Sender<SeActiveReport>,
    se_active_rx: Option<mpsc::Receiver<SeActiveReport>>,
    se_active_busy: Option<Instant>,
    se_active_checked: HashMap<String, Instant>,
    /// Chaîne → identifiant StreamElements (None : chaîne sans StreamElements)
    se_channels: HashMap<String, Option<String>>,
    /// Gagnants annoncés par le bot StreamElements (dans l'ordre, avec l'heure de la première
    /// annonce), en attente de la vérification du giveaway StreamElements de la chaîne
    se_announced: HashMap<String, Vec<String>>,
    se_announced_at: HashMap<String, Instant>,
    /// Relecture du statut live en cours depuis (oubliée après LIVE_CHECK_EVERY : tâche perdue)
    live_busy: Option<Instant>,
    /// Giveaways ouverts (toutes chaînes), relus à chaque publication : lus à chaque message
    open: Vec<GiveawayRow>,
    /// Giveaway → empreinte du titre du live, de la description, du message épinglé et du
    /// catalogue de cadeaux dernièrement passés au crible (repassés seulement quand ils
    /// changent : un cadeau retiré à la main le reste)
    gifts_scanned: HashMap<i64, u64>,
    view: Arc<RwLock<GiveawaysView>>,
    tap: TapSender,
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
    fn new(app: Arc<App>, view: Arc<RwLock<GiveawaysView>>, tap: TapSender) -> Self {
        let (live_tx, live_rx) = mpsc::channel(4);
        let (se_tx, se_rx) = mpsc::channel(4);
        let (se_active_tx, se_active_rx) = mpsc::channel(4);
        Self {
            app,
            games: vec![],
            live: HashMap::new(),
            gqls: vec![],
            live_tx,
            live_rx: Some(live_rx),
            se_tx,
            se_rx: Some(se_rx),
            se_busy: None,
            se_checked: HashMap::new(),
            gifts_scanned: HashMap::new(),
            se_active_tx,
            se_active_rx: Some(se_active_rx),
            se_active_busy: None,
            se_active_checked: HashMap::new(),
            se_channels: HashMap::new(),
            se_announced: HashMap::new(),
            se_announced_at: HashMap::new(),
            live_busy: None,
            open: vec![],
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
        let mut live_rx = self.live_rx.take().expect("moniteur lancé une fois");
        let mut se_rx = self.se_rx.take().expect("moniteur lancé une fois");
        let mut se_active_rx = self.se_active_rx.take().expect("moniteur lancé une fois");
        loop {
            tokio::select! {
                // Dans l'ordre : contrôle d'abord (une chaîne rejointe est connue avant ses premiers
                // messages), messages du chat en dernier
                biased;
                Some(event) = rx.control.recv() => self.on_tap(event),
                Some(cmd) = rx.cmd.recv() => self.on_cmd(cmd),
                event = next_anon(&mut self.anon_events) => match event {
                    Some(ChatEvent::Connected(c)) => {
                        self.anon_connected = c;
                        self.publish();
                    }
                    Some(_) => {}
                    None => self.anon_events = None,
                },
                // Statut live relu dans une tâche à part : la boucle reste libre pour le chat
                _ = live_tick.tick() => self.check_lives(),
                Some(report) = live_rx.recv() => self.on_live_report(report),
                Some(report) = se_rx.recv() => self.on_se_report(report),
                Some(report) = se_active_rx.recv() => self.on_se_active_report(report),
                _ = log_tick.tick() => self.flush_chat_log(),
                _ = tick.tick() => {
                    self.update_anonymous();
                    let now = chrono::Utc::now();
                    let at = |d: chrono::Duration| (now - d).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    let closed = self.app.db.close_giveaways_before(&at(AUTO_CLOSE_AFTER), &at(PAUSED_MAX_AGE)).unwrap_or(0);
                    if closed > 0 {
                        self.app.log("giveaways", EventKind::Info, format!("{closed} giveaway(s) ouvert(s) depuis longtemps, clos"));
                    }
                    let expired = self.app.db.expire_paused_giveaways(&at(PAUSED_DRAW_GRACE), &at(PAUSED_MAX_AGE)).unwrap_or(0);
                    if expired > 0 {
                        self.app.log("giveaways", EventKind::Info, format!("{expired} giveaway(s) en pause expiré(s), clos"));
                    }
                    if closed + expired > 0 {
                        self.publish();
                    }
                    self.presence_reminders();
                    self.check_se_giveaways(None);
                    self.check_se_active();
                }
                // Messages du chat en dernier : un chat très actif ne retarde ni les commandes
                // ni les relevés (sélection dans l'ordre)
                Some(event) = rx.tap.recv() => self.on_tap(event),
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
        // Pseudos StreamElements des fiches : identifiants StreamElements à retrouver
        crate::streamelements::set_page_names(&self.app.db.se_names().unwrap_or_default());
        self.se_channels.clear();
        for e in bots::set_custom_patterns(&self.app.db.detection_rules("winner").unwrap_or_default()) {
            tracing::warn!("formulation de gagnant écartée : {e}");
        }
        self.channels = self.app.db.giveaway_channels().unwrap_or_default();
        self.games = self.app.db.giveaway_games().unwrap_or_default();
        self.gqls.clear();
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
        self.open = self.app.db.open_giveaways(None).unwrap_or_default();
        let open = &self.open;
        self.open_commands.clear();
        for g in open {
            if let Some(c) = &g.command {
                self.open_commands.entry(g.channel.clone()).or_default().push((g.id, c.to_lowercase()));
            }
        }
        let monitored: HashSet<&str> = self.channels.iter().map(|c| c.login.as_str()).collect();
        self.giveaway_only = open.iter().filter(|g| !monitored.contains(g.channel.as_str())).map(|g| g.channel.clone()).collect();
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
            .open
            .iter()
            .filter(|g| g.open && !g.reminded && !g.hidden && g.details.require_presence == Some(true))
            .cloned()
            .filter_map(|g| {
                let at = chrono::DateTime::parse_from_rfc3339(g.details.draw_at.as_deref()?).ok()?.with_timezone(&chrono::Utc);
                let left = at - now;
                (left <= PRESENCE_REMINDER && left > -chrono::Duration::minutes(1)).then(|| (g, left.num_minutes().max(0)))
            })
            .collect();
        for (g, minutes) in due {
            let _ = self.app.db.set_giveaway_reminded(g.id);
            if let Some(cached) = self.open.iter_mut().find(|o| o.id == g.id) {
                cached.reminded = true;
            }
            let when = if minutes == 0 { "maintenant".to_owned() } else { format!("dans {minutes} min") };
            self.app.log("giveaways", EventKind::Giveaway, format!("Tirage {when} sur {} : présence requise", g.channel));
            self.app.notify(Alert::Reminder, &format!("Tirage {when} sur {}", g.channel), "Présence requise au moment du tirage : reste sur le live");
        }
    }

    /// Clients Twitch pour le statut live : un par compte connecté (le suivant sert si un
    /// jeton est refusé)
    fn gqls(&mut self) -> Vec<Gql> {
        if self.gqls.is_empty() {
            self.gqls = self
                .app
                .db
                .accounts()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|a| {
                    let tokens = self.app.secrets.load(&a.user_id)?;
                    Some(Gql::new(self.app.http.clone(), tokens.access_token, Some(a.device_id)))
                })
                .collect();
        }
        self.gqls.clone()
    }

    /// Relit le statut des chaînes dues dans une tâche à part ; résultat dans `on_live_report`
    fn check_lives(&mut self) {
        self.check_lives_now(false);
    }

    /// `fresh` : toutes les chaînes, sans le cache partagé (bouton « Actualiser »)
    fn check_lives_now(&mut self, fresh: bool) {
        if self.live_busy.is_some_and(|t| t.elapsed() < LIVE_CHECK_EVERY) {
            return;
        }
        let now = Instant::now();
        let due: Vec<String> = self
            .channels
            .iter()
            .map(|c| c.login.clone())
            .filter(|c| fresh || self.live.get(c).is_none_or(|l| now.duration_since(l.at) >= LIVE_CHECK_EVERY))
            .take(crate::lives::BATCH)
            .collect();
        if due.is_empty() {
            return;
        }
        let gqls = self.gqls();
        if gqls.is_empty() {
            return;
        }
        self.live_busy = Some(Instant::now());
        let tx = self.live_tx.clone();
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = tx.send(fetch_lives(&app, &gqls, due, fresh).await).await;
        });
    }

    /// Rythme de relecture d'un giveaway StreamElements : en pause ou chaîne hors ligne, le
    /// tirage est rare (relu au retour en live et à l'annonce du bot de toute façon)
    fn se_check_every(&self, g: &GiveawayRow) -> Duration {
        let offline = self.live.get(&g.channel).is_some_and(|l| l.info.is_none());
        if g.status == "paused" || offline {
            SE_CHECK_IDLE
        } else {
            SE_CHECK_EVERY
        }
    }

    /// Relit l'état des giveaways StreamElements suivis par leur identifiant (dus, ou ceux de
    /// `channel` tout de suite) dans une tâche à part ; résultat dans `on_se_report`
    fn check_se_giveaways(&mut self, channel: Option<&str>) {
        if self.se_busy.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        let now = Instant::now();
        let rows = self.app.db.se_giveaways_to_check().unwrap_or_default();
        // Annonces en attente dont le giveaway StreamElements n'est plus suivi (clos entre-temps) :
        // rangées avec les replis habituels, sans attendre
        let orphans: Vec<String> = self.se_announced.keys().filter(|c| !rows.iter().any(|g| &g.channel == *c)).cloned().collect();
        for channel in orphans {
            self.se_announced_at.remove(&channel);
            for winner in self.se_announced.remove(&channel).unwrap_or_default() {
                let target = self.fallback_target(&channel, "streamelements", true);
                self.record_winner(&channel, target, &winner, None, true);
            }
        }
        let due: Vec<(i64, String, String)> = rows
            .into_iter()
            // Annonce du bot en attente (lecture précédente échouée) : relu à chaque passage
            .filter(|g| {
                channel.is_some_and(|c| c == g.channel)
                    || self.se_announced.contains_key(&g.channel)
                    || self.se_checked.get(&g.id).is_none_or(|t| now.duration_since(*t) >= self.se_check_every(g))
            })
            // Page StreamElements du lien (partenaire, autre nom) quand ce n'est pas celle de la
            // chaîne : son identifiant est cherché par la page (clé « page:<nom> »)
            .filter_map(|g| {
                let url = g.url.as_deref()?;
                let se_id = se_giveaway_id(url)?.to_owned();
                let key = match crate::streamelements::parse_giveaway_url(url) {
                    Some((page, _)) if page != crate::streamelements::page_name(&g.channel) => format!("page:{page}"),
                    _ => g.channel.clone(),
                };
                Some((g.id, key, se_id))
            })
            .take(20)
            .collect();
        if due.is_empty() {
            return;
        }
        for (id, _, _) in &due {
            self.se_checked.insert(*id, now);
        }
        self.se_busy = Some(now);
        let known = self.se_channels.clone();
        let http = self.app.http.clone();
        let tx = self.se_tx.clone();
        tauri::async_runtime::spawn(async move {
            let mut channel_ids: HashMap<String, Option<String>> = HashMap::new();
            let mut results = vec![];
            for (id, channel, se_id) in due {
                let se_channel = match known.get(&channel).or_else(|| channel_ids.get(&channel)) {
                    Some(c) => c.clone(),
                    None => match match channel.strip_prefix("page:") {
                        Some(page) => crate::streamelements::channel_id_of_page(&http, page).await,
                        None => crate::streamelements::channel_id(&http, &channel).await,
                    } {
                        Ok(c) => {
                            channel_ids.insert(channel.clone(), c.clone());
                            c
                        }
                        Err(e) => {
                            tracing::debug!("StreamElements : chaîne {channel} : {e}");
                            results.push((id, Err(())));
                            continue;
                        }
                    },
                };
                let outcome = match se_channel {
                    Some(c) => crate::streamelements::giveaway_outcome(&http, &c, &se_id).await.map_err(|e| {
                        tracing::debug!("StreamElements : giveaway {se_id} de {channel} : {e}");
                    }),
                    None => Ok(None),
                };
                results.push((id, outcome));
            }
            let _ = tx.send(SeReport { results, channel_ids: channel_ids.into_iter().collect() }).await;
        });
    }

    /// Chaînes surveillées en live dans une catégorie suivie : giveaway StreamElements en cours
    /// cherché (au plus toutes les SE_ACTIVE_EVERY par chaîne), dans une tâche à part ; résultat
    /// dans `on_se_active_report`
    fn check_se_active(&mut self) {
        if self.se_active_busy.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        let now = Instant::now();
        let due: Vec<String> = self
            .channels
            .iter()
            .map(|c| c.login.clone())
            .filter(|c| self.live.get(c).is_some_and(|l| l.info.is_some()) && self.in_category(c))
            // Chaîne sans StreamElements : rien à chercher
            .filter(|c| !matches!(self.se_channels.get(c), Some(None)))
            .filter(|c| self.se_active_checked.get(c).is_none_or(|t| now.duration_since(*t) >= SE_ACTIVE_EVERY))
            .take(10)
            .collect();
        if due.is_empty() {
            return;
        }
        for c in &due {
            self.se_active_checked.insert(c.clone(), now);
        }
        self.se_active_busy = Some(now);
        let known = self.se_channels.clone();
        let app = self.app.clone();
        let tx = self.se_active_tx.clone();
        tauri::async_runtime::spawn(async move {
            let mut channel_ids = vec![];
            let mut results = vec![];
            for channel in due {
                let se_id = match known.get(&channel) {
                    Some(id) => id.clone(),
                    None => match crate::streamelements::channel_id(&app.http, &channel).await {
                        Ok(id) => {
                            channel_ids.push((channel.clone(), id.clone()));
                            id
                        }
                        Err(e) => {
                            tracing::debug!("StreamElements : chaîne {channel} : {e}");
                            continue;
                        }
                    },
                };
                let Some(se_id) = se_id else { continue };
                // Cache partagé avec la page StreamElements (même giveaway pour tous les comptes)
                let http = &app.http;
                let since = crate::lives::since(SE_ACTIVE_EVERY);
                match app.lives.se_giveaway.get(&se_id, since, || crate::streamelements::active_giveaway(http, &se_id)).await {
                    Ok(Some(g)) => results.push((channel, g)),
                    Ok(None) => {}
                    Err(e) => tracing::debug!("StreamElements : giveaway en cours de {channel} : {e}"),
                }
            }
            let _ = tx.send(SeActiveReport { results, channel_ids }).await;
        });
    }

    /// Giveaway StreamElements en cours sur une chaîne surveillée : créé s'il n'est pas encore
    /// connu (jamais rouvert : un giveaway clos à la main le reste)
    fn on_se_active_report(&mut self, report: SeActiveReport) {
        self.se_active_busy = None;
        self.se_channels.extend(report.channel_ids);
        for (channel, g) in report.results {
            if !self.monitored(&channel) || !self.detection_active(&channel) {
                continue;
            }
            let url = format!("https://streamelements.com/{}/giveaway/{}", crate::streamelements::page_name(&channel), g.id);
            // Déjà connu : par son lien, ou par l'identifiant StreamElements d'un giveaway ouvert
            let open = self.open_giveaways(&channel);
            let paused = self.app.db.paused_giveaways(Some(&channel)).unwrap_or_default();
            let same_id = |r: &GiveawayRow| r.url.as_deref().and_then(se_giveaway_id).is_some_and(|id| id.eq_ignore_ascii_case(&g.id));
            if open.iter().chain(paused.iter()).any(same_id) || !matches!(self.app.db.giveaway_by_url(&channel, &url), Ok(None)) {
                continue;
            }
            self.app.log("giveaways", EventKind::Giveaway, format!("Giveaway StreamElements en cours trouvé sur {channel}"));
            if let Some(id) = self.create_se_giveaway(&channel, &url, g.command.as_deref()) {
                if let Some(max) = g.max_tickets {
                    let _ = self.app.db.set_ticket_max_if_unknown(id, max);
                }
                self.publish();
            }
        }
    }

    /// Giveaway StreamElements tiré (un gagnant, même s'il n'est pas encore terminé) : gagnant noté
    /// et giveaway clos ; terminé ou annulé sans gagnant : clos ; participations fermées sans
    /// gagnant (tirage à venir) : reste ouvert. Annonce du bot sans giveaway tiré : elle
    /// concernait un autre tirage
    fn on_se_report(&mut self, report: SeReport) {
        self.se_busy = None;
        self.se_channels.extend(report.channel_ids);
        let mut drawn_channels: HashSet<String> = HashSet::new();
        let mut checked_channels: HashSet<String> = HashSet::new();
        let mut failed_channels: HashSet<String> = HashSet::new();
        for (id, outcome) in report.results {
            let Ok(Some(g)) = self.app.db.giveaway(id) else { continue };
            let outcome = match outcome {
                Ok(o) => o,
                Err(()) => {
                    failed_channels.insert(g.channel.clone());
                    continue;
                }
            };
            checked_channels.insert(g.channel.clone());
            let Some(outcome) = outcome else { continue };
            if g.status == "closed" {
                continue;
            }
            // Titre et description du giveaway StreamElements : cadeaux du catalogue repérés dedans
            self.gifts_from_se_giveaway(&g, &format!("{}\n{}", outcome.title, outcome.description));
            // Tous les gagnants tirés, dans l'ordre (déjà notés : ignorés)
            if !outcome.winners.is_empty() {
                drawn_channels.insert(g.channel.clone());
                let channel = g.channel.clone();
                let announced: Vec<String> = self.se_announced.get(&channel).cloned().unwrap_or_default();
                for (n, winner) in outcome.winners.iter().enumerate() {
                    let Ok(Some(row)) = self.app.db.giveaway(id) else { break };
                    if row.winners.iter().any(|w| w.eq_ignore_ascii_case(winner)) {
                        continue;
                    }
                    // Heure du tirage donnée par StreamElements : la réponse automatique vise le tirage
                    // plus son délai (voir win_reply) ; heure inconnue : seulement si le bot vient
                    // de l'annoncer
                    let drawn_at = outcome.drawn_at.get(n).cloned().flatten();
                    let just_announced = announced.iter().any(|w| w.eq_ignore_ascii_case(winner));
                    self.app.log("giveaways", EventKind::Info, format!("Giveaway StreamElements de {channel} tiré : {winner}"));
                    self.record_winner_at(&channel, Some(row), winner, None, drawn_at.is_some() || just_announced, drawn_at.as_deref());
                }
            } else if outcome.finished() {
                let _ = self.app.db.close_giveaway(g.id);
                self.app.log("giveaways", EventKind::Info, format!("Giveaway StreamElements de {} terminé sans gagnant", g.channel));
                self.publish();
            }
        }
        // Annonces du bot StreamElements : giveaway StreamElements tiré, ses gagnants viennent d'être
        // notés ; pas tiré, c'était un autre tirage (chaque gagnant, avec les replis habituels).
        // Lecture échouée : annonces gardées pour la relecture suivante, sauf après SE_ANNOUNCE_WAIT
        let gave_up: Vec<String> = failed_channels
            .iter()
            .filter(|c| !checked_channels.contains(*c) && self.se_announced_at.get(*c).is_some_and(|t| t.elapsed() >= SE_ANNOUNCE_WAIT))
            .cloned()
            .collect();
        for channel in &gave_up {
            tracing::warn!("StreamElements illisible pour {channel} : gagnants annoncés rangés sans vérification");
        }
        let resolved = checked_channels.iter().filter(|c| !failed_channels.contains(*c)).cloned().chain(gave_up);
        for channel in resolved.collect::<Vec<_>>() {
            self.se_announced_at.remove(&channel);
            let Some(winners) = self.se_announced.remove(&channel) else { continue };
            if drawn_channels.contains(&channel) {
                continue;
            }
            for winner in winners {
                let target = self.fallback_target(&channel, "streamelements", true);
                self.record_winner(&channel, target, &winner, None, true);
            }
        }
    }

    /// Giveaway d'un gagnant annoncé par `bot` : tiré il y a moins de MORE_WINNERS_WITHIN sans
    /// nouveau giveaway depuis (plusieurs gagnants), sinon ouvert, sinon en pause (même giveaway,
    /// tirage d'un live suivant), sinon clos récemment sans gagnant (fin du live juste avant le tirage).
    /// `skip_se` : le giveaway StreamElements de la chaîne n'est pas tiré, il est écarté
    fn fallback_target(&self, channel: &str, bot: &str, skip_se: bool) -> Option<GiveawayRow> {
        let keep = |rows: Vec<GiveawayRow>| -> Vec<GiveawayRow> { rows.into_iter().filter(|g| !skip_se || g.kind != "streamelements").collect() };
        // Tiré il y a moins de MORE_WINNERS_WITHIN sans nouveau giveaway depuis : un autre gagnant
        // du même tirage (plusieurs gagnants). Passé avant les ouverts : sinon un 2e giveaway
        // ouvert de la chaîne (détecté avant ce tirage) recevrait ce gagnant et serait clos à tort
        let since = (chrono::Utc::now() - MORE_WINNERS_WITHIN).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let drawn = keep(self.app.db.recent_drawn_giveaway(channel, &since).ok().flatten().into_iter().collect());
        if let Some(g) = winner_target(&drawn, bot) {
            return Some(g.clone());
        }
        let open = keep(self.open_giveaways(channel));
        winner_target(&open, bot)
            .cloned()
            .or_else(|| {
                let paused = keep(self.app.db.paused_giveaways(Some(channel)).unwrap_or_default());
                winner_target(&paused, bot).cloned()
            })
            .or_else(|| {
                let since = (chrono::Utc::now() - chrono::Duration::hours(RECENT_UNDRAWN_HOURS)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                let recent = keep(self.app.db.recent_undrawn_giveaways(channel, &since).unwrap_or_default());
                winner_target(&recent, bot).cloned()
            })
    }

    fn on_live_report(&mut self, report: LiveReport) {
        self.live_busy = None;
        if report.unauthorized {
            // Jeton renouvelé entre-temps par le superviseur du compte : on le relira
            self.gqls.clear();
        }
        let mut changed = false;
        let mut live_changed = false;
        for (channel, info) in report.results {
            let was_live = self.live.get(&channel).is_some_and(|l| l.info.is_some());
            if was_live && info.is_none() {
                // Fin du live : le tirage a eu lieu ou n'aura pas lieu, sauf giveaway sur plusieurs
                // lives (heure de tirage à venir, saisi à la main, StreamElements) : en pause
                let out = self.app.db.close_channel_giveaways(&channel).unwrap_or_default();
                if out.closed > 0 {
                    self.app.log("giveaways", EventKind::Info, format!("{channel} est hors ligne : {} giveaway(s) clos", out.closed));
                }
                if out.paused > 0 {
                    self.app.log("giveaways", EventKind::Info, format!("{channel} est hors ligne : {} giveaway(s) en pause", out.paused));
                }
                changed |= out.closed + out.paused > 0;
            }
            // Hors ligne sans fin de live vue (application fermée à ce moment, ou giveaway saisi
            // hors live) : giveaways sur plusieurs lives en pause plutôt que clos à l'ancienneté
            if info.is_none() {
                let paused = self.app.db.pause_offline_giveaways(&channel).unwrap_or(0);
                if paused > 0 {
                    self.app.log("giveaways", EventKind::Info, format!("{channel} est hors ligne : {paused} giveaway(s) en pause"));
                    changed = true;
                }
            }
            if !was_live && info.is_some() {
                // De retour en live : giveaways en pause repris, sans nouvelle alerte
                for g in self.app.db.resume_channel_giveaways(&channel, &now_rfc3339()).unwrap_or_default() {
                    // StreamElements relu au prochain passage (il l'était au rythme lent)
                    self.se_checked.remove(&g.id);
                    let what = g.command.as_deref().map(|c| format!(" ({c})")).unwrap_or_default();
                    self.app.log("giveaways", EventKind::Info, format!("Giveaway de {channel}{what} repris (live)"));
                    changed = true;
                }
            }
            if was_live != info.is_some() || self.live.get(&channel).map(|l| &l.info) != Some(&info) {
                live_changed = true;
            }
            self.live.insert(channel, LiveState { info, at: Instant::now() });
        }
        // Cadeaux annoncés dans le titre, la description ou le message épinglé : rattachés aux
        // giveaways ouverts
        changed |= self.gifts_from_live();
        // Giveaway clos, mis en pause ou repris : vue complète (et synchronisation) ; statut live,
        // spectateurs, titre seuls : chaînes seulement (les giveaways n'en dépendent pas)
        if changed {
            self.publish();
        } else if live_changed {
            self.publish_channels();
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
                    } else {
                        // Chaîne lue pour un giveaway ouvert : gagnant seulement, pas de détection
                        self.on_announcement(&channel, &user.to_lowercase(), &text, moderator, false);
                    }
                }
            }
        }
    }

    fn on_cmd(&mut self, cmd: MonitorCmd) {
        match cmd {
            MonitorCmd::Reload => self.reload(),
            MonitorCmd::Republish => self.publish(),
            MonitorCmd::Dismiss(id) => {
                let _ = self.app.db.set_giveaway_handled(id);
                self.publish();
            }
            MonitorCmd::Close(id) => {
                let _ = self.app.db.close_giveaway(id);
                self.publish();
            }
            MonitorCmd::Refresh => {
                self.live_busy = None;
                self.check_lives_now(true);
                self.se_busy = None;
                self.se_checked.clear();
                self.check_se_giveaways(None);
                self.publish();
            }
            MonitorCmd::Reopen(id) => {
                // Chaîne en live (ou statut pas encore relu) : ouvert ; hors ligne : en pause
                let live = self.app.db.giveaway(id).ok().flatten().is_none_or(|g| self.live.get(&g.channel).is_none_or(|l| l.info.is_some()));
                if self.app.db.reopen_closed_giveaway(id, live, &now_rfc3339()).unwrap_or(false) {
                    self.app.log("giveaways", EventKind::Info, if live { "Giveaway rouvert" } else { "Giveaway rouvert, en pause jusqu'au prochain live" });
                    self.publish();
                }
            }
            MonitorCmd::WinnersCleared(id) => {
                let live = self.app.db.giveaway(id).ok().flatten().is_none_or(|g| self.live.get(&g.channel).is_none_or(|l| l.info.is_some()));
                let _ = self.app.db.restart_giveaway(id, live, &now_rfc3339());
                self.reload();
                self.app.sync.poke();
            }
            MonitorCmd::Delete(id) => {
                let _ = self.app.db.delete_giveaway(id);
                self.publish();
            }
            MonitorCmd::ManualWinner { channel, winner, giveaway_id } => self.on_manual_winner(&channel, &winner, giveaway_id),
            MonitorCmd::DismissMention(id) => {
                self.mentions.retain(|m| m.id != id);
                let _ = self.app.db.delete_mention(id as i64);
                self.publish();
            }
        }
    }

    /// Giveaways ouverts d'une chaîne (cache relu à chaque publication), les plus récents d'abord
    fn open_giveaways(&self, channel: &str) -> Vec<GiveawayRow> {
        self.open.iter().filter(|g| g.channel == channel).cloned().collect()
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
            && bots::bot_events(&author, text, moderator || author == channel).iter().any(|e| matches!(e, BotEvent::Winner { .. })))
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
        // Relus en base : giveaway tout juste enregistré pas encore dans la liste des ouverts
        for g in self.app.db.open_giveaways(Some(channel)).unwrap_or_default() {
            added += self.app.db.attach_gifts(g.id, &found).unwrap_or(0);
        }
        if added > 0 {
            let names: Vec<&str> = gifts.iter().filter(|g| found.contains(&g.id)).map(|g| g.title.as_str()).collect();
            self.app.log("giveaways", EventKind::Giveaway, format!("Cadeaux repérés dans la réponse du bot sur {channel} : {}", names.join(", ")));
            self.app.sync.poke();
        }
    }

    /// Cadeaux repérés dans le titre et la description d'un giveaway StreamElements, ajoutés à
    /// ce giveaway (ceux déjà rattachés restent)
    fn gifts_from_se_giveaway(&mut self, g: &GiveawayRow, text: &str) {
        let gifts = self.app.db.gifts().unwrap_or_default();
        let found = crate::db::gifts_in_text(&gifts, text);
        if found.is_empty() || self.app.db.attach_gifts(g.id, &found).unwrap_or(0) == 0 {
            return;
        }
        let names: Vec<&str> = gifts.iter().filter(|x| found.contains(&x.id)).map(|x| x.title.as_str()).collect();
        self.app.log("giveaways", EventKind::Giveaway, format!("Cadeaux repérés dans le giveaway StreamElements de {} : {}", g.channel, names.join(", ")));
        self.publish();
    }

    /// Nouveau giveaway sur une chaîne : cadeaux des réponses de bot déjà connues, du titre du
    /// live, de la description et du message épinglé
    fn gifts_from_known_replies(&mut self, channel: &str) {
        for r in self.app.db.bot_replies(channel).unwrap_or_default() {
            self.gifts_from_reply(channel, &r.message);
        }
        self.gifts_from_live();
    }

    /// Cadeaux repérés dans le titre du live, la description et le message épinglé, ajoutés
    /// aux giveaways ouverts des chaînes en live (ceux déjà rattachés restent) ; chaque giveaway
    /// n'est repassé au crible que si ces textes ou le catalogue ont changé. Vrai si un cadeau a
    /// été ajouté
    fn gifts_from_live(&mut self) -> bool {
        use std::hash::{Hash, Hasher};
        let open = self.app.db.open_giveaways(None).unwrap_or_default();
        self.gifts_scanned.retain(|id, _| open.iter().any(|g| g.id == *id));
        if open.is_empty() {
            return false;
        }
        let gifts = self.app.db.gifts().unwrap_or_default();
        let mut catalog = std::collections::hash_map::DefaultHasher::new();
        for g in &gifts {
            (g.id, &g.title, &g.keywords).hash(&mut catalog);
        }
        let mut todo: Vec<(GiveawayRow, String, u64)> = Vec::new();
        for g in open {
            let Some(info) = self.live.get(&g.channel).and_then(|l| l.info.as_ref()) else { continue };
            let insight = self.app.insights.lock().unwrap().get(&g.channel).cloned().unwrap_or_default();
            let text = [Some(info.title.as_str()), insight.description.as_deref(), insight.pinned_message.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n");
            let mut h = catalog.clone();
            text.hash(&mut h);
            let print = h.finish();
            if self.gifts_scanned.get(&g.id) != Some(&print) {
                todo.push((g, text, print));
            }
        }
        let mut any = false;
        for (g, text, print) in todo {
            let found = crate::db::gifts_in_text(&gifts, &text);
            let added = if found.is_empty() { 0 } else { self.app.db.attach_gifts(g.id, &found).unwrap_or(0) };
            self.gifts_scanned.insert(g.id, print);
            if added > 0 {
                let names: Vec<&str> = gifts.iter().filter(|x| found.contains(&x.id)).map(|x| x.title.as_str()).collect();
                self.app.log("giveaways", EventKind::Giveaway, format!("Cadeaux repérés dans le live de {} : {}", g.channel, names.join(", ")));
                any = true;
            }
        }
        if any {
            self.app.sync.poke();
        }
        any
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

    /// Annonces des bots, des modérateurs et du streamer (gagnant, lien StreamElements, tickets) ;
    /// vrai si le message est traité (bot, ou gagnant annoncé). `monitored` : un lien
    /// StreamElements crée un giveaway (chaîne surveillée seulement)
    fn on_announcement(&mut self, channel: &str, user_l: &str, text: &str, moderator: bool, monitored: bool) -> bool {
        let privileged = moderator || user_l == channel;
        // Annonce du gagnant par un modérateur (bot ou humain) ou le streamer : badges du message,
        // qu'un spectateur ne peut pas imiter
        if privileged && !bots::is_bot_like(user_l) {
            if let Some(BotEvent::Winner { bot, winner, chance }) = bots::moderator_winner(user_l, text) {
                self.on_winner(channel, &bot, &winner, chance.as_deref());
                return true;
            }
        }
        if !bots::is_bot_like(user_l) {
            return false;
        }
        let events = bots::bot_events(user_l, text, privileged);
        let announces_winner = events.iter().any(|e| matches!(e, BotEvent::Winner { .. }));
        for event in events {
            match event {
                BotEvent::LingoEchoUrl(url) if monitored => self.on_lingo_echo_url(channel, &url, announces_winner),
                BotEvent::LingoEchoUrl(_) => {}
                BotEvent::Winner { bot, winner, chance } => self.on_winner(channel, &bot, &winner, chance.as_deref()),
                BotEvent::StreamElementsUrl(url) if monitored => self.on_streamelements_url(channel, &url),
                BotEvent::StreamElementsUrl(_) => {}
                // « @pseudo got N tickets! (N/MAX) » : le maximum par personne du giveaway en cours
                BotEvent::TicketInfo { max, .. } => {
                    // Seulement un giveaway StreamElements (sinon par tickets) : un giveaway par
                    // commande ne reçoit pas de réglage de tickets
                    let open = self.open_giveaways(channel);
                    let target = open.iter().find(|g| g.kind == "streamelements").or_else(|| open.iter().find(|g| g.kind == "ticket"));
                    if let Some(g) = target {
                        if self.app.db.set_ticket_max_if_unknown(g.id, max).unwrap_or(false) {
                            self.publish();
                        }
                    }
                }
                BotEvent::TicketPurchase { .. } => {}
            }
        }
        true
    }

    fn on_message(&mut self, channel: &str, user: &str, text: &str, moderator: bool) {
        let user_l = user.to_lowercase();
        if self.on_announcement(channel, &user_l, text, moderator, true) {
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
                if !g.hidden {
                    self.app.notify(Alert::Giveaway, &format!("Commande trouvée sur {channel}"), &format!("Le giveaway attend {command}"));
                }
                self.publish();
                schedule_auto_participation(self.app.clone(), g.id);
            }
            return;
        }
        // Même commande en pause ou close sans gagnant depuis peu (giveaway sur plusieurs lives) :
        // rouvert plutôt que dupliqué
        let since = (chrono::Utc::now() - REOPEN_WITHIN).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if let Ok(Some(g)) = self.app.db.reopenable_command_giveaway(channel, &command, &since) {
            if let Err(e) = self.app.db.reopen_giveaway(g.id, &now_rfc3339()) {
                tracing::error!("giveaway non rouvert : {e}");
                return;
            }
            self.app.log("giveaways", EventKind::Giveaway, format!("Giveaway de {channel} repris : {command} de nouveau dans le chat"));
            if !g.hidden {
                self.app.notify(Alert::Giveaway, &format!("Giveaway repris sur {channel}"), &format!("Commande {command} de nouveau reprise par le chat"));
            }
            self.publish();
            // Comptes déjà inscrits : pas de nouvel envoi
            schedule_auto_participation(self.app.clone(), g.id);
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

    /// Lien Lingo Echo posté par son bot (participation par commande dans le chat) : noté sur le
    /// giveaway par commande en cours de la chaîne, sinon nouveau giveaway par commande qui
    /// attend sa commande (remplie par la détection). Jamais créé depuis l'annonce du gagnant
    /// ni juste après un tirage (le lien y est repris)
    fn on_lingo_echo_url(&mut self, channel: &str, url: &str, announces_winner: bool) {
        let open: Vec<GiveawayRow> = self.open_giveaways(channel).into_iter().filter(|g| matches!(g.kind.as_str(), "command" | "ticket")).collect();
        if open.iter().any(|g| g.url.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(url))) {
            return;
        }
        if let Some(g) = open.iter().find(|g| g.url.as_deref().unwrap_or("").is_empty()) {
            if self.app.db.set_giveaway_url_if_unknown(g.id, url).unwrap_or(false) {
                self.publish();
            }
            return;
        }
        if announces_winner || !open.is_empty() {
            return;
        }
        let since = (chrono::Utc::now() - MORE_WINNERS_WITHIN).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if self.app.db.recent_drawn_giveaway(channel, &since).ok().flatten().is_some() {
            return;
        }
        let id = match self.app.db.insert_giveaway(&NewGiveaway {
            channel: channel.to_owned(),
            kind: "command".into(),
            command: None,
            url: Some(url.to_owned()),
            evidence: None,
            detected_at: now_rfc3339(),
        }) {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("giveaway Lingo Echo non enregistré : {e}");
                return;
            }
        };
        let _ = self.app.db.apply_streamer_conditions(id);
        self.app.log("giveaways", EventKind::Giveaway, format!("Giveaway Lingo Echo sur {channel}"));
        self.app.notify(Alert::Giveaway, &format!("Giveaway Lingo Echo sur {channel}"), "Commande à taper dans le chat : elle sera remplie dès que la détection la repère");
        self.gifts_from_known_replies(channel);
        self.publish();
    }

    fn on_streamelements_url(&mut self, channel: &str, url: &str) {
        let open = self.open_giveaways(channel);
        if open.iter().any(|g| g.url.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(url))) {
            return;
        }
        // Lien déjà vu (giveaway clos à la fin d'un live précédent) : rouvert, pas recréé ; tiré : rien
        match self.app.db.giveaway_by_url(channel, url) {
            Ok(Some(g)) if g.winner.is_some() => return,
            Ok(Some(g)) => {
                if let Err(e) = self.app.db.reopen_giveaway(g.id, &now_rfc3339()) {
                    tracing::error!("giveaway StreamElements non rouvert : {e}");
                    return;
                }
                self.app.log("giveaways", EventKind::Giveaway, format!("Giveaway StreamElements sur {channel} rouvert (lien reposté)"));
                self.publish();
                return;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!("giveaway StreamElements : {e}");
                return;
            }
        }
        self.create_se_giveaway(channel, url, None);
    }

    /// Nouveau giveaway StreamElements (lien posté par un bot, ou trouvé en cours) : conditions
    /// de la fiche, chaîne ajoutée à la page StreamElements, notification
    fn create_se_giveaway(&mut self, channel: &str, url: &str, command: Option<&str>) -> Option<i64> {
        let id = match self.app.db.insert_giveaway(&NewGiveaway {
            channel: channel.to_owned(),
            kind: "streamelements".into(),
            command: command.map(str::to_owned),
            url: Some(url.to_owned()),
            evidence: None,
            detected_at: now_rfc3339(),
        }) {
            Ok(id) => {
                let _ = self.app.db.apply_streamer_conditions(id);
                id
            }
            Err(e) => {
                tracing::error!("giveaway StreamElements non enregistré : {e}");
                return None;
            }
        };
        self.app.log("giveaways", EventKind::Giveaway, format!("Giveaway StreamElements sur {channel}"));
        let app = self.app.clone();
        let login = channel.to_owned();
        let channel_id = self.channels.iter().find(|c| c.login == channel).map(|c| c.channel_id.clone());
        tauri::async_runtime::spawn(async move { ensure_channel_in_list(&app, &login, channel_id, crate::db::ChannelList::Se).await });
        self.app.notify(Alert::Giveaway, &format!("Giveaway StreamElements sur {channel}"), "Tickets à acheter avec tes points StreamElements");
        self.gifts_from_known_replies(channel);
        self.publish();
        Some(id)
    }

    fn on_manual_winner(&mut self, channel: &str, winner: &str, giveaway_id: Option<i64>) {
        let target = manual_target(&self.app.db, channel, giveaway_id);
        // Désigné à la main après coup : pas de réponse automatique
        self.record_winner(channel, target, winner, None, false);
    }

    fn on_winner(&mut self, channel: &str, bot: &str, winner: &str, chance: Option<&str>) {
        // Annonce de StreamElements alors qu'un giveaway StreamElements suivi par son identifiant
        // est ouvert ou en pause : StreamElements dit lequel est tiré (vérifié tout de suite)
        if bot == "streamelements" {
            let followed = self
                .app
                .db
                .se_giveaways_to_check()
                .unwrap_or_default()
                .into_iter()
                .any(|g| g.channel == channel && g.url.as_deref().and_then(se_giveaway_id).is_some());
            if followed {
                // Plusieurs gagnants annoncés d'affilée : tous gardés, dans l'ordre
                let winner = winner.to_lowercase();
                let pending = self.se_announced.entry(channel.to_owned()).or_default();
                if !pending.contains(&winner) {
                    pending.push(winner);
                }
                self.se_announced_at.entry(channel.to_owned()).or_insert_with(Instant::now);
                self.se_busy = None;
                self.check_se_giveaways(Some(channel));
                return;
            }
        }
        let target = self.fallback_target(channel, bot, false);
        self.record_winner(channel, target, winner, chance, true);
    }

    /// `announced` : gagnant annoncé dans le chat à l'instant (réponse automatique possible)
    fn record_winner(&mut self, channel: &str, target: Option<GiveawayRow>, winner: &str, chance: Option<&str>, announced: bool) {
        self.record_winner_at(channel, target, winner, chance, announced, None);
    }

    /// `drawn_at` : heure du tirage si on la connaît (StreamElements), sinon maintenant
    fn record_winner_at(&mut self, channel: &str, target: Option<GiveawayRow>, winner: &str, chance: Option<&str>, announced: bool, drawn_at: Option<&str>) {
        let target = target.as_ref();
        // Heure normalisée comme les autres (secondes, UTC)
        let drawn = drawn_at.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&chrono::Utc));
        let at = drawn.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)).unwrap_or_else(now_rfc3339);
        // Ajouté aux gagnants du giveaway ; déjà gagnant (annonce répétée) : rien de neuf
        if let Some(g) = target {
            if !self.app.db.set_giveaway_winner(g.id, winner, chance, &at).unwrap_or(false) {
                return;
            }
        }
        let ours = self.accounts.iter().find(|(_, l)| l.eq_ignore_ascii_case(winner)).map(|(_, l)| l.clone());
        match ours {
            Some(login) => {
                self.app.log(&login, EventKind::Giveaway, format!("Tu as gagné le giveaway de {channel} !"));
                let body = format!("{login} a gagné le giveaway de {channel}");
                match self.win_reply(channel, target, &login, announced, drawn) {
                    Some((id, text, delay)) => {
                        let body = format!("{body}. Réponse dans {delay} s : « {text} »");
                        let cancel = crate::win_reply::cancel_url(id);
                        self.app.notify_full(Alert::Win, "Tu as gagné !", &body, Alert::Win.page(), Some(("Annuler la réponse", &cancel)));
                    }
                    None => self.app.notify(Alert::Win, "Tu as gagné !", &body),
                }
            }
            None if target.is_some() => {
                self.app.log("giveaways", EventKind::Info, format!("Tirage sur {channel} : {winner}"));
            }
            None => {}
        }
        self.publish();
    }

    /// Réponse au gain programmée (numéro, phrase, délai) : gagnant annoncé dans le chat, présence
    /// au tirage requise, phrase définie dans la fiche du streamer
    /// `drawn` : heure du tirage (StreamElements), sinon il vient d'être annoncé. La réponse part
    /// au tirage plus le délai de la fiche ; appris trop tard (au-delà de WIN_REPLY_SLACK après
    /// ce moment) : pas de réponse
    fn win_reply(&self, channel: &str, target: Option<&GiveawayRow>, login: &str, announced: bool, drawn: Option<chrono::DateTime<chrono::Utc>>) -> Option<(u64, String, u64)> {
        let target = target?;
        // Même gagnant déjà noté (annonce répétée par le bot) : une seule réponse
        if !announced || target.details.require_presence != Some(true) || target.winners.iter().any(|w| w.eq_ignore_ascii_case(login)) {
            return None;
        }
        let streamer = self.app.db.streamer(channel).ok().flatten()?;
        let text = streamer.win_reply.filter(|t| !t.trim().is_empty())?;
        let wanted = streamer.win_reply_delay.unwrap_or(crate::db::WIN_REPLY_DELAY).clamp(0, 600);
        let delay = reply_delay(wanted, drawn.map(|t| (chrono::Utc::now() - t).num_seconds()))?;
        let account_id = self.accounts.iter().find(|(_, l)| l == login).map(|(id, _)| *id)?;
        let id = crate::win_reply::schedule(&self.app, account_id, login, channel, &text, delay);
        Some((id, text, delay))
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
                reader: self.reader(&c.login).map(|src| match src {
                    ANONYMOUS => "anonyme".to_owned(),
                    id => self.account_login(id).unwrap_or("compte").to_owned(),
                }),
                live_checked: self.live.contains_key(&c.login),
                live: self.live.get(&c.login).and_then(|l| l.info.clone()),
                in_category: self.live.get(&c.login).is_some_and(|l| l.info.is_some()) && self.in_category(&c.login),
                profile_image: self.app.lives.avatar(&c.login),
            })
            .collect()
    }

    fn publish_channels(&mut self) {
        let channels = self.channel_views();
        self.view.write().unwrap().channels = channels;
        self.view.write().unwrap().anonymous_connected = self.anon_connected;
        self.app.touch_domain(crate::app::Domain::Giveaways);
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
        self.app.touch_domain(crate::app::Domain::Giveaways);
        // Changement détecté ici (giveaway, gagnant, clôture, pause, reprise) : envoyé au serveur
        // de synchronisation dans les 2 s, comme une modification faite à la main (sans changement
        // local, le passage n'envoie rien)
        self.app.sync.poke();
    }
}

fn build_views(app: &App, accounts: &[(i64, String)]) -> Vec<GiveawayView> {
    rows_to_views(app, app.db.giveaways(RECENT_GIVEAWAYS).unwrap_or_default(), accounts)
}

fn rows_to_views(app: &App, rows: Vec<GiveawayRow>, accounts: &[(i64, String)]) -> Vec<GiveawayView> {
    // Réponses au gain des fiches de streamers : une lecture pour tous les giveaways
    let replies: HashMap<String, (String, i64)> = app
        .db
        .streamers()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| {
            let text = s.win_reply.filter(|t| !t.trim().is_empty())?;
            Some((s.login, (text, s.win_reply_delay.unwrap_or(crate::db::WIN_REPLY_DELAY))))
        })
        .collect();
    // Tickets StreamElements de tous ces giveaways : une seule lecture
    let se_ids: Vec<&str> = rows.iter().filter_map(|r| r.url.as_deref().and_then(se_giveaway_id)).collect();
    let tickets = app.db.se_tickets_of(&se_ids).unwrap_or_default();
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
                    let Some(tickets) = tickets.get(&(*account_id, se_id.to_owned())).copied().filter(|t| *t > 0) else { continue };
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
            let reply = replies.get(&row.channel).filter(|_| row.details.require_presence == Some(true));
            GiveawayView {
                win_reply: reply.map(|(t, _)| t.clone()),
                win_reply_delay: reply.map(|(_, d)| *d),
                evidence: row.evidence.as_deref().and_then(|e| serde_json::from_str(e).ok()),
                entries_view,
                winner_ours: row
                    .winners
                    .iter()
                    .find_map(|w| accounts.iter().find(|(_, l)| l.eq_ignore_ascii_case(w)).map(|(_, l)| l.clone())),
                row,
            }
        })
        .collect()
}

/// Page des giveaways passés (terminés), les plus récents d'abord
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PastPage {
    pub giveaways: Vec<GiveawayView>,
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
}

pub fn past_views(app: &App, filter: &crate::db::PastFilter, page: usize, per_page: usize) -> PastPage {
    let per_page = per_page.clamp(5, 100);
    let accounts: Vec<(i64, String)> =
        app.db.accounts().unwrap_or_default().into_iter().map(|a| (a.id, a.login.to_lowercase())).collect();
    let (rows, total) = app.db.closed_giveaways(filter, per_page, page.saturating_sub(1) * per_page).unwrap_or_else(|e| {
        tracing::warn!("giveaways passés : {e}");
        (vec![], 0)
    });
    PastPage { giveaways: rows_to_views(app, rows, &accounts), total, page: page.max(1), per_page }
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
    app.touch_domain(crate::app::Domain::Giveaways);
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

/// Récompense du giveaway (par son nom, sans tenir compte de la casse), achetable telle quelle
async fn giveaway_reward(app: &App, giveaway: &GiveawayRow) -> Result<crate::twitch::gql::ChannelReward, String> {
    let name = giveaway.details.reward_name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let Some(name) = name else {
        return Err("Récompense à acheter non renseignée : modifie la fiche du giveaway".into());
    };
    let rewards = crate::twitch::gql::channel_rewards(&app.http, &giveaway.channel)
        .await
        .map_err(|e| format!("récompenses de {} : {e}", giveaway.channel))?;
    let reward = rewards
        .into_iter()
        .find(|r| r.title.trim().eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("{} ne propose plus de récompense « {name} »", giveaway.channel))?;
    if reward.channel_id.is_empty() {
        return Err(format!("chaîne {} introuvable", giveaway.channel));
    }
    if reward.paused {
        return Err(format!("« {name} » est en pause sur la chaîne"));
    }
    if !reward.in_stock {
        return Err(format!("« {name} » n'est plus disponible"));
    }
    if reward.input_required {
        return Err(format!("« {name} » demande un texte : achète-la depuis le chat Twitch"));
    }
    // Jamais plus cher que prévu sans le savoir
    if let Some(cost) = giveaway.details.reward_cost.filter(|c| *c as u64 != reward.cost) {
        return Err(format!(
            "« {name} » coûte maintenant {} points (et non {cost}) : corrige le coût dans la fiche pour confirmer",
            reward.cost
        ));
    }
    Ok(reward)
}

/// Identifiant de transaction d'un achat : le même pour un giveaway, un compte et une récompense
/// (titre, coût), d'un essai à l'autre. Twitch ne fait payer qu'une fois une même transaction
fn redeem_transaction_id(giveaway_id: i64, account_id: i64, reward: &crate::twitch::gql::ChannelReward) -> String {
    // FNV-1a : stable d'un lancement et d'une version à l'autre
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{}|{}|{}", reward.id, reward.title, reward.cost).bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:08x}{:08x}{h:016x}", giveaway_id as u32, account_id as u32)
}

/// Achète la récompense du giveaway avec les comptes choisis (une fois par compte)
async fn redeem(app: &Arc<App>, giveaway: &GiveawayRow, account_ids: &[i64]) -> Result<Vec<ParticipationResult>, String> {
    let reward = giveaway_reward(app, giveaway).await?;
    let accounts = app.db.accounts().map_err(|e| e.to_string())?;
    let mut results = vec![];
    for (i, id) in account_ids.iter().enumerate() {
        let Some(account) = accounts.iter().find(|a| a.id == *id) else { continue };
        let result = |ok: bool, message: &str| ParticipationResult { account_id: *id, login: account.login.clone(), ok, message: message.into() };
        let Some(_sending) = SendingGuard::take(app, giveaway.id, *id) else {
            results.push(result(true, "achat déjà en cours"));
            continue;
        };
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(rand::random_range(1500..4000))).await;
        }
        // Relu juste avant l'achat : clos, ou déjà inscrit (jamais deux achats pour un compte)
        match app.db.giveaway(giveaway.id) {
            Ok(Some(g)) if g.status == "closed" => {
                results.push(result(false, "giveaway terminé"));
                continue;
            }
            Ok(Some(g)) if g.entries.iter().any(|e| e.account_id == *id) => {
                results.push(result(true, "déjà inscrit"));
                continue;
            }
            Ok(Some(_)) => {}
            Ok(None) => return Err("giveaway introuvable".into()),
            Err(e) => return Err(e.to_string()),
        }
        let outcome = match app.secrets.load(&account.user_id) {
            None => Err("jeton introuvable : reconnecte ce compte".to_owned()),
            Some(tokens) => {
                let gql = Gql::new(app.http.clone(), tokens.access_token, Some(account.device_id.clone()));
                gql.redeem_reward(&reward, &redeem_transaction_id(giveaway.id, *id, &reward)).await.map_err(|e| match e {
                    // Réponse perdue : Twitch a peut-être fait l'achat (un nouvel essai ne repaie pas)
                    crate::twitch::TwitchError::Http(_) => format!("achat incertain, vérifie ton solde sur Twitch ({e})"),
                    e => e.to_string(),
                })
            }
        };
        match outcome {
            Ok(()) => {
                let _ = app.db.add_giveaway_entry(giveaway.id, *id, &now_rfc3339(), false);
                app.log(
                    &account.login,
                    EventKind::Giveaway,
                    format!("« {} » acheté sur {} ({} points)", reward.title, giveaway.channel, reward.cost),
                );
                results.push(result(true, "acheté"));
            }
            Err(e) => {
                app.log(&account.login, EventKind::Error, format!("« {} » non acheté sur {} : {e}", reward.title, giveaway.channel));
                results.push(result(false, &e));
            }
        }
    }
    refresh_view(app);
    Ok(results)
}

/// Envoie la commande du giveaway depuis les comptes choisis (une fois par compte)
pub async fn participate(app: &Arc<App>, giveaway_id: i64, account_ids: &[i64], auto: bool) -> Result<Vec<ParticipationResult>, String> {
    let giveaway = app.db.giveaway(giveaway_id).map_err(|e| e.to_string())?.ok_or("giveaway introuvable")?;
    // En pause (live terminé) : le chat peut encore être lu, la participation reste permise
    if giveaway.status == "closed" {
        return Err("Ce giveaway est terminé".into());
    }
    // Masqué : plus de participation automatique (à la main, toujours permise)
    if auto && giveaway.hidden {
        return Err("Giveaway masqué : pas de participation automatique".into());
    }
    let command = match (giveaway.kind.as_str(), giveaway.command.clone()) {
        ("streamelements", _) => return Err("Giveaway StreamElements : les tickets s'achètent depuis la page StreamElements".into()),
        // Des points sont dépensés : jamais d'achat automatique
        ("channelpoints", _) if auto => return Err("Giveaway par points de chaîne : la récompense ne s'achète qu'à la main".into()),
        ("channelpoints", _) => return redeem(app, &giveaway, account_ids).await,
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
        // Envoi déjà en cours pour ce compte (participation manuelle et automatique ensemble)
        let Some(_sending) = SendingGuard::take(app, giveaway_id, *id) else {
            results.push(ParticipationResult { account_id: *id, login: account.login.clone(), ok: true, message: "envoi déjà en cours".into() });
            continue;
        };
        // Pas plusieurs comptes à la même seconde
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(rand::random_range(1500..4000))).await;
        }
        // Relu juste avant l'envoi : clos, ou inscrit entre-temps (depuis le chat, ou un autre envoi)
        match app.db.giveaway(giveaway_id) {
            Ok(Some(g)) if g.status == "closed" => {
                results.push(ParticipationResult { account_id: *id, login: account.login.clone(), ok: false, message: "giveaway terminé".into() });
                continue;
            }
            Ok(Some(g)) if g.entries.iter().any(|e| e.account_id == *id) => {
                results.push(ParticipationResult { account_id: *id, login: account.login.clone(), ok: true, message: "déjà inscrit".into() });
                continue;
            }
            Ok(Some(_)) => {}
            Ok(None) => return Err("giveaway introuvable".into()),
            Err(e) => return Err(e.to_string()),
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

/// Envoi d'une participation réservé (giveaway, compte) ; libéré à la fin de l'envoi
struct SendingGuard<'a> {
    app: &'a App,
    key: (i64, i64),
}

impl<'a> SendingGuard<'a> {
    fn take(app: &'a App, giveaway_id: i64, account_id: i64) -> Option<Self> {
        let key = (giveaway_id, account_id);
        app.giveaways.sending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key).then_some(Self { app, key })
    }
}

impl Drop for SendingGuard<'_> {
    fn drop(&mut self) {
        self.app.giveaways.sending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&self.key);
    }
}

/// Statut live des chaînes, partagé avec les superviseurs des comptes : cache, sinon requête
/// groupée, sinon le premier client accepté (jeton refusé : compte suivant)
async fn fetch_lives(app: &App, gqls: &[Gql], due: Vec<String>, fresh: bool) -> LiveReport {
    let since = if fresh { Instant::now() } else { crate::lives::since(LIVE_CHECK_EVERY) };
    let wanted: Vec<(String, Instant)> = due.into_iter().map(|c| (c, since)).collect();
    let fetch = app.lives.streams_status(&app.http, gqls, &wanted).await;
    let mut results = vec![];
    for (channel, result) in fetch.results {
        match result {
            Ok(info) => results.push((channel, info)),
            Err(e) => tracing::warn!("giveaways : statut de {channel} : {e}"),
        }
    }
    LiveReport { results, unauthorized: fetch.refused }
}

/// Giveaway ouvert auquel rattacher le gagnant annoncé par `bot` : StreamElements ne tire que
/// ses giveaways ; les autres bots tirent les giveaways par commande, sinon le plus récent
/// qui n'est pas StreamElements
/// Gagnant annoncé sans giveaway ouvert : giveaway clos sans gagnant dans ces dernières heures
const RECENT_UNDRAWN_HOURS: i64 = 6;
/// Annonce d'un autre gagnant peu après un tirage : ajoutée au giveaway tiré (plusieurs gagnants),
/// sauf si un nouveau giveaway de la chaîne est arrivé entre-temps
const MORE_WINNERS_WITHIN: chrono::Duration = chrono::Duration::minutes(15);

/// Ancienneté du follow requise : 3650 jours au plus
pub const FOLLOW_MAX_SECS: i64 = 3650 * 86_400;

/// Durée écrite « 7j », « 2h 30min », « 1j12h », « 45s » (j, h, min, m, s ; espaces libres) en
/// secondes ; None : vide, unité inconnue ou nul
pub fn parse_duration(text: &str) -> Option<i64> {
    let s = text.trim().to_lowercase().replace(' ', "");
    if s.is_empty() {
        return None;
    }
    let (mut total, mut rest) = (0i64, s.as_str());
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        if digits == 0 {
            return None;
        }
        let n: i64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit = rest.find(|c: char| c.is_ascii_digit()).unwrap_or(rest.len());
        let factor = match &rest[..unit] {
            "j" | "d" => 86_400,
            "h" => 3_600,
            "min" | "m" | "mn" => 60,
            "s" | "sec" => 1,
            _ => return None,
        };
        rest = &rest[unit..];
        total = total.checked_add(n.checked_mul(factor)?)?;
    }
    (total > 0).then_some(total)
}
/// Annonce du bot StreamElements gardée tant que son giveaway StreamElements est illisible, au
/// plus ce temps (avant MORE_WINNERS_WITHIN : les gagnants suivants retrouvent le même giveaway)
const SE_ANNOUNCE_WAIT: Duration = Duration::from_secs(8 * 60);
/// Réponse automatique : écrite au tirage plus le délai de la fiche, à ce nombre de secondes près
/// (gagnant appris plus tard, en relisant StreamElements : pas de réponse)
const WIN_REPLY_SLACK: i64 = 10;

/// Secondes à attendre avant la réponse : délai voulu moins le temps déjà écoulé depuis le tirage
/// (`elapsed`, None : tirage à l'instant) ; None : trop tard
fn reply_delay(wanted: i64, elapsed: Option<i64>) -> Option<u64> {
    let elapsed = elapsed.unwrap_or(0).max(0);
    (elapsed <= wanted + WIN_REPLY_SLACK).then(|| (wanted - elapsed).max(0) as u64)
}

fn winner_target<'a>(open: &'a [GiveawayRow], bot: &str) -> Option<&'a GiveawayRow> {
    // StreamElements tire aussi ses giveaways par commande dans le chat (même annonce) : son
    // giveaway StreamElements d'abord, sinon celui par commande, sinon un autre
    if bot == "streamelements" {
        return open
            .iter()
            .find(|g| g.kind == "streamelements")
            .or_else(|| open.iter().find(|g| g.kind == "command"))
            .or_else(|| open.first());
    }
    open.iter().find(|g| g.kind == "command").or_else(|| open.iter().find(|g| g.kind != "streamelements"))
}

/// Giveaway d'un gagnant désigné à la main : celui choisi (de cette chaîne), sinon le plus
/// récent ouvert de la chaîne, sinon en pause, sinon le plus récent de la chaîne
pub fn manual_target(db: &crate::db::Db, channel: &str, giveaway_id: Option<i64>) -> Option<GiveawayRow> {
    if let Some(id) = giveaway_id {
        return db.giveaway(id).ok().flatten().filter(|g| g.channel == channel);
    }
    let open = db.open_giveaways(Some(channel)).unwrap_or_default();
    if let Some(g) = open.into_iter().next() {
        return Some(g);
    }
    if let Some(g) = db.paused_giveaways(Some(channel)).unwrap_or_default().into_iter().next() {
        return Some(g);
    }
    db.giveaways(RECENT_GIVEAWAYS).unwrap_or_default().into_iter().find(|g| g.channel == channel)
}

/// Participation automatique (si activée) : chaque compte pas encore inscrit, après un délai
/// aléatoire
fn schedule_auto_participation(app: Arc<App>, giveaway_id: i64) {
    let settings = app.db.settings();
    if !settings.auto_participate {
        return;
    }
    let entered: Vec<i64> =
        app.db.giveaway(giveaway_id).ok().flatten().map(|g| g.entries.iter().map(|e| e.account_id).collect()).unwrap_or_default();
    let accounts: Vec<i64> = app
        .db
        .accounts()
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.id)
        .filter(|id| settings.auto_accounts.is_empty() || settings.auto_accounts.contains(id))
        .filter(|id| !entered.contains(id))
        .collect();
    let (min, max) = (settings.auto_delay_min.min(settings.auto_delay_max), settings.auto_delay_max.max(settings.auto_delay_min));
    for id in accounts {
        let app = app.clone();
        let delay = rand::random_range(min..=max);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(delay)).await;
            // Mis en pause ou désactivé entre-temps : on n'envoie rien
            if app.is_paused() || !app.db.settings().auto_participate || app.db.giveaway_hidden(giveaway_id) {
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
    // Domaine sans tenir compte des majuscules (« https://StreamElements.com/… »), écrit en minuscules
    const SE_PREFIX: &str = "https://streamelements.com/";
    g.url = g.url.map(|u| u.trim().to_owned()).filter(|u| !u.is_empty());
    if let Some(url) = &mut g.url {
        match url.get(..SE_PREFIX.len()) {
            Some(head) if head.eq_ignore_ascii_case(SE_PREFIX) => *url = format!("{SE_PREFIX}{}", &url[SE_PREFIX.len()..]),
            _ => return Err("Le lien StreamElements doit commencer par https://streamelements.com/".into()),
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
    if d.follow_min_secs.is_some_and(|n| !(1..=FOLLOW_MAX_SECS).contains(&n)) {
        return Err("Ancienneté du follow invalide (1 s à 3650 j)".into());
    }
    // Ancienneté du follow : n'a de sens qu'avec le follow requis
    if d.require_follow != Some(true) {
        d.follow_min_secs = None;
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

    fn row(id: i64, kind: &str) -> GiveawayRow {
        let db = crate::db::Db::memory().unwrap();
        let gid = db
            .insert_giveaway(&NewGiveaway {
                channel: "lunarsea".into(),
                kind: kind.into(),
                command: None,
                url: None,
                evidence: None,
                detected_at: "2026-11-01T10:00:00Z".into(),
            })
            .unwrap();
        GiveawayRow { id, ..db.giveaway(gid).unwrap().unwrap() }
    }

    #[test]
    fn winner_targets() {
        let open = vec![row(1, "ticket"), row(2, "streamelements"), row(3, "command")];
        assert_eq!(winner_target(&open, "streamelements").map(|g| g.id), Some(2));
        assert_eq!(winner_target(&open, "nightbot").map(|g| g.id), Some(3));
        // Pas de giveaway StreamElements ouvert : tirage StreamElements par commande dans le chat
        let open = vec![row(1, "ticket"), row(3, "command")];
        assert_eq!(winner_target(&open, "streamelements").map(|g| g.id), Some(3));
        assert_eq!(winner_target(&[row(1, "ticket")], "streamelements").map(|g| g.id), Some(1));
        assert_eq!(winner_target(&[], "streamelements"), None);
        // Un autre bot ne tire pas un giveaway StreamElements
        assert_eq!(winner_target(&[row(2, "streamelements")], "moobot"), None);
        assert_eq!(winner_target(&[row(2, "streamelements"), row(1, "ticket")], "moobot").map(|g| g.id), Some(1));
    }

    #[test]
    fn manual_targets() {
        let db = crate::db::Db::memory().unwrap();
        let new = |channel: &str| NewGiveaway {
            channel: channel.into(),
            kind: "command".into(),
            command: Some("!join".into()),
            url: None,
            evidence: None,
            detected_at: "2026-11-01T10:00:00Z".into(),
        };
        let a = db.insert_giveaway(&new("lunarsea")).unwrap();
        let b = db.insert_giveaway(&new("kraken_tv")).unwrap();
        assert_eq!(manual_target(&db, "lunarsea", Some(a)).map(|g| g.id), Some(a));
        // Giveaway d'une autre chaîne : refusé
        assert_eq!(manual_target(&db, "lunarsea", Some(b)), None);
        assert_eq!(manual_target(&db, "lunarsea", None).map(|g| g.id), Some(a));
        // Giveaway en pause (live terminé) préféré au plus récent clos
        let paused = db.insert_giveaway(&new("lunarsea")).unwrap();
        db.conn().execute("UPDATE giveaways SET manual = 1, detected_at = '2026-10-01T10:00:00Z' WHERE id = ?1", [paused]).unwrap();
        db.close_channel_giveaways("lunarsea").unwrap();
        assert_eq!(db.giveaway(a).unwrap().unwrap().status, "closed");
        assert_eq!(manual_target(&db, "lunarsea", None).map(|g| g.id), Some(paused));
        db.close_giveaway(paused).unwrap();
        assert_eq!(manual_target(&db, "lunarsea", None).map(|g| g.id), Some(a));
        assert_eq!(manual_target(&db, "nobody", None), None);
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
        f.details.follow_min_secs = Some(30);
        assert_eq!(parse_duration("7j"), Some(7 * 86_400));
        // Réponse : délai voulu moins le temps écoulé, 10 s de marge, sinon rien
        assert_eq!(reply_delay(10, None), Some(10));
        assert_eq!(reply_delay(10, Some(4)), Some(6));
        assert_eq!(reply_delay(10, Some(18)), Some(0));
        assert_eq!(reply_delay(10, Some(21)), None);
        assert_eq!(reply_delay(0, Some(3600)), None);
        assert_eq!(parse_duration(" 2h 30min "), Some(9_000));
        assert_eq!(parse_duration("1j12h"), Some(129_600));
        assert_eq!(parse_duration("45s"), Some(45));
        assert_eq!(parse_duration("5m"), Some(300));
        assert_eq!(parse_duration("2 semaines"), None);
        assert_eq!(parse_duration("12"), None);
        assert_eq!(parse_duration("0h"), None);
        assert_eq!(validate_input(f.clone()).unwrap().details.follow_min_secs, None);
        f.details.require_follow = Some(true);
        assert_eq!(validate_input(f).unwrap().details.follow_min_secs, Some(30));

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
