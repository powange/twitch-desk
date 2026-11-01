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

use crate::app::{App, EventKind};
use crate::db::{GiveawayChannel, GiveawayRow, NewGiveaway};
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

pub enum MonitorCmd {
    /// Chaînes, réglages ou comptes modifiés
    Reload,
    /// Retirer un giveaway de la file d'alertes sans participer
    Dismiss(i64),
    Close(i64),
    Delete(i64),
    DismissMention(u64),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GiveawayEntryView {
    pub account_id: i64,
    pub login: String,
    pub at: String,
    pub auto: bool,
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
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GiveawaysView {
    pub channels: Vec<GiveawayChannelView>,
    pub giveaways: Vec<GiveawayView>,
    pub mentions: Vec<MentionView>,
    pub anonymous_connected: bool,
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

struct Monitor {
    app: Arc<App>,
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
}

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
        }
    }

    async fn run(mut self, mut rx: MonitorReceivers) {
        self.reload();
        let mut tick = tokio::time::interval(Duration::from_secs(60));
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
                _ = tick.tick() => {
                    let before = (chrono::Utc::now() - AUTO_CLOSE_AFTER).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    if self.app.db.close_giveaways_before(&before).unwrap_or(0) > 0 {
                        self.publish();
                    }
                }
            }
        }
    }

    fn monitored(&self, channel: &str) -> bool {
        self.channels.iter().any(|c| c.login == channel)
    }

    fn reload(&mut self) {
        self.channels = self.app.db.giveaway_channels().unwrap_or_default();
        self.accounts = self.app.db.accounts().unwrap_or_default().into_iter().map(|a| (a.id, a.login.to_lowercase())).collect();
        let monitored: HashSet<String> = self.channels.iter().map(|c| c.login.clone()).collect();
        let stale: Vec<String> = self.coverage.keys().filter(|c| !monitored.contains(*c)).cloned().collect();
        for c in stale {
            self.detector.forget(&c);
        }
        self.update_anonymous();
        self.publish();
    }

    /// Chaînes que la connexion anonyme doit rejoindre : surveillées et lues par aucun compte
    fn update_anonymous(&mut self) {
        let wanted: BTreeSet<String> = self
            .channels
            .iter()
            .map(|c| c.login.clone())
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
            TapEvent::Message { source, channel, user, text } => {
                // Un seul lecteur par chaîne : sinon chaque message compterait double
                if self.monitored(&channel) && self.reader(&channel) == Some(source) {
                    self.on_message(&channel, &user, &text);
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
            MonitorCmd::DismissMention(id) => {
                self.mentions.retain(|m| m.id != id);
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

    fn on_message(&mut self, channel: &str, user: &str, text: &str) {
        let user_l = user.to_lowercase();
        if bots::is_chat_bot(&user_l) {
            for event in bots::parse_bot_message(&user_l, text) {
                match event {
                    BotEvent::Winner { bot, winner, chance } => self.on_winner(channel, &bot, &winner, chance.as_deref()),
                    BotEvent::StreamElementsUrl(url) => self.on_streamelements_url(channel, &url),
                    BotEvent::TicketInfo { .. } | BotEvent::TicketPurchase { .. } => {}
                }
            }
            return;
        }
        self.on_mention(channel, &user_l, text);

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
        let share = evidence.count * 100 / evidence.window.max(1);
        self.app.log("giveaways", EventKind::Giveaway, format!("Giveaway détecté sur {channel} : {command} ({share} % du chat)"));
        self.app.notify(&format!("Giveaway sur {channel}"), &format!("Commande {command}, reprise par {share} % du chat"));
        self.publish();
        schedule_auto_participation(self.app.clone(), id);
    }

    fn on_streamelements_url(&mut self, channel: &str, url: &str) {
        let open = self.open_giveaways(channel);
        if open.iter().any(|g| g.url.as_deref().is_some_and(|u| u.eq_ignore_ascii_case(url))) {
            return;
        }
        let _ = self.app.db.insert_giveaway(&NewGiveaway {
            channel: channel.to_owned(),
            kind: "streamelements".into(),
            command: None,
            url: Some(url.to_owned()),
            evidence: None,
            detected_at: now_rfc3339(),
        });
        self.app.log("giveaways", EventKind::Giveaway, format!("Giveaway StreamElements sur {channel}"));
        self.app.notify(&format!("Giveaway StreamElements sur {channel}"), "Tickets à acheter avec tes points StreamElements");
        self.publish();
    }

    fn on_winner(&mut self, channel: &str, bot: &str, winner: &str, chance: Option<&str>) {
        let open = self.open_giveaways(channel);
        // Giveaway du type tiré par ce bot, sinon le plus récent
        let wanted_kind = if bot == "streamelements" { "streamelements" } else { "command" };
        let target = open.iter().find(|g| g.kind == wanted_kind).or_else(|| open.first());
        if let Some(g) = target {
            let _ = self.app.db.set_giveaway_winner(g.id, winner, chance, &now_rfc3339());
        }
        let ours = self.accounts.iter().find(|(_, l)| l.eq_ignore_ascii_case(winner)).map(|(_, l)| l.clone());
        match ours {
            Some(login) => {
                self.app.log(&login, EventKind::Giveaway, format!("Tu as gagné le giveaway de {channel} !"));
                self.app.notify("Tu as gagné !", &format!("{login} a gagné le giveaway de {channel}"));
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
            self.mention_seq += 1;
            let message: String = text.chars().take(300).collect();
            self.mentions.push_front(MentionView {
                id: self.mention_seq,
                at: now_rfc3339(),
                channel: channel.to_owned(),
                account: login.clone(),
                author: author.to_owned(),
                message: message.clone(),
            });
            self.mentions.truncate(MAX_MENTIONS);
            self.app.notify(&format!("{author} mentionne {login} sur {channel}"), &message);
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
        let view = GiveawaysView {
            channels: self.channel_views(),
            giveaways: build_views(&self.app, &self.accounts),
            mentions: self.mentions.iter().cloned().collect(),
            anonymous_connected: self.anon_connected,
        };
        *self.view.write().unwrap() = view;
        self.app.touch();
    }
}

fn build_views(app: &App, accounts: &[(i64, String)]) -> Vec<GiveawayView> {
    app.db
        .giveaways(RECENT_GIVEAWAYS)
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            let login_of = |id: i64| accounts.iter().find(|(a, _)| *a == id).map(|(_, l)| l.clone()).unwrap_or_default();
            GiveawayView {
                evidence: row.evidence.as_deref().and_then(|e| serde_json::from_str(e).ok()),
                entries_view: row
                    .entries
                    .iter()
                    .map(|e| GiveawayEntryView { account_id: e.account_id, login: login_of(e.account_id), at: e.at.clone(), auto: e.auto })
                    .collect(),
                winner_ours: row
                    .winner
                    .as_deref()
                    .and_then(|w| accounts.iter().find(|(_, l)| l.eq_ignore_ascii_case(w)).map(|(_, l)| l.clone())),
                row,
            }
        })
        .collect()
}

/// Rafraîchit la vue après une participation (hors de la boucle du moniteur)
pub fn refresh_view(app: &App) {
    let accounts: Vec<(i64, String)> =
        app.db.accounts().unwrap_or_default().into_iter().map(|a| (a.id, a.login.to_lowercase())).collect();
    let giveaways = build_views(app, &accounts);
    app.giveaways.view.write().unwrap().giveaways = giveaways;
    app.touch();
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
    let command = giveaway
        .command
        .clone()
        .ok_or("Giveaway StreamElements : les tickets s'achètent depuis la page StreamElements")?;
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
