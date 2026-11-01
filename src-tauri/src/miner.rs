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
use tokio::time::interval;

use crate::app::{AccountStatus, AccountView, App, ChannelState, ChannelView, EventKind};
use crate::db::{Account, Channel, STREAMELEMENTS, TWITCH};
use crate::streamelements::{self, Giveaway, Loyalty, TicketUpdate};
use crate::twitch::chat::{self, ChatEvent};
use crate::twitch::auth::{self, Tokens};
use crate::twitch::gql::{Gql, StreamInfo};
use crate::twitch::pubsub::{self, PubSubEvent};
use crate::twitch::watch::{ChannelWatch, WatchContext};
use crate::twitch::TwitchError;

/// Chaînes dont le temps de visionnage est compté en même temps, par compte
pub const MAX_WATCHED: usize = 2;
const STEP: Duration = Duration::from_secs(2);
const LIVE_CHECK_EVERY: Duration = Duration::from_secs(120);
const LIVE_CHECKS_PER_STEP: usize = 3;
const CONTEXT_WATCHING_EVERY: Duration = Duration::from_secs(5 * 60);
const CONTEXT_OTHERS_EVERY: Duration = Duration::from_secs(20 * 60);
const CONTEXTS_PER_STEP: usize = 2;
const VALIDATE_EVERY: Duration = Duration::from_secs(3600);
const FAILURES_BEFORE_COOLDOWN: u32 = 5;
const FAILED_CHANNEL_COOLDOWN: Duration = Duration::from_secs(600);
const RETRY_CONNECT_AFTER: Duration = Duration::from_secs(60);
// Après un « stream-up », Twitch met quelques secondes à exposer le live
const STREAM_UP_DELAY: Duration = Duration::from_secs(15);
// StreamElements donne ses points toutes les 10 min pendant le live
const SE_POLL_EVERY: Duration = Duration::from_secs(10 * 60);
const SE_RETRY_AFTER: Duration = Duration::from_secs(15 * 60);
// Giveaway StreamElements en cours : relu souvent en live (nombre de participants)
const SE_GIVEAWAY_LIVE_EVERY: Duration = Duration::from_secs(2 * 60);
const SE_GIVEAWAY_OFFLINE_EVERY: Duration = Duration::from_secs(15 * 60);
const SE_LOYALTY_EVERY: Duration = Duration::from_secs(6 * 3600);

#[derive(Debug, Clone)]
pub enum MinerCommand {
    /// Les chaînes ou les réglages du compte ont changé
    Reload,
    /// Réévaluer tout de suite (pause / reprise)
    Wake,
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
            se_my_tickets: None,
        }
    }

    fn cooling(&self, now: Instant) -> bool {
        self.cooldown_until.is_some_and(|t| t > now)
    }
}

/// Indices des chaînes à regarder : les premières de la liste qui sont en live
pub fn select_watched(candidates: &[(bool, bool, bool)]) -> Vec<usize> {
    // (activée, en live, en pause après erreurs)
    candidates
        .iter()
        .enumerate()
        .filter(|(_, (enabled, live, cooling))| *enabled && *live && !*cooling)
        .map(|(i, _)| i)
        .take(MAX_WATCHED)
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
    last_validate: Instant,
    day: chrono::NaiveDate,
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
        last_validate: Instant::now(),
        day: chrono::Local::now().date_naive(),
    };
    miner.main(&mut commands).await;
    miner.stop_pubsub();
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
            match self.connect().await {
                Ok(()) => {}
                Err(stop) => {
                    let retry = self.on_stop(stop);
                    // Erreur réseau : on retente ; jeton refusé : on attend une reconnexion
                    let wait = async {
                        match retry {
                            true => tokio::time::sleep(RETRY_CONNECT_AFTER).await,
                            false => std::future::pending::<()>().await,
                        }
                    };
                    tokio::select! {
                        _ = wait => continue,
                        cmd = commands.recv() => match cmd {
                            None => return,
                            Some(_) => continue,
                        },
                    }
                }
            }
            self.status = AccountStatus::Running;
            self.app.log(&self.account.login, EventKind::Info, "Compte connecté");
            self.reload_channels();
            self.restart_pubsub();
            self.publish();

            let mut tick = interval(STEP);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        if let Err(stop) = self.step().await {
                            self.stop_pubsub();
                            let retry = self.on_stop(stop);
                            if retry {
                                tokio::time::sleep(RETRY_CONNECT_AFTER).await;
                            } else {
                                // Jeton refusé : attendre une commande (reconnexion du compte)
                                if commands.recv().await.is_none() {
                                    return;
                                }
                            }
                            break;
                        }
                    }
                    cmd = commands.recv() => match cmd {
                        None => return,
                        Some(MinerCommand::Reload) => {
                            self.reload_account();
                            self.reload_channels();
                            self.restart_pubsub();
                        }
                        Some(MinerCommand::Wake) => {}
                    },
                    event = next_event(&mut self.chat_rx) => {
                        if let Some(event) = event {
                            self.on_chat_event(event);
                        }
                    }
                    event = next_event(&mut self.pubsub_rx) => {
                        if let Some(event) = event {
                            if let Err(stop) = self.on_event(event).await {
                                self.stop_pubsub();
                                if !self.on_stop(stop) && commands.recv().await.is_none() {
                                    return;
                                }
                                break;
                            }
                        }
                    }
                }
                self.publish();
            }
        }
    }

    /// Met à jour l'état après un arrêt ; true si l'on peut retenter seul
    fn on_stop(&mut self, stop: Stop) -> bool {
        let (status, text, retry) = match stop {
            Stop::Expired(t) => (AccountStatus::Expired, t, false),
            Stop::Error(t) => (AccountStatus::Error, t, true),
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
        self.last_validate = Instant::now();
        Ok(())
    }

    async fn refresh_tokens(&self, tokens: &Tokens) -> Result<Tokens, Stop> {
        let expired = || Stop::Expired("Jeton expiré : reconnecte ce compte".into());
        let refresh_token = tokens.refresh_token.as_deref().ok_or_else(expired)?;
        let mut fresh = auth::refresh(&self.app.http, refresh_token).await.map_err(|_| expired())?;
        if fresh.refresh_token.is_none() {
            fresh.refresh_token = tokens.refresh_token.clone();
        }
        auth::validate(&self.app.http, &fresh.access_token).await.map_err(|_| expired())?;
        let _ = self.app.secrets.save(&self.account.user_id, &fresh);
        Ok(fresh)
    }

    /// Jeton refusé en cours de route : un essai de renouvellement
    async fn reauth(&mut self) -> Result<(), Stop> {
        let tokens = self.tokens.clone().ok_or_else(|| Stop::Expired("Jeton absent".into()))?;
        let fresh = self.refresh_tokens(&tokens).await?;
        if let Some(gql) = &self.gql {
            gql.set_token(fresh.access_token.clone());
        }
        self.tokens = Some(fresh);
        self.stop_chat();
        self.restart_pubsub();
        Ok(())
    }

    fn reload_account(&mut self) {
        if let Some(a) = self.app.db.accounts().ok().and_then(|all| all.into_iter().find(|a| a.id == self.account.id)) {
            self.account = a;
        }
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

    fn restart_pubsub(&mut self) {
        self.stop_pubsub();
        let Some(tokens) = &self.tokens else { return };
        let ids: Vec<String> = self.channels.iter().map(|c| c.ch.channel_id.clone()).collect();
        let topics = pubsub::topics_for(&self.account.user_id, &ids);
        let (tx, rx) = mpsc::channel(256);
        let token = tokens.access_token.clone();
        self.pubsub_task = Some(tokio::spawn(pubsub::run(token, topics, tx)));
        self.pubsub_rx = Some(rx);
        self.ensure_chat();
    }

    /// Arrête aussi la présence dans le chat
    fn stop_pubsub(&mut self) {
        if let Some(task) = self.pubsub_task.take() {
            task.abort();
        }
        self.pubsub_rx = None;
        self.realtime = false;
        self.stop_chat();
    }

    // --- Présence dans le chat (points StreamElements) ---

    fn ensure_chat(&mut self) {
        if self.chat_task.is_some() || self.chat_refused {
            return;
        }
        let Some(tokens) = &self.tokens else { return };
        let (tx, rx) = mpsc::channel(64);
        let task = chat::run(
            self.account.login.clone(),
            tokens.access_token.clone(),
            self.chat_wanted.subscribe(),
            tx,
        );
        self.chat_task = Some(tokio::spawn(task));
        self.chat_rx = Some(rx);
    }

    fn stop_chat(&mut self) {
        if let Some(task) = self.chat_task.take() {
            task.abort();
        }
        self.chat_rx = None;
        self.chat_connected = false;
        for ch in &mut self.channels {
            ch.in_chat = false;
        }
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

    fn on_chat_event(&mut self, event: ChatEvent) {
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
                self.chat_refused = true;
                self.stop_chat();
                self.app.log(
                    &self.account.login,
                    EventKind::Error,
                    "Chat refusé par Twitch : reconnecte ce compte pour les points StreamElements",
                );
            }
        }
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
        match streamelements::loyalty(&self.app.http, &se_id).await {
            Ok(loyalty) => self.channels[i].se_loyalty = loyalty,
            Err(e) => tracing::warn!("StreamElements ({}) : {e}", self.channels[i].ch.login),
        }
        true
    }

    async fn poll_se_giveaway(&mut self, now: Instant) -> bool {
        let Some(i) = self.channels.iter().position(|c| {
            let every = if c.live.is_some() { SE_GIVEAWAY_LIVE_EVERY } else { SE_GIVEAWAY_OFFLINE_EVERY };
            Self::se_ready(c).is_some() && c.se_giveaway_at.is_none_or(|t| now.duration_since(t) >= every)
        }) else {
            return false;
        };
        self.channels[i].se_giveaway_at = Some(now);
        let se_id = Self::se_ready(&self.channels[i]).unwrap_or_default();
        let login = self.channels[i].ch.login.clone();
        let giveaway = match streamelements::active_giveaway(&self.app.http, &se_id).await {
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
        self.channels[i].se_giveaway = giveaway;
        true
    }

    /// Solde StreamElements
    async fn poll_se_points(&mut self, now: Instant) -> bool {
        let username = self.account.login.clone();
        let Some(i) = self.channels.iter().position(|c| {
            c.ch.wants_chat()
                && match c.ch.se_channel_id.as_deref() {
                    Some("") => false,
                    None => c.se_polled.is_none_or(|t| now.duration_since(t) >= SE_RETRY_AFTER),
                    Some(_) => match c.se_polled {
                        None => true,
                        Some(t) => c.live.is_some() && now.duration_since(t) >= SE_POLL_EVERY,
                    },
                }
        }) else {
            return false;
        };
        self.channels[i].se_polled = Some(now);
        let http = self.app.http.clone();
        let login = self.channels[i].ch.login.clone();
        let se_id = match self.channels[i].ch.se_channel_id.clone() {
            Some(id) => id,
            None => match streamelements::channel_id(&http, &login).await {
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

        if self.last_validate.elapsed() >= VALIDATE_EVERY {
            self.last_validate = now;
            match auth::validate(&self.app.http, &gql.token()).await {
                Err(TwitchError::Unauthorized) => self.reauth().await?,
                Err(e) => tracing::warn!("validation du jeton : {e}"),
                Ok(_) => {}
            }
        }

        self.check_lives(&gql, now).await?;

        // Choix des chaînes regardées
        let selected = if self.active() {
            let candidates: Vec<_> =
                self.channels.iter().map(|c| (c.ch.farms_points(), c.live.is_some(), c.cooling(now))).collect();
            select_watched(&candidates)
        } else {
            vec![]
        };
        for (i, ch) in self.channels.iter_mut().enumerate() {
            let watch = selected.contains(&i);
            if ch.watching && !watch {
                ch.watch = ChannelWatch::default();
            }
            ch.watching = watch;
        }

        for i in selected {
            self.watch_tick(&gql, i, now).await?;
        }

        self.refresh_contexts(&gql, now).await?;
        self.update_chat_targets();
        self.poll_streamelements(now).await;
        Ok(())
    }

    async fn check_lives(&mut self, gql: &Gql, now: Instant) -> Result<(), Stop> {
        let mut done = 0;
        for i in 0..self.channels.len() {
            if done >= LIVE_CHECKS_PER_STEP {
                break;
            }
            let ch = &self.channels[i];
            // Le live sert aussi aux points StreamElements (relus seulement en live)
            if !ch.ch.farms_points() && !ch.ch.wants_chat() {
                continue;
            }
            let due = ch.recheck_at.is_some_and(|t| t <= now)
                || ch.live_checked.is_none_or(|t| t.elapsed() >= LIVE_CHECK_EVERY);
            if !due {
                continue;
            }
            done += 1;
            let login = ch.ch.login.clone();
            match gql.stream_info(&login).await {
                Ok(info) => self.set_live(i, info),
                Err(TwitchError::Unauthorized) => {
                    self.reauth().await?;
                }
                Err(e) => self.channels[i].error = Some(e.to_string()),
            }
            let ch = &mut self.channels[i];
            ch.live_checked = Some(Instant::now());
            ch.recheck_at = None;
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
                self.app.touch();
            }
            PubSubEvent::ClaimAvailable { channel_id, claim_id } => {
                if let (Some(i), Some(gql)) = (self.index_of(&channel_id), self.gql.clone()) {
                    self.claim(&gql, i, &claim_id).await?;
                }
            }
            PubSubEvent::StreamUp { channel_id } => {
                if let Some(i) = self.index_of(&channel_id) {
                    self.channels[i].recheck_at = Some(Instant::now() + STREAM_UP_DELAY);
                }
            }
            PubSubEvent::StreamDown { channel_id } => {
                if let Some(i) = self.index_of(&channel_id) {
                    self.set_live(i, None);
                }
            }
            PubSubEvent::Viewers { channel_id, viewers } => {
                if let Some(live) = self.index_of(&channel_id).and_then(|i| self.channels[i].live.as_mut()) {
                    live.viewers = viewers;
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
                se_rank: c.se_rank,
                se_loyalty: c.se_loyalty.clone(),
                se_giveaway: c.se_giveaway.clone(),
                se_my_tickets: c.se_my_tickets,
                state: match (c.ch.farms_points(), c.live_checked.is_some(), c.live.is_some(), c.watching) {
                    (false, ..) => ChannelState::Disabled,
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
    use super::*;

    #[test]
    fn selection_takes_first_two_live() {
        // (activée, en live, en pause après erreurs)
        let c = [(true, false, false), (true, true, false), (false, true, false), (true, true, true), (true, true, false), (true, true, false)];
        assert_eq!(select_watched(&c), vec![1, 4]);
        assert!(select_watched(&[(true, false, false)]).is_empty());
    }
}
