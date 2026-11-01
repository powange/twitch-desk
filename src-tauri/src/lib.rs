mod app;
mod commands;
mod db;
mod drops;
mod giveaways;
mod miner;
mod secrets;
mod streamelements;
mod tray;
mod twitch;

use std::path::Path;
use std::sync::Arc;

use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use crate::twitch::gql::{self, OperationsFile};

/// Argument passé par le démarrage automatique avec Windows
const MINIMIZED_ARG: &str = "--minimized";

fn init_logging(dir: &Path) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::prelude::*;
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("twitch-desk")
        .filename_suffix("log")
        .max_log_files(7)
        .build(dir)
        .expect("dossier des journaux");
    let (writer, guard) = tracing_appender::non_blocking(file);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,twitch_desk_lib=debug".into());
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(writer).with_ansi(false))
        .with(tracing_subscriber::fmt::layer())
        .init();
    guard
}

/// Empreintes GQL : copie locale au démarrage, puis version publiée sur GitHub
async fn refresh_operations(http: reqwest::Client, cache: std::path::PathBuf) {
    if let Some(file) = std::fs::read_to_string(&cache).ok().and_then(|s| serde_json::from_str::<OperationsFile>(&s).ok()) {
        gql::install_operations(file);
    }
    let fetched = async {
        let res = http.get(gql::REMOTE_OPERATIONS_URL).send().await.ok()?;
        if !res.status().is_success() {
            return None;
        }
        res.json::<OperationsFile>().await.ok()
    }
    .await;
    if let Some(file) = fetched {
        let text = serde_json::to_string_pretty(&file).unwrap_or_default();
        if gql::install_operations(file) {
            let _ = std::fs::write(&cache, text);
            tracing::info!("empreintes GQL à jour : {:?}", gql::operations_info());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![MINIMIZED_ARG])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let log_dir = data_dir.join("logs");
            std::fs::create_dir_all(&log_dir)?;
            // Le journal doit vivre aussi longtemps que l'application
            app.manage(init_logging(&log_dir));
            tracing::info!("Twitch Desk {} — données dans {}", app.package_info().version, data_dir.display());

            let db = db::Db::open(&data_dir.join("twitch-desk.db"))?;
            let settings = db.settings();
            let secrets = secrets::Secrets::new(data_dir.clone());
            let state = app::App::new(app.handle().clone(), db, secrets, data_dir.clone());
            app.manage(state.clone());

            tray::setup(app.handle())?;

            let started_by_autostart = std::env::args().any(|a| a == MINIMIZED_ARG);
            if settings.start_minimized || started_by_autostart {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            let http = state.http.clone();
            let cache = data_dir.join("operations.json");
            let starter = state.clone();
            tauri::async_runtime::spawn(async move {
                refresh_operations(http, cache).await;
                starter.giveaways.start(starter.clone());
                starter.start_all();
            });
            state.spawn_emitter();
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let to_tray = window
                    .app_handle()
                    .try_state::<Arc<app::App>>()
                    .map(|s| s.db.settings().close_to_tray)
                    .unwrap_or(true);
                if to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::start_login,
            commands::cancel_login,
            commands::remove_account,
            commands::restart_account,
            commands::set_points_enabled,
            commands::add_channel,
            commands::remove_channel,
            commands::reorder_channels,
            commands::set_channel_options,
            commands::set_paused,
            commands::get_settings,
            commands::save_settings,
            commands::get_paths,
            commands::connect_streamelements,
            commands::set_streamelements_token,
            commands::disconnect_streamelements,
            commands::buy_se_tickets,
            commands::set_se_refresh,
            commands::set_drops_enabled,
            commands::add_drop_game,
            commands::remove_drop_game,
            commands::set_drop_game_enabled,
            commands::reorder_drop_games,
            commands::refresh_drops,
            commands::add_giveaway_channel,
            commands::import_giveaway_channels,
            commands::remove_giveaway_channel,
            commands::set_giveaway_excluded,
            commands::participate_giveaway,
            commands::dismiss_giveaway,
            commands::close_giveaway,
            commands::delete_giveaway,
            commands::dismiss_mention,
        ])
        .run(tauri::generate_context!())
        .expect("démarrage de Twitch Desk");
}
