//! Clic sur une notification : la fenêtre principale s'ouvre sur la page de l'alerte.
//!
//! Sous Windows, la notification porte un lien `twitch-desk://open/<page>` (activation par
//! protocole) : Windows le lance au clic, que la notification soit encore à l'écran ou rangée
//! dans le Centre de notifications. Le lien arrive à l'instance déjà ouverte (instance unique),
//! ou démarre l'application s'il n'y en a pas. Le protocole est enregistré pour l'utilisateur
//! à chaque démarrage (l'application peut avoir été déplacée).
//!
//! Ailleurs, et en développement (application non installée), le clic passe par le module de
//! notification, tant que la notification est à l'écran.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

pub const SCHEME: &str = "twitch-desk";

/// Page demandée par le lien qui a démarré l'application, lue par l'interface à son ouverture
#[derive(Default)]
pub struct StartPage(pub Mutex<Option<String>>);

/// Page d'un lien `twitch-desk://open/<page>` (chemin de l'interface, lettres, chiffres, `-`,
/// `_` et `/`, avec au plus un paramètre `?clé=valeur`)
pub fn page_of_url(url: &str) -> Option<String> {
    let rest = url.trim().trim_matches('"').strip_prefix(SCHEME)?.strip_prefix("://open")?;
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    let word = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let path = format!("/{}", path.trim_matches('/'));
    if !path.split('/').all(word) {
        return None;
    }
    match query {
        None => Some(path),
        Some(q) => {
            let (key, value) = q.split_once('=')?;
            (word(key) && word(value) && !key.is_empty()).then(|| format!("{path}?{key}={value}"))
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn url_of_page(page: &str) -> String {
    format!("{SCHEME}://open{page}")
}

/// Fenêtre principale affichée sur cette page
pub fn open_page(app: &AppHandle, page: &str) {
    crate::tray::show_main(app);
    let _ = app.emit_to("main", "navigate", page);
}

/// Page à ouvrir au démarrage (application lancée par un clic sur une notification)
pub fn start_page(args: impl IntoIterator<Item = String>) -> Option<String> {
    args.into_iter().find_map(|a| page_of_url(&a))
}

/// Page demandée au démarrage, rendue une seule fois
#[tauri::command]
pub fn take_start_page(app: AppHandle) -> Option<String> {
    app.try_state::<StartPage>().and_then(|s| s.0.lock().unwrap().take())
}

/// Notification Windows avec activation par protocole ; `false` : à montrer autrement
/// (développement, ou échec)
#[cfg(windows)]
pub fn show(identifier: &str, title: &str, body: &str, page: &str) -> bool {
    if cfg!(debug_assertions) {
        return false;
    }
    match win::show(identifier, title, body, &url_of_page(page)) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("notification Windows : {e}");
            false
        }
    }
}

#[cfg(not(windows))]
pub fn show(_identifier: &str, _title: &str, _body: &str, _page: &str) -> bool {
    false
}

/// Protocole `twitch-desk://` enregistré pour l'utilisateur (HKCU), vers cet exécutable
#[cfg(windows)]
pub fn register_scheme() {
    if cfg!(debug_assertions) {
        return;
    }
    if let Err(e) = win::register_scheme() {
        tracing::warn!("protocole {SCHEME}:// : {e}");
    }
}

#[cfg(not(windows))]
pub fn register_scheme() {}

#[cfg(windows)]
mod win {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    use super::SCHEME;

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
    }

    pub fn show(identifier: &str, title: &str, body: &str, url: &str) -> windows::core::Result<()> {
        let xml = format!(
            r#"<toast activationType="protocol" launch="{}"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
            escape(url),
            escape(title),
            escape(body)
        );
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(identifier))?.Show(&toast)
    }

    pub fn register_scheme() -> Result<(), Box<dyn std::error::Error>> {
        let exe = std::env::current_exe()?;
        let key = windows_registry::CURRENT_USER.create(format!(r"Software\Classes\{SCHEME}"))?;
        key.set_string("", "URL:Twitch Desk")?;
        key.set_string("URL Protocol", "")?;
        let command = key.create(r"shell\open\command")?;
        command.set_string("", format!("\"{}\" \"%1\"", exe.display()))?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn escaped() {
            assert_eq!(super::escape(r#"<a & "b">"#), "&lt;a &amp; &quot;b&quot;&gt;");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        assert_eq!(page_of_url(&url_of_page("/giveaways/scanner")).as_deref(), Some("/giveaways/scanner"));
        assert_eq!(page_of_url("twitch-desk://open/giveaways/").as_deref(), Some("/giveaways"));
        assert_eq!(page_of_url("\"twitch-desk://open/giveaways/wins\"").as_deref(), Some("/giveaways/wins"));
        assert_eq!(page_of_url("twitch-desk://open").as_deref(), Some("/"));
        assert_eq!(page_of_url("twitch-desk://open/../x?y=1"), None);
        assert_eq!(page_of_url("twitch-desk://open/giveaways/scanner?highlight=some_one").as_deref(), Some("/giveaways/scanner?highlight=some_one"));
        assert_eq!(page_of_url("twitch-desk://open/giveaways/scanner?highlight=a&b=c"), None);
        assert_eq!(page_of_url("twitch-desk://open/x?<script>"), None);
        assert_eq!(page_of_url("twitch-desk://other/x"), None);
        assert_eq!(page_of_url("--minimized"), None);
        assert_eq!(start_page(["exe".to_owned(), "twitch-desk://open/drops".to_owned()]).as_deref(), Some("/drops"));
    }
}
