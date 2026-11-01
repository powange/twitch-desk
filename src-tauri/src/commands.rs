//! Commandes appelées par l'interface (invoke).

use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, State};

use crate::app::{App, EventKind, Snapshot};
use crate::db::{ChannelList, Settings};
use crate::giveaways::MonitorCmd;
use crate::miner::MinerCommand;
use crate::secrets::Secrets;
use crate::streamelements::{self, SeError};
use crate::twitch::auth::{self, DeviceCode, PollResult};
use crate::twitch::gql::Gql;
use crate::twitch::random_device_id;

type AppState<'a> = State<'a, Arc<App>>;
type CmdResult<T> = Result<T, String>;

fn db_err(e: rusqlite::Error) -> String {
    format!("base de données : {e}")
}

#[tauri::command]
pub fn get_snapshot(app: AppState) -> Snapshot {
    app.snapshot()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum LoginEvent {
    Done { login: String },
    Expired,
    Denied,
    Error { message: String },
}

/// Lance une connexion par code d'appareil ; le résultat arrive par l'événement « login »
#[tauri::command]
pub async fn start_login(app: AppState<'_>) -> CmdResult<DeviceCode> {
    let app: Arc<App> = app.inner().clone();
    let code = auth::start_device_flow(&app.http).await.map_err(|e| e.to_string())?;
    let task_app = app.clone();
    let device = code.clone();
    let task = tauri::async_runtime::spawn(async move {
        let event = poll_login(&task_app, &device).await;
        let _ = task_app.handle().emit("login", event);
    });
    if let Some(old) = app.login_task.lock().unwrap().replace(task) {
        old.abort();
    }
    Ok(code)
}

async fn poll_login(app: &Arc<App>, code: &DeviceCode) -> LoginEvent {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(code.expires_in);
    let mut every = Duration::from_secs(code.interval.max(1));
    loop {
        tokio::time::sleep(every).await;
        if tokio::time::Instant::now() >= deadline {
            return LoginEvent::Expired;
        }
        match auth::poll_device_flow(&app.http, &code.device_code).await {
            Ok(PollResult::Pending) => {}
            Ok(PollResult::SlowDown) => every += Duration::from_secs(1),
            Ok(PollResult::Expired) => return LoginEvent::Expired,
            Ok(PollResult::Denied) => return LoginEvent::Denied,
            Ok(PollResult::Done(tokens)) => return finish_login(app, tokens).await,
            // Coupure réseau passagère : on continue jusqu'à l'expiration du code
            Err(e) => tracing::warn!("connexion : {e}"),
        }
    }
}

async fn finish_login(app: &Arc<App>, tokens: auth::Tokens) -> LoginEvent {
    let validation = match auth::validate(&app.http, &tokens.access_token).await {
        Ok(v) => v,
        Err(e) => return LoginEvent::Error { message: e.to_string() },
    };
    let device_id = random_device_id();
    let id = match app.db.upsert_account(&validation.user_id, &validation.login, &device_id) {
        Ok(id) => id,
        Err(e) => return LoginEvent::Error { message: db_err(e) },
    };
    if let Err(e) = app.secrets.save(&validation.user_id, &tokens) {
        return LoginEvent::Error { message: format!("enregistrement du jeton : {e}") };
    }
    app.log(&validation.login, EventKind::Info, "Compte ajouté");
    if let Some(account) = app.db.accounts().ok().and_then(|all| all.into_iter().find(|a| a.id == id)) {
        app.start_miner(account);
    }
    LoginEvent::Done { login: validation.login }
}

#[tauri::command]
pub fn cancel_login(app: AppState) {
    if let Some(task) = app.login_task.lock().unwrap().take() {
        task.abort();
    }
}

#[tauri::command]
pub fn remove_account(app: AppState, id: i64) -> CmdResult<()> {
    let account = app.db.accounts().map_err(db_err)?.into_iter().find(|a| a.id == id);
    app.stop_miner(id);
    if let Some(account) = account {
        app.secrets.delete(&account.user_id);
        app.secrets.delete(&Secrets::se_key(&account.user_id));
        app.log(&account.login, EventKind::Info, "Compte retiré");
    }
    app.db.delete_account(id).map_err(db_err)?;
    app.touch();
    Ok(())
}

/// Relance le superviseur d'un compte (après une erreur)
#[tauri::command]
pub fn restart_account(app: AppState, id: i64) -> CmdResult<()> {
    let account = app.db.accounts().map_err(db_err)?.into_iter().find(|a| a.id == id);
    match account {
        Some(account) => {
            app.start_miner(account);
            Ok(())
        }
        None => Err("compte introuvable".into()),
    }
}

#[tauri::command]
pub fn set_points_enabled(app: AppState, id: i64, enabled: bool) -> CmdResult<()> {
    app.db.set_points_enabled(id, enabled).map_err(db_err)?;
    app.send(id, MinerCommand::Reload);
    Ok(())
}

/// Login Twitch tel que saisi (pseudo, @pseudo ou adresse de la chaîne) → login normalisé
pub fn normalize_login(input: &str) -> Option<String> {
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^[a-z0-9_]{2,25}$").unwrap());
    let mut s = input.trim().to_lowercase();
    for prefix in ["https://", "http://", "www.", "m.", "twitch.tv/", "@"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_owned();
        }
    }
    let s = s.split(['/', '?', '#']).next().unwrap_or("").to_owned();
    RE.is_match(&s).then_some(s)
}

#[tauri::command]
pub async fn add_channel(app: AppState<'_>, account_id: i64, login: String, list: ChannelList) -> CmdResult<String> {
    let login = normalize_login(&login).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    let account = app
        .db
        .accounts()
        .map_err(db_err)?
        .into_iter()
        .find(|a| a.id == account_id)
        .ok_or("compte introuvable")?;
    let tokens = app.secrets.load(&account.user_id).ok_or("Reconnecte ce compte avant d'ajouter une chaîne")?;
    let gql = Gql::new(app.http.clone(), tokens.access_token, Some(account.device_id));
    let channel_id = gql
        .channel_id(&login)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("La chaîne « {login} » n'existe pas"))?;
    app.db.add_channel(account_id, &login, &channel_id, list).map_err(db_err)?;
    app.send(account_id, MinerCommand::Reload);
    Ok(login)
}

#[tauri::command]
pub fn remove_channel(app: AppState, id: i64, list: ChannelList) -> CmdResult<()> {
    let account = app.db.channel_account(id).map_err(db_err)?;
    app.db.remove_channel(id, list).map_err(db_err)?;
    if let Some(account) = account {
        app.send(account, MinerCommand::Reload);
    }
    Ok(())
}

#[tauri::command]
pub fn reorder_channels(app: AppState, account_id: i64, ids: Vec<i64>) -> CmdResult<()> {
    app.db.reorder_channels(account_id, &ids).map_err(db_err)?;
    app.send(account_id, MinerCommand::Reload);
    Ok(())
}

#[tauri::command]
pub fn set_channel_options(
    app: AppState,
    id: i64,
    enabled: bool,
    auto_claim: bool,
    chat_presence: bool,
) -> CmdResult<()> {
    app.db.set_channel_options(id, enabled, auto_claim, chat_presence).map_err(db_err)?;
    if let Some(account) = app.db.channel_account(id).map_err(db_err)? {
        app.send(account, MinerCommand::Reload);
    }
    Ok(())
}

/// Solde de points de chaîne à atteindre (None : pas de limite)
#[tauri::command]
pub fn set_max_balance(app: AppState, id: i64, max: Option<i64>) -> CmdResult<()> {
    if max.is_some_and(|m| !(1..=2_000_000_000).contains(&m)) {
        return Err("Solde maximum invalide".into());
    }
    app.db.set_max_balance(id, max).map_err(db_err)?;
    if let Some(account) = app.db.channel_account(id).map_err(db_err)? {
        app.send(account, MinerCommand::Reload);
    }
    Ok(())
}

/// Relevé du solde StreamElements d'une chaîne, en minutes (None : automatique)
#[tauri::command]
pub fn set_se_refresh(app: AppState, id: i64, minutes: Option<i64>) -> CmdResult<()> {
    if minutes.is_some_and(|m| !(1..=60).contains(&m)) {
        return Err("Fréquence invalide".into());
    }
    app.db.set_se_refresh(id, minutes).map_err(db_err)?;
    if let Some(account) = app.db.channel_account(id).map_err(db_err)? {
        app.send(account, MinerCommand::Reload);
    }
    Ok(())
}

#[tauri::command]
pub fn set_paused(app: AppState, paused: bool) {
    app.set_paused(paused);
    crate::tray::refresh(app.handle(), paused);
}

#[tauri::command]
pub fn get_settings(app: AppState) -> Settings {
    app.db.settings()
}

#[tauri::command]
pub fn save_settings(app: AppState, settings: Settings) -> CmdResult<()> {
    app.db.save_settings(&settings).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paths {
    data: String,
    logs: String,
}

#[tauri::command]
pub fn get_paths(app: AppState) -> Paths {
    Paths {
        data: app.data_dir.display().to_string(),
        logs: app.data_dir.join("logs").display().to_string(),
    }
}

// --- Journaux ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFile {
    name: String,
    size: u64,
    modified: Option<String>,
}

/// Fichier du dossier des journaux, d'après son seul nom (pas de chemin)
fn log_path(app: &App, name: &str) -> CmdResult<std::path::PathBuf> {
    let valid = name.starts_with("twitch-desk") && name.ends_with(".log") && !name.contains(['/', '\\']) && !name.contains("..");
    if !valid {
        return Err("Journal inconnu".into());
    }
    let path = app.data_dir.join("logs").join(name);
    if !path.is_file() {
        return Err("Ce journal n'existe plus".into());
    }
    Ok(path)
}

/// Journaux des derniers jours, le plus récent d'abord
#[tauri::command]
pub fn list_logs(app: AppState) -> CmdResult<Vec<LogFile>> {
    let dir = app.data_dir.join("logs");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(vec![]),
    };
    let mut files: Vec<LogFile> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            (name.starts_with("twitch-desk") && name.ends_with(".log")).then(|| LogFile {
                name,
                size: meta.len(),
                modified: meta
                    .modified()
                    .ok()
                    .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            })
        })
        .collect();
    files.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(files)
}

/// Fin d'un journal (2 Mo au plus) ; `truncated` : le début n'est pas inclus
#[derive(Serialize)]
pub struct LogContent {
    text: String,
    truncated: bool,
}

const LOG_TAIL: u64 = 2 * 1024 * 1024;

#[tauri::command]
pub fn read_log(app: AppState, name: String) -> CmdResult<LogContent> {
    use std::io::{Read, Seek, SeekFrom};
    let path = log_path(&app, &name)?;
    let mut file = std::fs::File::open(&path).map_err(|e| format!("lecture du journal : {e}"))?;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let truncated = len > LOG_TAIL;
    if truncated {
        file.seek(SeekFrom::Start(len - LOG_TAIL)).map_err(|e| format!("lecture du journal : {e}"))?;
    }
    let mut bytes = vec![];
    file.read_to_end(&mut bytes).map_err(|e| format!("lecture du journal : {e}"))?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    // Première ligne coupée par la lecture partielle : retirée
    if truncated {
        if let Some(i) = text.find('\n') {
            text.drain(..=i);
        }
    }
    Ok(LogContent { text, truncated })
}

/// Copie un journal dans le fichier choisi
#[tauri::command]
pub fn export_log(app: AppState, name: String, path: String) -> CmdResult<()> {
    let source = log_path(&app, &name)?;
    std::fs::copy(source, &path).map_err(|e| format!("copie du journal : {e}"))?;
    Ok(())
}

/// Invitation Discord d'un streamer : celle de sa fiche, sinon celle trouvée sur sa chaîne
/// Twitch (réseaux sociaux, description) ; None si aucune
#[tauri::command]
pub async fn streamer_discord(app: AppState<'_>, login: String) -> CmdResult<Option<String>> {
    let login = normalize_login(&login).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    if let Some(url) = app.db.streamer(&login).map_err(db_err)?.and_then(|s| s.discord_url) {
        return Ok(Some(url));
    }
    let profile = crate::twitch::gql::channel_profile(&app.http, &login).await.map_err(|e| format!("chaîne Twitch : {e}"))?;
    Ok(profile.and_then(|p| p.discord).filter(|u| u.starts_with("https://")))
}

// --- Journal du chat ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatLogMeta {
    /// Commandes vues, avec leur nombre
    commands: Vec<(String, i64)>,
    /// Dernières réponses des bots aux commandes d'information
    replies: Vec<crate::db::BotReply>,
}

/// Messages enregistrés d'une chaîne (200 par page, les plus récents d'abord)
#[tauri::command]
pub fn get_chat_log(
    app: AppState,
    channel: String,
    kinds: Vec<String>,
    commands: Vec<String>,
    author: Option<String>,
    before_id: Option<i64>,
) -> CmdResult<Vec<crate::db::ChatLogEntry>> {
    let channel = normalize_login(&channel).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    app.db.chat_log(&channel, &kinds, &commands, author.as_deref(), before_id, 200).map_err(db_err)
}

/// Gagnant désigné à la main (tirage non détecté) : giveaway ouvert de la chaîne, sinon le plus récent
#[tauri::command]
pub fn set_manual_winner(app: AppState, channel: String, winner: String) -> CmdResult<()> {
    let channel = normalize_login(&channel).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    let winner = normalize_login(&winner).ok_or("Pseudo du gagnant invalide")?;
    let exists = app.db.giveaways(200).map_err(db_err)?.iter().any(|g| g.channel == channel);
    if !exists {
        return Err(format!("Aucun giveaway sur {channel} : ajoute-le d'abord"));
    }
    app.giveaways.send(MonitorCmd::ManualWinner { channel, winner });
    app.sync.poke();
    Ok(())
}

#[tauri::command]
pub fn get_chat_log_meta(app: AppState, channel: String) -> CmdResult<ChatLogMeta> {
    let channel = normalize_login(&channel).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    Ok(ChatLogMeta {
        commands: app.db.chat_log_commands(&channel).map_err(db_err)?,
        replies: app.db.bot_replies(&channel).map_err(db_err)?,
    })
}

// --- Follow ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowState {
    account_id: i64,
    login: String,
    /// None : impossible à savoir (compte à reconnecter, erreur Twitch)
    follows: Option<bool>,
    /// Date du follow
    followed_at: Option<String>,
}

/// Comptes qui suivent (ou non) une chaîne
#[tauri::command]
pub async fn follow_status(app: AppState<'_>, channel: String) -> CmdResult<Vec<FollowState>> {
    let channel = normalize_login(&channel).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    let mut out = vec![];
    for account in app.db.accounts().map_err(db_err)? {
        let state = match app.secrets.load(&account.user_id) {
            Some(tokens) => {
                let gql = Gql::new(app.http.clone(), tokens.access_token, Some(account.device_id.clone()));
                match gql.follow_state(&channel).await {
                    Ok(state) => state.map(|(_, since)| since),
                    Err(e) => {
                        tracing::warn!("follow de {} sur {channel} : {e}", account.login);
                        None
                    }
                }
            }
            None => None,
        };
        out.push(FollowState {
            account_id: account.id,
            login: account.login,
            follows: state.as_ref().map(Option::is_some),
            followed_at: state.flatten(),
        });
    }
    Ok(out)
}

/// Fait suivre la chaîne aux comptes choisis ; renvoie l'état à jour
#[tauri::command]
pub async fn follow_channel(app: AppState<'_>, channel: String, account_ids: Vec<i64>) -> CmdResult<Vec<FollowState>> {
    let channel = normalize_login(&channel).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    let mut errors = vec![];
    for id in account_ids {
        let account = find_account(&app, id)?;
        let tokens = app.secrets.load(&account.user_id).ok_or_else(|| format!("Reconnecte {}", account.login))?;
        let gql = Gql::new(app.http.clone(), tokens.access_token, Some(account.device_id.clone()));
        let result = match gql.follow_state(&channel).await {
            Ok(Some((_, Some(_)))) => Ok(()),
            Ok(Some((channel_id, None))) => gql.follow(&channel_id).await,
            Ok(None) => Err(crate::twitch::TwitchError::Gql("chaîne introuvable".into())),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => app.log(&account.login, crate::app::EventKind::Info, format!("Follow de {channel}")),
            Err(e) => errors.push(format!("{} : {e}", account.login)),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join(" · "));
    }
    follow_status(app, channel).await
}

// --- Sons des alertes ---

const SOUND_EXTENSIONS: [&str; 3] = ["wav", "mp3", "ogg"];
const MAX_SOUND: u64 = 5 * 1024 * 1024;

fn sound_kind(kind: &str) -> CmdResult<&'static str> {
    crate::app::Alert::ALL.iter().map(|a| a.key()).find(|k| *k == kind).ok_or_else(|| "Alerte inconnue".to_owned())
}

/// Fichier son copié pour une alerte (dossier « sounds » des données)
fn sound_file(app: &App, kind: &str) -> Option<std::path::PathBuf> {
    SOUND_EXTENSIONS.iter().map(|ext| app.data_dir.join("sounds").join(format!("{kind}.{ext}"))).find(|p| p.is_file())
}

/// Copie le fichier choisi comme son d'une alerte ; renvoie son nom (affichage)
#[tauri::command]
pub fn import_sound(app: AppState, kind: String, path: String) -> CmdResult<String> {
    let kind = sound_kind(&kind)?;
    let source = std::path::PathBuf::from(&path);
    let ext = source.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
    if !SOUND_EXTENSIONS.contains(&ext.as_str()) {
        return Err("Choisis un fichier WAV, MP3 ou OGG".into());
    }
    let size = std::fs::metadata(&source).map_err(|e| format!("fichier illisible : {e}"))?.len();
    if size > MAX_SOUND {
        return Err("Fichier trop gros (5 Mo au plus)".into());
    }
    let dir = app.data_dir.join("sounds");
    std::fs::create_dir_all(&dir).map_err(|e| format!("dossier des sons : {e}"))?;
    for other in SOUND_EXTENSIONS {
        let _ = std::fs::remove_file(dir.join(format!("{kind}.{other}")));
    }
    std::fs::copy(&source, dir.join(format!("{kind}.{ext}"))).map_err(|e| format!("copie du son : {e}"))?;
    Ok(source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// Contenu du son choisi pour une alerte (octets bruts)
#[tauri::command]
pub fn read_sound(app: AppState, kind: String) -> CmdResult<tauri::ipc::Response> {
    let kind = sound_kind(&kind)?;
    let path = sound_file(&app, kind).ok_or("Aucun fichier son pour cette alerte")?;
    let bytes = std::fs::read(path).map_err(|e| format!("lecture du son : {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

// --- Drops ---

#[tauri::command]
pub fn set_drops_enabled(app: AppState, id: i64, enabled: bool) -> CmdResult<()> {
    app.db.set_drops_enabled(id, enabled).map_err(db_err)?;
    app.send(id, MinerCommand::Reload);
    Ok(())
}

/// Ajoute un jeu souhaité, d'après son nom exact ou l'adresse de sa catégorie Twitch ;
/// renvoie le nom du jeu
#[tauri::command]
pub async fn add_drop_game(app: AppState<'_>, account_id: i64, input: String) -> CmdResult<String> {
    let input = input.trim().to_owned();
    if input.is_empty() {
        return Err("Indique le nom du jeu ou l'adresse de sa catégorie".into());
    }
    let account = find_account(&app, account_id)?;
    let tokens = app.secrets.load(&account.user_id).ok_or("Reconnecte ce compte avant d'ajouter un jeu")?;
    let gql = Gql::new(app.http.clone(), tokens.access_token, Some(account.device_id));
    let slug = match crate::drops::category_slug(&input) {
        Some(slug) => slug,
        None => gql
            .game_by_name(&input)
            .await
            .map_err(|e| e.to_string())?
            .map(|(_, slug)| slug)
            .ok_or_else(|| format!("Jeu « {input} » introuvable : écris son nom exact comme sur Twitch, ou colle l'adresse de sa catégorie"))?,
    };
    let game = gql
        .game_by_slug(&slug)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Catégorie « {slug} » introuvable sur Twitch"))?;
    app.db
        .add_drop_game(account_id, &game.id, &game.name, &slug, game.box_art.as_deref())
        .map_err(db_err)?;
    app.send(account_id, MinerCommand::Reload);
    Ok(game.name)
}

#[tauri::command]
pub fn remove_drop_game(app: AppState, id: i64) -> CmdResult<()> {
    if let Some(account) = app.db.remove_drop_game(id).map_err(db_err)? {
        app.send(account, MinerCommand::Reload);
    }
    Ok(())
}

#[tauri::command]
pub fn set_drop_game_enabled(app: AppState, id: i64, enabled: bool) -> CmdResult<()> {
    if let Some(account) = app.db.set_drop_game_enabled(id, enabled).map_err(db_err)? {
        app.send(account, MinerCommand::Reload);
    }
    Ok(())
}

#[tauri::command]
pub fn refresh_drops(app: AppState, account_id: i64) {
    app.send(account_id, MinerCommand::RefreshDrops);
}

#[tauri::command]
pub fn reorder_drop_games(app: AppState, account_id: i64, ids: Vec<i64>) -> CmdResult<()> {
    app.db.reorder_drop_games(account_id, &ids).map_err(db_err)?;
    app.send(account_id, MinerCommand::Reload);
    Ok(())
}

// --- Giveaways ---

/// Ajoute une chaîne à surveiller ; renvoie son login
#[tauri::command]
pub async fn add_giveaway_channel(app: AppState<'_>, login: String) -> CmdResult<String> {
    let login = normalize_login(&login).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    // N'importe quel compte connecté sert à vérifier que la chaîne existe
    let account = app.db.accounts().map_err(db_err)?.into_iter().next().ok_or("Connecte d'abord un compte Twitch")?;
    let tokens = app.secrets.load(&account.user_id).ok_or("Reconnecte ton compte Twitch")?;
    let gql = Gql::new(app.http.clone(), tokens.access_token, Some(account.device_id));
    let channel_id = gql
        .channel_id(&login)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("La chaîne « {login} » n'existe pas"))?;
    app.db.add_giveaway_channel(&login, &channel_id).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(login)
}

/// Ajoute d'un coup les chaînes des listes Points de chaîne et StreamElements ; renvoie le nombre ajouté
#[tauri::command]
pub fn import_giveaway_channels(app: AppState) -> CmdResult<usize> {
    let existing: Vec<String> = app.db.giveaway_channels().map_err(db_err)?.into_iter().map(|c| c.login).collect();
    let mut added = 0;
    for account in app.db.accounts().map_err(db_err)? {
        for ch in app.db.channels(account.id).map_err(db_err)? {
            if !existing.contains(&ch.login) {
                app.db.add_giveaway_channel(&ch.login, &ch.channel_id).map_err(db_err)?;
                added += 1;
            }
        }
    }
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(added)
}

/// Ajoute une catégorie où surveiller les giveaways (nom exact ou adresse de catégorie) ; renvoie son nom
#[tauri::command]
pub async fn add_giveaway_game(app: AppState<'_>, input: String) -> CmdResult<String> {
    let input = input.trim().to_owned();
    if input.is_empty() {
        return Err("Indique le nom de la catégorie ou son adresse".into());
    }
    let account = app.db.accounts().map_err(db_err)?.into_iter().next().ok_or("Connecte d'abord un compte Twitch")?;
    let tokens = app.secrets.load(&account.user_id).ok_or("Reconnecte ton compte Twitch")?;
    let gql = Gql::new(app.http.clone(), tokens.access_token, Some(account.device_id));
    let slug = match crate::drops::category_slug(&input) {
        Some(slug) => slug,
        None => gql
            .game_by_name(&input)
            .await
            .map_err(|e| e.to_string())?
            .map(|(_, slug)| slug)
            .ok_or_else(|| format!("Catégorie « {input} » introuvable : écris son nom exact comme sur Twitch, ou colle son adresse"))?,
    };
    let game = gql
        .game_by_slug(&slug)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Catégorie « {slug} » introuvable sur Twitch"))?;
    app.db.add_giveaway_game(&game.id, &game.name, &slug, game.box_art.as_deref()).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(game.name)
}

#[tauri::command]
pub fn remove_giveaway_game(app: AppState, id: i64) -> CmdResult<()> {
    app.db.remove_giveaway_game(id).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(())
}

#[tauri::command]
pub fn remove_giveaway_channel(app: AppState, id: i64) -> CmdResult<()> {
    app.db.remove_giveaway_channel(id).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(())
}

#[tauri::command]
/// Commandes du chat ignorées par la détection sur la chaîne d'un streamer (fiche du streamer)
pub fn set_ignored_commands(app: AppState, login: String, commands: Vec<String>) -> CmdResult<Vec<String>> {
    let login = normalize_login(&login).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    let list = crate::giveaways::detect::normalize_excluded(&commands);
    app.db.set_ignored_commands(&login, &list).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(list)
}

#[tauri::command]
pub async fn participate_giveaway(
    app: AppState<'_>,
    giveaway_id: i64,
    account_ids: Vec<i64>,
) -> CmdResult<Vec<crate::giveaways::ParticipationResult>> {
    let app: Arc<App> = app.inner().clone();
    crate::giveaways::participate(&app, giveaway_id, &account_ids, false).await
}

/// Ce qu'on sait d'une chaîne au moment d'y créer un giveaway (pré-remplissage du formulaire)
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSuggestion {
    /// Giveaway StreamElements actif sur la chaîne
    pub streamelements: Option<SeSuggestion>,
    /// Commande du dernier giveaway encore ouvert détecté dans son chat
    pub detected_command: Option<String>,
    /// Fiche publique de la chaîne (None : chaîne inconnue de Twitch)
    pub profile: Option<crate::twitch::gql::ChannelProfile>,
    /// Ce qu'on a noté sur ce streamer
    pub streamer: Option<crate::db::Streamer>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeSuggestion {
    pub url: String,
    pub title: String,
    pub description: String,
    pub command: Option<String>,
    pub cost: i64,
    pub max_tickets: Option<i64>,
}

#[tauri::command]
pub async fn inspect_giveaway_channel(app: AppState<'_>, login: String) -> CmdResult<ChannelSuggestion> {
    let Some(login) = normalize_login(&login) else { return Ok(ChannelSuggestion::default()) };
    let detected_command = app
        .db
        .giveaways(60)
        .map_err(db_err)?
        .into_iter()
        .find(|g| g.open && g.channel == login && g.kind == "command")
        .and_then(|g| g.command);
    let profile = crate::twitch::gql::channel_profile(&app.http, &login).await.unwrap_or_else(|e| {
        tracing::warn!("fiche de {login} : {e}");
        None
    });
    // Meilleur effort : StreamElements injoignable = pas de suggestion
    let streamelements = match streamelements::channel_id(&app.http, &login).await {
        Ok(Some(se_id)) => streamelements::active_giveaway(&app.http, &se_id).await.ok().flatten().map(|g| SeSuggestion {
            url: format!("https://streamelements.com/{login}/giveaway/{}", g.id),
            title: g.title,
            description: g.description,
            command: g.command,
            cost: g.cost,
            max_tickets: g.max_tickets,
        }),
        _ => None,
    };
    let streamer = app.db.streamer(&login).map_err(db_err)?;
    Ok(ChannelSuggestion { streamelements, detected_command, profile, streamer })
}

/// StreamElements → page StreamElements ; points de chaîne → page Points (prioritaire)
async fn add_giveaway_channel_to_lists(app: &Arc<App>, input: &crate::db::GiveawayInput) {
    let list = match input.kind.as_str() {
        "streamelements" => crate::db::ChannelList::Se,
        "channelpoints" => crate::db::ChannelList::Points,
        _ => return,
    };
    crate::giveaways::ensure_channel_in_list(app, &input.channel, None, list).await;
}

/// Enregistre ce qu'on sait d'un streamer (lien Discord, notes)
#[tauri::command]
pub fn save_streamer(app: AppState, streamer: crate::db::Streamer) -> CmdResult<()> {
    let login = normalize_login(&streamer.login).ok_or("Ce n'est pas un nom de chaîne Twitch valide")?;
    let discord_url = streamer.discord_url.map(|d| d.trim().to_owned()).filter(|d| !d.is_empty());
    if let Some(url) = &discord_url {
        if !url.starts_with("https://") || url.len() > 300 || url.contains(char::is_whitespace) {
            return Err("Lien Discord invalide (il doit commencer par https://)".into());
        }
    }
    let notes = streamer.notes.map(|n| n.trim().chars().take(2000).collect::<String>()).filter(|n| !n.is_empty());
    let ignored_commands = crate::giveaways::detect::normalize_excluded(&streamer.ignored_commands);
    app.db.save_streamer(&crate::db::Streamer { login, discord_url, notes, ignored_commands }).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(())
}

#[tauri::command]
pub async fn create_giveaway(app: AppState<'_>, input: crate::db::GiveawayInput) -> CmdResult<i64> {
    let app: Arc<App> = app.inner().clone();
    let mut input = crate::giveaways::validate_input(input)?;
    input.gift_ids = known_gifts(&app, &input.gift_ids);
    let at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let id = app.db.insert_manual_giveaway(&input, &at).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    add_giveaway_channel_to_lists(&app, &input).await;
    Ok(id)
}

#[tauri::command]
pub async fn update_giveaway(app: AppState<'_>, id: i64, input: crate::db::GiveawayInput) -> CmdResult<()> {
    let app: Arc<App> = app.inner().clone();
    let mut input = crate::giveaways::validate_input(input)?;
    input.gift_ids = known_gifts(&app, &input.gift_ids);
    app.db.update_giveaway(id, &input).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    add_giveaway_channel_to_lists(&app, &input).await;
    Ok(())
}

// --- Catalogue de cadeaux et gains ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GiftCatalog {
    pub gifts: Vec<crate::db::Gift>,
    pub groups: Vec<crate::db::GiftGroup>,
    /// Nombre de giveaways par cadeau
    pub usage: std::collections::HashMap<i64, i64>,
}

#[tauri::command]
pub fn get_gift_catalog(app: AppState) -> CmdResult<GiftCatalog> {
    Ok(GiftCatalog {
        gifts: app.db.gifts().map_err(db_err)?,
        groups: app.db.gift_groups().map_err(db_err)?,
        usage: app.db.gift_usage().map_err(db_err)?,
    })
}

/// Image d'un cadeau : data URL d'image, 2 Mo au plus (réduite par l'interface avant l'envoi)
const MAX_GIFT_IMAGE: usize = 2 * 1024 * 1024;

/// Ajoute (id 0) ou modifie un cadeau ; son groupe est un groupe de sa catégorie ou sans catégorie
#[tauri::command]
pub fn save_gift(app: AppState, mut gift: crate::db::Gift) -> CmdResult<i64> {
    gift.title = gift.title.trim().to_owned();
    if gift.title.is_empty() {
        return Err("Donne un titre au cadeau".into());
    }
    if gift.title.chars().count() > 120 {
        return Err("Titre trop long (120 caractères au plus)".into());
    }
    gift.image = gift.image.map(|i| i.trim().to_owned()).filter(|i| !i.is_empty());
    if let Some(image) = &gift.image {
        let uploaded = image.starts_with("data:image/") && image.len() <= MAX_GIFT_IMAGE;
        let online = image.starts_with("https://") && image.len() <= 2048 && !image.contains(char::is_whitespace);
        if !uploaded && !online {
            return Err("Image invalide : colle une image, ou l'adresse https:// d'une image en ligne".into());
        }
    }
    gift.game_id = gift.game_id.filter(|g| !g.trim().is_empty());
    if gift.game_id.is_none() {
        gift.game_name = None;
    }
    let mut keywords: Vec<String> = vec![];
    for k in &gift.keywords {
        let k = k.trim().to_lowercase();
        if !k.is_empty() && !keywords.contains(&k) {
            keywords.push(k);
        }
    }
    gift.keywords = keywords;
    if let Some(group_id) = gift.group_id {
        let groups = app.db.gift_groups().map_err(db_err)?;
        let group = groups.iter().find(|g| g.id == group_id).ok_or("Ce groupe n'existe plus")?;
        if group.game_id.is_some() && group.game_id != gift.game_id {
            return Err(format!("Le groupe « {} » appartient à une autre catégorie", group.name));
        }
    }
    let id = app.db.save_gift(&gift).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(id)
}

/// Cadeau recherché ou non (propre à ce poste)
#[tauri::command]
pub fn set_gift_favorite(app: AppState, id: i64, favorite: bool) -> CmdResult<()> {
    app.db.set_gift_favorite(id, favorite).map_err(db_err)
}

#[tauri::command]
pub fn delete_gift(app: AppState, id: i64) -> CmdResult<()> {
    app.db.delete_gift(id).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(())
}

/// Ajoute (id 0) ou renomme un groupe de cadeaux ; la catégorie n'est retenue qu'à la création
#[tauri::command]
pub fn save_gift_group(
    app: AppState,
    id: i64,
    name: String,
    game_id: Option<String>,
    game_name: Option<String>,
) -> CmdResult<i64> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Donne un nom au groupe".into());
    }
    if name.chars().count() > 60 {
        return Err("Nom trop long (60 caractères au plus)".into());
    }
    let taken = || format!("Le groupe « {name} » existe déjà dans cette catégorie");
    app.sync.poke();
    if id > 0 {
        return if app.db.rename_gift_group(id, name).map_err(db_err)? { Ok(id) } else { Err(taken()) };
    }
    let game_id = game_id.filter(|g| !g.trim().is_empty());
    let game_name = game_name.filter(|_| game_id.is_some());
    app.db.add_gift_group(name, game_id.as_deref(), game_name.as_deref()).map_err(db_err)?.ok_or_else(taken)
}

#[tauri::command]
pub fn delete_gift_group(app: AppState, id: i64) -> CmdResult<()> {
    app.db.delete_gift_group(id).map_err(db_err)?;
    app.sync.poke();
    Ok(())
}

#[tauri::command]
pub fn get_wins(app: AppState) -> Vec<crate::giveaways::GiveawayView> {
    crate::giveaways::won_views(&app)
}

#[tauri::command]
pub fn set_prize(app: AppState, id: i64, received: bool, note: Option<String>) -> CmdResult<()> {
    let note = note.map(|n| n.trim().chars().take(500).collect::<String>()).filter(|n| !n.is_empty());
    app.db.set_prize(id, received, note.as_deref()).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(())
}

/// Catégorie Twitch (jeu) d'après son nom exact ou l'adresse de sa catégorie
/// Autocomplétion des catégories Twitch
#[tauri::command]
pub async fn search_categories(app: AppState<'_>, query: String) -> CmdResult<Vec<crate::twitch::gql::CategoryHit>> {
    crate::twitch::gql::search_categories(&app.http, &query).await.map_err(|e| e.to_string())
}

/// Jaquettes de jeux d'après leur identifiant (identifiant → adresse)
#[tauri::command]
pub async fn game_box_arts(app: AppState<'_>, ids: Vec<String>) -> CmdResult<std::collections::HashMap<String, String>> {
    crate::twitch::gql::box_arts(&app.http, &ids).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resolve_category(app: AppState<'_>, input: String) -> CmdResult<crate::drops::Game> {
    let input = input.trim().to_owned();
    if input.is_empty() {
        return Err("Indique le nom de la catégorie ou son adresse".into());
    }
    let account = app.db.accounts().map_err(db_err)?.into_iter().next().ok_or("Connecte d'abord un compte Twitch")?;
    let tokens = app.secrets.load(&account.user_id).ok_or("Reconnecte ton compte Twitch")?;
    let gql = Gql::new(app.http.clone(), tokens.access_token, Some(account.device_id));
    let slug = match crate::drops::category_slug(&input) {
        Some(slug) => slug,
        None => gql
            .game_by_name(&input)
            .await
            .map_err(|e| e.to_string())?
            .map(|(_, slug)| slug)
            .ok_or_else(|| format!("Catégorie « {input} » introuvable : écris son nom exact comme sur Twitch, ou colle son adresse"))?,
    };
    gql.game_by_slug(&slug)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Catégorie « {slug} » introuvable sur Twitch"))
}

/// Fichier d'export du catalogue : autonome (groupes par nom, catégories par identifiant)
#[derive(Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GiftExport {
    format: String,
    version: u32,
    exported_at: String,
    gifts: Vec<GiftExportItem>,
}

#[derive(Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GiftExportItem {
    title: String,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    game_id: Option<String>,
    #[serde(default)]
    game_name: Option<String>,
}

const GIFT_EXPORT_FORMAT: &str = "twitch-desk-gifts";

/// Enregistre le catalogue dans le fichier choisi ; renvoie le nombre de cadeaux
#[tauri::command]
pub fn export_gifts(app: AppState, path: String) -> CmdResult<usize> {
    let groups = app.db.gift_groups().map_err(db_err)?;
    let gifts = app.db.gifts().map_err(db_err)?;
    let export = GiftExport {
        format: GIFT_EXPORT_FORMAT.into(),
        version: 1,
        exported_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        gifts: gifts
            .into_iter()
            .map(|g| GiftExportItem {
                group: g.group_id.and_then(|id| groups.iter().find(|x| x.id == id).map(|x| x.name.clone())),
                title: g.title,
                image: g.image,
                keywords: g.keywords,
                game_id: g.game_id,
                game_name: g.game_name,
            })
            .collect(),
    };
    let count = export.gifts.len();
    let text = serde_json::to_string_pretty(&export).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("écriture du fichier : {e}"))?;
    Ok(count)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GiftImportResult {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
}

/// Fusionne un fichier exporté : ajoute les cadeaux absents, met à jour ceux de même
/// titre et même catégorie
#[tauri::command]
pub fn import_gifts(app: AppState, path: String) -> CmdResult<GiftImportResult> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("lecture du fichier : {e}"))?;
    let export: GiftExport = serde_json::from_str(&text).map_err(|_| "Ce fichier n'est pas un export de cadeaux Twitch Desk".to_owned())?;
    if export.format != GIFT_EXPORT_FORMAT {
        return Err("Ce fichier n'est pas un export de cadeaux Twitch Desk".into());
    }
    let existing = app.db.gifts().map_err(db_err)?;
    let mut result = GiftImportResult { added: 0, updated: 0, skipped: 0 };
    for item in export.gifts {
        let title = item.title.trim().to_owned();
        let image_ok = item
            .image
            .as_deref()
            .is_none_or(|i| (i.starts_with("data:image/") && i.len() <= MAX_GIFT_IMAGE) || (i.starts_with("https://") && i.len() <= 2048));
        if title.is_empty() || title.chars().count() > 120 || !image_ok {
            result.skipped += 1;
            continue;
        }
        let game_id = item.game_id.filter(|g| !g.is_empty());
        let group_id = match item.group.map(|g| g.trim().to_owned()).filter(|g| !g.is_empty()) {
            Some(name) => {
                let game_name = item.game_name.as_deref().filter(|_| game_id.is_some());
                Some(app.db.ensure_gift_group(&name, game_id.as_deref(), game_name).map_err(db_err)?)
            }
            None => None,
        };
        let same = existing.iter().find(|g| g.title.eq_ignore_ascii_case(&title) && g.game_id == game_id);
        let gift = crate::db::Gift {
            id: same.map_or(0, |g| g.id),
            title,
            image: item.image,
            group_id,
            keywords: item.keywords.into_iter().map(|k| k.trim().to_lowercase()).filter(|k| !k.is_empty()).collect(),
            game_name: if game_id.is_some() { item.game_name } else { None },
            favorite: false,
            game_id,
        };
        app.db.save_gift(&gift).map_err(db_err)?;
        if same.is_some() {
            result.updated += 1;
        } else {
            result.added += 1;
        }
    }
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(result)
}

/// Garde les cadeaux qui existent encore dans le catalogue
fn known_gifts(app: &App, ids: &[i64]) -> Vec<i64> {
    let known: Vec<i64> = app.db.gifts().unwrap_or_default().into_iter().map(|g| g.id).collect();
    ids.iter().copied().filter(|id| known.contains(id)).collect()
}

#[tauri::command]
pub fn dismiss_giveaway(app: AppState, id: i64) {
    app.giveaways.send(MonitorCmd::Dismiss(id));
}

/// Modification rapide de la commande d'un giveaway (vide : pas encore connue)
#[tauri::command]
pub fn set_giveaway_command(app: AppState, id: i64, command: Option<String>) -> CmdResult<()> {
    let command = command.map(|c| c.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")).filter(|c| !c.is_empty());
    if command.as_ref().is_some_and(|c| c.chars().count() > 50) {
        return Err("Commande trop longue (50 caractères au plus)".into());
    }
    app.db.set_giveaway_command(id, command.as_deref()).map_err(db_err)?;
    app.giveaways.send(MonitorCmd::Reload);
    app.sync.poke();
    Ok(())
}

#[tauri::command]
pub fn close_giveaway(app: AppState, id: i64) {
    app.giveaways.send(MonitorCmd::Close(id));
    app.sync.poke();
}

#[tauri::command]
pub fn delete_giveaway(app: AppState, id: i64) {
    app.giveaways.send(MonitorCmd::Delete(id));
    app.sync.poke();
}

#[tauri::command]
pub fn dismiss_mention(app: AppState, id: u64) {
    app.giveaways.send(MonitorCmd::DismissMention(id));
}

// --- Compte StreamElements (lecture exacte et achat de nos tickets) ---

const SE_LOGIN_URL: &str = "https://id.streamelements.com/auth/twitch?return=https%3A%2F%2Fstreamelements.com%2F";
const SE_LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
// Cookies où streamelements.com range son jeton de session
const SE_TOKEN_COOKIES: &[&str] = &["id_token_creator", "se-token"];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum SeLoginEvent {
    Done { account_id: i64, username: String },
    WrongAccount { account_id: i64, username: String },
    Cancelled { account_id: i64 },
    Error { account_id: i64, message: String },
}

fn find_account(app: &App, id: i64) -> CmdResult<crate::db::Account> {
    app.db.accounts().map_err(db_err)?.into_iter().find(|a| a.id == id).ok_or_else(|| "compte introuvable".into())
}

/// Vérifie que le jeton appartient bien au compte Twitch, puis l'enregistre
async fn store_se_token(app: &Arc<App>, account: &crate::db::Account, token: &str) -> SeLoginEvent {
    let account_id = account.id;
    let me = match streamelements::me(&app.http, token).await {
        Ok(me) => me,
        Err(SeError::Unauthorized) => {
            return SeLoginEvent::Error { account_id, message: "StreamElements refuse ce jeton".into() }
        }
        Err(e) => return SeLoginEvent::Error { account_id, message: e.to_string() },
    };
    if !me.provider_id.is_empty() && me.provider_id != account.user_id {
        return SeLoginEvent::WrongAccount { account_id, username: me.username };
    }
    if let Err(e) = app.secrets.save_raw(&Secrets::se_key(&account.user_id), token) {
        return SeLoginEvent::Error { account_id, message: format!("enregistrement du jeton : {e}") };
    }
    app.log(&account.login, EventKind::Info, format!("Compte StreamElements connecté ({})", me.username));
    app.send(account_id, MinerCommand::Reload);
    SeLoginEvent::Done { account_id, username: me.username }
}

/// Ouvre la connexion streamelements.com dans une fenêtre privée de l'app et
/// récupère le jeton de session à la fin ; le résultat arrive par l'événement « se-login »
#[tauri::command]
pub async fn connect_streamelements(app: AppState<'_>, account_id: i64) -> CmdResult<()> {
    let app: Arc<App> = app.inner().clone();
    let account = find_account(&app, account_id)?;
    let label = format!("se-login-{account_id}");
    if let Some(old) = tauri::Manager::get_webview_window(app.handle(), &label) {
        let _ = old.close();
    }
    let url = SE_LOGIN_URL.parse().map_err(|e: url::ParseError| e.to_string())?;
    // Fenêtre privée : la session Twitch d'un autre compte ne s'y connecte pas toute seule
    let window = tauri::WebviewWindowBuilder::new(app.handle(), &label, tauri::WebviewUrl::External(url))
        .title(format!("Connexion StreamElements — {}", account.login))
        .inner_size(520.0, 760.0)
        .incognito(true)
        .build()
        .map_err(|e| e.to_string())?;

    tauri::async_runtime::spawn(async move {
        let started = tokio::time::Instant::now();
        let site: url::Url = "https://streamelements.com/".parse().expect("adresse valide");
        let event = loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            // Fenêtre fermée par l'utilisateur
            if tauri::Manager::get_webview_window(app.handle(), &label).is_none() {
                break SeLoginEvent::Cancelled { account_id };
            }
            if started.elapsed() > SE_LOGIN_TIMEOUT {
                let _ = window.close();
                break SeLoginEvent::Cancelled { account_id };
            }
            // Lecture des cookies hors du fil principal (blocage connu de WebView2 sinon)
            let w = window.clone();
            let u = site.clone();
            let cookies = tokio::task::spawn_blocking(move || w.cookies_for_url(u)).await;
            let token = cookies
                .ok()
                .and_then(Result::ok)
                .and_then(|list| {
                    SE_TOKEN_COOKIES
                        .iter()
                        .find_map(|name| list.iter().find(|c| c.name() == *name).map(|c| c.value().to_owned()))
                })
                .filter(|t| !t.is_empty());
            if let Some(token) = token {
                let _ = window.close();
                break store_se_token(&app, &account, &token).await;
            }
        };
        let _ = app.handle().emit("se-login", event);
    });
    Ok(())
}

/// Secours : jeton d'accès personnel collé depuis la page Sécurité de StreamElements
#[tauri::command]
pub async fn set_streamelements_token(app: AppState<'_>, account_id: i64, token: String) -> CmdResult<SeLoginEvent> {
    let app: Arc<App> = app.inner().clone();
    let account = find_account(&app, account_id)?;
    let token = token.trim().trim_start_matches("Bearer ").trim();
    if token.is_empty() {
        return Err("Colle le jeton StreamElements".into());
    }
    Ok(store_se_token(&app, &account, token).await)
}

#[tauri::command]
pub fn disconnect_streamelements(app: AppState, account_id: i64) -> CmdResult<()> {
    let account = find_account(&app, account_id)?;
    app.secrets.delete(&Secrets::se_key(&account.user_id));
    app.log(&account.login, EventKind::Info, "Compte StreamElements déconnecté");
    app.send(account_id, MinerCommand::Reload);
    Ok(())
}

/// Achète des tickets d'un giveaway StreamElements ; renvoie nos tickets après l'achat
#[tauri::command]
pub async fn buy_se_tickets(
    app: AppState<'_>,
    account_id: i64,
    channel_id: i64,
    giveaway_id: String,
    tickets: i64,
) -> CmdResult<i64> {
    if !(1..=100_000).contains(&tickets) {
        return Err("Nombre de tickets invalide".into());
    }
    let app: Arc<App> = app.inner().clone();
    let account = find_account(&app, account_id)?;
    let channel = app
        .db
        .channels(account_id)
        .map_err(db_err)?
        .into_iter()
        .find(|c| c.id == channel_id)
        .ok_or("chaîne introuvable")?;
    let se_channel = channel.se_channel_id.filter(|id| !id.is_empty()).ok_or("Cette chaîne n'utilise pas StreamElements")?;
    let token = app
        .secrets
        .load_raw(&Secrets::se_key(&account.user_id))
        .ok_or("Connecte d'abord le compte StreamElements (page Comptes)")?;
    let se_err = |e: SeError| match e {
        SeError::Unauthorized => "Jeton StreamElements expiré : reconnecte StreamElements dans Comptes".to_owned(),
        e => e.to_string(),
    };
    streamelements::enter(&app.http, &token, &se_channel, &giveaway_id, tickets).await.map_err(se_err)?;
    let total = streamelements::my_entry(&app.http, &token, &se_channel, &giveaway_id).await.map_err(se_err)?;
    app.log(
        &account.login,
        EventKind::Info,
        format!("{tickets} ticket(s) acheté(s) sur {} ({total} au total)", channel.login),
    );
    app.send(account_id, MinerCommand::SeTickets { channel_id, giveaway_id, tickets: total });
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::normalize_login;

    #[test]
    fn logins() {
        assert_eq!(normalize_login("  LunarSea ").as_deref(), Some("lunarsea"));
        assert_eq!(normalize_login("@kraken_tv").as_deref(), Some("kraken_tv"));
        assert_eq!(normalize_login("https://www.twitch.tv/captainsparrow/videos").as_deref(), Some("captainsparrow"));
        assert_eq!(normalize_login("twitch.tv/abc?x=1").as_deref(), Some("abc"));
        assert_eq!(normalize_login("pas valide !"), None);
        assert_eq!(normalize_login(""), None);
    }
}
