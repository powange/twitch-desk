//! Superviseur d'un compte : points de chaîne.
//!
//! Toutes les 2 s, il fait ce qui est dû : vérifier les lives, choisir les
//! 2 chaînes à regarder (Twitch n'en compte pas plus par compte), simuler le
//! visionnage, relire le solde et réclamer le bonus. Les événements PubSub
//! accélèrent tout ça (bonus disponible, début de live).
//!
//! Il garde aussi le compte présent dans le chat des chaînes choisies : c'est
//! ce qui fait gagner les points StreamElements, relus toutes les 10 min.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::TimeZone;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{interval, MissedTickBehavior};

use crate::app::{AccountStatus, AccountView, App, ChannelState, ChannelView, EventKind, SeAccount};
use crate::db::{Account, Channel, STREAMELEMENTS, TWITCH};
use crate::drops::DropsEngine;
use crate::secrets::Secrets;
use crate::streamelements::{self, Giveaway, Loyalty, SeError, TicketUpdate};
use crate::twitch::chat::{self, ChatEvent, TapEvent};
use crate::twitch::auth::{self, Tokens};
use crate::twitch::gql::{Gql, StreamInfo};
use crate::twitch::pubsub::{self, PubSubEvent};
use crate::twitch::watch::{ChannelWatch, WatchContext};
use crate::twitch::TwitchError;

/// Chaînes dont le temps de visionnage est compté en même temps, par compte
pub const MAX_WATCHED: usize = 2;
const STEP: Duration = Duration::from_secs(2);
const LIVE_CHECK_EVERY: Duration = Duration::from_secs(120);
const CONTEXT_WATCHING_EVERY: Duration = Duration::from_secs(5 * 60);
const CONTEXT_OTHERS_EVERY: Duration = Duration::from_secs(20 * 60);
const CONTEXTS_PER_STEP: usize = 2;
const VALIDATE_EVERY: Duration = Duration::from_secs(3600);
const FAILURES_BEFORE_COOLDOWN: u32 = 5;
const FAILED_CHANNEL_COOLDOWN: Duration = Duration::from_secs(600);
const RETRY_CONNECT_AFTER: Duration = Duration::from_secs(60);
// Nouveaux essais de plus en plus espacés (60 s, 2, 4, 8, 16 min, puis 30 min au plus)
const RETRY_CONNECT_MAX: Duration = Duration::from_secs(30 * 60);
// Compte resté connecté ce temps : les échecs suivants repartent du délai minimal
const STABLE_AFTER: Duration = Duration::from_secs(10 * 60);
// Renouvellement du jeton : au plus un toutes les 5 min
const REAUTH_MIN_GAP: Duration = Duration::from_secs(5 * 60);
// Après un « stream-up », Twitch met quelques secondes à exposer le live
const STREAM_UP_DELAY: Duration = Duration::from_secs(15);
// Relevé automatique du solde StreamElements : le rythme de distribution de la
// chaîne (réglé par le streamer), borné entre 2 et 10 min ; 2 min s'il est inconnu
const SE_POLL_MIN: u64 = 2;
const SE_POLL_MAX: u64 = 10;
const SE_RETRY_AFTER: Duration = Duration::from_secs(15 * 60);
/// Chaîne vue sans StreamElements : nouvelle recherche à ce rythme
const SE_NONE_RECHECK: Duration = Duration::from_secs(24 * 3600);
// Giveaway StreamElements en cours : relu souvent en live (nombre de participants)
const SE_GIVEAWAY_LIVE_EVERY: Duration = Duration::from_secs(2 * 60);
const SE_GIVEAWAY_OFFLINE_EVERY: Duration = Duration::from_secs(15 * 60);
const SE_LOYALTY_EVERY: Duration = Duration::from_secs(6 * 3600);
// Achat automatique : un ticket au plus toutes les 20 s par chaîne ; achat refusé (solde pas encore
// à jour, giveaway réservé aux abonnés…) : nouvel essai 5 min plus tard
const SE_AUTO_BUY_GAP: Duration = Duration::from_secs(20);
const SE_AUTO_BUY_RETRY: Duration = Duration::from_secs(5 * 60);

/// Maximum de tickets visé : le plus petit entre celui du giveaway (par personne) et le plafond
/// choisi pour la chaîne (0 : sans plafond)
fn auto_buy_max(per_person: Option<i64>, ours: i64) -> Option<i64> {
    let ours = (ours > 0).then_some(ours);
    match (per_person, ours) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Tickets à acheter d'un coup : tout ce que le solde permet, sans dépasser le maximum par
/// personne (nos tickets doivent alors être connus) ; 0 : rien à acheter
fn auto_buy_count(cost: i64, points: Option<i64>, max: Option<i64>, mine: Option<i64>) -> i64 {
    let Some(points) = points.filter(|_| cost > 0) else { return 0 };
    let affordable = points / cost;
    match max {
        None => affordable,
        Some(max) => match mine {
            Some(mine) => affordable.min(max - mine).max(0),
            None => 0,
        },
    }
}

#[derive(Debug, Clone)]
pub enum MinerCommand {
    /// Les chaînes ou les réglages du compte ont changé
    Reload,
    /// Réévaluer tout de suite (pause / reprise)
    Wake,
    /// Relire l'inventaire des drops tout de suite
    RefreshDrops,
    /// Bouton « Actualiser » d'une page : statut live, soldes (points de chaîne) ou soldes, règles
    /// et giveaways StreamElements, relus tout de suite sans les caches partagés
    Refresh(crate::db::ChannelList),
    /// Tickets connus après un achat depuis l'app
    SeTickets { channel_id: i64, giveaway_id: String, tickets: i64 },
}

struct ChannelRt {
    ch: Channel,
    live: Option<StreamInfo>,
    live_checked: Option<Instant>,
    recheck_at: Option<Instant>,
    watch: ChannelWatch,
    watching: bool,
    context_at: Option<Instant>,
    failures: u32,
    cooldown_until: Option<Instant>,
    error: Option<String>,
    earned_today: i64,
    last_bonus_at: Option<String>,
    in_chat: bool,
    se_polled: Option<Instant>,
    se_earned_today: i64,
    se_rank: Option<i64>,
    se_loyalty: Option<Loyalty>,
    se_loyalty_at: Option<Instant>,
    se_giveaway: Option<Giveaway>,
    se_giveaway_at: Option<Instant>,
    /// Prochain achat automatique de ticket permis
    se_auto_buy_at: Option<Instant>,
    se_my_tickets: Option<i64>,
}

impl ChannelRt {
    fn new(ch: Channel) -> Self {
        Self {
            ch,
            live: None,
            live_checked: None,
            recheck_at: None,
            watch: ChannelWatch::default(),
            watching: false,
            context_at: None,
            failures: 0,
            cooldown_until: None,
            error: None,
            earned_today: 0,
            last_bonus_at: None,
            in_chat: false,
            se_polled: None,
            se_earned_today: 0,
            se_rank: None,
            se_loyalty: None,
            se_loyalty_at: None,
            se_giveaway: None,
            se_giveaway_at: None,
            se_auto_buy_at: None,
            se_my_tickets: None,
        }
    }

    fn cooling(&self, now: Instant) -> bool {
        self.cooldown_until.is_some_and(|t| t > now)
    }
}

/// Fréquence de relevé du solde StreamElements d'une chaîne
fn se_poll_every(c: &ChannelRt) -> Duration {
    let interval = c.se_loyalty.as_ref().filter(|l| l.enabled && l.interval > 0).map(|l| l.interval as u64);
    se_poll_minutes(c.ch.se_refresh, interval)
}

/// Réglage de la chaîne s'il existe, sinon le rythme de distribution borné
pub fn se_poll_minutes(setting: Option<i64>, loyalty_interval: Option<u64>) -> Duration {
    let minutes = match setting {
        Some(m) if m > 0 => m as u64,
        _ => loyalty_interval.unwrap_or(SE_POLL_MIN).clamp(SE_POLL_MIN, SE_POLL_MAX),
    };
    Duration::from_secs(minutes * 60)
}

/// Indices des chaînes à regarder : les premières de la liste qui sont en live,
/// dans la limite des `slots` places restantes (les drops en prennent une)
pub fn select_watched(candidates: &[(bool, bool, bool)], slots: usize) -> Vec<usize> {
    // (activée, en live, en pause après erreurs)
    candidates
        .iter()
        .enumerate()
        .filter(|(_, (enabled, live, cooling))| *enabled && *live && !*cooling)
        .map(|(i, _)| i)
        .take(slots)
        .collect()
}

enum Stop {
    Expired(String),
    Error(String),
}

struct Miner {
    app: Arc<App>,
    account: Account,
    tokens: Option<Tokens>,
    gql: Option<Gql>,
    channels: Vec<ChannelRt>,
    status: AccountStatus,
    status_text: Option<String>,
    realtime: bool,
    pubsub_task: Option<JoinHandle<()>>,
    pubsub_rx: Option<mpsc::Receiver<PubSubEvent>>,
    chat_task: Option<JoinHandle<()>>,
    chat_rx: Option<mpsc::Receiver<ChatEvent>>,
    chat_wanted: watch::Sender<BTreeSet<String>>,
    chat_connected: bool,
    chat_refused: bool,
    /// Jeton du compte StreamElements (lecture exacte et achat de nos tickets)
    se_token: Option<String>,
    se_token_refused: bool,
    drops: DropsEngine,
    last_validate: Instant,
    day: chrono::NaiveDate,
    /// Dernier renouvellement du jeton (limité à un toutes les 5 min)
    last_reauth: Option<Instant>,
    /// Jeton refusé par PubSub alors qu'un renouvellement venait d'avoir lieu : à refaire plus tard
    pending_reauth: bool,
    /// Échecs de connexion à la suite (délai d'attente croissant)
    retries: u32,
    running_since: Option<Instant>,
}

/// Délai avant le n-ième nouvel essai (n ≥ 1)
fn retry_delay(retries: u32) -> Duration {
    let factor = 1u32 << retries.saturating_sub(1).min(5);
    (RETRY_CONNECT_AFTER * factor).min(RETRY_CONNECT_MAX)
}

/// Arrêt du superviseur (tâche annulée par stop_miner) : les tâches filles s'arrêtent avec lui
impl Drop for Miner {
    fn drop(&mut self) {
        self.stop_pubsub();
        self.stop_chat();
    }
}

pub async fn run(app: Arc<App>, account: Account, mut commands: mpsc::Receiver<MinerCommand>) {
    let mut miner = Miner {
        app,
        account,
        tokens: None,
        gql: None,
        channels: vec![],
        status: AccountStatus::Starting,
        status_text: None,
        realtime: false,
        pubsub_task: None,
        pubsub_rx: None,
        chat_task: None,
        chat_rx: None,
        chat_wanted: watch::channel(BTreeSet::new()).0,
        chat_connected: false,
        chat_refused: false,
        se_token: None,
        se_token_refused: false,
        drops: DropsEngine::default(),
        last_validate: Instant::now(),
        day: chrono::Local::now().date_naive(),
        last_reauth: None,
        pending_reauth: false,
        retries: 0,
        running_since: None,
    };
    miner.main(&mut commands).await;
    // PubSub et chat : arrêtés par Drop
}

async fn next_event<T>(rx: &mut Option<mpsc::Receiver<T>>) -> Option<T> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

impl Miner {
    async fn main(&mut self, commands: &mut mpsc::Receiver<MinerCommand>) {
        loop {
            self.status = AccountStatus::Starting;
            self.status_text = None;
            self.publish();
            if let Err(stop) = self.connect().await {
                let retry = self.on_stop(stop);
                // Erreur passagère : on retente ; jeton refusé : on attend une reconnexion
                if !Self::wait_retry(retry, commands).await {
                    return;
                }
                continue;
            }
            self.status = AccountStatus::Running;
            self.running_since = Some(Instant::now());
            self.app.log(&self.account.login, EventKind::Info, "Compte connecté");
            self.reload_channels();
            self.reload_drop_games();
            self.load_se_token();
            self.restart_pubsub();
            self.ensure_chat();
            self.publish();

            let mut tick = interval(STEP);
            // Passage long (inventaire des drops…) : pas de rafale de passages en retard
            tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                let result = tokio::select! {
                    _ = tick.tick() => self.step().await,
                    cmd = commands.recv() => match cmd {
                        None => return,
                        Some(MinerCommand::Reload) => {
                            let before = self.channel_ids();
                            self.reload_account();
                            self.load_se_token();
                            self.reload_channels();
                            self.reload_drop_games();
                            // Simple changement d'ordre ou de réglage : pas de reconnexion.
                            // Le chat suit ses propres cibles (chat_wanted) : il n'est pas relancé
                            if self.channel_ids() != before {
                                self.restart_pubsub();
                            }
                            Ok(())
                        }
                        Some(MinerCommand::Wake) => Ok(()),
                        Some(MinerCommand::RefreshDrops) => {
                            self.drops.refresh_inventory();
                            Ok(())
                        }
                        Some(MinerCommand::Refresh(list)) => {
                            self.refresh_now(list);
                            Ok(())
                        }
                        Some(MinerCommand::SeTickets { channel_id, giveaway_id, tickets }) => {
                            self.on_se_tickets(channel_id, &giveaway_id, tickets);
                            Ok(())
                        }
                    },
                    event = next_event(&mut self.chat_rx) => match event {
                        Some(event) => self.on_chat_event(event).await,
                        // Tâche du chat terminée : elle pourra être relancée
                        None => {
                            self.stop_chat();
                            Ok(())
                        }
                    },
                    event = next_event(&mut self.pubsub_rx) => match event {
                        Some(event) => self.on_event(event).await,
                        // Toutes les connexions PubSub sont arrêtées (jeton refusé)
                        None => {
                            self.stop_pubsub();
                            Ok(())
                        }
                    },
                };
                if let Err(stop) = result {
                    self.stop_pubsub();
                    self.stop_chat();
                    let retry = self.on_stop(stop);
                    if !Self::wait_retry(retry, commands).await {
                        return;
                    }
                    break;
                }
                self.publish();
            }
        }
    }

    /// Attend le délai d'un nouvel essai (ou, sans délai, une commande : reconnexion du compte).
    /// Une commande reçue pendant l'attente relance tout de suite ; false si le canal est fermé
    async fn wait_retry(retry: Option<Duration>, commands: &mut mpsc::Receiver<MinerCommand>) -> bool {
        let wait = async {
            match retry {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = wait => true,
            cmd = commands.recv() => cmd.is_some(),
        }
    }

    /// Met à jour l'état après un arrêt ; délai avant un nouvel essai, None s'il faut
    /// attendre une reconnexion du compte
    fn on_stop(&mut self, stop: Stop) -> Option<Duration> {
        if self.running_since.take().is_some_and(|t| t.elapsed() >= STABLE_AFTER) {
            self.retries = 0;
        }
        let (status, text, retry) = match stop {
            Stop::Expired(t) => (AccountStatus::Expired, t, None),
            Stop::Error(t) => {
                self.retries = self.retries.saturating_add(1);
                (AccountStatus::Error, t, Some(retry_delay(self.retries)))
            }
        };
        self.status = status;
        self.app.log(&self.account.login, EventKind::Error, text.clone());
        self.status_text = Some(text);
        for ch in &mut self.channels {
            ch.watching = false;
        }
        self.realtime = false;
        self.publish();
        retry
    }

    async fn connect(&mut self) -> Result<(), Stop> {
        let http = self.app.http.clone();
        let tokens = self
            .app
            .secrets
            .load(&self.account.user_id)
            .ok_or_else(|| Stop::Expired("Jeton introuvable : reconnecte ce compte".into()))?;
        let tokens = match auth::validate(&http, &tokens.access_token).await {
            Ok(_) => tokens,
            Err(TwitchError::Unauthorized) => self.refresh_tokens(&tokens).await?,
            Err(e) => return Err(Stop::Error(format!("Twitch injoignable : {e}"))),
        };
        self.gql = Some(Gql::new(http, tokens.access_token.clone(), Some(self.account.device_id.clone())));
        self.tokens = Some(tokens);
        // Nouveau jeton (ou jeton revalidé) : le chat a droit à un nouvel essai
        self.chat_refused = false;
        self.last_validate = Instant::now();
        Ok(())
    }

    async fn refresh_tokens(&mut self, tokens: &Tokens) -> Result<Tokens, Stop> {
        let expired = || Stop::Expired("Jeton expiré : reconnecte ce compte".into());
        let refresh_token = tokens.refresh_token.as_deref().ok_or_else(expired)?;
        self.last_reauth = Some(Instant::now());
        // Seul un refus explicite (400 / 401) veut dire « reconnecter le compte » ; réseau,
        // 5xx ou 429 : nouvel essai plus tard
        let mut fresh = match auth::refresh(&self.app.http, refresh_token).await {
            Ok(t) => t,
            Err(TwitchError::Unauthorized) => return Err(expired()),
            Err(e) => return Err(Stop::Error(format!("Renouvellement du jeton impossible pour l'instant : {e}"))),
        };
        if fresh.refresh_token.is_none() {
            fresh.refresh_token = tokens.refresh_token.clone();
        }
        // Enregistré avant la validation : Twitch peut avoir invalidé l'ancien jeton de renouvellement
        if let Err(e) = self.app.secrets.save(&self.account.user_id, &fresh) {
            tracing::error!("[{}] jeton renouvelé non enregistré : {e}", self.account.login);
            let text = format!("Jeton renouvelé mais non enregistré ({e}) : reconnecte ce compte s'il est refusé au prochain démarrage");
            self.app.log(&self.account.login, EventKind::Error, text.clone());
            self.status_text = Some(text);
        }
        match auth::validate(&self.app.http, &fresh.access_token).await {
            Ok(_) => {}
            Err(TwitchError::Unauthorized) => return Err(expired()),
            // Jeton tout juste délivré : une validation impossible n'empêche pas de s'en servir
            Err(e) => tracing::warn!("[{}] validation du jeton renouvelé : {e}", self.account.login),
        }
        Ok(fresh)
    }

    fn reauth_allowed(&self) -> bool {
        self.last_reauth.is_none_or(|t| t.elapsed() >= REAUTH_MIN_GAP)
    }

    /// Jeton refusé en cours de route : un essai de renouvellement, au plus un toutes les 5 min.
    /// Refusé de nouveau juste après : erreur passagère, le superviseur retente de plus en plus tard
    async fn reauth(&mut self) -> Result<(), Stop> {
        if !self.reauth_allowed() {
            return Err(Stop::Error("Jeton refusé de nouveau juste après son renouvellement : nouvel essai plus tard".into()));
        }
        let tokens = self.tokens.clone().ok_or_else(|| Stop::Expired("Jeton absent".into()))?;
        let fresh = self.refresh_tokens(&tokens).await?;
        if let Some(gql) = &self.gql {
            gql.set_token(fresh.access_token.clone());
        }
        self.tokens = Some(fresh);
        self.pending_reauth = false;
        // Le chat et PubSub repartent avec le nouveau jeton
        self.stop_chat();
        self.chat_refused = false;
        self.restart_pubsub();
        self.ensure_chat();
        Ok(())
    }

    /// Chaînes suivies, triées : sert à savoir s'il faut se réabonner aux événements
    fn channel_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.channels.iter().map(|c| c.ch.channel_id.clone()).collect();
        ids.sort();
        ids
    }

    fn load_se_token(&mut self) {
        let token = self.app.secrets.load_raw(&Secrets::se_key(&self.account.user_id));
        if token != self.se_token {
            self.se_token = token;
            self.se_token_refused = false;
            // Nouveau jeton : relire tout de suite nos tickets
            for ch in &mut self.channels {
                ch.se_giveaway_at = None;
            }
        }
    }

    fn on_se_tickets(&mut self, channel_id: i64, giveaway_id: &str, tickets: i64) {
        let _ = self.app.db.set_se_tickets(self.account.id, giveaway_id, tickets);
        crate::giveaways::refresh_view(&self.app);
        if let Some(ch) = self.channels.iter_mut().find(|c| c.ch.id == channel_id) {
            if ch.se_giveaway.as_ref().is_some_and(|g| g.id == giveaway_id) {
                ch.se_my_tickets = Some(tickets);
            }
            // Le solde a baissé : le relire, sans compter la baisse comme un gain
            ch.se_polled = None;
            ch.se_giveaway_at = None;
        }
    }

    fn reload_account(&mut self) {
        if let Some(a) = self.app.db.accounts().ok().and_then(|all| all.into_iter().find(|a| a.id == self.account.id)) {
            self.account = a;
        }
    }

    fn reload_drop_games(&mut self) {
        self.drops.set_games(self.app.db.drop_games(self.account.id).unwrap_or_default());
        self.drops.set_ignored(self.app.db.ignored_drop_campaigns(self.account.id).unwrap_or_default());
    }

    fn reload_channels(&mut self) {
        let fresh = self.app.db.channels(self.account.id).unwrap_or_default();
        let mut old: Vec<ChannelRt> = std::mem::take(&mut self.channels);
        let gains = self.today_gains(TWITCH);
        let se_gains = self.today_gains(STREAMELEMENTS);
        for ch in fresh {
            let rt = match old.iter().position(|o| o.ch.id == ch.id) {
                Some(i) => {
                    let mut rt = old.swap_remove(i);
                    rt.ch = ch;
                    rt
                }
                None => {
                    let mut rt = ChannelRt::new(ch);
                    rt.earned_today = gains.iter().find(|(l, _)| *l == rt.ch.login).map_or(0, |(_, g)| *g);
                    rt.se_earned_today = se_gains.iter().find(|(l, _)| *l == rt.ch.login).map_or(0, |(_, g)| *g);
                    rt
                }
            };
            self.channels.push(rt);
        }
    }

    fn today_gains(&self, currency: &str) -> Vec<(String, i64)> {
        let midnight = chrono::Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
        let since = chrono::Local
            .from_local_datetime(&midnight)
            .earliest()
            .map(|d| d.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_default();
        self.app.db.gains_since(self.account.id, currency, &since).unwrap_or_default()
    }

    /// Relance PubSub seul : la présence dans le chat n'est pas touchée
    fn restart_pubsub(&mut self) {
        self.stop_pubsub();
        let Some(tokens) = &self.tokens else { return };
        let ids: Vec<String> = self.channels.iter().map(|c| c.ch.channel_id.clone()).collect();
        let topics = pubsub::topics_for(&self.account.user_id, &ids);
        let (tx, rx) = mpsc::channel(256);
        let token = tokens.access_token.clone();
        self.pubsub_task = Some(tokio::spawn(pubsub::run(token, topics, tx)));
        self.pubsub_rx = Some(rx);
    }

    fn stop_pubsub(&mut self) {
        if let Some(task) = self.pubsub_task.take() {
            task.abort();
        }
        self.pubsub_rx = None;
        self.realtime = false;
    }

    // --- Présence dans le chat (points StreamElements) ---

    fn ensure_chat(&mut self) {
        if self.chat_task.is_some() || self.chat_refused {
            return;
        }
        let Some(tokens) = &self.tokens else { return };
        let (tx, rx) = mpsc::channel(64);
        let auth = chat::ChatAuth::Account { login: self.account.login.clone(), token: tokens.access_token.clone() };
        let tap = Some((self.account.id, self.app.giveaways.tap()));
        let task = chat::run(auth, self.chat_wanted.subscribe(), tx, tap);
        self.chat_task = Some(tokio::spawn(task));
        self.chat_rx = Some(rx);
    }

    fn stop_chat(&mut self) {
        if let Some(task) = self.chat_task.take() {
            task.abort();
            // Tâche annulée : elle ne préviendra pas le moniteur de giveaways, qui garderait
            // ce compte comme lecteur de ses chaînes
            self.tap_disconnected();
        }
        self.chat_rx = None;
        self.chat_connected = false;
        for ch in &mut self.channels {
            ch.in_chat = false;
        }
    }

    fn tap_disconnected(&self) {
        // File des événements de contrôle : jamais pleine, l'événement ne se perd pas
        self.app.giveaways.tap().send(TapEvent::Disconnected { source: self.account.id });
    }

    /// Chaînes où le compte doit être présent dans le chat
    fn update_chat_targets(&self) {
        let targets: BTreeSet<String> = if self.active() {
            self.channels.iter().filter(|c| c.ch.wants_chat()).map(|c| c.ch.login.clone()).collect()
        } else {
            BTreeSet::new()
        };
        self.chat_wanted.send_if_modified(|current| {
            let changed = *current != targets;
            *current = targets;
            changed
        });
    }

    async fn on_chat_event(&mut self, event: ChatEvent) -> Result<(), Stop> {
        match event {
            ChatEvent::Connected(connected) => {
                tracing::info!("[{}] chat {}", self.account.login, if connected { "connecté" } else { "déconnecté" });
                self.chat_connected = connected;
                if !connected {
                    for ch in &mut self.channels {
                        ch.in_chat = false;
                    }
                }
            }
            ChatEvent::Joined(ref login) | ChatEvent::Parted(ref login) => {
                let joined = matches!(event, ChatEvent::Joined(_));
                tracing::info!("[{}] chat {} : {}", self.account.login, login, if joined { "rejoint" } else { "quitté" });
                if let Some(ch) = self.channels.iter_mut().find(|c| c.ch.login == *login) {
                    ch.in_chat = joined;
                }
            }
            ChatEvent::BotMessage { channel, bot, text } => self.on_bot_message(&channel, &bot, &text),
            ChatEvent::AuthFailed => {
                self.stop_chat();
                if self.reauth_allowed() {
                    // Jeton sans doute expiré : un renouvellement relance le chat
                    tracing::info!("[{}] chat refusé : renouvellement du jeton", self.account.login);
                    self.reauth().await?;
                } else {
                    // Refusé malgré un jeton récent : plus d'essai avant le prochain renouvellement
                    self.chat_refused = true;
                    self.app.log(
                        &self.account.login,
                        EventKind::Error,
                        "Chat refusé par Twitch : reconnecte ce compte pour les points StreamElements",
                    );
                }
            }
        }
        Ok(())
    }

    /// Réponse d'un bot dans un chat suivi : nos tickets StreamElements
    fn on_bot_message(&mut self, channel: &str, bot: &str, text: &str) {
        if bot != "streamelements" {
            return;
        }
        let Some(update) = streamelements::parse_ticket_message(text, &self.account.login) else { return };
        let Some(ch) = self.channels.iter_mut().find(|c| c.ch.login == channel) else { return };
        let tickets = match update {
            TicketUpdate::Total { current, .. } => current,
            TicketUpdate::Bought(n) => ch.se_my_tickets.unwrap_or(0) + n,
        };
        ch.se_my_tickets = Some(tickets);
        match &ch.se_giveaway {
            Some(g) => {
                let _ = self.app.db.set_se_tickets(self.account.id, &g.id, tickets);
                crate::giveaways::refresh_view(&self.app);
            }
            // Giveaway pas encore connu : on le relit tout de suite pour y rattacher les tickets
            None => ch.se_giveaway_at = None,
        }
        let login = channel.to_owned();
        self.app.log(&self.account.login, EventKind::Info, format!("{tickets} ticket(s) StreamElements sur {login}"));
    }

    /// StreamElements : une requête au plus par passage, dans l'ordre solde,
    /// réglage de fidélité, giveaway en cours
    async fn poll_streamelements(&mut self, now: Instant) {
        if self.poll_se_points(now).await {
            return;
        }
        if self.poll_se_loyalty(now).await {
            return;
        }
        self.poll_se_giveaway(now).await;
    }

    /// Tout relire au prochain passage pour les chaînes de la page : statut live (sans le cache
    /// partagé), puis solde de points, ou solde, règle de gain et giveaway StreamElements
    fn refresh_now(&mut self, list: crate::db::ChannelList) {
        let now = Instant::now();
        for ch in &mut self.channels {
            match list {
                crate::db::ChannelList::Points if ch.ch.in_points => {
                    ch.recheck_at = Some(now);
                    ch.context_at = None;
                }
                crate::db::ChannelList::Se if ch.ch.in_se => {
                    ch.recheck_at = Some(now);
                    ch.se_polled = None;
                    ch.se_loyalty_at = None;
                    ch.se_giveaway_at = None;
                    if let Some(id) = ch.ch.se_channel_id.as_deref().filter(|id| !id.is_empty()) {
                        self.app.lives.se_giveaway.invalidate(id);
                        self.app.lives.se_loyalty.invalidate(id);
                    }
                }
                _ => {}
            }
        }
    }

    fn se_ready(c: &ChannelRt) -> Option<String> {
        if !c.ch.wants_chat() {
            return None;
        }
        c.ch.se_channel_id.clone().filter(|id| !id.is_empty())
    }

    async fn poll_se_loyalty(&mut self, now: Instant) -> bool {
        let Some(i) = self.channels.iter().position(|c| {
            Self::se_ready(c).is_some() && c.se_loyalty_at.is_none_or(|t| now.duration_since(t) >= SE_LOYALTY_EVERY)
        }) else {
            return false;
        };
        self.channels[i].se_loyalty_at = Some(now);
        let se_id = Self::se_ready(&self.channels[i]).unwrap_or_default();
        // Réglage de la chaîne, le même pour tous les comptes : partagé
        let http = &self.app.http;
        let since = crate::lives::since(SE_LOYALTY_EVERY);
        match self.app.lives.se_loyalty.get(&se_id, since, || streamelements::loyalty(http, &se_id)).await {
            Ok(loyalty) => self.channels[i].se_loyalty = loyalty,
            Err(e) => tracing::warn!("StreamElements ({}) : {e}", self.channels[i].ch.login),
        }
        true
    }

    async fn poll_se_giveaway(&mut self, now: Instant) -> bool {
        let every = |c: &ChannelRt| if c.live.is_some() { SE_GIVEAWAY_LIVE_EVERY } else { SE_GIVEAWAY_OFFLINE_EVERY };
        let Some(i) = self.channels.iter().position(|c| {
            Self::se_ready(c).is_some() && c.se_giveaway_at.is_none_or(|t| now.duration_since(t) >= every(c))
        }) else {
            return false;
        };
        self.channels[i].se_giveaway_at = Some(now);
        let se_id = Self::se_ready(&self.channels[i]).unwrap_or_default();
        let login = self.channels[i].ch.login.clone();
        // Giveaway de la chaîne, le même pour tous les comptes : partagé (nos tickets restent à part)
        let http = &self.app.http;
        let since = crate::lives::since(every(&self.channels[i]));
        let giveaway = match self.app.lives.se_giveaway.get(&se_id, since, || streamelements::active_giveaway(http, &se_id)).await {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!("StreamElements ({login}) : {e}");
                return true;
            }
        };
        let previous = self.channels[i].se_giveaway.as_ref().map(|g| g.id.clone());
        let current = giveaway.as_ref().map(|g| g.id.clone());
        if previous != current {
            let ch = &mut self.channels[i];
            ch.se_my_tickets = current.as_deref().and_then(|id| self.app.db.se_tickets(self.account.id, id));
            if let Some(g) = &giveaway {
                let command = g.command.as_deref().map(|c| format!(" avec {c}")).unwrap_or_default();
                self.app.log(
                    &self.account.login,
                    EventKind::Info,
                    format!("Giveaway StreamElements sur {login}{command} ({} points le ticket)", g.cost),
                );
            }
        }
        // Nos tickets exacts, si le compte StreamElements est connecté ; déjà au maximum par
        // personne sur ce giveaway : plus rien ne peut changer, pas de relecture
        let at_max = previous == current
            && giveaway.as_ref().and_then(|g| g.max_tickets).is_some_and(|max| self.channels[i].se_my_tickets.is_some_and(|t| t >= max));
        if let (Some(g), Some(token), false) = (&giveaway, self.se_token.clone(), at_max) {
            if !self.se_token_refused {
                match streamelements::my_entry(&self.app.http, &token, &se_id, &g.id).await {
                    Ok(tickets) => {
                        // Inchangés : ni écriture ni reconstruction de la vue
                        if self.channels[i].se_my_tickets != Some(tickets) {
                            self.channels[i].se_my_tickets = Some(tickets);
                            let _ = self.app.db.set_se_tickets(self.account.id, &g.id, tickets);
                            crate::giveaways::refresh_view(&self.app);
                        }
                    }
                    Err(SeError::Unauthorized) => {
                        self.se_token_refused = true;
                        self.app.log(
                            &self.account.login,
                            EventKind::Error,
                            "Jeton StreamElements expiré : reconnecte StreamElements dans Comptes",
                        );
                    }
                    Err(e) => tracing::warn!("StreamElements ({login}) : {e}"),
                }
            }
        }
        self.channels[i].se_giveaway = giveaway;
        true
    }

    /// Achat automatique (option de la chaîne) : en une fois, autant de tickets du giveaway
    /// StreamElements en cours que le solde le permet, jusqu'au maximum par personne
    async fn auto_buy_se(&mut self, now: Instant) {
        let Some(token) = self.se_token.clone().filter(|_| !self.se_token_refused) else { return };
        let Some(i) = self.channels.iter().position(|c| {
            c.ch.se_auto_buy
                && Self::se_ready(c).is_some()
                && c.se_auto_buy_at.is_none_or(|t| now >= t)
                && c.se_giveaway.as_ref().is_some_and(|g| auto_buy_count(g.cost, c.ch.se_points, auto_buy_max(g.max_tickets, c.ch.se_auto_buy_max), c.se_my_tickets) > 0)
        }) else {
            return;
        };
        let se_id = Self::se_ready(&self.channels[i]).unwrap_or_default();
        let Some(g) = self.channels[i].se_giveaway.clone() else { return };
        let login = self.channels[i].ch.login.clone();
        let cap = auto_buy_max(g.max_tickets, self.channels[i].ch.se_auto_buy_max);
        let count = auto_buy_count(g.cost, self.channels[i].ch.se_points, cap, self.channels[i].se_my_tickets);
        self.channels[i].se_auto_buy_at = Some(now + SE_AUTO_BUY_GAP);
        let bought = match streamelements::enter(&self.app.http, &token, &se_id, &g.id, count).await {
            Ok(()) => streamelements::my_entry(&self.app.http, &token, &se_id, &g.id).await,
            Err(e) => Err(e),
        };
        match bought {
            Ok(total) => {
                let ch = &mut self.channels[i];
                ch.se_my_tickets = Some(total);
                // Solde baissé des tickets achetés en attendant sa relecture (pas compté comme un gain)
                if let Some(p) = ch.ch.se_points {
                    let left = (p - g.cost * count).max(0);
                    ch.ch.se_points = Some(left);
                    let _ = self.app.db.set_se_points(ch.ch.id, left);
                }
                let _ = self.app.db.set_se_tickets(self.account.id, &g.id, total);
                crate::giveaways::refresh_view(&self.app);
                let max = g.max_tickets.map(|m| format!("/{m}")).unwrap_or_default();
                self.app.log(&self.account.login, EventKind::Info, format!("Achat automatique : {count} ticket(s) sur {login} ({total}{max})"));
                self.publish();
            }
            Err(SeError::Unauthorized) => {
                self.se_token_refused = true;
                self.app.log(&self.account.login, EventKind::Error, "Jeton StreamElements expiré : reconnecte StreamElements dans Comptes");
            }
            Err(e) => {
                self.channels[i].se_auto_buy_at = Some(now + SE_AUTO_BUY_RETRY);
                self.app.log(&self.account.login, EventKind::Error, format!("Achat automatique sur {login} refusé : {e}. Nouvel essai dans 5 min"));
            }
        }
    }

    /// Solde StreamElements
    async fn poll_se_points(&mut self, now: Instant) -> bool {
        let username = self.account.login.clone();
        let Some(i) = self.channels.iter().position(|c| {
            c.ch.wants_chat()
                && match c.ch.se_channel_id.as_deref() {
                    // Vue sans StreamElements : revérifiée au lancement, sur « Actualiser » et toutes
                    // les SE_NONE_RECHECK (chaîne inscrite depuis, ou réponse d'un incident)
                    Some("") => c.se_polled.is_none_or(|t| now.duration_since(t) >= SE_NONE_RECHECK),
                    None => c.se_polled.is_none_or(|t| now.duration_since(t) >= SE_RETRY_AFTER),
                    Some(_) => match c.se_polled {
                        None => true,
                        Some(t) => c.live.is_some() && now.duration_since(t) >= se_poll_every(c),
                    },
                }
        }) else {
            return false;
        };
        self.channels[i].se_polled = Some(now);
        let http = self.app.http.clone();
        let login = self.channels[i].ch.login.clone();
        let se_id = match self.channels[i].ch.se_channel_id.clone() {
            Some(id) if !id.is_empty() => id,
            _ => match streamelements::channel_id(&http, &login).await {
                Ok(found) => {
                    let id = found.unwrap_or_default();
                    let _ = self.app.db.set_se_channel(self.channels[i].ch.id, &id);
                    self.channels[i].ch.se_channel_id = Some(id.clone());
                    if id.is_empty() {
                        return true;
                    }
                    id
                }
                Err(e) => {
                    tracing::warn!("StreamElements ({login}) : {e}");
                    return true;
                }
            },
        };
        match streamelements::points(&http, &se_id, &username).await {
            Ok(points) => {
                let ch = &mut self.channels[i];
                ch.se_rank = points.rank;
                if let Some(previous) = ch.ch.se_points {
                    let gain = points.points - previous;
                    // Une baisse = des points dépensés (tickets de giveaway…) : pas un gain
                    if gain > 0 {
                        ch.se_earned_today += gain;
                        let _ = self.app.db.record_gain(self.account.id, &login, STREAMELEMENTS, gain, "WATCH");
                    }
                }
                if ch.ch.se_points != Some(points.points) {
                    ch.ch.se_points = Some(points.points);
                    let _ = self.app.db.set_se_points(ch.ch.id, points.points);
                }
            }
            Err(e) => tracing::warn!("StreamElements ({login}) : {e}"),
        }
        true
    }

    fn active(&self) -> bool {
        self.account.points_enabled && !self.app.is_paused()
    }

    async fn step(&mut self) -> Result<(), Stop> {
        let now = Instant::now();
        let gql = self.gql.clone().ok_or_else(|| Stop::Error("non connecté".into()))?;

        let today = chrono::Local::now().date_naive();
        if today != self.day {
            self.day = today;
            for ch in &mut self.channels {
                ch.earned_today = 0;
                ch.se_earned_today = 0;
            }
        }

        // PubSub a refusé le jeton juste après un renouvellement : nouvel essai une fois permis
        if self.pending_reauth && self.reauth_allowed() {
            self.reauth().await?;
        }

        if self.last_validate.elapsed() >= VALIDATE_EVERY {
            self.last_validate = now;
            match auth::validate(&self.app.http, &gql.token()).await {
                Err(TwitchError::Unauthorized) => self.reauth().await?,
                Err(e) => tracing::warn!("validation du jeton : {e}"),
                Ok(_) => {}
            }
        }

        self.check_lives(&gql, now).await?;

        // Drops : passent avant les points et occupent une des 2 places
        let not_paused = !self.app.is_paused();
        match self.drops.step(&self.app, &self.account, &gql, not_paused).await {
            Err(TwitchError::Unauthorized) => self.reauth().await?,
            Err(e) => tracing::warn!("drops : {e}"),
            Ok(()) => {}
        }
        let drop_login = self.drops.watching_login().map(str::to_owned);

        // Choix des chaînes regardées pour les points : celles d'un giveaway par points de
        // chaîne ouvert d'abord, puis l'ordre de la liste
        let selected = if self.active() {
            let priority = self.app.point_priority.read().unwrap().clone();
            let mut order: Vec<usize> = (0..self.channels.len()).collect();
            order.sort_by_key(|i| !priority.contains(&self.channels[*i].ch.login));
            let candidates: Vec<_> = order
                .iter()
                .map(|i| {
                    let c = &self.channels[*i];
                    let taken = drop_login.as_deref() == Some(c.ch.login.as_str());
                    (c.ch.farms_points() && !taken && !c.ch.balance_reached(), c.live.is_some(), c.cooling(now))
                })
                .collect();
            select_watched(&candidates, MAX_WATCHED - usize::from(drop_login.is_some()))
                .into_iter()
                .map(|k| order[k])
                .collect()
        } else {
            vec![]
        };
        for (i, ch) in self.channels.iter_mut().enumerate() {
            let watch = selected.contains(&i);
            if ch.watching && !watch {
                ch.watch = ChannelWatch::default();
            }
            // Chaîne déjà regardée pour les drops : ses points sont comptés aussi
            let by_drops = ch.ch.farms_points() && drop_login.as_deref() == Some(ch.ch.login.as_str());
            ch.watching = watch || by_drops;
        }

        for i in selected {
            self.watch_tick(&gql, i, now).await?;
        }

        self.refresh_contexts(&gql, now).await?;
        self.update_chat_targets();
        self.poll_streamelements(now).await;
        self.auto_buy_se(now).await;
        Ok(())
    }

    async fn check_lives(&mut self, gql: &Gql, now: Instant) -> Result<(), Stop> {
        // Statut partagé entre comptes : relu seulement s'il a plus de LIVE_CHECK_EVERY, en une
        // requête groupée pour toutes les chaînes dues
        let regular = crate::lives::since(LIVE_CHECK_EVERY);
        let mut due: Vec<usize> = vec![];
        let mut wanted: Vec<(String, Instant)> = vec![];
        for (i, ch) in self.channels.iter().enumerate() {
            if wanted.len() >= crate::lives::BATCH {
                break;
            }
            // Le live sert aussi aux points StreamElements (relus seulement en live)
            if !ch.ch.farms_points() && !ch.ch.wants_chat() {
                continue;
            }
            let recheck = ch.recheck_at.filter(|t| *t <= now);
            if recheck.is_none() && ch.live_checked.is_some_and(|t| t.elapsed() < LIVE_CHECK_EVERY) {
                continue;
            }
            // Après un « stream-up » : seulement un statut lu depuis l'heure de la relecture
            wanted.push((ch.ch.login.clone(), recheck.unwrap_or(regular)));
            due.push(i);
        }
        if due.is_empty() {
            return Ok(());
        }
        let fetch = self.app.lives.streams_status(&self.app.http, std::slice::from_ref(gql), &wanted).await;
        let mut refused = false;
        for i in due {
            // Absente : pas relue cette fois (repli une par une limité), au passage suivant
            match fetch.results.get(&self.channels[i].ch.login) {
                None => continue,
                Some(Ok(info)) => self.set_live(i, info.clone()),
                Some(Err(TwitchError::Unauthorized)) => refused = true,
                Some(Err(e)) => self.channels[i].error = Some(e.to_string()),
            }
            let ch = &mut self.channels[i];
            ch.live_checked = Some(Instant::now());
            ch.recheck_at = None;
        }
        if refused {
            self.reauth().await?;
        }
        Ok(())
    }

    fn set_live(&mut self, i: usize, info: Option<StreamInfo>) {
        let login = self.channels[i].ch.login.clone();
        let was_live = self.channels[i].live.is_some();
        match (&info, was_live) {
            (Some(s), false) => {
                let game = s.game_name.as_deref().map(|g| format!(" sur {g}")).unwrap_or_default();
                self.app.log(&self.account.login, EventKind::Live, format!("{login} passe en live{game}"));
            }
            (None, true) => self.app.log(&self.account.login, EventKind::Offline, format!("{login} est hors ligne")),
            _ => {}
        }
        let ch = &mut self.channels[i];
        if info.is_none() {
            ch.failures = 0;
            ch.cooldown_until = None;
        }
        ch.live = info;
        ch.error = None;
    }

    async fn watch_tick(&mut self, gql: &Gql, i: usize, now: Instant) -> Result<(), Stop> {
        let user_id = self.account.user_id.clone();
        let ch = &mut self.channels[i];
        let Some(live) = ch.live.clone() else { return Ok(()) };
        let ctx = WatchContext {
            login: &ch.ch.login,
            channel_id: &ch.ch.channel_id,
            broadcast_id: &live.broadcast_id,
            user_id: &user_id,
            game_id: live.game_id.as_deref(),
            game_name: live.game_name.as_deref(),
        };
        match ch.watch.tick(gql, &ctx).await {
            Ok(_) => {
                ch.failures = 0;
                ch.error = None;
            }
            Err(TwitchError::Unauthorized) => return self.reauth().await,
            Err(e) => {
                ch.failures += 1;
                ch.error = Some(e.to_string());
                if ch.failures >= FAILURES_BEFORE_COOLDOWN {
                    ch.failures = 0;
                    ch.cooldown_until = Some(now + FAILED_CHANNEL_COOLDOWN);
                    ch.watch = ChannelWatch::default();
                    let login = ch.ch.login.clone();
                    self.app.log(
                        &self.account.login,
                        EventKind::Error,
                        format!("{login} : visionnage en échec, nouvel essai dans 10 min ({e})"),
                    );
                }
            }
        }
        Ok(())
    }

    async fn refresh_contexts(&mut self, gql: &Gql, now: Instant) -> Result<(), Stop> {
        let mut done = 0;
        for i in 0..self.channels.len() {
            if done >= CONTEXTS_PER_STEP {
                break;
            }
            let ch = &self.channels[i];
            if !ch.ch.farms_points() {
                continue;
            }
            // Solde lu une fois au démarrage pour toutes les chaînes, puis seulement en live
            let every = if ch.watching { CONTEXT_WATCHING_EVERY } else { CONTEXT_OTHERS_EVERY };
            let due = match ch.context_at {
                None => true,
                Some(t) => ch.live.is_some() && now.duration_since(t) >= every,
            };
            if !due {
                continue;
            }
            done += 1;
            let login = ch.ch.login.clone();
            self.channels[i].context_at = Some(now);
            match gql.points_context(&login).await {
                Ok(Some(context)) => {
                    self.set_balance(i, context.balance);
                    if let Some(claim_id) = context.claim_id {
                        self.claim(gql, i, &claim_id).await?;
                    }
                }
                Ok(None) => {}
                Err(TwitchError::Unauthorized) => self.reauth().await?,
                Err(e) => self.channels[i].error = Some(e.to_string()),
            }
        }
        Ok(())
    }

    fn set_balance(&mut self, i: usize, balance: i64) {
        let ch = &mut self.channels[i];
        if ch.ch.balance != Some(balance) {
            ch.ch.balance = Some(balance);
            let _ = self.app.db.set_balance(self.account.id, &ch.ch.login, balance);
        }
    }

    async fn claim(&mut self, gql: &Gql, i: usize, claim_id: &str) -> Result<(), Stop> {
        if !self.active() || !self.channels[i].ch.auto_claim {
            return Ok(());
        }
        let channel_id = self.channels[i].ch.channel_id.clone();
        let login = self.channels[i].ch.login.clone();
        match gql.claim_bonus(&channel_id, claim_id).await {
            Ok(()) => {
                self.channels[i].last_bonus_at =
                    Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
                self.app.log(&self.account.login, EventKind::Bonus, format!("Bonus récupéré sur {login}"));
            }
            Err(TwitchError::Unauthorized) => self.reauth().await?,
            Err(e) => {
                self.app.log(&self.account.login, EventKind::Error, format!("Bonus non récupéré sur {login} : {e}"))
            }
        }
        Ok(())
    }

    fn index_of(&self, channel_id: &str) -> Option<usize> {
        self.channels.iter().position(|c| c.ch.channel_id == channel_id)
    }

    async fn on_event(&mut self, event: PubSubEvent) -> Result<(), Stop> {
        match event {
            PubSubEvent::Connection { connected } => self.realtime = connected,
            PubSubEvent::AuthFailed => {
                self.realtime = false;
                if self.reauth_allowed() {
                    tracing::info!("[{}] PubSub refuse le jeton : renouvellement", self.account.login);
                    self.reauth().await?;
                } else {
                    self.pending_reauth = true;
                }
            }
            PubSubEvent::PointsEarned { channel_id, gain, reason, balance } => {
                let Some(i) = self.index_of(&channel_id) else { return Ok(()) };
                if let Some(b) = balance {
                    self.set_balance(i, b);
                }
                let login = self.channels[i].ch.login.clone();
                self.channels[i].earned_today += gain;
                let _ = self.app.db.record_gain(self.account.id, &login, TWITCH, gain, &reason);
                // Les gains de visionnage (toutes les 5 min) gonfleraient le fil pour rien
                if reason != "WATCH" {
                    self.app.log(
                        &self.account.login,
                        EventKind::Gain,
                        format!("+{gain} sur {login} ({})", reason_label(&reason)),
                    );
                }
                self.app.touch_domain(crate::app::Domain::Accounts);
            }
            PubSubEvent::ClaimAvailable { channel_id, claim_id } => {
                if let (Some(i), Some(gql)) = (self.index_of(&channel_id), self.gql.clone()) {
                    self.claim(&gql, i, &claim_id).await?;
                }
            }
            PubSubEvent::StreamUp { channel_id } => {
                if let Some(i) = self.index_of(&channel_id) {
                    // Statut partagé périmé pour tous les comptes ; relu après le délai
                    self.app.lives.streams.invalidate(&self.channels[i].ch.login);
                    self.channels[i].recheck_at = Some(Instant::now() + STREAM_UP_DELAY);
                }
            }
            PubSubEvent::StreamDown { channel_id } => {
                if let Some(i) = self.index_of(&channel_id) {
                    self.app.lives.streams.put(&self.channels[i].ch.login, None);
                    self.set_live(i, None);
                }
            }
            PubSubEvent::Viewers { channel_id, viewers } => {
                if let Some(i) = self.index_of(&channel_id) {
                    self.app.lives.streams.update(&self.channels[i].ch.login, |info| {
                        if let Some(live) = info {
                            live.viewers = viewers;
                        }
                    });
                    if let Some(live) = self.channels[i].live.as_mut() {
                        live.viewers = viewers;
                    }
                }
            }
            PubSubEvent::DropProgress { drop_id, current } => {
                self.drops.on_progress(&drop_id, current);
                self.app.touch_domain(crate::app::Domain::Accounts);
            }
            PubSubEvent::DropClaim { instance_id } => {
                if let Some(gql) = self.gql.clone() {
                    match self.drops.claim_now(&self.app, &self.account, &gql, &instance_id).await {
                        Err(TwitchError::Unauthorized) => self.reauth().await?,
                        Err(e) => tracing::warn!("drops : {e}"),
                        Ok(()) => {}
                    }
                }
            }
            PubSubEvent::Raid { channel_id, target_login, .. } => {
                if let Some(i) = self.index_of(&channel_id) {
                    let login = self.channels[i].ch.login.clone();
                    self.app.log(&self.account.login, EventKind::Raid, format!("{login} lance un raid vers {target_login}"));
                }
            }
        }
        Ok(())
    }

    fn publish(&self) {
        let now = Instant::now();
        let channels = self
            .channels
            .iter()
            .map(|c| ChannelView {
                id: c.ch.id,
                login: c.ch.login.clone(),
                channel_id: c.ch.channel_id.clone(),
                enabled: c.ch.enabled,
                auto_claim: c.ch.auto_claim,
                balance: c.ch.balance,
                earned_today: c.earned_today,
                live: c.live.clone(),
                in_points: c.ch.in_points,
                in_se: c.ch.in_se,
                priority: self.app.point_priority.read().unwrap().contains(&c.ch.login),
                se_rank: c.se_rank,
                se_loyalty: c.se_loyalty.clone(),
                se_giveaway: c.se_giveaway.clone(),
                se_my_tickets: c.se_my_tickets,
                se_refresh: c.ch.se_refresh,
                max_balance: c.ch.max_balance,
                se_poll_minutes: se_poll_every(c).as_secs() / 60,
                state: match (c.ch.farms_points(), c.live_checked.is_some(), c.live.is_some(), c.watching) {
                    (false, ..) => ChannelState::Disabled,
                    _ if c.ch.balance_reached() && !c.watching => ChannelState::BalanceReached,
                    (_, _, _, true) => ChannelState::Watching,
                    (_, _, true, false) => ChannelState::Waiting,
                    (_, true, false, _) => ChannelState::Offline,
                    _ => ChannelState::Unknown,
                },
                error: match c.cooling(now) {
                    true => Some("Visionnage en échec, nouvel essai dans quelques minutes".into()),
                    false => c.error.clone(),
                },
                last_bonus_at: c.last_bonus_at.clone(),
                chat_presence: c.ch.chat_presence,
                in_chat: c.in_chat,
                se_enabled: c.ch.se_channel_id.as_deref().map(|id| !id.is_empty()),
                se_points: c.ch.se_points,
                se_earned_today: c.se_earned_today,
                profile_image: self.app.lives.avatar(&c.ch.login),
                se_page: crate::streamelements::page_name(&c.ch.login),
                se_auto_buy: c.ch.se_auto_buy,
                se_auto_buy_max: c.ch.se_auto_buy_max,
            })
            .collect();
        self.app.set_view(AccountView {
            id: self.account.id,
            login: self.account.login.clone(),
            user_id: self.account.user_id.clone(),
            points_enabled: self.account.points_enabled,
            status: self.status,
            status_text: self.status_text.clone(),
            realtime: self.realtime,
            chat_connected: self.chat_connected,
            drops: self.drops.view(self.account.drops_enabled),
            se_account: match (&self.se_token, self.se_token_refused) {
                (None, _) => SeAccount::None,
                (Some(_), false) => SeAccount::Connected,
                (Some(_), true) => SeAccount::Expired,
            },
            channels,
        });
    }
}

fn reason_label(reason: &str) -> &str {
    match reason {
        "WATCH" => "visionnage",
        "CLAIM" => "bonus",
        "WATCH_STREAK" => "série de visionnage",
        "RAID" => "raid",
        "PREDICTION" => "prédiction",
        _ => reason,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn auto_buy_counts() {
        // 100 points, ticket à 30, 5 au plus, 1 déjà : 3 achetés d'un coup
        assert_eq!(super::auto_buy_count(30, Some(100), Some(5), Some(1)), 3);
        // Limité par le maximum
        assert_eq!(super::auto_buy_count(10, Some(1000), Some(5), Some(3)), 2);
        // Au maximum, solde insuffisant, tickets inconnus ou gratuit : rien
        assert_eq!(super::auto_buy_count(10, Some(1000), Some(5), Some(5)), 0);
        assert_eq!(super::auto_buy_count(30, Some(20), Some(5), Some(0)), 0);
        assert_eq!(super::auto_buy_count(30, Some(100), Some(5), None), 0);
        assert_eq!(super::auto_buy_count(0, Some(100), None, None), 0);
        // Sans maximum : tout ce que le solde permet
        assert_eq!(super::auto_buy_count(30, Some(100), None, None), 3);
        // Plafond de la chaîne : le plus petit des deux, 0 sans plafond
        assert_eq!(super::auto_buy_max(Some(10), 3), Some(3));
        assert_eq!(super::auto_buy_max(Some(2), 3), Some(2));
        assert_eq!(super::auto_buy_max(None, 4), Some(4));
        assert_eq!(super::auto_buy_max(Some(10), 0), Some(10));
        assert_eq!(super::auto_buy_max(None, 0), None);
    }

    use super::*;

    #[test]
    fn se_poll_rhythm() {
        let min = |d: Duration| d.as_secs() / 60;
        assert_eq!(min(se_poll_minutes(None, None)), 2);
        assert_eq!(min(se_poll_minutes(None, Some(2))), 2);
        assert_eq!(min(se_poll_minutes(None, Some(10))), 10);
        assert_eq!(min(se_poll_minutes(None, Some(30))), 10);
        assert_eq!(min(se_poll_minutes(None, Some(1))), 2);
        assert_eq!(min(se_poll_minutes(Some(5), Some(2))), 5);
    }

    #[test]
    fn retry_backoff_grows() {
        let s = |n| retry_delay(n).as_secs();
        assert_eq!((s(1), s(2), s(3), s(5)), (60, 120, 240, 960));
        assert_eq!(s(6), 1800);
        assert_eq!(s(50), 1800);
    }

    #[test]
    fn selection_takes_first_two_live() {
        // (activée, en live, en pause après erreurs)
        let c = [(true, false, false), (true, true, false), (false, true, false), (true, true, true), (true, true, false), (true, true, false)];
        assert_eq!(select_watched(&c, 2), vec![1, 4]);
        assert_eq!(select_watched(&c, 1), vec![1]);
        assert!(select_watched(&[(true, false, false)], 2).is_empty());
    }
}
