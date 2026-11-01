//! Optional hosting of the browser build (I2/I3): when `WEB_DIR` points at a
//! folder with the output of `scripts/build-web.sh`, the relay serves the
//! page itself. One deployed app then gives players a single link, and the
//! page can reach `/ws` on its own address (so `wss://` comes for free
//! behind a host's HTTPS).
//!
//! Only three fixed files are served, never "whatever path was asked for",
//! so there is no way to read other files on the machine.

use std::path::PathBuf;

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

/// (URL path, file name inside the web folder, content type). The browser
/// refuses to compile a `.wasm` that isn't served as `application/wasm`.
const FILES: [(&str, &str, &str); 4] = [
    ("/", "index.html", "text/html; charset=utf-8"),
    ("/index.html", "index.html", "text/html; charset=utf-8"),
    (
        "/mq_js_bundle.js",
        "mq_js_bundle.js",
        "text/javascript; charset=utf-8",
    ),
    (
        "/riposte-game.wasm",
        "riposte-game.wasm",
        "application/wasm",
    ),
];

pub fn routes(dir: PathBuf) -> Router {
    let mut router = Router::new();
    for (url, file, content_type) in FILES {
        let path = dir.join(file);
        router = router.route(url, get(move || serve(path.clone(), content_type)));
    }
    router
}

async fn serve(path: PathBuf, content_type: &'static str) -> Response {
    // Read on every request, so rebuilding the game doesn't need a restart.
    match tokio::fs::read(&path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, content_type)], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
