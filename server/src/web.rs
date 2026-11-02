//! Fichiers de la web app mobile, servis sous `/app` (dossier `SYNC_WEB_DIR`).
//!
//! Les fichiers nommés par leur empreinte (`/app/assets/…`) sont gardés en cache un an ; les
//! autres (index.html, manifeste, service worker) sont revérifiés à chaque visite. Une adresse
//! inconnue sans extension renvoie index.html (navigation dans la web app) ; un fichier absent
//! renvoie 404.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use axum::extract::Request;
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use tower::ServiceExt as _;
use tower_http::services::{ServeDir, ServeFile};

/// Dossier par défaut dans l'image Docker
pub const DEFAULT_WEB_DIR: &str = "/usr/share/twitch-desk-sync/web";

/// En-têtes de cache et de sécurité des fichiers de la web app
async fn headers(request: Request, next: Next) -> Response {
    let immutable = request.uri().path().starts_with("/app/assets/");
    let mut response = next.run(request).await;
    let cached = response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED;
    let h = response.headers_mut();
    if cached {
        let cache = if immutable { "public, max-age=31536000, immutable" } else { "no-cache" };
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    }
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

/// Routes `/app` (à fusionner avec celles de l'API, ou seules sur le port de la web app)
pub fn router<S: Clone + Send + Sync + 'static>(dir: &Path) -> Router<S> {
    let index = dir.join("index.html");
    // Navigation dans la web app : index.html ; fichier absent (ancienne version) : 404
    let fallback = tower::service_fn(move |request: Request| {
        let index: PathBuf = index.clone();
        async move {
            let path = request.uri().path();
            let last = path.rsplit('/').next().unwrap_or_default();
            if path.starts_with("/assets/") || last.contains('.') {
                return Ok::<_, Infallible>(StatusCode::NOT_FOUND.into_response());
            }
            Ok(ServeFile::new(index).oneshot(request).await.map(IntoResponse::into_response).unwrap_or_else(|e| match e {}))
        }
    });
    let files = ServeDir::new(dir).append_index_html_on_directories(true).fallback(fallback);
    Router::new().nest_service("/app", files).layer(middleware::from_fn(headers))
}

/// Port propre à la web app (`SYNC_WEB_PORT`) : `/app` seulement, la racine y mène
pub fn standalone<S: Clone + Send + Sync + 'static>(dir: &Path) -> Router<S> {
    router(dir).route("/", get(|| async { Redirect::temporary("/app/") }))
}
