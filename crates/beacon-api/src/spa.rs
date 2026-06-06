//! Serve the SvelteKit principal app from inside the Beacon binary.
//!
//! At compile time the entire `apps/principal/build` directory is embedded
//! via `include_dir!`. At request time we look the path up; if missing
//! (because of client-side routing), we fall back to `index.html` — the SPA
//! pattern.
//!
//! ## Caching
//!
//! `_app/immutable/*` paths are hashed by Vite — we serve them with
//! `cache-control: public, max-age=31536000, immutable`. Everything else
//! gets `no-store` (matching the security-headers default).
//!
//! ## Build-time requirement
//!
//! `apps/principal/build/` must exist when this crate is compiled. The
//! `scripts/setup.ps1` and the Dockerfile's Node stage build it first.

use axum::{
    body::Body,
    extract::Path,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use include_dir::{include_dir, Dir};

static SPA: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../apps/principal/build");

pub async fn serve(uri: Uri) -> Response {
    // Strip the leading `/app/` (or `/app`) prefix so we can look up files
    // inside the embedded tree.
    let path = uri.path();
    let rel = path
        .strip_prefix("/app/")
        .or_else(|| path.strip_prefix("/app").map(|_| ""))
        .unwrap_or("");

    serve_path(rel)
}

pub async fn serve_with_path(Path(rest): Path<String>) -> Response {
    serve_path(&rest)
}

fn serve_path(rel: &str) -> Response {
    // Strip leading slash defensively.
    let rel = rel.trim_start_matches('/');

    let response = if rel.is_empty() {
        spa_fallback()
    } else if let Some(file) = SPA.get_file(rel) {
        file_response(rel, file.contents())
    } else {
        // SPA fallback — every unknown route is client-routed.
        spa_fallback()
    };
    response
}

fn spa_fallback() -> Response {
    match SPA.get_file("index.html") {
        Some(file) => file_response("index.html", file.contents()),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Frontend build missing. Run `pnpm --filter @paschal/principal build`.",
        )
            .into_response(),
    }
}

fn file_response(path: &str, bytes: &[u8]) -> Response {
    let ct = guess_mime(path);
    let mut builder = Response::builder().status(StatusCode::OK);
    if let Ok(v) = HeaderValue::from_str(ct) {
        builder = builder.header(header::CONTENT_TYPE, v);
    }
    if path.starts_with("_app/immutable/") {
        builder = builder.header(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    builder.body(Body::from(bytes.to_vec())).unwrap_or_else(|_| {
        (StatusCode::INTERNAL_SERVER_ERROR, "spa response error").into_response()
    })
}

fn guess_mime(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        "map" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}
