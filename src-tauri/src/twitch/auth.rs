//! Connexion par code d'appareil (twitch.tv/activate), validation et renouvellement du jeton.

use serde::{Deserialize, Serialize};

use super::{Result, TwitchError, CLIENT_ID, CLIENT_ORIGIN};

const DEVICE_URL: &str = "https://id.twitch.tv/oauth2/device";
const TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
const VALIDATE_URL: &str = "https://id.twitch.tv/oauth2/validate";
// Portées acceptées par le client Android TV (celles de Twitch-Channel-Points-Miner-v2)
const SCOPES: &str = "channel_read chat:read user_blocks_edit user_blocks_read user_follows_edit user_read";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// Jamais les valeurs dans les journaux
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"<masqué>")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<masqué>"))
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Validation {
    pub login: String,
    pub user_id: String,
    pub client_id: String,
}

pub enum PollResult {
    Pending,
    SlowDown,
    Done(Tokens),
    Expired,
    Denied,
}

fn with_origin(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    req.header("Origin", CLIENT_ORIGIN).header("Referer", format!("{CLIENT_ORIGIN}/"))
}

pub async fn start_device_flow(http: &reqwest::Client) -> Result<DeviceCode> {
    let res = with_origin(http.post(DEVICE_URL))
        .form(&[("client_id", CLIENT_ID), ("scopes", SCOPES)])
        .send()
        .await?;
    if !res.status().is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(TwitchError::Unexpected(format!("code d'appareil refusé : {body}")));
    }
    Ok(res.json().await?)
}

pub async fn poll_device_flow(http: &reqwest::Client, device_code: &str) -> Result<PollResult> {
    let res = with_origin(http.post(TOKEN_URL))
        .form(&[
            ("client_id", CLIENT_ID),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await?;
    if res.status().is_success() {
        return Ok(PollResult::Done(res.json().await?));
    }
    // Twitch en difficulté ou trop sollicité : on continue d'attendre
    if res.status().is_server_error() || res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Ok(PollResult::Pending);
    }
    let body = res.text().await.unwrap_or_default();
    Ok(classify_poll_error(&body))
}

fn classify_poll_error(body: &str) -> PollResult {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_owned))
        .unwrap_or_default();
    match message.as_str() {
        "authorization_pending" => PollResult::Pending,
        "slow_down" => PollResult::SlowDown,
        "access_denied" => PollResult::Denied,
        // « invalid device code » : le code a expiré
        "invalid device code" | "expired_token" => PollResult::Expired,
        // Réponse inconnue : on continue jusqu'à l'expiration du code (délai connu)
        _ => PollResult::Pending,
    }
}

pub async fn validate(http: &reqwest::Client, access_token: &str) -> Result<Validation> {
    let res = http
        .get(VALIDATE_URL)
        .header("Authorization", format!("OAuth {access_token}"))
        .send()
        .await?;
    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(TwitchError::Unauthorized);
    }
    if !res.status().is_success() {
        return Err(TwitchError::Unexpected(format!("validation : HTTP {}", res.status())));
    }
    let v: Validation = res.json().await?;
    if v.client_id != CLIENT_ID {
        // Jeton d'une autre application : inutilisable pour les requêtes internes
        return Err(TwitchError::Unauthorized);
    }
    Ok(v)
}

pub async fn refresh(http: &reqwest::Client, refresh_token: &str) -> Result<Tokens> {
    let res = with_origin(http.post(TOKEN_URL))
        .form(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await?;
    let status = res.status();
    // 400 / 401 : jeton de renouvellement invalide ou révoqué. Le reste (5xx, 429…) est passager
    if status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(TwitchError::Unauthorized);
    }
    if !status.is_success() {
        return Err(TwitchError::Unexpected(format!("renouvellement du jeton : HTTP {status}")));
    }
    Ok(res.json().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_errors() {
        assert!(matches!(
            classify_poll_error(r#"{"status":400,"message":"authorization_pending"}"#),
            PollResult::Pending
        ));
        assert!(matches!(
            classify_poll_error(r#"{"status":400,"message":"invalid device code"}"#),
            PollResult::Expired
        ));
        assert!(matches!(classify_poll_error(r#"{"message":"slow_down"}"#), PollResult::SlowDown));
        // Réponse inconnue : on continue d'attendre
        assert!(matches!(classify_poll_error("pas du json"), PollResult::Pending));
    }

    #[test]
    fn tokens_debug_is_redacted() {
        let t = Tokens { access_token: "abc123".into(), refresh_token: Some("def456".into()) };
        let shown = format!("{t:?}");
        assert!(!shown.contains("abc123") && !shown.contains("def456"), "{shown}");
    }
}
