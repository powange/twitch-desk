//! Fenêtre multi-stream : streams des giveaways (et leur chat) dans une fenêtre à part,
//! déplaçable, qui reste ouverte jusqu'à ce qu'on la ferme.
//!
//! Le lecteur et le chat de Twitch exigent le domaine de la page qui les affiche (paramètre
//! `parent`) servie en HTTPS : la page multi-stream (`public/viewer.html`) est servie par
//! l'application en `https://tauri.localhost`, ou en secours par la page GitHub du projet
//! (réglage). Dans les deux cas, la fenêtre n'a aucun droit sur l'application.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::app::App;

const LABEL: &str = "viewer";
const PIN_ID: &str = "viewer-pin";
const RELOAD_ID: &str = "viewer-reload";
/// Bloqueur de publicités Twitch (vaft, MIT) : voir vendor/vaft/README.md
const VAFT: &str = include_str!("../vendor/vaft/vaft.user.js");
/// Page multi-stream ; `TWITCH_DESK_VIEWER_URL` la remplace (essais)
const VIEWER_URL: &str = "https://powange.github.io/twitch-desk/viewer.html";

fn viewer_url() -> String {
    std::env::var("TWITCH_DESK_VIEWER_URL").ok().filter(|u| u.starts_with("https://")).unwrap_or_else(|| VIEWER_URL.to_owned())
}

fn valid_login(login: &str) -> bool {
    !login.is_empty() && login.len() <= 25 && login.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Adresse sur twitch.tv (connexion, chat), en https
fn is_twitch(url: &url::Url) -> bool {
    url.scheme() == "https" && url.host_str().is_some_and(|h| h == "twitch.tv" || h.ends_with(".twitch.tv"))
}

fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let pin = CheckMenuItem::with_id(app, PIN_ID, "Toujours au premier plan", true, false, None::<&str>)?;
    let reload = MenuItem::with_id(app, RELOAD_ID, "Recharger les streams", true, None::<&str>)?;
    let window = Submenu::with_items(app, "Fenêtre", true, &[&pin, &reload])?;
    Menu::with_items(app, &[&window])
}

/// Ajoute un stream à la fenêtre multi-stream (ouverte au besoin) et la met au premier plan.
/// Commande asynchrone obligatoire : sous Windows, créer une fenêtre depuis une commande
/// synchrone bloque toute l'application (fenêtre blanche, plus rien ne répond)
#[tauri::command]
pub async fn watch_stream(app: State<'_, Arc<App>>, login: String) -> Result<(), String> {
    let login = login.trim().to_lowercase();
    if !valid_login(&login) {
        return Err("Ce n'est pas un nom de chaîne Twitch valide".into());
    }
    let handle = app.handle();
    if let Some(window) = handle.get_webview_window(LABEL) {
        // Fenêtre déjà ouverte : ajout sans recharger les autres streams (nom vérifié ci-dessus)
        window
            .eval(format!("window.twitchDeskAdd && window.twitchDeskAdd('{login}')"))
            .map_err(|e| format!("fenêtre des streams : {e}"))?;
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    open_window(&app, &format!("channels={login}&chat={login}"))?;
    Ok(())
}

/// Adresse de la page multi-stream gardée : seuls les caractères qu'elle produit sont admis
fn clean_query(query: &str) -> Option<String> {
    let ok = !query.is_empty() && query.len() <= 4000 && query.chars().all(|c| c.is_ascii_alphanumeric() || "_,:=&.%-".contains(c));
    ok.then(|| query.to_owned())
}

/// Garde la fenêtre des streams (streams, chat, vidéo, volume, qualité) pour la rouvrir au
/// prochain lancement ; `open` : à rouvrir (faux quand l'utilisateur la ferme lui-même)
pub fn remember(app: &App, open: bool) {
    let Some(window) = app.handle().get_webview_window(LABEL) else { return };
    if let Some(query) = window.url().ok().and_then(|u| u.query().and_then(clean_query)) {
        let _ = app.db.set_state("viewer_query", &query);
    }
    let _ = app.db.set_state("viewer_open", if open { "1" } else { "0" });
}

/// Au lancement : rouvre la fenêtre des streams si elle était ouverte, telle qu'elle était ;
/// puis la garde toutes les 20 s (mise à jour ou arrêt brutal)
pub fn restore(app: &Arc<App>) {
    if app.db.get_state("viewer_open").as_deref() == Some("1") {
        if let Some(query) = app.db.get_state("viewer_query").and_then(|q| clean_query(&q)) {
            if let Err(e) = open_window(app, &query) {
                tracing::warn!("fenêtre des streams : {e}");
            }
        }
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            if app.handle().get_webview_window(LABEL).is_some() {
                remember(&app, true);
            }
        }
    });
}

fn open_window(app: &App, query: &str) -> Result<(), String> {
    let handle = app.handle();
    let source = if app.db.settings().viewer_source == "github" {
        let url = format!("{}?{query}", viewer_url());
        WebviewUrl::External(url.parse().map_err(|_| "Adresse de la page des streams invalide".to_owned())?)
    } else {
        WebviewUrl::App(format!("viewer.html?{query}").into())
    };
    let mut builder = WebviewWindowBuilder::new(handle, LABEL, source);
    if app.db.settings().viewer_adblock {
        // Seulement dans les lecteurs (player.twitch.tv), pas dans la page ni le chat
        builder = builder.initialization_script_for_all_frames(format!("if (location.hostname === 'player.twitch.tv') {{\n{VAFT}\n}}"));
    }
    let window = builder
        // Page de l'application servie en https://tauri.localhost (Windows) : Twitch exige HTTPS
        .use_https_scheme(true)
        .title("Streams")
        .inner_size(1280.0, 760.0)
        .min_inner_size(480.0, 320.0)
        .menu(menu(handle).map_err(|e| format!("menu : {e}"))?)
        // Bouton « Fermer la page » de la page vide : adresse convenue (…?close=1)
        .on_navigation({
            let handle = handle.clone();
            move |url| {
                if url.query() == Some("close=1") {
                    let handle = handle.clone();
                    let _ = handle.clone().run_on_main_thread(move || {
                        if let Some(app) = handle.try_state::<Arc<App>>() {
                            let _ = app.db.set_state("viewer_open", "0");
                        }
                        if let Some(w) = handle.get_webview_window(LABEL) {
                            let _ = w.destroy();
                        }
                    });
                    return false;
                }
                true
            }
        })
        // Fenêtres surgissantes : celles de Twitch (connexion au chat) s'ouvrent dans
        // l'application, les autres liens dans le navigateur
        .on_new_window({
            let handle = handle.clone();
            move |url, _features| {
                if is_twitch(&url) {
                    tauri::webview::NewWindowResponse::Allow
                } else {
                    use tauri_plugin_opener::OpenerExt;
                    if matches!(url.scheme(), "http" | "https") {
                        let _ = handle.opener().open_url(url.as_str(), None::<&str>);
                    }
                    tauri::webview::NewWindowResponse::Deny
                }
            }
        })
        .build()
        .map_err(|e| format!("fenêtre des streams : {e}"))?;
    window.on_menu_event(|window, event| match event.id().as_ref() {
        PIN_ID => {
            let pinned = window.is_always_on_top().unwrap_or(false);
            let _ = window.set_always_on_top(!pinned);
            if let Some(item) = window.menu().and_then(|m| m.get(PIN_ID)).and_then(|i| i.as_check_menuitem().cloned()) {
                let _ = item.set_checked(!pinned);
            }
        }
        RELOAD_ID => {
            if let Some(w) = window.get_webview_window(LABEL) {
                let _ = w.eval("location.reload()");
            }
        }
        _ => {}
    });
    // Fermée par l'utilisateur : ne pas la rouvrir au prochain lancement
    let handle = handle.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { .. } = event {
            if let Some(app) = handle.try_state::<Arc<App>>() {
                remember(&app, false);
            }
        }
    });
    Ok(())
}

/// Windows : protection contre le pistage du navigateur intégré désactivée pour l'application.
/// Réglée sur « Équilibrée » par défaut, elle prive le lecteur et le chat de Twitch, intégrés
/// dans la page multi-stream, de la session twitch.tv (connexion redemandée à chaque ouverture,
/// « Autoriser l'accès au stockage »). Le réglage vaut pour tout le profil de l'application.
#[cfg(windows)]
pub fn allow_embedded_twitch_session(window: &tauri::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Profile3, ICoreWebView2_13, COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_NONE,
    };
    use windows_core::Interface;
    let result = window.with_webview(|webview| unsafe {
        let Ok(core) = webview.controller().CoreWebView2() else { return };
        let Ok(core) = core.cast::<ICoreWebView2_13>() else { return };
        let Ok(profile) = core.Profile() else { return };
        let Ok(profile) = profile.cast::<ICoreWebView2Profile3>() else { return };
        if let Err(e) = profile.SetPreferredTrackingPreventionLevel(COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_NONE) {
            tracing::warn!("protection contre le pistage : {e}");
        }
    });
    if let Err(e) = result {
        tracing::warn!("navigateur intégré : {e}");
    }
}

#[cfg(not(windows))]
pub fn allow_embedded_twitch_session(_window: &tauri::WebviewWindow) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_twitch_popups_stay_in_the_app() {
        let ok = |u: &str| is_twitch(&u.parse().unwrap());
        assert!(ok("https://www.twitch.tv/login?popup=true"));
        assert!(ok("https://id.twitch.tv/oauth2/authorize"));
        assert!(!ok("http://www.twitch.tv/login"));
        assert!(!ok("https://twitch.tv.evil.example/login"));
        assert!(!ok("https://example.com"));
    }
}
