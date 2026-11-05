//! Réponse automatique au gain : quand un de nos comptes est annoncé gagnant d'un giveaway avec
//! présence requise, il écrit la phrase choisie dans la fiche du streamer, après le délai choisi.
//! Pendant ce délai, la réponse peut être annulée (notification Windows, interface).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, State};

use crate::app::{App, EventKind};

/// Réponse en attente de son délai
struct Pending {
    login: String,
    channel: String,
    giveaway_id: i64,
}

pub struct WinReplies {
    next: AtomicU64,
    pending: Mutex<HashMap<u64, Pending>>,
}

/// Numéros propres à chaque lancement (heure du lancement en millisecondes × 1000) : le lien
/// « Annuler » d'une notification d'un lancement précédent ne vise aucune réponse de celui-ci.
/// Sous 2^53 : lisible tel quel par l'interface
impl Default for WinReplies {
    fn default() -> Self {
        let start = chrono::Utc::now().timestamp_millis().max(0) as u64 * 1000;
        Self { next: AtomicU64::new(start), pending: Mutex::default() }
    }
}

/// Réponse annoncée à l'interface (bouton « Annuler » pendant le délai)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WinReplyView {
    pub id: u64,
    pub login: String,
    pub channel: String,
    pub text: String,
    pub delay: u64,
}

/// Fin d'une réponse : « sent », « failed » (avec le motif) ou « cancelled »
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WinReplyDone {
    id: u64,
    login: String,
    channel: String,
    outcome: &'static str,
    message: Option<String>,
}

/// Programme la réponse de `account_id` dans le chat de `channel` ; renvoie son numéro
pub fn schedule(app: &Arc<App>, account_id: i64, giveaway_id: i64, login: &str, channel: &str, text: &str, delay: u64) -> u64 {
    let id = app.win_replies.next.fetch_add(1, Ordering::Relaxed) + 1;
    app.win_replies.pending.lock().unwrap().insert(id, Pending { login: login.to_owned(), channel: channel.to_owned(), giveaway_id });
    let view = WinReplyView { id, login: login.to_owned(), channel: channel.to_owned(), text: text.to_owned(), delay };
    let _ = app.handle().emit_to("main", "win-reply", &view);
    app.log(login, EventKind::Giveaway, format!("Réponse au gain sur {channel} dans {delay} s : {text}"));
    let app = app.clone();
    let text = text.to_owned();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay)).await;
        // Annulée pendant le délai : plus en attente
        let Some(p) = app.win_replies.pending.lock().unwrap().remove(&id) else { return };
        // Situation changée pendant le délai (gagnant retiré, giveaway masqué, application en
        // pause) : pas de réponse
        if let Some(why) = stale(&app, &p) {
            app.log(&p.login, EventKind::Info, format!("Réponse au gain sur {} abandonnée : {why}", p.channel));
            let done = WinReplyDone { id, login: p.login, channel: p.channel, outcome: "cancelled", message: Some(why.to_owned()) };
            let _ = app.handle().emit_to("main", "win-reply-done", &done);
            return;
        }
        let account = app.db.accounts().ok().and_then(|list| list.into_iter().find(|a| a.id == account_id));
        let result = match account.and_then(|a| app.secrets.load(&a.user_id)) {
            None => Err("jeton introuvable : reconnecte ce compte".to_owned()),
            Some(tokens) => crate::twitch::chat::send_message(&p.login, &tokens.access_token, &p.channel, &text).await,
        };
        let (outcome, message) = match result {
            Ok(()) => {
                app.log(&p.login, EventKind::Giveaway, format!("Réponse au gain envoyée sur {} : {text}", p.channel));
                ("sent", None)
            }
            Err(e) => {
                app.log(&p.login, EventKind::Error, format!("Réponse au gain non envoyée sur {} : {e}", p.channel));
                ("failed", Some(e))
            }
        };
        let done = WinReplyDone { id, login: p.login, channel: p.channel, outcome, message };
        let _ = app.handle().emit_to("main", "win-reply-done", &done);
    });
    id
}

/// Raison de ne plus envoyer une réponse programmée, relue juste avant l'envoi
fn stale(app: &App, p: &Pending) -> Option<&'static str> {
    if app.is_paused() {
        return Some("application en pause");
    }
    match app.db.giveaway(p.giveaway_id) {
        Ok(Some(g)) if g.hidden => Some("giveaway masqué"),
        Ok(Some(g)) if !g.winners.iter().any(|w| w.eq_ignore_ascii_case(&p.login)) => Some("ce compte n'est plus noté gagnant"),
        Ok(Some(_)) => None,
        Ok(None) => Some("giveaway supprimé"),
        // Base illisible : la réponse part, comme avant la vérification
        Err(_) => None,
    }
}

/// Annule une réponse encore en attente ; `false` : déjà partie (ou inconnue)
pub fn cancel(app: &App, id: u64) -> bool {
    let Some(p) = app.win_replies.pending.lock().unwrap().remove(&id) else { return false };
    app.log(&p.login, EventKind::Info, format!("Réponse au gain sur {} annulée", p.channel));
    let done = WinReplyDone { id, login: p.login, channel: p.channel, outcome: "cancelled", message: None };
    let _ = app.handle().emit_to("main", "win-reply-done", &done);
    true
}

/// Lien d'annulation porté par la notification Windows
pub fn cancel_url(id: u64) -> String {
    format!("{}://cancel-reply/{id}", crate::toast::SCHEME)
}

/// Numéro de la réponse d'un lien d'annulation
pub fn id_of_cancel_url(url: &str) -> Option<u64> {
    url.trim().trim_matches('"').strip_prefix(crate::toast::SCHEME)?.strip_prefix("://cancel-reply/")?.trim_end_matches('/').parse().ok()
}

#[tauri::command]
pub fn cancel_win_reply(app: State<'_, Arc<App>>, id: u64) -> bool {
    cancel(&app, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_links() {
        assert_eq!(id_of_cancel_url(&cancel_url(42)), Some(42));
        assert_eq!(id_of_cancel_url("\"twitch-desk://cancel-reply/7/\""), Some(7));
        assert_eq!(id_of_cancel_url("twitch-desk://open/giveaways"), None);
        assert_eq!(id_of_cancel_url("twitch-desk://cancel-reply/x"), None);
    }
}
