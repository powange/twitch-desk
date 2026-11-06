//! Réponse automatique au gain : quand un de nos comptes est annoncé gagnant d'un giveaway avec
//! présence requise, il écrit les phrases choisies dans la fiche du streamer (une par ligne), la
//! première après le délai choisi, les suivantes espacées du délai entre deux phrases. Jusqu'à la
//! dernière, la réponse peut être annulée (notification Windows, interface).

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
    /// Giveaway du gain (None : gain annoncé sans giveaway trouvé, d'après la fiche du streamer)
    giveaway_id: Option<i64>,
    /// Phrases et heure d'envoi de la première (affichées sur le téléphone, annulables)
    text: String,
    send_at: String,
}

/// Réponses en attente, pour l'état publié au téléphone (bouton « Annuler »)
pub fn pending_view(app: &App) -> Vec<serde_json::Value> {
    let pending = app.win_replies.pending.lock().unwrap();
    let mut list: Vec<serde_json::Value> = pending
        .iter()
        .map(|(id, p)| serde_json::json!({ "id": id.to_string(), "login": p.login, "channel": p.channel, "text": p.text, "sendAt": p.send_at }))
        .collect();
    list.sort_by(|a, b| a["sendAt"].as_str().cmp(&b["sendAt"].as_str()));
    list
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

/// Programme la réponse de `account_id` dans le chat de `channel` ; renvoie son numéro. Plusieurs
/// phrases (une par ligne) : la première après `delay`, chacune des suivantes `gap` secondes plus
/// tard ; annulable jusqu'à l'envoi de la dernière
#[allow(clippy::too_many_arguments)]
pub fn schedule(app: &Arc<App>, account_id: i64, giveaway_id: Option<i64>, login: &str, channel: &str, text: &str, delay: u64, gap: u64) -> u64 {
    let id = app.win_replies.next.fetch_add(1, Ordering::Relaxed) + 1;
    let send_at = (chrono::Utc::now() + chrono::Duration::seconds(delay as i64)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    app.win_replies.pending.lock().unwrap().insert(
        id,
        Pending { login: login.to_owned(), channel: channel.to_owned(), giveaway_id, text: text.to_owned(), send_at },
    );
    app.sync.state_changed();
    let view = WinReplyView { id, login: login.to_owned(), channel: channel.to_owned(), text: text.to_owned(), delay };
    let _ = app.handle().emit_to("main", "win-reply", &view);
    app.log(login, EventKind::Giveaway, format!("Réponse au gain sur {channel} dans {delay} s : {}", text.replace('\n', " / ")));
    let app = app.clone();
    let phrases: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
    tauri::async_runtime::spawn(async move {
        let mut failure: Option<String> = None;
        let mut sent = 0;
        for (i, phrase) in phrases.iter().enumerate() {
            tokio::time::sleep(Duration::from_secs(if i == 0 { delay } else { gap })).await;
            // Annulée en route : plus en attente (le message d'annulation est déjà parti)
            let Some(p) = app.win_replies.pending.lock().unwrap().get(&id).map(|p| Pending {
                login: p.login.clone(),
                channel: p.channel.clone(),
                giveaway_id: p.giveaway_id,
                text: String::new(),
                send_at: String::new(),
            }) else {
                return;
            };
            // Situation changée en route (gagnant retiré, giveaway masqué, application en pause) :
            // pas de phrase de plus
            if let Some(why) = stale(&app, &p) {
                app.win_replies.pending.lock().unwrap().remove(&id);
                app.sync.state_changed();
                app.log(&p.login, EventKind::Info, format!("Réponse au gain sur {} abandonnée : {why}", p.channel));
                let done = WinReplyDone { id, login: p.login, channel: p.channel, outcome: "cancelled", message: Some(why.to_owned()) };
                let _ = app.handle().emit_to("main", "win-reply-done", &done);
                return;
            }
            let account = app.db.accounts().ok().and_then(|list| list.into_iter().find(|a| a.id == account_id));
            let result = match account.and_then(|a| app.secrets.load(&a.user_id)) {
                None => Err("jeton introuvable : reconnecte ce compte".to_owned()),
                Some(tokens) => crate::twitch::chat::send_message(&p.login, &tokens.access_token, &p.channel, phrase).await,
            };
            match result {
                Ok(()) => {
                    sent += 1;
                    app.log(&p.login, EventKind::Giveaway, format!("Réponse au gain envoyée sur {} : {phrase}", p.channel));
                }
                Err(e) => {
                    app.log(&p.login, EventKind::Error, format!("Réponse au gain non envoyée sur {} : {e}", p.channel));
                    failure = Some(e);
                    break;
                }
            }
        }
        let Some(p) = app.win_replies.pending.lock().unwrap().remove(&id) else { return };
        app.sync.state_changed();
        let (outcome, message) = match failure {
            None => ("sent", None),
            Some(e) if sent > 0 => ("failed", Some(format!("{sent} phrase(s) envoyée(s), puis : {e}"))),
            Some(e) => ("failed", Some(e)),
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
    // Sans giveaway (gain annoncé hors giveaway connu) : rien d'autre à revérifier
    let id = p.giveaway_id?;
    match app.db.giveaway(id) {
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
    app.sync.state_changed();
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
