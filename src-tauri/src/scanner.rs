//! Scanner de streams : lives des catégories surveillées (page Catégories) dont le
//! titre contient un mot-clé de giveaway, comme le scanner du site Giveaway Twitch.
//! Scan à la demande, ou automatique toutes les 3 min avec notification des nouveaux.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::State;

use crate::app::{App, EventKind};
use crate::db::Settings;
use crate::twitch::gql::{self, CategoryStream};

const AUTO_EVERY: Duration = Duration::from_secs(3 * 60);
/// Lives lus par catégorie (100 par page) : une grande catégorie dépasse souvent les 100 lives,
/// et les petits streamers sont en fin de liste
const STREAMS_PER_CATEGORY: usize = 1000;
/// Au-delà, une seule notification résume les nouveaux streams
const MAX_NOTIFICATIONS: usize = 3;
/// Fiches lues par scan (description, message épinglé), streams les plus regardés d'abord
const MAX_PROFILES: usize = 40;
/// Giveaways StreamElements vérifiés par scan
const MAX_SE_CHECKS: usize = 20;
/// Streams retenus gardés dans la vue (les plus regardés)
const MAX_MATCHES: usize = 500;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanMatch {
    #[serde(flatten)]
    pub stream: CategoryStream,
    /// Mots-clés trouvés dans le titre
    pub keywords: Vec<String>,
    /// Déjà dans les chaînes surveillées
    pub monitored: bool,
    /// Ignoré jusqu'à la fin de ce live
    pub ignored: bool,
    /// Description de la chaîne et message épinglé (pour repérer les cadeaux)
    pub description: Option<String>,
    pub pinned_message: Option<String>,
    /// Type du giveaway en cours, s'il est connu : giveaway ouvert de la chaîne, sinon
    /// giveaway StreamElements actif (None : on ne sait pas)
    pub giveaway_kind: Option<String>,
    pub giveaway_command: Option<String>,
    /// Giveaway StreamElements actif : titre et description (pour repérer les cadeaux)
    pub giveaway_text: Option<String>,
}

/// Fiche gardée pour la durée d'un live : début du live, description, message épinglé
type ProfileCache = HashMap<String, (Option<String>, Option<String>, Option<String>)>;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScannerView {
    pub matches: Vec<ScanMatch>,
    pub scanned_at: Option<String>,
    pub scanned_streams: usize,
    pub scanning: bool,
    pub error: Option<String>,
}

/// Live ignoré : son début (None : inconnu) et le moment où il a été ignoré (secondes Unix)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Ignored {
    started: Option<String>,
    at: i64,
}

/// Clé de l'état gardé en base (table app_state)
const IGNORED_KEY: &str = "scanner_ignored";
/// Un live ignoré absent des résultats (filtres changés, relevé incomplet) est oublié après ce délai
const IGNORED_MAX_SECS: i64 = 48 * 3600;

/// Chaînes ignorées gardées en base, relues au démarrage
fn load_ignored(app: &App) -> HashMap<String, Ignored> {
    app.db.get_state(IGNORED_KEY).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

fn save_ignored(app: &App, ignored: &HashMap<String, Ignored>) {
    if let Ok(v) = serde_json::to_string(ignored) {
        if let Err(e) = app.db.set_state(IGNORED_KEY, &v) {
            tracing::warn!("scanner : chaînes ignorées non enregistrées ({e})");
        }
    }
}

/// Garde un live ignoré tant qu'il continue : même début s'il est dans les résultats, sinon
/// jusqu'à `IGNORED_MAX_SECS` (il peut manquer d'un relevé sans être terminé)
fn still_ignored(entry: &Ignored, live: Option<&Option<String>>, now: i64) -> bool {
    match live {
        Some(started) => *started == entry.started,
        None => now - entry.at < IGNORED_MAX_SECS,
    }
}

#[derive(Default)]
pub struct ScannerState {
    view: ScannerView,
    /// Ignorés jusqu'à la fin du live : login → live ignoré (gardé en base : survit à un redémarrage)
    ignored: HashMap<String, Ignored>,
    /// Déjà signalés par une notification : (login, début du live)
    alerted: HashSet<(String, Option<String>)>,
    last_scan: Option<Instant>,
    profiles: ProfileCache,
    /// Streams de toutes les catégories au dernier relevé : un changement de filtre les
    /// trie à nouveau sans tout relire
    streams: Vec<CategoryStream>,
    fetched_at: Option<Instant>,
    fetch_errors: Vec<String>,
    /// Filtres changés pendant un scan : on refiltre à la fin
    rerun: bool,
}

impl ScannerState {
    pub fn view(&self) -> ScannerView {
        self.view.clone()
    }
}

/// Titre sans accents ni majuscules, découpé en mots
fn words(text: &str) -> Vec<String> {
    let folded = crate::db::fold(text);
    folded.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

/// Mots-clés présents au début d'un mot du titre (« key » trouve « keys », pas « monkey ») ;
/// mot-clé de plusieurs mots : mots qui se suivent (« free key » trouve « FREE-KEYS »)
pub fn matching_keywords(title: &str, keywords: &[String]) -> Vec<String> {
    let title = words(title).join(" ");
    keywords
        .iter()
        .filter(|k| crate::db::starts_word(&title, &words(k).join(" ")))
        .cloned()
        .collect()
}

/// Streams retenus selon les réglages. Aucun mot-clé coché : pas de filtre sur le titre
pub fn filter_streams(streams: Vec<CategoryStream>, settings: &Settings) -> Vec<(CategoryStream, Vec<String>)> {
    let streams = streams.into_iter().filter(|s| !settings.scanner_games_off.contains(&s.game_id));
    let languages: Vec<String> = settings.scanner_languages.iter().map(|l| l.to_lowercase()).collect();
    let keywords: Vec<String> =
        settings.scanner_keywords.iter().filter(|k| !settings.scanner_keywords_off.contains(k)).cloned().collect();
    let mut seen = HashSet::new();
    let mut out: Vec<(CategoryStream, Vec<String>)> = streams
        .filter(|s| s.viewers >= settings.scanner_min_viewers as i64)
        .filter(|s| languages.is_empty() || s.language.as_ref().is_some_and(|l| languages.contains(l)))
        .filter_map(|s| {
            let found = matching_keywords(&s.title, &keywords);
            (keywords.is_empty() || !found.is_empty()).then_some((s, found))
        })
        .filter(|(s, _)| seen.insert(s.login.clone()))
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.0.viewers));
    out
}

pub fn spawn(app: Arc<App>) {
    app.scanner.lock().unwrap().ignored = load_ignored(&app);
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let due = app.scanner.lock().unwrap().last_scan.is_none_or(|t| t.elapsed() >= AUTO_EVERY);
            if due && app.db.settings().scanner_auto && !app.is_paused() {
                run_scan(&app, true, true).await;
            }
        }
    });
}

/// `fetch` : relit les streams des catégories ; sinon refiltre ceux du dernier relevé
/// (relus quand même s'ils datent de plus de 3 min)
async fn run_scan(app: &Arc<App>, notify: bool, fetch: bool) {
    {
        let mut state = app.scanner.lock().unwrap();
        if state.view.scanning {
            state.rerun = true;
            return;
        }
        state.view.scanning = true;
    }
    app.touch_domain(crate::app::Domain::Scanner);
    let mut fetch = fetch;
    loop {
        scan_once(app, notify, fetch).await;
        let mut state = app.scanner.lock().unwrap();
        if !std::mem::take(&mut state.rerun) {
            state.view.scanning = false;
            break;
        }
        fetch = false;
    }
    app.touch_domain(crate::app::Domain::Scanner);
}

async fn scan_once(app: &Arc<App>, notify: bool, fetch: bool) {
    let games = app.db.giveaway_games().unwrap_or_default();
    let stale = app.scanner.lock().unwrap().fetched_at.is_none_or(|t| t.elapsed() >= AUTO_EVERY);
    let (streams, errors) = if fetch || stale {
        let mut streams = vec![];
        let mut errors = vec![];
        // Jeton du premier compte : sans lui, Twitch ne donne qu'une page de lives par requête
        let account_gql = app.db.accounts().ok().and_then(|a| a.into_iter().next()).and_then(|account| {
            let tokens = app.secrets.load(&account.user_id)?;
            Some(gql::Gql::new(app.http.clone(), tokens.access_token, Some(account.device_id)))
        });
        // Toutes les catégories, même décochées : les recocher est alors immédiat
        for game in &games {
            match gql::category_streams(&app.http, account_gql.as_ref(), &game.slug, STREAMS_PER_CATEGORY).await {
                Ok(list) => streams.extend(list),
                Err(e) => errors.push(format!("{} : {e}", game.name)),
            }
        }
        let mut state = app.scanner.lock().unwrap();
        state.streams = streams.clone();
        state.fetch_errors = errors.clone();
        state.fetched_at = Some(Instant::now());
        state.last_scan = Some(Instant::now());
        (streams, errors)
    } else {
        let state = app.scanner.lock().unwrap();
        (state.streams.clone(), state.fetch_errors.clone())
    };
    // Lu après le relevé : un filtre changé pendant ce temps est déjà pris en compte
    let mut settings = app.db.settings();
    // Aucune catégorie cochée : comme si toutes l'étaient
    if games.iter().all(|g| settings.scanner_games_off.contains(&g.game_id)) {
        settings.scanner_games_off.clear();
    }
    let scanned = streams.iter().filter(|s| !settings.scanner_games_off.contains(&s.game_id)).count();
    let monitored: HashSet<String> = app.db.giveaway_channels().unwrap_or_default().into_iter().map(|c| c.login).collect();
    let mut found = filter_streams(streams, &settings);
    found.truncate(MAX_MATCHES);

    // Type du giveaway : giveaway ouvert (ou en pause) connu de la chaîne, sinon StreamElements actif
    let open = app.db.giveaways(200).unwrap_or_default();
    // Chaîne → (type, commande, texte du giveaway StreamElements : titre et description)
    let mut kinds: HashMap<String, (String, Option<String>, Option<String>)> = HashMap::new();
    for (s, _) in &found {
        if let Some(g) = open.iter().find(|g| g.status != "closed" && g.channel == s.login) {
            kinds.insert(s.login.clone(), (g.kind.clone(), g.command.clone(), None));
        }
    }
    let to_check: Vec<String> =
        found.iter().map(|(s, _)| s.login.clone()).filter(|l| !kinds.contains_key(l)).take(MAX_SE_CHECKS).collect();
    for login in to_check {
        let Ok(se_id) = crate::streamelements::channel_id_cached(app, &login).await else { continue };
        let Some(se_id) = se_id else { continue };
        if let Ok(Some(g)) = crate::streamelements::active_giveaway_cached(app, &se_id, crate::lives::SE_SHARED_MAX_AGE).await {
            let text = format!("{}\n{}", g.title, g.description);
            kinds.insert(login, ("streamelements".into(), g.command, Some(text)));
        }
    }

    // Description et message épinglé des streams retenus, relus seulement pour un nouveau live
    let cache = app.scanner.lock().unwrap().profiles.clone();
    let mut profiles: ProfileCache = HashMap::new();
    for (s, _) in found.iter().take(MAX_PROFILES) {
        match cache.get(&s.login).filter(|(started, _, _)| *started == s.started_at) {
            Some(cached) => {
                profiles.insert(s.login.clone(), cached.clone());
            }
            None => {
                if let Ok(Some(p)) = gql::channel_profile(&app.http, &s.login).await {
                    profiles.insert(s.login.clone(), (s.started_at.clone(), p.description, p.pinned_message));
                }
            }
        }
    }

    let mut fresh = vec![];
    {
        let mut state = app.scanner.lock().unwrap();
        // Un streamer ignoré l'est jusqu'à la fin de ce live : nouveau live, il revient
        let live: HashMap<String, Option<String>> = found.iter().map(|(s, _)| (s.login.clone(), s.started_at.clone())).collect();
        let now = chrono::Utc::now().timestamp();
        let before = state.ignored.len();
        state.ignored.retain(|login, entry| still_ignored(entry, live.get(login), now));
        if state.ignored.len() != before {
            save_ignored(app, &state.ignored);
        }
        state.alerted.retain(|(login, started)| live.get(login).is_some_and(|s| s == started));
        let matches: Vec<ScanMatch> = found
            .into_iter()
            .map(|(stream, keywords)| {
                let (_, description, pinned_message) = profiles.get(&stream.login).cloned().unwrap_or_default();
                let (giveaway_kind, giveaway_command, giveaway_text) = match kinds.get(&stream.login) {
                    Some((kind, command, text)) => (Some(kind.clone()), command.clone(), text.clone()),
                    None => (None, None, None),
                };
                ScanMatch {
                    giveaway_kind,
                    giveaway_command,
                    giveaway_text,
                    monitored: monitored.contains(&stream.login),
                    ignored: state.ignored.contains_key(&stream.login),
                    description,
                    pinned_message,
                    stream,
                    keywords,
                }
            })
            .collect();
        state.profiles = profiles;
        for m in &matches {
            let key = (m.stream.login.clone(), m.stream.started_at.clone());
            if !m.monitored && !m.ignored && !state.alerted.contains(&key) {
                state.alerted.insert(key);
                fresh.push(m.clone());
            }
        }
        state.view = ScannerView {
            matches,
            scanned_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            scanned_streams: scanned,
            scanning: true,
            error: if games.is_empty() {
                Some("Aucune catégorie : ajoute-en dans Giveaways → Catégories".into())
            } else if errors.is_empty() {
                None
            } else {
                Some(errors.join(" · "))
            },
        };
    }
    app.touch_domain(crate::app::Domain::Scanner);

    if notify && !fresh.is_empty() {
        for m in fresh.iter().take(MAX_NOTIFICATIONS) {
            app.log("scanner", EventKind::Giveaway, format!("Scanner : {} ({}) : {}", m.stream.login, m.stream.game_name, m.stream.title));
            // Clic : page Scanner, ce stream mis en avant
            let page = format!("{}?highlight={}", crate::app::Alert::Scanner.page(), m.stream.login);
            app.notify_page(crate::app::Alert::Scanner, &format!("Stream repéré : {}", m.stream.display_name), &m.stream.title, &page);
        }
        if fresh.len() > MAX_NOTIFICATIONS {
            let rest = fresh.len() - MAX_NOTIFICATIONS;
            app.notify(crate::app::Alert::Scanner, "Scanner", &format!("{rest} autre(s) stream(s) repéré(s) : voir la page Scanner"));
        }
    }
}

/// Scan à la demande (sans notification : les résultats sont affichés)
#[tauri::command]
pub async fn scan_streams(app: State<'_, Arc<App>>) -> Result<(), String> {
    let app: Arc<App> = app.inner().clone();
    run_scan(&app, false, true).await;
    Ok(())
}

/// Filtres changés : les résultats sont refaits sur les streams déjà relevés
#[tauri::command]
pub async fn scanner_refilter(app: State<'_, Arc<App>>) -> Result<(), String> {
    let app: Arc<App> = app.inner().clone();
    run_scan(&app, false, false).await;
    Ok(())
}

/// Ignore (jusqu'à la fin de son live) ou rétablit un stream
#[tauri::command]
pub fn scanner_ignore(app: State<'_, Arc<App>>, login: String, ignore: bool) {
    let mut state = app.scanner.lock().unwrap();
    let started = state.view.matches.iter().find(|m| m.stream.login == login).and_then(|m| m.stream.started_at.clone());
    if ignore {
        state.ignored.insert(login.clone(), Ignored { started, at: chrono::Utc::now().timestamp() });
    } else {
        state.ignored.remove(&login);
    }
    save_ignored(&app, &state.ignored);
    for m in &mut state.view.matches {
        if m.stream.login == login {
            m.ignored = ignore;
        }
    }
    drop(state);
    app.touch_domain(crate::app::Domain::Scanner);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(login: &str, title: &str, viewers: i64, lang: &str) -> CategoryStream {
        CategoryStream {
            login: login.into(),
            display_name: login.into(),
            title: title.into(),
            viewers,
            started_at: Some("2026-11-01T08:00:00Z".into()),
            language: Some(lang.into()),
            profile_image: None,
            game_id: "490377".into(),
            game_name: "Sea of Thieves".into(),
            tags: vec![],
        }
    }

    #[test]
    fn keywords_match_word_starts() {
        let k: Vec<String> = ["giveaway", "key", "clé", "drop"].iter().map(|s| s.to_string()).collect();
        assert_eq!(matching_keywords("🎃GIVEAWAY🎃 ce soir", &k), vec!["giveaway"]);
        assert_eq!(matching_keywords("!giveaway !tts", &k), vec!["giveaway"]);
        assert_eq!(matching_keywords("Une CLE steam à gagner", &k), vec!["clé"]);
        assert_eq!(matching_keywords("3 keys tonight", &k), vec!["key"]);
        assert!(matching_keywords("monkey business", &k).is_empty());
        assert_eq!(matching_keywords("DROPS activés", &k), vec!["drop"]);
        // Mots-clés de plusieurs mots : mots qui se suivent
        let k: Vec<String> = ["free key", "clé steam"].iter().map(|s| s.to_string()).collect();
        assert_eq!(matching_keywords("FREE-KEYS tonight", &k), vec!["free key"]);
        assert_eq!(matching_keywords("Une CLE  Steam à gagner", &k), vec!["clé steam"]);
        assert!(matching_keywords("free stuff, key later", &k).is_empty());
    }

    #[test]
    fn filters() {
        let mut settings = Settings { scanner_min_viewers: 10, ..Settings::default() };
        let list = vec![
            stream("a", "GIVEAWAY ce soir", 200, "fr"),
            stream("b", "giveaway", 5, "fr"),
            stream("c", "Chill", 500, "fr"),
            stream("d", "Key giveaway", 300, "en"),
            stream("a", "GIVEAWAY ce soir", 200, "fr"),
        ];
        let found = filter_streams(list.clone(), &settings);
        assert_eq!(found.iter().map(|(s, _)| s.login.as_str()).collect::<Vec<_>>(), vec!["d", "a"]);
        settings.scanner_languages = vec!["FR".into()];
        let found = filter_streams(list.clone(), &settings);
        assert_eq!(found.iter().map(|(s, _)| s.login.as_str()).collect::<Vec<_>>(), vec!["a"]);
        // Un mot-clé décoché n'est plus cherché
        settings.scanner_keywords_off = vec!["giveaway".into()];
        assert!(filter_streams(list.clone(), &settings).is_empty());
        // Aucun mot-clé coché : pas de filtre sur le titre
        settings.scanner_keywords_off = settings.scanner_keywords.clone();
        let found = filter_streams(list.clone(), &settings);
        assert_eq!(found.iter().map(|(s, _)| s.login.as_str()).collect::<Vec<_>>(), vec!["c", "a"]);
        assert!(found[0].1.is_empty());
        // Une catégorie décochée : ses streams sont écartés
        settings.scanner_keywords_off = vec![];
        settings.scanner_games_off = vec!["490377".into()];
        assert!(filter_streams(list, &settings).is_empty());
    }

    #[test]
    fn ignored_live_rules() {
        let entry = Ignored { started: Some("2026-11-02T10:00:00Z".into()), at: 1_000 };
        // Même live dans les résultats : gardé ; nouveau live : oublié
        assert!(still_ignored(&entry, Some(&Some("2026-11-02T10:00:00Z".into())), 999_999));
        assert!(!still_ignored(&entry, Some(&Some("2026-11-03T10:00:00Z".into())), 2_000));
        // Absent des résultats : gardé 48 h
        assert!(still_ignored(&entry, None, 1_000 + IGNORED_MAX_SECS - 1));
        assert!(!still_ignored(&entry, None, 1_000 + IGNORED_MAX_SECS));
        // Relu tel quel depuis la base
        let json = serde_json::to_string(&HashMap::from([("abc".to_string(), entry.clone())])).unwrap();
        let back: HashMap<String, Ignored> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.get("abc"), Some(&entry));
    }
}
