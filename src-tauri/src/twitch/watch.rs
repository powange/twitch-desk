//! Visionnage simulé d'une chaîne, sans lecteur ni téléchargement de vidéo.
//!
//! Depuis octobre 2026, Twitch ne compte le temps de visionnage que si les
//! segments vidéo sont demandés. Toutes les ~10 s, on relit la liste de lecture
//! en qualité minimale et on envoie un HEAD sur chaque nouveau segment ; toutes
//! les ~59 s, on envoie en plus l'événement « minute-watched ».

use std::collections::{HashSet, VecDeque};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::json;
use tokio::sync::Mutex;

use super::gql::Gql;
use super::{Result, TwitchError, BROWSER_USER_AGENT};

pub const SEGMENT_INTERVAL: Duration = Duration::from_secs(10);
pub const SPADE_INTERVAL: Duration = Duration::from_secs(59);
const SEEN_CAPACITY: usize = 256;
const SPADE_URL_TTL: Duration = Duration::from_secs(6 * 3600);

/// Ce que l'événement de visionnage doit décrire
pub struct WatchContext<'a> {
    pub login: &'a str,
    pub channel_id: &'a str,
    pub broadcast_id: &'a str,
    pub user_id: &'a str,
    pub game_id: Option<&'a str>,
    pub game_name: Option<&'a str>,
}

#[derive(Default)]
pub struct ChannelWatch {
    playlist_url: Option<String>,
    seen: VecDeque<String>,
    seen_set: HashSet<String>,
    last_segments: Option<Instant>,
    last_spade: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct TickReport {
    pub segments: usize,
    pub spade_sent: bool,
}

impl ChannelWatch {
    /// À appeler souvent : ne fait que ce qui est dû
    pub async fn tick(&mut self, gql: &Gql, ctx: &WatchContext<'_>) -> Result<TickReport> {
        let mut report = TickReport::default();
        if self.last_segments.is_none_or(|t| t.elapsed() >= SEGMENT_INTERVAL) {
            self.last_segments = Some(Instant::now());
            report.segments = self.touch_segments(gql, ctx.login).await?;
        }
        if self.last_spade.is_none_or(|t| t.elapsed() >= SPADE_INTERVAL) {
            self.last_spade = Some(Instant::now());
            send_minute_watched(gql.http(), ctx).await?;
            report.spade_sent = true;
        }
        Ok(report)
    }

    async fn touch_segments(&mut self, gql: &Gql, login: &str) -> Result<usize> {
        let http = gql.http();
        if self.playlist_url.is_none() {
            let (token, sig) = gql.playback_token(login).await?;
            let master = url::Url::parse_with_params(
                &format!("https://usher.ttvnw.net/api/channel/hls/{login}.m3u8"),
                &[("sig", sig.as_str()), ("token", token.as_str()), ("allow_source", "true")],
            )
            .map_err(|e| TwitchError::Unexpected(e.to_string()))?;
            let text = fetch_text(http, master.as_str()).await?;
            let base = url::Url::parse(master.as_str()).map_err(|e| TwitchError::Unexpected(e.to_string()))?;
            // Twitch liste les qualités de la meilleure à la plus basse, l'audio seul en dernier
            self.playlist_url = lowest_video_variant(&base, &text)?;
        }
        let Some(playlist) = self.playlist_url.clone() else {
            return Err(TwitchError::Unexpected("liste de lecture vide".into()));
        };
        let segments = match fetch_playlist(http, &playlist).await {
            Ok(s) => s,
            Err(e) => {
                // Jeton de lecture expiré : on repartira de la liste principale
                self.playlist_url = None;
                return Err(e);
            }
        };
        let mut touched = 0;
        for segment in segments {
            if self.seen_set.contains(&segment) {
                continue;
            }
            let res = http.head(&segment).send().await?;
            if !res.status().is_success() {
                self.playlist_url = None;
                return Err(TwitchError::Unexpected(format!("segment : HTTP {}", res.status())));
            }
            self.remember(segment);
            touched += 1;
        }
        Ok(touched)
    }

    fn remember(&mut self, segment: String) {
        if self.seen.len() >= SEEN_CAPACITY {
            if let Some(old) = self.seen.pop_front() {
                self.seen_set.remove(&old);
            }
        }
        self.seen_set.insert(segment.clone());
        self.seen.push_back(segment);
    }
}

async fn fetch_text(http: &reqwest::Client, address: &str) -> Result<String> {
    let res = http.get(address).header("Connection", "close").send().await?;
    if !res.status().is_success() {
        return Err(TwitchError::Unexpected(format!("liste de lecture : HTTP {}", res.status())));
    }
    Ok(res.text().await?)
}

async fn fetch_playlist(http: &reqwest::Client, address: &str) -> Result<Vec<String>> {
    let text = fetch_text(http, address).await?;
    let base = url::Url::parse(address).map_err(|e| TwitchError::Unexpected(e.to_string()))?;
    parse_playlist(&base, &text)
}

/// Variante vidéo la plus basse de la liste principale. L'audio seul ne compte pas comme un
/// visionnage : écarté, sauf s'il n'y a rien d'autre
fn lowest_video_variant(base: &url::Url, text: &str) -> Result<Option<String>> {
    let all = parse_playlist(base, text)?;
    let mut video = vec![];
    let mut info = "";
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with("#EXT-X-STREAM-INF") {
            info = line;
        } else if !line.starts_with('#') {
            let audio_only = info.to_ascii_lowercase().contains("audio_only") || line.contains("audio_only");
            if !audio_only {
                if let Some(u) = base.join(line).ok().filter(|u| matches!(u.scheme(), "http" | "https")) {
                    video.push(String::from(u));
                }
            }
            info = "";
        }
    }
    Ok(video.last().or(all.last()).cloned())
}

/// Entrées d'une liste M3U8 (variantes ou segments), en adresses absolues
fn parse_playlist(base: &url::Url, text: &str) -> Result<Vec<String>> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if lines.next() != Some("#EXTM3U") {
        return Err(TwitchError::Unexpected("liste de lecture illisible".into()));
    }
    let entries: Vec<String> = lines
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| base.join(l).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .map(String::from)
        .collect();
    if entries.is_empty() {
        return Err(TwitchError::Unexpected("liste de lecture vide".into()));
    }
    Ok(entries)
}

// --- Événement minute-watched ---

static SPADE_URL: LazyLock<Mutex<Option<(String, Instant)>>> = LazyLock::new(|| Mutex::new(None));

async fn spade_url(http: &reqwest::Client, login: &str) -> Result<String> {
    let mut cached = SPADE_URL.lock().await;
    if let Some((url, at)) = cached.as_ref() {
        if at.elapsed() < SPADE_URL_TTL {
            return Ok(url.clone());
        }
    }
    let page = http
        .get(format!("https://www.twitch.tv/{login}"))
        .header("User-Agent", BROWSER_USER_AGENT)
        .send()
        .await?
        .text()
        .await?;
    let found = match extract_spade_url(&page) {
        Some(url) => Some(url),
        None => match extract_settings_url(&page) {
            Some(settings) => {
                let js = http
                    .get(settings)
                    .header("User-Agent", BROWSER_USER_AGENT)
                    .send()
                    .await?
                    .text()
                    .await?;
                extract_spade_url(&js)
            }
            None => None,
        },
    };
    let url = found.ok_or_else(|| TwitchError::Unexpected("adresse de l'événement de visionnage introuvable".into()))?;
    *cached = Some((url.clone(), Instant::now()));
    Ok(url)
}

async fn forget_spade_url() {
    *SPADE_URL.lock().await = None;
}

fn extract_settings_url(page: &str) -> Option<String> {
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(https://(?:static\.twitchcdn\.net|assets\.twitch\.tv)/config/settings\.[^"']*?\.js)"#).unwrap()
    });
    RE.captures(page).map(|c| c[1].to_owned())
}

fn extract_spade_url(text: &str) -> Option<String> {
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#""(?:spade_url|beacon_url|beaconUrl)"\s*:\s*"([^"]+)""#).unwrap());
    RE.captures(text).map(|c| c[1].replace("\\/", "/"))
}

fn minute_watched_payload(ctx: &WatchContext<'_>) -> String {
    let mut properties = json!({
        "channel_id": ctx.channel_id,
        "broadcast_id": ctx.broadcast_id,
        "player": "site",
        "user_id": ctx.user_id,
        "live": true,
        "channel": ctx.login,
    });
    if let (Some(id), Some(name)) = (ctx.game_id, ctx.game_name) {
        properties["game_id"] = json!(id);
        properties["game"] = json!(name);
    }
    let events = json!([{ "event": "minute-watched", "properties": properties }]);
    base64::engine::general_purpose::STANDARD.encode(events.to_string())
}

async fn send_minute_watched(http: &reqwest::Client, ctx: &WatchContext<'_>) -> Result<()> {
    let url = spade_url(http, ctx.login).await?;
    let res = http
        .post(&url)
        .form(&[("data", minute_watched_payload(ctx))])
        .send()
        .await?;
    if res.status() == reqwest::StatusCode::NO_CONTENT || res.status().is_success() {
        Ok(())
    } else {
        forget_spade_url().await;
        Err(TwitchError::Unexpected(format!("événement de visionnage : HTTP {}", res.status())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_playlist_keeps_order_and_resolves() {
        let base = url::Url::parse("https://usher.ttvnw.net/api/channel/hls/x.m3u8?sig=a").unwrap();
        let text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=6000000\nhttps://cdn.example/1080.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=230000\nhttps://cdn.example/160.m3u8\n";
        let v = parse_playlist(&base, text).unwrap();
        assert_eq!(v.last().unwrap(), "https://cdn.example/160.m3u8");
    }

    #[test]
    fn audio_only_variant_skipped() {
        let base = url::Url::parse("https://usher.ttvnw.net/api/channel/hls/x.m3u8?sig=a").unwrap();
        let text = "#EXTM3U\n#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,VIDEO=\"chunked\"\nhttps://cdn.example/1080.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=230000,RESOLUTION=284x160,VIDEO=\"160p30\"\nhttps://cdn.example/160.m3u8\n\
            #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"audio_only\",NAME=\"audio_only\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\",VIDEO=\"audio_only\"\nhttps://cdn.example/audio.m3u8\n";
        assert_eq!(lowest_video_variant(&base, text).unwrap().as_deref(), Some("https://cdn.example/160.m3u8"));
        // Rien d'autre que l'audio : on le garde
        let only = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=160000,VIDEO=\"audio_only\"\nhttps://cdn.example/audio.m3u8\n";
        assert_eq!(lowest_video_variant(&base, only).unwrap().as_deref(), Some("https://cdn.example/audio.m3u8"));
    }

    #[test]
    fn media_playlist_relative_segments() {
        let base = url::Url::parse("https://video.example/v1/playlist/abc.m3u8").unwrap();
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.000,live\nseg1.ts\n#EXTINF:2.000,live\nseg2.ts\n";
        let v = parse_playlist(&base, text).unwrap();
        assert_eq!(v, vec!["https://video.example/v1/playlist/seg1.ts", "https://video.example/v1/playlist/seg2.ts"]);
    }

    #[test]
    fn invalid_playlists() {
        let base = url::Url::parse("https://x.example/").unwrap();
        assert!(parse_playlist(&base, "<html>").is_err());
        assert!(parse_playlist(&base, "#EXTM3U\n#EXT-X-ENDLIST\n").is_err());
    }

    #[test]
    fn spade_and_settings_urls() {
        let page = r#"<script src="https://static.twitchcdn.net/config/settings.0123abcd.js"></script>"#;
        assert_eq!(
            extract_settings_url(page).as_deref(),
            Some("https://static.twitchcdn.net/config/settings.0123abcd.js")
        );
        let js = r#"window.__twilightSettings = {"spade_url":"https:\/\/video-edge-abc.twitch.tv\/v1\/segment","x":1}"#;
        assert_eq!(extract_spade_url(js).as_deref(), Some("https://video-edge-abc.twitch.tv/v1/segment"));
        assert_eq!(extract_spade_url(r#""beacon_url":"https://b.example/track""#).as_deref(), Some("https://b.example/track"));
    }

    #[test]
    fn payload_is_base64_json() {
        let ctx = WatchContext {
            login: "lunarsea",
            channel_id: "11",
            broadcast_id: "22",
            user_id: "33",
            game_id: Some("490377"),
            game_name: Some("Sea of Thieves"),
        };
        let raw = base64::engine::general_purpose::STANDARD.decode(minute_watched_payload(&ctx)).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v[0]["event"], "minute-watched");
        assert_eq!(v[0]["properties"]["channel"], "lunarsea");
        assert_eq!(v[0]["properties"]["game"], "Sea of Thieves");
    }
}
