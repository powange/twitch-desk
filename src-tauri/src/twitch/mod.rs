//! Accès à Twitch : connexion, requêtes GQL internes, PubSub, visionnage simulé.
//!
//! Twitch n'expose ni les points de chaîne ni les drops dans son API officielle.
//! On parle donc comme l'application Android TV de Twitch, dont les jetons
//! passent sans contrôle d'intégrité (méthode des projets TwitchDropsMiner,
//! Twitch-Channel-Points-Miner-v2 et twitch-drops-miner, relevée le 2026-11-01).

pub mod auth;
pub mod chat;
pub mod gql;
pub mod pubsub;
pub mod watch;

use std::time::Duration;

/// Identifiant client de l'application Android TV de Twitch
pub const CLIENT_ID: &str = "ue6666qo983tsx6so1t0vnawi233wa";
pub const CLIENT_ORIGIN: &str = "https://android.tv.twitch.tv";
pub const USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 7.1; Smart Box C1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";
/// User-Agent de navigateur pour lire les pages de twitch.tv (adresse de l'événement de visionnage)
pub const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:144.0) Gecko/20100101 Firefox/144.0";

#[derive(Debug, thiserror::Error)]
pub enum TwitchError {
    #[error("réseau : {0}")]
    Http(reqwest::Error),
    #[error("jeton refusé par Twitch")]
    Unauthorized,
    #[error("requête Twitch inconnue ({0}) : empreinte à mettre à jour dans operations.json")]
    PersistedQueryNotFound(String),
    #[error("Twitch a renvoyé une erreur : {0}")]
    Gql(String),
    #[error("réponse inattendue : {0}")]
    Unexpected(String),
}

impl From<reqwest::Error> for TwitchError {
    /// Sans l'adresse : celle du lecteur (usher) porte la signature et le jeton de lecture
    fn from(e: reqwest::Error) -> Self {
        TwitchError::Http(e.without_url())
    }
}

pub type Result<T> = std::result::Result<T, TwitchError>;

/// Client HTTP partagé (pool de connexions) ; chaque compte garde ses propres en-têtes
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .gzip(true)
        .build()
        .expect("client HTTP")
}

/// Identifiant d'appareil aléatoire, propre à chaque compte (32 caractères alphanumériques)
pub fn random_device_id() -> String {
    use rand::distr::{Alphanumeric, SampleString};
    Alphanumeric.sample_string(&mut rand::rng(), 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn http_errors_hide_url() {
        // Port fermé : erreur de connexion immédiate, l'adresse ne doit pas apparaître
        let e = reqwest::Client::new().get("http://127.0.0.1:9/x?token=secret").send().await.unwrap_err();
        let shown = TwitchError::from(e).to_string();
        assert!(!shown.contains("secret"), "{shown}");
    }
}
