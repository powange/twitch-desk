//! Informations par chaîne partagées entre comptes : statut live, réglage de fidélité et
//! giveaway en cours StreamElements.
//!
//! Plusieurs comptes suivent souvent les mêmes chaînes : sans ce cache, chaque superviseur et
//! le moniteur des giveaways relisaient le même statut chacun de leur côté. Une valeur assez
//! récente est rendue telle quelle ; sinon une seule lecture a lieu, les appels simultanés pour
//! la même chaîne attendent son résultat. Une erreur n'est pas gardée : l'appelant suivant
//! relit avec son propre jeton (repli sur un autre compte si un jeton a expiré).

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::streamelements::{Giveaway, Loyalty};
use crate::twitch::gql::{self, Gql, StreamInfo};
use crate::twitch::{Result, TwitchError};

/// Lectures du statut live une par une (avec un compte) au plus par appel, si la requête
/// groupée sans compte échoue : les autres chaînes attendent le passage suivant
pub const FALLBACK_PER_CALL: usize = 3;
/// Chaînes au plus par relecture groupée
pub const BATCH: usize = gql::STREAMS_BATCH;

/// Instant à partir duquel une valeur lue a au plus `max_age`
pub fn since(max_age: Duration) -> Instant {
    let now = Instant::now();
    // Horloge trop jeune (démarrage de la machine) : tout est relu
    now.checked_sub(max_age).unwrap_or(now)
}

struct Entry<V> {
    /// None : valeur invalidée à `at` (une lecture commencée avant n'est pas gardée)
    value: Option<V>,
    at: Instant,
}

struct Slot<V> {
    entry: Mutex<Option<Entry<V>>>,
    /// Tenu pendant une lecture : les autres appelants attendent son résultat
    fetching: tokio::sync::Mutex<()>,
}

impl<V> Default for Slot<V> {
    fn default() -> Self {
        Self { entry: Mutex::new(None), fetching: tokio::sync::Mutex::new(()) }
    }
}

/// Cache clé → valeur avec date de lecture, lectures regroupées par clé
pub struct SharedCache<V> {
    slots: Mutex<HashMap<String, Arc<Slot<V>>>>,
}

impl<V> Default for SharedCache<V> {
    fn default() -> Self {
        Self { slots: Mutex::new(HashMap::new()) }
    }
}

impl<V: Clone> SharedCache<V> {
    fn slot(&self, key: &str) -> Arc<Slot<V>> {
        let mut slots = self.slots.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        slots.entry(key.to_owned()).or_default().clone()
    }

    fn fresh(slot: &Slot<V>, since: Instant) -> Option<V> {
        let entry = slot.entry.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        entry.as_ref().filter(|e| e.at >= since).and_then(|e| e.value.clone())
    }

    /// Garde une valeur lue depuis `started`, sauf si une plus récente est arrivée entre-temps
    /// (événement PubSub)
    fn store(slot: &Slot<V>, value: V, started: Instant) {
        let mut entry = slot.entry.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if entry.as_ref().is_none_or(|e| e.at < started) {
            *entry = Some(Entry { value: Some(value), at: Instant::now() });
        }
    }

    /// Valeur lue depuis `since`, sans lecture
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn peek(&self, key: &str, since: Instant) -> Option<V> {
        Self::fresh(&self.slot(key), since)
    }

    /// Valeur connue sans lecture (événement PubSub)
    pub fn put(&self, key: &str, value: V) {
        let slot = self.slot(key);
        *slot.entry.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Entry { value: Some(value), at: Instant::now() });
    }

    /// Oublie la valeur : la prochaine demande relit, une lecture en cours n'est pas gardée
    pub fn invalidate(&self, key: &str) {
        let slot = self.slot(key);
        *slot.entry.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Entry { value: None, at: Instant::now() });
    }

    /// Retouche la valeur connue sans changer sa date (nombre de spectateurs)
    pub fn update(&self, key: &str, f: impl FnOnce(&mut V)) {
        let slot = self.slot(key);
        let mut entry = slot.entry.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(value) = entry.as_mut().and_then(|e| e.value.as_mut()) {
            f(value);
        }
    }

    /// Valeur lue depuis `since`, sinon `fetch` (une seule lecture à la fois par clé)
    pub async fn get<E, Fut>(&self, key: &str, since: Instant, fetch: impl FnOnce() -> Fut) -> std::result::Result<V, E>
    where
        Fut: Future<Output = std::result::Result<V, E>>,
    {
        let slot = self.slot(key);
        if let Some(v) = Self::fresh(&slot, since) {
            return Ok(v);
        }
        let _fetching = slot.fetching.lock().await;
        // Lue par un autre appelant pendant l'attente
        if let Some(v) = Self::fresh(&slot, since) {
            return Ok(v);
        }
        let started = Instant::now();
        let value = fetch().await?;
        Self::store(&slot, value.clone(), started);
        Ok(value)
    }

    /// Comme `get` pour plusieurs clés (chacune avec son `since`) : les valeurs périmées sont
    /// lues ensemble par `fetch`. Une clé absente de son résultat n'a pas été lue (absente du
    /// retour aussi)
    pub async fn get_many<E, Fut>(
        &self,
        wanted: &[(String, Instant)],
        fetch: impl FnOnce(Vec<String>) -> Fut,
    ) -> HashMap<String, std::result::Result<V, E>>
    where
        Fut: Future<Output = HashMap<String, std::result::Result<V, E>>>,
    {
        let mut out = HashMap::new();
        // Une fois chaque clé (la plus exigeante), dans l'ordre : verrous pris toujours dans le
        // même ordre, sans interblocage entre appels qui se recoupent
        let mut keys: Vec<(String, Instant)> = vec![];
        for (key, since) in wanted {
            match keys.iter_mut().find(|(k, _)| k == key) {
                Some((_, s)) => *s = (*s).max(*since),
                None => keys.push((key.clone(), *since)),
            }
        }
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        let mut stale: Vec<(String, Instant, Arc<Slot<V>>)> = vec![];
        for (key, since) in keys {
            let slot = self.slot(&key);
            match Self::fresh(&slot, since) {
                Some(v) => {
                    out.insert(key, Ok(v));
                }
                None => stale.push((key, since, slot)),
            }
        }
        if stale.is_empty() {
            return out;
        }
        let mut guards = Vec::with_capacity(stale.len());
        for (_, _, slot) in &stale {
            guards.push(slot.fetching.lock().await);
        }
        let mut missing = vec![];
        for (key, since, slot) in &stale {
            match Self::fresh(slot, *since) {
                Some(v) => {
                    out.insert(key.clone(), Ok(v));
                }
                None => missing.push(key.clone()),
            }
        }
        if !missing.is_empty() {
            let started = Instant::now();
            let results = fetch(missing).await;
            for (key, result) in results {
                let Some((_, _, slot)) = stale.iter().find(|(k, _, _)| *k == key) else { continue };
                if let Ok(v) = &result {
                    Self::store(slot, v.clone(), started);
                }
                out.insert(key, result);
            }
        }
        drop(guards);
        out
    }
}

/// Résultat d'une relecture du statut live
pub struct LiveFetch {
    /// Chaînes relues (ou connues) ; les autres attendent le passage suivant
    pub results: HashMap<String, Result<Option<StreamInfo>>>,
    /// Un jeton a été refusé en cours de route
    pub refused: bool,
}

/// Informations par chaîne partagées par tous les comptes (champ `lives` de l'App)
#[derive(Default)]
pub struct Lives {
    /// login → statut live (None : hors ligne)
    pub streams: SharedCache<Option<StreamInfo>>,
    /// Identifiant StreamElements → réglage de fidélité
    pub se_loyalty: SharedCache<Option<Loyalty>>,
    /// Identifiant StreamElements → giveaway en cours
    pub se_giveaway: SharedCache<Option<Giveaway>>,
}

impl Lives {
    /// Statut live des chaînes demandées (login, lu depuis) : cache, sinon une requête groupée
    /// sans compte, sinon une par une avec les clients donnés (jeton refusé : client suivant)
    pub async fn streams_status(&self, http: &reqwest::Client, gqls: &[Gql], wanted: &[(String, Instant)]) -> LiveFetch {
        let refused = AtomicBool::new(false);
        let results = self.streams.get_many(wanted, |logins| fetch_streams(http, gqls, logins, &refused)).await;
        LiveFetch { results, refused: refused.into_inner() }
    }
}

async fn fetch_streams(
    http: &reqwest::Client,
    gqls: &[Gql],
    logins: Vec<String>,
    refused: &AtomicBool,
) -> HashMap<String, Result<Option<StreamInfo>>> {
    let mut out: HashMap<String, Result<Option<StreamInfo>>> = HashMap::new();
    match gql::streams_info(http, &logins).await {
        Ok(found) => out.extend(found.into_iter().map(|(login, info)| (login, Ok(info)))),
        Err(e) => tracing::debug!("statut live groupé : {e}"),
    }
    let pending: Vec<String> = logins.into_iter().filter(|l| !out.contains_key(l)).take(FALLBACK_PER_CALL).collect();
    let mut gqls: Vec<&Gql> = gqls.iter().collect();
    for login in pending {
        loop {
            let Some(gql) = gqls.first() else { return out };
            match gql.stream_info(&login).await {
                Ok(info) => {
                    out.insert(login, Ok(info));
                    break;
                }
                Err(TwitchError::Unauthorized) => {
                    refused.store(true, Ordering::Relaxed);
                    if gqls.len() == 1 {
                        // Plus aucun jeton accepté : l'appelant le saura
                        out.insert(login, Err(TwitchError::Unauthorized));
                        return out;
                    }
                    gqls.remove(0);
                }
                Err(e) => {
                    out.insert(login, Err(e));
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn ago(secs: u64) -> Instant {
        since(Duration::from_secs(secs))
    }

    #[tokio::test]
    async fn fresh_value_is_reused() {
        let cache: SharedCache<u32> = SharedCache::default();
        let calls = &AtomicUsize::new(0);
        let fetch = || async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(7)
        };
        assert_eq!(cache.get("a", ago(60), fetch).await, Ok(7));
        assert_eq!(cache.get("a", ago(60), fetch).await, Ok(7));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Exigence plus stricte que l'âge de la valeur : relue
        assert_eq!(cache.get("a", Instant::now(), fetch).await, Ok(7));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn errors_are_not_kept() {
        let cache: SharedCache<u32> = SharedCache::default();
        assert_eq!(cache.get("a", ago(60), || async { Err::<u32, _>("jeton refusé") }).await, Err("jeton refusé"));
        // Appelant suivant (autre compte) : nouvelle lecture
        assert_eq!(cache.get("a", ago(60), || async { Ok::<_, &str>(3) }).await, Ok(3));
        assert_eq!(cache.peek("a", ago(60)), Some(3));
    }

    #[tokio::test]
    async fn concurrent_gets_share_one_fetch() {
        let cache: SharedCache<u32> = SharedCache::default();
        let calls = &AtomicUsize::new(0);
        let fetch = || async move {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<_, ()>(5)
        };
        let (a, b) = tokio::join!(cache.get("x", ago(60), fetch), cache.get("x", ago(60), fetch));
        assert_eq!((a, b), (Ok(5), Ok(5)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_batches_share_fetches() {
        let cache: SharedCache<u32> = SharedCache::default();
        let fetched = &Mutex::new(vec![]);
        let fetch = |keys: Vec<String>| async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            fetched.lock().unwrap().extend(keys.clone());
            keys.into_iter().map(|k| (k, Ok::<_, ()>(1))).collect::<HashMap<_, _>>()
        };
        let first = vec![("a".to_owned(), ago(60)), ("b".to_owned(), ago(60))];
        let second = vec![("b".to_owned(), ago(60)), ("c".to_owned(), ago(60))];
        let (x, y) = tokio::join!(cache.get_many(&first, fetch), cache.get_many(&second, fetch));
        assert_eq!(x.len(), 2);
        assert_eq!(y.len(), 2);
        let mut keys = fetched.lock().unwrap().clone();
        keys.sort();
        // « b » lu une seule fois
        assert_eq!(keys, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn batch_reads_only_stale_keys() {
        let cache: SharedCache<u32> = SharedCache::default();
        cache.put("a", 1);
        let wanted = vec![("a".to_owned(), ago(60)), ("b".to_owned(), ago(60)), ("c".to_owned(), ago(60))];
        let out = cache
            .get_many(&wanted, |keys| async move {
                assert_eq!(keys, vec!["b", "c"]);
                // « c » non lu : absent du résultat, pas gardé
                HashMap::from([("b".to_owned(), Ok::<_, ()>(2))])
            })
            .await;
        assert_eq!(out.get("a"), Some(&Ok(1)));
        assert_eq!(out.get("b"), Some(&Ok(2)));
        assert!(!out.contains_key("c"));
        assert_eq!(cache.peek("c", ago(60)), None);
    }

    #[tokio::test]
    async fn invalidation_forces_a_new_read() {
        let cache: SharedCache<Option<u32>> = SharedCache::default();
        cache.put("a", Some(10));
        cache.update("a", |v| *v = v.map(|n| n + 1));
        assert_eq!(cache.peek("a", ago(60)), Some(Some(11)));
        cache.invalidate("a");
        assert_eq!(cache.peek("a", ago(60)), None);
        // Fin de live connue par PubSub : gardée comme une lecture
        cache.put("a", None);
        assert_eq!(cache.peek("a", ago(60)), Some(None));
    }

    #[tokio::test]
    async fn read_started_before_an_event_is_not_kept() {
        let cache = &SharedCache::<Option<u32>>::default();
        let out = cache
            .get("a", ago(60), || async move {
                // Fin de live reçue pendant la lecture
                tokio::time::sleep(Duration::from_millis(5)).await;
                cache.put("a", None);
                Ok::<_, ()>(Some(3))
            })
            .await;
        assert_eq!(out, Ok(Some(3)));
        assert_eq!(cache.peek("a", ago(60)), Some(None));
    }
}
