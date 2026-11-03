//! État partagé de l'application et vue envoyée à l'interface.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
    /// Solde voulu atteint : plus regardée
    BalanceReached,
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
    /// Renseigné par le superviseur mais pas affiché : non envoyé à l'interface
    #[serde(skip_serializing)]
    #[allow(dead_code)]
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
    pub max_balance: Option<i64>,
    pub se_poll_minutes: u64,
    /// None : pas encore vérifié ; false : la chaîne n'utilise pas StreamElements
    pub se_enabled: Option<bool>,
    pub se_points: Option<i64>,
    pub se_earned_today: i64,
    /// Photo de profil (connue après la première relecture du statut live)
    pub profile_image: Option<String>,
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

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize)]
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

/// Type d'alerte : chacun a son son, activable dans les réglages
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Alert {
    /// Giveaway détecté dans un chat
    Giveaway,
    /// Un de nos comptes a gagné
    Win,
    /// Un de nos pseudos est mentionné
    Mention,
    /// Tirage imminent avec présence requise
    Reminder,
    /// Nouveau stream repéré par le scanner
    Scanner,
}

impl Alert {
    pub const ALL: [Alert; 5] = [Alert::Giveaway, Alert::Win, Alert::Mention, Alert::Reminder, Alert::Scanner];

    pub fn key(self) -> &'static str {
        match self {
            Alert::Giveaway => "giveaway",
            Alert::Win => "win",
            Alert::Mention => "mention",
            Alert::Reminder => "reminder",
            Alert::Scanner => "scanner",
        }
    }

    /// Page ouverte par un clic sur la notification
    pub fn page(self) -> &'static str {
        match self {
            Alert::Giveaway | Alert::Mention | Alert::Reminder => "/giveaways",
            Alert::Win => "/giveaways/wins",
            Alert::Scanner => "/giveaways/scanner",
        }
    }
}

/// Clic sur une notification encore à l'écran (Linux, et Windows en développement) : fenêtre
/// principale affichée, sur la page de l'alerte
pub fn on_notification_click(handle: &AppHandle) {
    use tauri_plugin_notification::NotificationExt;
    let app = handle.clone();
    let _ = handle.notification().on_action(move |action| {
        if action.action_id() != "tap" {
            return;
        }
        let page = action.notification().and_then(|n| n.extra().get("page")).and_then(|p| p.as_str()).map(str::to_owned);
        let app = app.clone();
        // Fenêtre manipulée depuis le fil principal (le gestionnaire tourne sur un autre fil)
        let _ = app.clone().run_on_main_thread(move || match page {
            Some(page) => crate::toast::open_page(&app, &page),
            None => crate::tray::show_main(&app),
        });
    });
}

/// Parties de l'état envoyées séparément à l'interface. Chaque partie n'est reconstruite
/// que si elle a été signalée modifiée, et n'est envoyée que si son JSON a changé.
///
/// Clés envoyées (forme de `Snapshot` côté interface) :
/// - `Misc` : `paused`, `operationsVersion`, `operationsUpdated`
/// - `Accounts` : `accounts` (chaînes et drops compris)
/// - `Events` : `events` (ajouts seulement dans les mises à jour)
/// - `Giveaways` : `giveaways.channels`, `giveaways.games`, `giveaways.giveaways`,
///   `giveaways.mentions`, `giveaways.botReplies`
/// - `Languages` : `languages` (login → code ISO 639-1, pour les drapeaux)
/// - `Scanner` : `scanner`
/// - `Insights` : `insights` (giveaway StreamElements actif, description, message épinglé)
/// - `Sync` : `sync`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Misc,
    Accounts,
    Events,
    Giveaways,
    Languages,
    Scanner,
    Insights,
    Sync,
}

impl Domain {
    const ALL: u32 = (1 << 8) - 1;

    fn bit(self) -> u32 {
        1 << self as u32
    }
}

/// Fil d'événements, le plus récent d'abord ; `seq` compte tous les événements ajoutés
struct EventLog {
    list: VecDeque<LogEvent>,
    seq: u64,
}

/// Dernier état envoyé à l'interface, partie par partie (JSON déjà sérialisé)
#[derive(Default)]
struct Emitted {
    /// Augmente à chaque mise à jour envoyée : l'interface repère un trou et relit tout
    version: u64,
    parts: BTreeMap<&'static str, String>,
    /// Valeur de `EventLog::seq` au dernier envoi
    events_seq: u64,
}

fn to_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| {
        tracing::warn!("état illisible : {e}");
        "null".into()
    })
}

/// Mémorise les parties fraîches ; rend les clés de celles qui ont changé depuis le dernier envoi
fn diff_parts(sent: &mut BTreeMap<&'static str, String>, fresh: Vec<(&'static str, String)>) -> Vec<&'static str> {
    let mut changed = vec![];
    for (key, json) in fresh {
        if sent.get(key) != Some(&json) {
            sent.insert(key, json);
            changed.push(key);
        }
    }
    changed
}

/// Objet JSON des clés données ; une clé `a.b` est rangée sous l'objet `a` si `nest`
fn compose(parts: &BTreeMap<&'static str, String>, keys: &[&'static str], nest: bool) -> String {
    let mut top: Vec<(String, String)> = vec![];
    let mut groups: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for key in keys {
        let Some(json) = parts.get(key) else { continue };
        match key.split_once('.') {
            Some((group, sub)) if nest => groups.entry(group).or_default().push((sub, json)),
            _ => top.push((to_json(key), json.clone())),
        }
    }
    for (group, subs) in groups {
        let inner: Vec<String> = subs.iter().map(|(k, v)| format!("{}:{v}", to_json(k))).collect();
        top.push((to_json(&group), format!("{{{}}}", inner.join(","))));
    }
    let fields: Vec<String> = top.into_iter().map(|(k, v)| format!("{k}:{v}")).collect();
    format!("{{{}}}", fields.join(","))
}

fn raw(json: String) -> Box<serde_json::value::RawValue> {
    serde_json::value::RawValue::from_string(json).expect("JSON produit par serde_json")
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
    events: Mutex<EventLog>,
    paused: AtomicBool,
    dirty: Notify,
    /// Parties de l'état modifiées depuis le dernier envoi (bits de `Domain`)
    dirty_domains: AtomicU32,
    emitted: Mutex<Emitted>,
    pub login_task: Mutex<Option<JoinHandle<()>>>,
    pub giveaways: crate::giveaways::GiveawayHandle,
    /// Chaînes à regarder en premier pour les points : giveaway par points de chaîne ouvert
    pub point_priority: RwLock<std::collections::HashSet<String>>,
    pub scanner: Mutex<crate::scanner::ScannerState>,
    pub insights: Mutex<crate::insights::InsightsState>,
    pub sync: crate::sync::SyncHandle,
    /// Réponses automatiques au gain en attente de leur délai
    pub win_replies: crate::win_reply::WinReplies,
    languages: RwLock<HashMap<String, String>>,
    /// Statut live et infos StreamElements par chaîne, partagés entre comptes
    pub lives: crate::lives::Lives,
}

impl App {
    pub fn new(handle: AppHandle, db: Db, secrets: Secrets, data_dir: PathBuf) -> Arc<Self> {
        let events = db.recent_events(MAX_EVENTS).unwrap_or_default();
        Arc::new(Self {
            db,
            secrets,
            http: crate::twitch::http_client(),
            data_dir,
            handle,
            miners: Mutex::new(HashMap::new()),
            views: RwLock::new(HashMap::new()),
            // Fil d'événements gardé d'un lancement (et d'une mise à jour) à l'autre
            events: Mutex::new(EventLog { list: events.into(), seq: 0 }),
            paused: AtomicBool::new(false),
            dirty: Notify::new(),
            dirty_domains: AtomicU32::new(Domain::ALL),
            emitted: Mutex::new(Emitted::default()),
            login_task: Mutex::new(None),
            giveaways: crate::giveaways::GiveawayHandle::default(),
            point_priority: RwLock::new(std::collections::HashSet::new()),
            scanner: Mutex::new(crate::scanner::ScannerState::default()),
            insights: Mutex::new(crate::insights::InsightsState::default()),
            sync: crate::sync::SyncHandle::default(),
            win_replies: crate::win_reply::WinReplies::default(),
            languages: RwLock::new(HashMap::new()),
            lives: crate::lives::Lives::default(),
        })
    }

    /// Langue de diffusion d'une chaîne (code ISO 639-1), si connue
    pub fn channel_language(&self, login: &str) -> Option<String> {
        self.languages.read().unwrap().get(login).cloned()
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
        // Démarrages simultanés (connexion et relance) : un seul superviseur par compte
        if let Some(previous) = self.miners.lock().unwrap().insert(id, MinerHandle { commands: tx, task }) {
            previous.task.abort();
        }
    }

    pub fn stop_miner(&self, account_id: i64) {
        if let Some(handle) = self.miners.lock().unwrap().remove(&account_id) {
            handle.task.abort();
        }
        self.views.write().unwrap().remove(&account_id);
        self.touch_domain(Domain::Accounts);
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
        self.touch_domain(Domain::Misc);
        let _ = self.handle.emit("paused", paused);
    }

    pub fn set_view(&self, view: AccountView) {
        self.views.write().unwrap().insert(view.id, view);
        self.touch_domain(Domain::Accounts);
    }

    pub fn log(&self, account: &str, kind: EventKind, message: impl Into<String>) {
        let event = LogEvent {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            account: account.to_owned(),
            kind,
            message: message.into(),
        };
        tracing::info!("[{}] {}", event.account, event.message);
        if let Err(e) = self.db.add_event(&event, MAX_EVENTS) {
            tracing::warn!("fil d'événements : {e}");
        }
        let mut events = self.events.lock().unwrap();
        events.list.push_front(event);
        events.list.truncate(MAX_EVENTS);
        events.seq += 1;
        drop(events);
        self.touch_domain(Domain::Events);
    }

    /// Comptes dans l'ordre de la base ; un compte sans superviseur apparaît quand même
    pub(crate) fn account_views(&self) -> Vec<AccountView> {
        let views = self.views.read().unwrap();
        self.db
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
            .collect()
    }

    /// Parties à jour des domaines demandés (bits de `Domain`), fil d'événements à part
    fn build_parts(&self, mask: u32) -> Vec<(&'static str, String)> {
        let wants = |d: Domain| mask & d.bit() != 0;
        let mut parts = vec![];
        if wants(Domain::Misc) {
            let (operations_version, operations_updated) = gql::operations_info();
            parts.push(("paused", to_json(&self.is_paused())));
            parts.push(("operationsVersion", to_json(&operations_version)));
            parts.push(("operationsUpdated", to_json(&operations_updated)));
        }
        if wants(Domain::Accounts) {
            parts.push(("accounts", to_json(&self.account_views())));
        }
        if wants(Domain::Giveaways) {
            let g = self.giveaways.view();
            parts.push(("giveaways.channels", to_json(&g.channels)));
            parts.push(("giveaways.games", to_json(&g.games)));
            parts.push(("giveaways.giveaways", to_json(&g.giveaways)));
            parts.push(("giveaways.mentions", to_json(&g.mentions)));
            parts.push(("giveaways.botReplies", to_json(&g.bot_replies)));
        }
        if wants(Domain::Languages) {
            parts.push(("languages", to_json(&*self.languages.read().unwrap())));
        }
        if wants(Domain::Scanner) {
            parts.push(("scanner", to_json(&self.scanner.lock().unwrap().view())));
        }
        if wants(Domain::Insights) {
            parts.push(("insights", to_json(&self.insights.lock().unwrap().view())));
        }
        if wants(Domain::Sync) {
            parts.push(("sync", to_json(&self.sync.view())));
        }
        parts
    }

    /// Reconstruit les parties signalées modifiées et envoie à l'interface celles dont le JSON
    /// a changé (événement « snapshot-patch ») : `{ version, domains: { clé: valeur },
    /// eventsAppend?, eventsMax? }`. Le fil d'événements n'envoie que ses nouveaux éléments
    /// (`eventsAppend`, les plus récents d'abord), ou tout le fil (`domains.events`) au besoin.
    fn flush(&self, emitted: &mut Emitted) {
        let mask = self.dirty_domains.swap(0, Ordering::AcqRel);
        if mask == 0 {
            return;
        }
        let mut changed = diff_parts(&mut emitted.parts, self.build_parts(mask));
        let mut append = None;
        if mask & Domain::Events.bit() != 0 {
            let log = self.events.lock().unwrap();
            let fresh = log.seq.saturating_sub(emitted.events_seq) as usize;
            let known = emitted.parts.contains_key("events");
            if fresh > 0 || !known {
                emitted.parts.insert("events", to_json(&log.list));
                emitted.events_seq = log.seq;
                if known && fresh < log.list.len() {
                    append = Some(to_json(&log.list.iter().take(fresh).collect::<Vec<_>>()));
                } else {
                    changed.push("events");
                }
            }
        }
        if changed.is_empty() && append.is_none() {
            return;
        }
        emitted.version += 1;
        let mut payload =
            format!("{{\"version\":{},\"domains\":{}", emitted.version, compose(&emitted.parts, &changed, false));
        if let Some(items) = append {
            payload.push_str(&format!(",\"eventsAppend\":{items},\"eventsMax\":{MAX_EVENTS}"));
        }
        payload.push('}');
        if let Err(e) = self.handle.emit("snapshot-patch", raw(payload)) {
            tracing::warn!("envoi de l'état : {e}");
        }
    }

    /// État complet (forme `Snapshot` de l'interface) et sa version, pour le chargement de
    /// l'interface : les mises à jour suivantes (« snapshot-patch ») s'appliquent par-dessus
    pub fn snapshot(&self) -> Box<serde_json::value::RawValue> {
        let mut emitted = self.emitted.lock().unwrap();
        // Modifications en attente envoyées d'abord : l'état rendu est exactement celui de la version
        self.flush(&mut emitted);
        let keys: Vec<&'static str> = emitted.parts.keys().copied().collect();
        let mut parts = emitted.parts.clone();
        parts.insert("version", emitted.version.to_string());
        let mut keys = keys;
        keys.push("version");
        raw(compose(&parts, &keys, true))
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
                            app.touch_domain(Domain::Languages);
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

    /// Notification Windows et alerte sonore (jouée par l'interface), selon les réglages ;
    /// notification push sur les téléphones connectés
    pub fn notify(&self, alert: Alert, title: &str, body: &str) {
        self.notify_page(alert, title, body, alert.page());
    }

    /// Comme `notify`, ouverte au clic sur `page` (ex. le stream à mettre en avant)
    pub fn notify_page(&self, alert: Alert, title: &str, body: &str, page: &str) {
        self.notify_full(alert, title, body, page, None);
    }

    /// Comme `notify_page`, avec un bouton (libellé, lien `twitch-desk://`) sur la notification
    /// Windows de l'application installée
    pub fn notify_full(&self, alert: Alert, title: &str, body: &str, page: &str, action: Option<(&str, &str)>) {
        use tauri_plugin_notification::NotificationExt;
        // Téléphones connectés : chacun a choisi ses alertes, quels que soient les réglages du PC
        self.sync.phone_alert(alert.key(), title, body);
        let settings = self.db.settings();
        if settings.sound_alerts && settings.sound_events.iter().any(|e| e == alert.key()) {
            let sound = settings.sound_choices.get(alert.key()).cloned();
            let payload = serde_json::json!({ "kind": alert.key(), "volume": settings.sound_volume.min(100), "sound": sound });
            if let Err(e) = self.handle.emit("sound-alert", payload) {
                tracing::warn!("alerte sonore : {e}");
            }
        }
        if !settings.notifications {
            return;
        }
        // Windows : notification ouverte sur sa page même depuis le Centre de notifications
        if crate::toast::show(&self.handle.config().identifier, title, body, page, action) {
            return;
        }
        if let Err(e) = self.handle.notification().builder().title(title).body(body).extra("page", page).show() {
            tracing::warn!("notification : {e}");
        }
    }

    /// État modifié sans préciser quoi : tout est reconstruit (seul ce qui a changé part)
    pub fn touch(&self) {
        self.dirty_domains.fetch_or(Domain::ALL, Ordering::AcqRel);
        self.dirty.notify_one();
    }

    /// Une partie de l'état a changé : elle seule est reconstruite au prochain envoi
    pub fn touch_domain(&self, domain: Domain) {
        // Giveaways (participations, statut live) ou comptes : état de la web app à republier
        if matches!(domain, Domain::Giveaways | Domain::Accounts) {
            self.sync.state_changed();
        }
        self.dirty_domains.fetch_or(domain.bit(), Ordering::AcqRel);
        self.dirty.notify_one();
    }

    /// Envoie à l'interface les parties modifiées de l'état, au plus toutes les 250 ms
    pub fn spawn_emitter(self: &Arc<Self>) {
        let app = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                app.dirty.notified().await;
                tokio::time::sleep(EMIT_DEBOUNCE).await;
                app.flush(&mut app.emitted.lock().unwrap());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_parts_ne_garde_que_ce_qui_change() {
        let mut sent = BTreeMap::new();
        let first = diff_parts(&mut sent, vec![("accounts", "[]".into()), ("sync", "{\"a\":1}".into())]);
        assert_eq!(first, vec!["accounts", "sync"]);
        // Même JSON : rien à envoyer
        assert!(diff_parts(&mut sent, vec![("accounts", "[]".into()), ("sync", "{\"a\":1}".into())]).is_empty());
        // Une seule partie change ; les absentes (non reconstruites) restent mémorisées
        assert_eq!(diff_parts(&mut sent, vec![("sync", "{\"a\":2}".into())]), vec!["sync"]);
        assert_eq!(sent.get("accounts").map(String::as_str), Some("[]"));
        assert_eq!(sent.get("sync").map(String::as_str), Some("{\"a\":2}"));
    }

    #[test]
    fn compose_range_les_sous_cles() {
        let mut parts = BTreeMap::new();
        parts.insert("paused", "false".to_string());
        parts.insert("giveaways.games", "[]".to_string());
        parts.insert("giveaways.mentions", "[1]".to_string());
        let keys: Vec<&'static str> = parts.keys().copied().collect();
        let nested: serde_json::Value = serde_json::from_str(&compose(&parts, &keys, true)).unwrap();
        assert_eq!(nested, serde_json::json!({ "paused": false, "giveaways": { "games": [], "mentions": [1] } }));
        let flat: serde_json::Value = serde_json::from_str(&compose(&parts, &["giveaways.games"], false)).unwrap();
        assert_eq!(flat, serde_json::json!({ "giveaways.games": [] }));
        assert_eq!(compose(&parts, &[], false), "{}");
    }

    #[test]
    fn domaines_distincts() {
        let all = [
            Domain::Misc,
            Domain::Accounts,
            Domain::Events,
            Domain::Giveaways,
            Domain::Languages,
            Domain::Scanner,
            Domain::Insights,
            Domain::Sync,
        ];
        let mask = all.iter().fold(0, |m, d| {
            assert_eq!(m & d.bit(), 0);
            m | d.bit()
        });
        assert_eq!(mask, Domain::ALL);
    }
}
