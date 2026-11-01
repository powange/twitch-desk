//! Fenêtre multi-stream : streams des giveaways (et leur chat) dans une fenêtre à part,
//! déplaçable, qui reste ouverte jusqu'à ce qu'on la ferme.
//!
//! Le lecteur et le chat de Twitch n'acceptent que des pages sur un vrai domaine (paramètre
//! `parent`) : la fenêtre charge la page multi-stream publiée avec la page GitHub du projet
//! (`site/viewer.html`). Cette page vient d'internet : elle n'a aucun accès à l'application.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::app::App;

const LABEL: &str = "viewer";
const PIN_ID: &str = "viewer-pin";
const RELOAD_ID: &str = "viewer-reload";
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
    let url = format!("{}?channels={login}&chat={login}", viewer_url());
    let url = url.parse().map_err(|_| "Adresse de la page des streams invalide".to_owned())?;
    let window = WebviewWindowBuilder::new(handle, LABEL, WebviewUrl::External(url))
        .title("Streams")
        .inner_size(1280.0, 760.0)
        .min_inner_size(480.0, 320.0)
        .menu(menu(handle).map_err(|e| format!("menu : {e}"))?)
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
    Ok(())
}

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
