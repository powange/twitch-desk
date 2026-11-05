//! Fenêtre multi-stream : streams des giveaways (et leur chat) dans une fenêtre à part,
//! déplaçable, qui reste ouverte jusqu'à ce qu'on la ferme.
//!
//! Le lecteur et le chat de Twitch exigent le domaine de la page qui les affiche (paramètre
//! `parent`) servie en HTTPS : la page multi-stream (`public/viewer.html`) est servie par
//! l'application en `https://tauri.localhost`, ou en secours par la page GitHub du projet
//! (réglage). Dans les deux cas, la fenêtre n'a aucun droit sur l'application : aucune
//! capacité ne la nomme (capabilities/default.json ne vaut que pour « main ») et les
//! commandes de l'application sont déclarées à Tauri (build.rs), donc refusées hors de la
//! fenêtre principale. Elle ne parle à l'application que par des adresses convenues.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::app::App;

const LABEL: &str = "viewer";
const PIN_ID: &str = "viewer-pin";
const RELOAD_ID: &str = "viewer-reload";
/// Bloqueur de publicités Twitch (vaft, MIT) : voir vendor/vaft/README.md
const VAFT: &str = include_str!("../vendor/vaft/vaft.user.js");
fn valid_login(login: &str) -> bool {
    !login.is_empty() && login.len() <= 25 && login.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Adresse sur twitch.tv (connexion, chat), en https
fn is_twitch(url: &url::Url) -> bool {
    url.scheme() == "https" && url.host_str().is_some_and(|h| h == "twitch.tv" || h.ends_with(".twitch.tv"))
}

/// Domaines de Twitch admis dans la fenêtre des streams : sous Linux (et macOS), le contrôle
/// des navigations voit aussi celles des cadres (lecteurs, chat, extensions du chat)
fn is_twitch_frame(url: &url::Url) -> bool {
    const DOMAINS: [&str; 4] = ["twitch.tv", "ext-twitch.tv", "twitchcdn.net", "jtvnw.net"];
    url.scheme() == "https" && url.host_str().is_some_and(|h| DOMAINS.iter().any(|d| h == *d || h.ends_with(&format!(".{d}"))))
}

/// Même origine (schéma, hôte, port)
fn same_origin(a: &url::Url, b: &url::Url) -> bool {
    a.origin() == b.origin()
}

/// Page de l'application : https://tauri.localhost (Windows), tauri://localhost (Linux, macOS)
fn is_app_page(url: &url::Url) -> bool {
    match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "https" | "http" => url.host_str() == Some("tauri.localhost"),
        _ => false,
    }
}

/// Navigation admise dans la fenêtre des streams : la page elle-même (`page` : son adresse),
/// Twitch, et les cadres vides
fn navigation_allowed(url: &url::Url, page: &url::Url) -> bool {
    matches!(url.scheme(), "about" | "blob")
        || same_origin(url, page)
        || (is_app_page(page) && is_app_page(url))
        || is_twitch_frame(url)
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
        let result = add_in_page(&window, &login).await;
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return match result.as_deref() {
            Some("added" | "present") => Ok(()),
            Some("full") => Err(full_message()),
            // Page pas encore prête (chargement) ou sans réponse : rechargée avec la chaîne
            // ajoutée à son adresse, les autres streams restent
            _ => reload_with(&window, &login),
        };
    }
    open_window(&app, &format!("channels={login}&chat={login}"))?;
    Ok(())
}

/// Nombre de streams de la page (MAX de viewer.js)
const MAX_STREAMS: usize = 9;

fn full_message() -> String {
    format!("La fenêtre des streams en affiche déjà {MAX_STREAMS} : fermez-en un d'abord")
}

/// Ajout sans recharger les autres streams (nom vérifié par l'appelant) : « added »,
/// « present » (déjà affichée), « full » (plus de place), « pending » (page pas prête) ;
/// `None` sans réponse de la page
async fn add_in_page(window: &tauri::WebviewWindow, login: &str) -> Option<String> {
    let script = format!(
        "(function () {{
  if (typeof window.twitchDeskAdd !== 'function') return 'pending';
  var list = (new URLSearchParams(location.search).get('channels') || '').split(',').filter(Boolean);
  if (list.indexOf('{login}') >= 0) return 'present';
  if (list.length >= {MAX_STREAMS}) return 'full';
  window.twitchDeskAdd('{login}');
  return 'added';
}})()"
    );
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = Mutex::new(Some(tx));
    let sent = window.eval_with_callback(script, move |value| {
        if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
            let _ = tx.send(value);
        }
    });
    if sent.is_err() {
        return None;
    }
    let value = tokio::time::timeout(Duration::from_secs(3), rx).await.ok()?.ok()?;
    // Résultat sérialisé en JSON : chaîne entre guillemets
    Some(value.trim().trim_matches('"').to_owned())
}

/// Adresse de la page avec `login` ajoutée aux streams (et au chat) ; `Err` quand la page en
/// affiche déjà le maximum
fn query_with(query: &str, login: &str) -> Result<String, String> {
    let mut pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    let mut add_to = |key: &str, limit: bool| -> Result<(), String> {
        let pos = match pairs.iter().position(|(k, _)| k == key) {
            Some(p) => p,
            None => {
                pairs.push((key.to_owned(), String::new()));
                pairs.len() - 1
            }
        };
        let mut list: Vec<&str> = pairs[pos].1.split(',').filter(|c| !c.is_empty()).collect();
        if list.contains(&login) {
            return Ok(());
        }
        if limit && list.len() >= MAX_STREAMS {
            return Err(full_message());
        }
        list.push(login);
        pairs[pos].1 = list.join(",");
        Ok(())
    };
    add_to("channels", true)?;
    add_to("chat", false)?;
    // Virgules et deux-points gardés tels quels, comme les écrit la page
    Ok(pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"))
}

/// Recharge la page des streams avec `login` en plus (nom vérifié par l'appelant)
fn reload_with(window: &tauri::WebviewWindow, login: &str) -> Result<(), String> {
    let mut url = window.url().map_err(|e| format!("fenêtre des streams : {e}"))?;
    let query = query_with(url.query().unwrap_or(""), login)?;
    let query = clean_query(&query).ok_or("Adresse de la page des streams invalide")?;
    url.set_query(Some(&query));
    window.navigate(url).map_err(|e| format!("fenêtre des streams : {e}"))
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
        let mut last = LAST_QUERY.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_deref() != Some(query.as_str()) && app.db.set_state("viewer_query", &query).is_ok() {
            *last = Some(query);
        }
    }
    remember_open(app, open);
}

/// Dernières valeurs écrites : rien n'est réécrit tant qu'elles ne changent pas (relevé toutes
/// les 20 s). Toute écriture de `viewer_query` et `viewer_open` passe par ici
static LAST_QUERY: Mutex<Option<String>> = Mutex::new(None);
static LAST_OPEN: Mutex<Option<bool>> = Mutex::new(None);

fn remember_open(app: &App, open: bool) {
    let mut last = LAST_OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if *last != Some(open) && app.db.set_state("viewer_open", if open { "1" } else { "0" }).is_ok() {
        *last = Some(open);
    }
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
    // `page` : adresse de la page, dont l'origine reste admise dans la fenêtre
    // Page intégrée à l'application ; en développement, elle vient du serveur de Vite (devUrl)
    let dev = if cfg!(dev) { handle.config().build.dev_url.clone() } else { None };
    let page: url::Url = dev.unwrap_or_else(|| "https://tauri.localhost/".parse().expect("adresse fixe"));
    let source = WebviewUrl::App(format!("viewer.html?{query}").into());
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
        // Adresses convenues de la page : « Fermer la page » (…?close=1), fiche d'un streamer
        // (…?fiche=<chaîne>), ouverte dans la fenêtre principale, et participation à un giveaway
        // (…?participer=<identifiant>) avec les comptes qui n'y sont pas encore
        .on_navigation({
            let handle = handle.clone();
            move |url| {
                if let Some(id) = url.query().and_then(|q| q.strip_prefix("participer=")).and_then(|i| i.parse::<i64>().ok()) {
                    if let Some(app) = handle.try_state::<Arc<App>>() {
                        let app: Arc<App> = app.inner().clone();
                        tauri::async_runtime::spawn(async move { participate_from_page(&app, id).await });
                    }
                    return false;
                }
                if let Some(login) = url.query().and_then(|q| q.strip_prefix("fiche=")).filter(|l| valid_login(l)) {
                    let _ = handle.emit_to("main", "open-streamer", login.to_owned());
                    if let Some(main) = handle.get_webview_window("main") {
                        let _ = main.unminimize();
                        let _ = main.show();
                        let _ = main.set_focus();
                    }
                    return false;
                }
                if url.query() == Some("close=1") {
                    let handle = handle.clone();
                    let _ = handle.clone().run_on_main_thread(move || {
                        if let Some(app) = handle.try_state::<Arc<App>>() {
                            remember_open(&app, false);
                        }
                        if let Some(w) = handle.get_webview_window(LABEL) {
                            let _ = w.destroy();
                        }
                    });
                    return false;
                }
                if navigation_allowed(url, &page) {
                    return true;
                }
                // Tout autre lien : dans le navigateur. Sous Windows, seule la page elle-même
                // passe ici ; ailleurs les cadres aussi (publicités, pistage), simplement bloqués
                // sans ouvrir le navigateur
                if cfg!(windows) && matches!(url.scheme(), "http" | "https") {
                    use tauri_plugin_opener::OpenerExt;
                    let _ = handle.opener().open_url(url.as_str(), None::<&str>);
                } else {
                    tracing::debug!("fenêtre des streams : navigation bloquée vers {}", url.host_str().unwrap_or(url.scheme()));
                }
                false
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
    spawn_giveaway_push(handle.clone());
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

/// Giveaways ouverts à rejoindre à la main, par chaîne (le plus récent) : commande, comptes qui
/// n'y sont pas encore ; envoyés à la page pour le bouton « Participer » de chaque stream
fn giveaways_to_join(app: &App) -> serde_json::Map<String, serde_json::Value> {
    let accounts = app.db.accounts().unwrap_or_default();
    let mut out = serde_json::Map::new();
    let view = app.giveaways.view();
    let mut list: Vec<_> = view.giveaways.iter().collect();
    list.sort_by(|a, b| b.row.detected_at.cmp(&a.row.detected_at));
    for g in list {
        let r = &g.row;
        let Some(command) = r.command.as_deref().filter(|c| !c.trim().is_empty()) else { continue };
        if r.status != "open" || r.hidden || !matches!(r.kind.as_str(), "command" | "ticket") || out.contains_key(&r.channel) {
            continue;
        }
        let missing: Vec<&str> =
            accounts.iter().filter(|a| !g.entries_view.iter().any(|e| e.account_id == a.id)).map(|a| a.login.as_str()).collect();
        if missing.is_empty() {
            continue;
        }
        out.insert(r.channel.clone(), serde_json::json!({ "id": r.id, "command": command, "accounts": missing }));
    }
    out
}

/// Tant que la fenêtre des streams est ouverte : giveaways à rejoindre envoyés à la page quand ils
/// changent (et toutes les 20 s, pour une page rechargée)
fn spawn_giveaway_push(handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last: Option<(String, std::time::Instant)> = None;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Some(window) = handle.get_webview_window(LABEL) else { return };
            let Some(app) = handle.try_state::<Arc<App>>() else { return };
            let json = serde_json::Value::Object(giveaways_to_join(&app)).to_string();
            if last.as_ref().is_some_and(|(j, at)| *j == json && at.elapsed() < Duration::from_secs(20)) {
                continue;
            }
            let script = format!("typeof window.twitchDeskGiveaways === 'function' && window.twitchDeskGiveaways({json})");
            if window.eval(script).is_ok() {
                last = Some((json, std::time::Instant::now()));
            }
        }
    });
}

/// « Participer » d'un stream : commande envoyée avec les comptes qui n'y sont pas encore ;
/// résultat affiché sur le stream
async fn participate_from_page(app: &Arc<App>, id: i64) {
    let entered: Vec<i64> = app
        .giveaways
        .view()
        .giveaways
        .iter()
        .find(|g| g.row.id == id)
        .map(|g| g.entries_view.iter().map(|e| e.account_id).collect())
        .unwrap_or_default();
    let ids: Vec<i64> = app.db.accounts().unwrap_or_default().into_iter().map(|a| a.id).filter(|a| !entered.contains(a)).collect();
    let (ok, message) = match crate::giveaways::participate(app, id, &ids, false).await {
        _ if ids.is_empty() => (true, "Tous tes comptes participent déjà".to_owned()),
        Ok(results) if results.iter().all(|r| r.ok) => (true, format!("Envoyé avec {}", results.iter().map(|r| r.login.as_str()).collect::<Vec<_>>().join(", "))),
        Ok(results) => (false, results.iter().filter(|r| !r.ok).map(|r| format!("{} : {}", r.login, r.message)).collect::<Vec<_>>().join(" · ")),
        Err(e) => (false, e),
    };
    if let Some(window) = app.handle().get_webview_window(LABEL) {
        let detail = serde_json::json!({ "id": id, "ok": ok, "message": message });
        let _ = window.eval(format!("typeof window.twitchDeskParticipated === 'function' && window.twitchDeskParticipated({detail})"));
    }
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

    #[test]
    fn only_the_page_and_twitch_load_in_the_window() {
        let local: url::Url = "https://tauri.localhost/".parse().unwrap();
        let ok = |u: &str, page: &url::Url| navigation_allowed(&u.parse().unwrap(), page);
        assert!(ok("https://tauri.localhost/viewer.html?channels=a", &local));
        assert!(ok("tauri://localhost/viewer.html?channels=a", &local));
        assert!(ok("https://player.twitch.tv/?channel=a&parent=tauri.localhost", &local));
        assert!(ok("https://abc.ext-twitch.tv/x", &local));
        assert!(ok("about:blank", &local));
        assert!(!ok("https://powange.github.io/twitch-desk/viewer.html", &local));
        assert!(!ok("https://evil.example/", &local));
        assert!(!ok("https://twitch.tv.evil.example/", &local));
        assert!(!ok("http://player.twitch.tv/", &local));
    }

    #[test]
    fn stream_added_to_the_address() {
        assert_eq!(query_with("channels=a,b&chat=a&vol=a:0.5", "c").unwrap(), "channels=a,b,c&chat=a,c&vol=a:0.5");
        assert_eq!(query_with("", "c").unwrap(), "channels=c&chat=c");
        assert_eq!(query_with("channels=a&chat=a", "a").unwrap(), "channels=a&chat=a");
        assert!(query_with("channels=a,b,c,d,e,f,g,h,i", "j").is_err());
    }
}
