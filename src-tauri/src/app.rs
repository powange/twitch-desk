//! État partagé de l'application et vue envoyée à l'interface.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, Notify};
use tauri::async_runtime::JoinHandle;

use crate::db::{Account, Db};
use crate::miner::{self, MinerCommand};
use crate::secrets::Secrets;
use crate::twitch::gql::{self, StreamInfo};

const MAX_EVENTS: usize = 300;
const EMIT_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AccountStatus {
    Starting,
    Running,
    /// Jeton refusé : il faut se reconnecter
    Expired,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ChannelState {
    /// Temps de visionnage compté (dans les 2 chaînes retenues)
    Watching,
    /// En live mais hors des 2 chaînes retenues
    Waiting,
    Offline,
    Disabled,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelView {
    pub id: i64,
    pub login: String,
    pub channel_id: String,
    pub enabled: bool,
    pub auto_claim: bool,
    pub balance: Option<i64>,
    pub earned_today: i64,
    pub live: Option<StreamInfo>,
    pub state: ChannelState,
    pub error: Option<String>,
    pub last_bonus_at: Option<String>,
    pub in_points: bool,
    pub in_se: bool,
    /// Prioritaire pour les points : giveaway par points de chaîne ouvert
    pub priority: bool,
    pub chat_presence: bool,
    pub in_chat: bool,
    pub se_rank: Option<i64>,
    pub se_loyalty: Option<crate::streamelements::Loyalty>,
    pub se_giveaway: Option<crate::streamelements::Giveaway>,
    pub se_my_tickets: Option<i64>,
    /// Relevé choisi (None : automatique) et relevé appliqué, en minutes
    pub se_refresh: Option<i64>,
    pub se_poll_minutes: u64,
    /// None : pas encore vérifié ; false : la chaîne n'utilise pas StreamElements
    pub se_enabled: Option<bool>,
    pub se_points: Option<i64>,
    pub se_earned_today: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: i64,
    pub login: String,
    pub user_id: String,
    pub points_enabled: bool,
    pub status: AccountStatus,
    pub status_text: Option<String>,
    pub realtime: bool,
    pub chat_connected: bool,
    pub drops: crate::drops::DropsView,
    pub se_account: SeAccount,
    pub channels: Vec<ChannelView>,
}

/// Compte StreamElements du spectateur
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SeAccount {
    None,
    Connected,
    /// Jeton refusé : il faut reconnecter StreamElements
    Expired,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EventKind {
    Bonus,
    Gain,
    Live,
    Offline,
    Raid,
    Drop,
    Giveaway,
    Info,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEvent {
    pub at: String,
    pub account: String,
    pub kind: EventKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub paused: bool,
    pub accounts: Vec<AccountView>,
    pub events: Vec<LogEvent>,
    pub operations_version: u32,
    pub operations_updated: String,
    pub giveaways: crate::giveaways::GiveawaysView,
    /// Langue de diffusion des chaînes (login → code ISO 639-1), pour les drapeaux
    pub languages: HashMap<String, String>,
    pub scanner: crate::scanner::ScannerView,
    /// Chaînes surveillées en live : giveaway StreamElements actif, description, message épinglé
    pub insights: HashMap<String, crate::insights::ChannelInsight>,
}

struct MinerHandle {
    commands: mpsc::Sender<MinerCommand>,
    task: JoinHandle<()>,
}

pub struct App {
    pub db: Db,
    pub secrets: Secrets,
    pub http: reqwest::Client,
    pub data_dir: PathBuf,
    handle: AppHandle,
    miners: Mutex<HashMap<i64, MinerHandle>>,
    views: RwLock<HashMap<i64, AccountView>>,
    events: Mutex<VecDeque<LogEvent>>,
    paused: AtomicBool,
    dirty: Notify,
    pub login_task: Mutex<Option<JoinHandle<()>>>,
    pub giveaways: crate::giveaways::GiveawayHandle,
    /// Chaînes à regarder en premier pour les points : giveaway par points de chaîne ouvert
    pub point_priority: RwLock<std::collections::HashSet<String>>,
    pub scanner: Mutex<crate::scanner::ScannerState>,
    pub insights: Mutex<crate::insights::InsightsState>,
    languages: RwLock<HashMap<String, String>>,
}

impl App {
    pub fn new(handle: AppHandle, db: Db, secrets: Secrets, data_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            db,
            secrets,
            http: crate::twitch::http_client(),
            data_dir,
            handle,
            miners: Mutex::new(HashMap::new()),
            views: RwLock::new(HashMap::new()),
            events: Mutex::new(VecDeque::new()),
            paused: AtomicBool::new(false),
            dirty: Notify::new(),
            login_task: Mutex::new(None),
            giveaways: crate::giveaways::GiveawayHandle::default(),
            point_priority: RwLock::new(std::collections::HashSet::new()),
            scanner: Mutex::new(crate::scanner::ScannerState::default()),
            insights: Mutex::new(crate::insights::InsightsState::default()),
            languages: RwLock::new(HashMap::new()),
        })
    }

    pub fn handle(&self) -> &AppHandle {
        &self.handle
    }

    /// Démarre un superviseur par compte enregistré
    pub fn start_all(self: &Arc<Self>) {
        match self.db.accounts() {
            Ok(accounts) => {
                // Démarrages décalés : pas de rafale de connexions au lancement
                for (i, account) in accounts.into_iter().enumerate() {
                    let app = self.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(3 * i as u64)).await;
                        app.start_miner(account);
                    });
                }
            }
            Err(e) => tracing::error!("lecture des comptes : {e}"),
        }
    }

    pub fn start_miner(self: &Arc<Self>, account: Account) {
        self.stop_miner(account.id);
        let (tx, rx) = mpsc::channel(32);
        let id = account.id;
        let task = tauri::async_runtime::spawn(miner::run(self.clone(), account, rx));
        self.miners.lock().unwrap().insert(id, MinerHandle { commands: tx, task });
    }

    pub fn stop_miner(&self, account_id: i64) {
        if let Some(handle) = self.miners.lock().unwrap().remove(&account_id) {
            handle.task.abort();
        }
        self.views.write().unwrap().remove(&account_id);
        self.touch();
    }

    pub fn send(&self, account_id: i64, command: MinerCommand) {
        let sender = self.miners.lock().unwrap().get(&account_id).map(|h| h.commands.clone());
        if let Some(sender) = sender {
            let _ = sender.try_send(command);
        }
    }

    pub fn broadcast(&self, command: MinerCommand) {
        let senders: Vec<_> = self.miners.lock().unwrap().values().map(|h| h.commands.clone()).collect();
        for sender in senders {
            let _ = sender.try_send(command.clone());
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
        self.broadcast(MinerCommand::Wake);
        self.touch();
        let _ = self.handle.emit("paused", paused);
    }

    pub fn set_view(&self, view: AccountView) {
        self.views.write().unwrap().insert(view.id, view);
        self.touch();
    }

    pub fn log(&self, account: &str, kind: EventKind, message: impl Into<String>) {
        let event = LogEvent {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            account: account.to_owned(),
            kind,
            message: message.into(),
        };
        tracing::info!("[{}] {}", event.account, event.message);
        let mut events = self.events.lock().unwrap();
        events.push_front(event);
        events.truncate(MAX_EVENTS);
        drop(events);
        self.touch();
    }

    pub fn snapshot(&self) -> Snapshot {
        let views = self.views.read().unwrap();
        // Ordre des comptes = ordre en base ; un compte sans superviseur apparaît quand même
        let accounts = self
            .db
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .map(|a| {
                views.get(&a.id).cloned().unwrap_or(AccountView {
                    id: a.id,
                    login: a.login,
                    user_id: a.user_id,
                    points_enabled: a.points_enabled,
                    status: AccountStatus::Starting,
                    status_text: None,
                    realtime: false,
                    chat_connected: false,
                    drops: crate::drops::DropsEngine::default().view(a.drops_enabled),
                    se_account: SeAccount::None,
                    channels: vec![],
                })
            })
            .collect();
        let (operations_version, operations_updated) = gql::operations_info();
        Snapshot {
            paused: self.is_paused(),
            accounts,
            events: self.events.lock().unwrap().iter().cloned().collect(),
            operations_version,
            operations_updated,
            giveaways: self.giveaways.view(),
            languages: self.languages.read().unwrap().clone(),
            scanner: self.scanner.lock().unwrap().view(),
            insights: self.insights.lock().unwrap().view(),
        }
    }

    /// Toutes les chaînes affichées : listes des comptes, chaînes surveillées, chaîne des drops
    fn known_logins(&self) -> Vec<String> {
        let mut logins: Vec<String> = vec![];
        for account in self.db.accounts().unwrap_or_default() {
            logins.extend(self.db.channels(account.id).unwrap_or_default().into_iter().map(|c| c.login));
        }
        logins.extend(self.db.giveaway_channels().unwrap_or_default().into_iter().map(|c| c.login));
        for view in self.views.read().unwrap().values() {
            if let Some(t) = &view.drops.target {
                logins.push(t.login.clone());
            }
        }
        logins.sort();
        logins.dedup();
        logins
    }

    /// Relève la langue des chaînes : les nouvelles chaque minute, toutes toutes les 6 h
    pub fn spawn_languages(self: &Arc<Self>) {
        let app = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut full_at: Option<std::time::Instant> = None;
            loop {
                let full = full_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(6 * 3600));
                let known = app.languages.read().unwrap().clone();
                let wanted: Vec<String> =
                    app.known_logins().into_iter().filter(|l| full || !known.contains_key(l)).collect();
                if !wanted.is_empty() {
                    match crate::twitch::gql::channel_languages(&app.http, &wanted).await {
                        Ok(found) if !found.is_empty() => {
                            app.languages.write().unwrap().extend(found);
                            app.touch();
                            if full {
                                full_at = Some(std::time::Instant::now());
                            }
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!("langues des chaînes : {e}"),
                    }
                }
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
    }

    /// Notification Windows, si l'utilisateur ne les a pas coupées
    pub fn notify(&self, title: &str, body: &str) {
        use tauri_plugin_notification::NotificationExt;
        if !self.db.settings().notifications {
            return;
        }
        if let Err(e) = self.handle.notification().builder().title(title).body(body).show() {
            tracing::warn!("notification : {e}");
        }
    }

    pub fn touch(&self) {
        self.dirty.notify_one();
    }

    /// Envoie l'état à l'interface au plus toutes les 250 ms
    pub fn spawn_emitter(self: &Arc<Self>) {
        let app = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                app.dirty.notified().await;
                tokio::time::sleep(EMIT_DEBOUNCE).await;
                let _ = app.handle.emit("snapshot", app.snapshot());
            }
        });
    }
}
