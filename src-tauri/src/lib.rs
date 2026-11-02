mod app;
mod commands;
mod db;
mod drops;
mod giveaways;
mod insights;
mod lives;
mod miner;
mod scanner;
mod secrets;
mod streamelements;
mod sync;
mod tray;
mod viewer;
mod twitch;

use std::path::Path;
use std::sync::Arc;

use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use crate::twitch::gql::{self, OperationsFile};

/// Argument passé par le démarrage automatique avec Windows
const MINIMIZED_ARG: &str = "--minimized";

fn window_flags() -> tauri_plugin_window_state::StateFlags {
    tauri_plugin_window_state::StateFlags::all() - tauri_plugin_window_state::StateFlags::VISIBLE
}

/// Enregistre taille et position des fenêtres : à faire avant toute sortie qui ne passe pas
/// par la fermeture normale (fenêtre cachée, « Quitter », installation d'une mise à jour)
pub fn save_windows(app: &tauri::AppHandle) {
    use tauri_plugin_window_state::AppHandleExt;
    // Fenêtre des streams ouverte : rouverte au prochain lancement
    if let Some(state) = app.try_state::<Arc<app::App>>() {
        viewer::remember(&state, true);
    }
    if let Err(e) = app.save_window_state(window_flags()) {
        tracing::warn!("taille des fenêtres : {e}");
    }
}

/// Clé posée juste avant une mise à jour quand la fenêtre principale est affichée
const SHOW_AFTER_UPDATE: &str = "show_after_update";

/// `updating` : une mise à jour va relancer l'application. Fenêtre affichée à ce moment :
/// elle revient affichée, même avec « Démarrer réduit »
#[tauri::command]
fn save_window_state(app: tauri::AppHandle, updating: Option<bool>) {
    if updating == Some(false) {
        // Mise à jour échouée : démarrage habituel
        if let Some(state) = app.try_state::<Arc<app::App>>() {
            let _ = state.db.conn().execute("DELETE FROM app_state WHERE key = ?1", [SHOW_AFTER_UPDATE]);
        }
    }
    if updating == Some(true) {
        let visible = app.get_webview_window("main").and_then(|w| w.is_visible().ok()).unwrap_or(false);
        if let Some(state) = app.try_state::<Arc<app::App>>() {
            let _ = state.db.set_state(SHOW_AFTER_UPDATE, if visible { "1" } else { "0" });
        }
    }
    save_windows(&app);
}

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

/// Relecture périodique des empreintes GQL publiées
const OPERATIONS_EVERY: std::time::Duration = std::time::Duration::from_secs(4 * 3600);
/// Relecture anticipée (empreinte inconnue de Twitch) : au plus une toutes les 10 min
const OPERATIONS_MIN_GAP: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Empreintes GQL : copie locale au démarrage, puis version publiée sur GitHub
async fn refresh_operations(http: reqwest::Client, cache: std::path::PathBuf) {
    if let Some(file) = std::fs::read_to_string(&cache).ok().and_then(|s| serde_json::from_str::<OperationsFile>(&s).ok()) {
        gql::install_operations(file);
    }
    fetch_operations(&http, &cache).await;
}

/// Toutes les 4 h, ou plus tôt quand Twitch ne reconnaît plus une empreinte
async fn operations_loop(http: reqwest::Client, cache: std::path::PathBuf) {
    let mut last = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(last + OPERATIONS_EVERY) => {}
            _ = gql::operations_stale() => {
                // Plusieurs requêtes en échec à la suite : une seule relecture
                tokio::time::sleep_until(last + OPERATIONS_MIN_GAP).await;
                tracing::info!("empreinte GQL inconnue de Twitch : relecture d'operations.json");
            }
        }
        last = tokio::time::Instant::now();
        fetch_operations(&http, &cache).await;
    }
}

async fn fetch_operations(http: &reqwest::Client, cache: &Path) {
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
            let _ = std::fs::write(cache, text);
            tracing::info!("empreintes GQL à jour : {:?}", gql::operations_info());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)))
        // Taille, position et plein écran des fenêtres retenus ; la visibilité au démarrage
        // dépend du réglage « Démarrer réduit »
        .plugin(tauri_plugin_window_state::Builder::default().with_state_flags(window_flags()).build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![MINIMIZED_ARG])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let log_dir = data_dir.join("logs");
            std::fs::create_dir_all(&log_dir)?;
            // Le journal doit vivre aussi longtemps que l'application
            app.manage(init_logging(&log_dir));
            tracing::info!("Twitch Desk {} — données dans {}", app.package_info().version, data_dir.display());

            let db = match db::Db::open(&data_dir.join("twitch-desk.db")) {
                Ok(db) => db,
                Err(e) => {
                    // Base illisible (fichier abîmé, mise à jour du schéma ratée) : message à
                    // l'utilisateur plutôt qu'un arrêt muet, puis fermeture
                    tracing::error!("base de données : {e}");
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(format!(
                            "La base de données n'a pas pu être ouverte :\n{e}\n\nDossier : {}\nUne copie d'avant la mise à jour (twitch-desk.db.bak-v…) peut s'y trouver.",
                            data_dir.display()
                        ))
                        .title("Twitch Desk")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                    return Ok(());
                }
            };
            let settings = db.settings();
            let secrets = secrets::Secrets::new(data_dir.clone());
            let state = app::App::new(app.handle().clone(), db, secrets, data_dir.clone());
            app.manage(state.clone());

            tray::setup(app.handle())?;

            let started_by_autostart = std::env::args().any(|a| a == MINIMIZED_ARG);
            let after_update = state.db.get_state(SHOW_AFTER_UPDATE);
            if after_update.is_some() {
                let _ = state.db.conn().execute("DELETE FROM app_state WHERE key = ?1", [SHOW_AFTER_UPDATE]);
            }
            let hide = match after_update.as_deref() {
                // Relance après une mise à jour : la fenêtre retrouve l'état d'avant
                Some(shown) => shown != "1",
                None => settings.start_minimized || started_by_autostart,
            };
            if hide {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            let http = state.http.clone();
            let cache = data_dir.join("operations.json");
            let starter = state.clone();
            tauri::async_runtime::spawn(async move {
                refresh_operations(http.clone(), cache.clone()).await;
                tauri::async_runtime::spawn(operations_loop(http, cache));
                starter.giveaways.start(starter.clone());
                starter.start_all();
            });
            state.spawn_emitter();
            state.spawn_languages();
            scanner::spawn(state.clone());
            insights::spawn(state.clone());
            if let Some(main) = app.get_webview_window("main") {
                viewer::allow_embedded_twitch_session(&main);
            }
            viewer::restore(&state);
            sync::spawn(state.clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Seule la fenêtre principale se cache dans la zone de notification ; les autres
            // (multi-stream) se ferment vraiment
            if let (WindowEvent::CloseRequested { api, .. }, "main") = (event, window.label()) {
                let to_tray = window
                    .app_handle()
                    .try_state::<Arc<app::App>>()
                    .map(|s| s.db.settings().close_to_tray)
                    .unwrap_or(true);
                if to_tray {
                    api.prevent_close();
                    save_windows(window.app_handle());
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
            commands::set_max_balance,
            viewer::watch_stream,
            commands::import_sound,
            commands::read_sound,
            commands::list_logs,
            commands::read_log,
            commands::export_log,
            commands::set_drops_enabled,
            commands::add_drop_game,
            commands::remove_drop_game,
            commands::set_drop_game_enabled,
            commands::reorder_drop_games,
            commands::refresh_drops,
            commands::add_giveaway_channel,
            commands::import_giveaway_channels,
            commands::remove_giveaway_channel,
            commands::set_ignored_commands,
            save_window_state,
            commands::participate_giveaway,
            commands::dismiss_giveaway,
            commands::close_giveaway,
            commands::delete_giveaway,
            commands::dismiss_mention,
            commands::create_giveaway,
            commands::inspect_giveaway_channel,
            commands::get_gift_catalog,
            commands::save_gift,
            commands::set_gift_favorite,
            commands::follow_status,
            commands::greeting_status,
            commands::say_hello,
            commands::test_winner_phrase,
            commands::get_detection_rules,
            commands::set_detection_rules,
            commands::get_chat_log,
            commands::set_giveaway_command,
            commands::get_chat_log_meta,
            commands::set_manual_winner,
            commands::clear_giveaway_winner,
            commands::streamer_discord,
            commands::follow_channel,
            commands::delete_gift,
            commands::save_gift_group,
            commands::delete_gift_group,
            commands::get_wins,
            commands::set_prize,
            commands::resolve_category,
            commands::search_categories,
            commands::game_box_arts,
            commands::save_streamer,
            commands::get_streamers,
            scanner::scan_streams,
            scanner::scanner_ignore,
            scanner::scanner_refilter,
            sync::get_sync_config,
            sync::save_sync_config,
            sync::test_sync,
            sync::sync_now,
            sync::set_sync_skipped,
            sync::sync_resync,
            commands::export_gifts,
            commands::import_gifts,
            commands::update_giveaway,
            commands::add_giveaway_game,
            commands::remove_giveaway_game,
        ])
        .run(tauri::generate_context!())
        .expect("démarrage de Twitch Desk");
}
