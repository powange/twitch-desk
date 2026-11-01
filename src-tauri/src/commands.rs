//! Commandes appelées par l'interface (invoke).

use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, State};

use crate::app::{App, EventKind, Snapshot};
use crate::db::Settings;
use crate::miner::MinerCommand;
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
pub async fn add_channel(app: AppState<'_>, account_id: i64, login: String) -> CmdResult<String> {
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
    app.db.add_channel(account_id, &login, &channel_id).map_err(db_err)?;
    app.send(account_id, MinerCommand::Reload);
    Ok(login)
}

#[tauri::command]
pub fn remove_channel(app: AppState, id: i64) -> CmdResult<()> {
    let account = app.db.channel_account(id).map_err(db_err)?;
    app.db.remove_channel(id).map_err(db_err)?;
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
pub fn set_channel_options(app: AppState, id: i64, enabled: bool, auto_claim: bool) -> CmdResult<()> {
    app.db.set_channel_options(id, enabled, auto_claim).map_err(db_err)?;
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
    app.db.save_settings(&settings).map_err(db_err)
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
