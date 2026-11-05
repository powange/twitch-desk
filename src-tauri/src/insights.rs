//! Ce qui rend une chaîne surveillée intéressante pendant son live : giveaway StreamElements
//! actif, description et message épinglé (où les streamers annoncent souvent les lots).
//! Relu toutes les 5 min pour les chaînes en live, effacé quand elles passent hors ligne.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::app::App;
use crate::twitch::gql;

const REFRESH_EVERY: Duration = Duration::from_secs(5 * 60);
const TICK: Duration = Duration::from_secs(20);
/// Chaînes relues par passage, pour étaler les requêtes
const PER_TICK: usize = 4;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeActive {
    pub title: String,
    pub description: String,
    pub command: Option<String>,
    pub cost: i64,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInsight {
    pub description: Option<String>,
    pub pinned_message: Option<String>,
    /// Giveaway StreamElements actif sur la chaîne
    pub streamelements: Option<SeActive>,
    #[serde(skip_serializing)]
    pub checked_at: Option<String>,
}

#[derive(Default)]
pub struct InsightsState {
    view: HashMap<String, ChannelInsight>,
    /// Live relu : identifiant de diffusion et moment du relevé
    checked: HashMap<String, (String, Instant)>,
    /// Identifiant StreamElements par chaîne (None : pas de StreamElements)
    se_ids: HashMap<String, Option<String>>,
}

impl InsightsState {
    /// Pseudo StreamElements de la chaîne changé : identifiant à retrouver
    pub fn forget_se(&mut self, login: &str) {
        self.se_ids.remove(login);
    }

    pub fn view(&self) -> HashMap<String, ChannelInsight> {
        self.view.clone()
    }

    pub fn get(&self, login: &str) -> Option<&ChannelInsight> {
        self.view.get(login)
    }
}

pub fn spawn(app: Arc<App>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            if !app.is_paused() {
                refresh(&app).await;
            }
        }
    });
}

async fn refresh(app: &Arc<App>) {
    // Chaînes surveillées en live : login → identifiant de diffusion
    let live: HashMap<String, String> = app
        .giveaways
        .view()
        .channels
        .into_iter()
        .filter_map(|c| c.live.map(|l| (c.login, l.broadcast_id)))
        .collect();
    let due: Vec<String> = {
        let mut state = app.insights.lock().unwrap();
        let before = state.view.len();
        let live_logins: HashSet<&String> = live.keys().collect();
        state.view.retain(|login, _| live_logins.contains(login));
        state.checked.retain(|login, _| live_logins.contains(login));
        if state.view.len() != before {
            app.touch_domain(crate::app::Domain::Insights);
        }
        live.iter()
            .filter(|(login, broadcast)| {
                state.checked.get(*login).is_none_or(|(b, at)| b != *broadcast || at.elapsed() >= REFRESH_EVERY)
            })
            .map(|(login, _)| login.clone())
            .take(PER_TICK)
            .collect()
    };
    for login in due {
        let mut insight = ChannelInsight::default();
        if let Ok(Some(p)) = gql::channel_profile(&app.http, &login).await {
            crate::commands::keep_pinned(app, &login, &p);
            insight.description = p.description;
            insight.pinned_message = p.pinned_message;
        }
        let known = app.insights.lock().unwrap().se_ids.get(&login).cloned();
        let se_id = match known {
            Some(id) => id,
            None => match crate::streamelements::channel_id(&app.http, &login).await {
                Ok(id) => {
                    app.insights.lock().unwrap().se_ids.insert(login.clone(), id.clone());
                    id
                }
                Err(_) => None,
            },
        };
        if let Some(se_id) = se_id {
            if let Ok(Some(g)) = crate::streamelements::active_giveaway(&app.http, &se_id).await {
                insight.streamelements =
                    Some(SeActive { title: g.title, description: g.description, command: g.command, cost: g.cost });
            }
        }
        insight.checked_at = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        let mut state = app.insights.lock().unwrap();
        state.checked.insert(login.clone(), (live[&login].clone(), Instant::now()));
        state.view.insert(login, insight);
        drop(state);
        app.touch_domain(crate::app::Domain::Insights);
    }
}
