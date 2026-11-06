//! Mémoire de WebView2 quand une fenêtre n'est pas visible (Windows) : fenêtre principale cachée
//! dans la zone de notification ou réduite → rendu coupé (IsVisible) et mémoire visée basse ;
//! fenêtre des streams réduite → mémoire visée basse seulement (les vidéos et le son continuent).
//! La page continue de tourner (mises à jour, sons d'alerte, notifications) ; tout revient à
//! l'affichage. Rien à faire ailleurs que sous Windows.

use tauri::{AppHandle, Manager, WebviewWindow};

/// Fenêtre à l'écran ou non : réglage de son navigateur intégré mis à jour s'il a changé
pub fn apply(window: &WebviewWindow) {
    let hidden = !window.is_visible().unwrap_or(true) || window.is_minimized().unwrap_or(false);
    set(window, hidden);
}

/// Juste avant d'afficher une fenêtre : rendu et mémoire normaux (pas d'image vide à l'ouverture)
pub fn foreground(window: &WebviewWindow) {
    set(window, false);
}

/// Filet de sécurité : état des fenêtres relu régulièrement (fenêtre cachée ou réduite par un
/// chemin sans événement)
pub fn spawn_watch(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            for window in app.webview_windows().into_values() {
                apply(&window);
            }
        }
    });
}

#[cfg(windows)]
fn set(window: &WebviewWindow, hidden: bool) {
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use windows_core::Interface;
    // Dernier état appliqué par fenêtre : rien à refaire s'il n'a pas changé
    static APPLIED: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);
    let label = window.label().to_owned();
    {
        let mut applied = APPLIED.lock().unwrap_or_else(|e| e.into_inner());
        if applied.get(&label) == Some(&hidden) {
            return;
        }
        applied.insert(label.clone(), hidden);
    }
    // Fenêtre des streams : ses lecteurs continuent ; seule la mémoire visée change
    let stop_rendering = label == "main";
    let result = window.with_webview(move |webview| unsafe {
        let controller = webview.controller();
        if stop_rendering {
            if let Err(e) = controller.SetIsVisible(!hidden) {
                tracing::debug!("navigateur intégré (rendu) : {e}");
            }
        }
        let Ok(core) = controller.CoreWebView2() else { return };
        let Ok(core) = core.cast::<ICoreWebView2_19>() else { return };
        let level = if hidden { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
        if let Err(e) = core.SetMemoryUsageTargetLevel(level) {
            tracing::debug!("navigateur intégré (mémoire) : {e}");
        }
    });
    if let Err(e) = result {
        tracing::debug!("navigateur intégré : {e}");
    }
}

#[cfg(not(windows))]
fn set(_window: &WebviewWindow, _hidden: bool) {}
