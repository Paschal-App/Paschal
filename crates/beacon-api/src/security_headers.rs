//! Security headers applied to every response.
//!
//! These are defence-in-depth; they don't replace authentication, TLS, or
//! input validation. They make a successful XSS / clickjacking / sniffing
//! attack less likely to compound.
//!
//! Trade-offs noted inline.

use axum::{
    http::{header, HeaderValue, Request},
    middleware::Next,
    response::Response,
};

/// Apply the standard header set on every response.
pub async fn middleware(req: Request<axum::body::Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();

    // Browsers should never sniff the Content-Type. Without this, a text
    // file served as text/plain could be interpreted as JS by IE/Edge.
    insert(headers, "x-content-type-options", "nosniff");

    // The Beacon is API + a static page; nobody should ever frame it.
    insert(headers, "x-frame-options", "DENY");

    // Don't leak the full URL (including session tokens in queries) when
    // a recipient navigates away from the claim page.
    insert(headers, "referrer-policy", "no-referrer");

    // Strict HSTS — production runs behind TLS only. Dev runs on http://
    // so we only emit this when the request looked encrypted; the value
    // is harmless if the proxy strips it.
    insert(
        headers,
        "strict-transport-security",
        "max-age=31536000; includeSubDomains",
    );

    // CSP differs by surface. Three profiles, picked by URL prefix:
    //   * Browser-rendered pages (/claim, /docs, /app/*, /) — need to
    //     load fonts from Google, sometimes Swagger UI from jsdelivr,
    //     and SvelteKit's inline bootstrap snippets.
    //   * Everything else — JSON / YAML / Prometheus responses that
    //     browsers parse but don't render. Strictest policy possible.
    let is_browser_page = path == "/"
        || path == "/claim"
        || path == "/docs"
        || path == "/app"
        || path.starts_with("/app/");

    // Block unused browser APIs (geolocation, payments, sensors). The SPA
    // records audio/video letters via MediaRecorder, so mic and camera are
    // self-allowed on browser pages only; API responses stay fully locked.
    let permissions = if is_browser_page {
        "accelerometer=(), camera=(self), geolocation=(), gyroscope=(), magnetometer=(), microphone=(self), payment=(), usb=()"
    } else {
        "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()"
    };
    insert(headers, "permissions-policy", permissions);

    let csp = if is_browser_page {
        "default-src 'self'; \
         script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
         style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net https://fonts.googleapis.com; \
         font-src 'self' https://fonts.gstatic.com data:; \
         img-src 'self' data: https://cdn.jsdelivr.net; \
         connect-src 'self'; \
         frame-ancestors 'none'; \
         base-uri 'none'"
    } else {
        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'"
    };
    insert(headers, "content-security-policy", csp);

    // Cache control — API responses should never be cached. Browser pages
    // get the immutable header from the SPA serve handler where applicable;
    // otherwise leave their cache headers alone (the browser default is fine
    // for the SPA shell which fingerprints all assets).
    if !is_browser_page {
        insert(headers, "cache-control", "no-store");
    }

    // Reflect the operator brand so a recipient can identify what served
    // their content even when the network is being weird.
    insert(headers, "x-paschal-version", env!("CARGO_PKG_VERSION"));

    resp
}

fn insert(headers: &mut axum::http::HeaderMap, name: &'static str, value: &str) {
    let header_name = header::HeaderName::from_static(name);
    if let Ok(v) = HeaderValue::from_str(value) {
        // Don't overwrite if a handler already set a more specific value.
        headers.entry(header_name).or_insert(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};

    async fn dummy() -> &'static str {
        "ok"
    }

    #[tokio::test]
    async fn applies_headers_on_get() {
        let app: Router = Router::new()
            .route("/x", get(dummy))
            .layer(axum::middleware::from_fn(middleware));

        let req = Request::builder()
            .uri("/x")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();

        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
        assert!(resp.headers().get("content-security-policy").is_some());
        assert!(resp
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("no-store"));
    }

    #[tokio::test]
    async fn claim_page_csp_allows_inline() {
        let app: Router = Router::new()
            .route("/claim", get(dummy))
            .layer(axum::middleware::from_fn(middleware));

        let req = Request::builder()
            .uri("/claim")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();

        let csp = resp
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("script-src 'self' 'unsafe-inline'"));
    }

    #[tokio::test]
    async fn spa_csp_allows_google_fonts() {
        let app: Router = Router::new()
            .route("/app", get(dummy))
            .route("/app/dashboard", get(dummy))
            .layer(axum::middleware::from_fn(middleware));

        for path in ["/app", "/app/dashboard"] {
            let req = Request::builder()
                .uri(path)
                .body(axum::body::Body::empty())
                .unwrap();
            let resp = tower::ServiceExt::oneshot(app.clone(), req).await.unwrap();
            let csp = resp
                .headers()
                .get("content-security-policy")
                .unwrap()
                .to_str()
                .unwrap();
            assert!(
                csp.contains("https://fonts.googleapis.com"),
                "fonts CSS host missing from CSP for {path}: {csp}"
            );
            assert!(
                csp.contains("https://fonts.gstatic.com"),
                "fonts file host missing from CSP for {path}: {csp}"
            );
            assert!(csp.contains("default-src 'self'"));
            assert!(
                !csp.contains("default-src 'none'"),
                "SPA must not have the API CSP at {path}: {csp}"
            );
        }
    }

    #[tokio::test]
    async fn spa_permissions_policy_allows_self_mic_and_camera() {
        let app: Router = Router::new()
            .route("/app/letters/new", get(dummy))
            .layer(axum::middleware::from_fn(middleware));

        let req = Request::builder()
            .uri("/app/letters/new")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
        let pp = resp
            .headers()
            .get("permissions-policy")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(pp.contains("microphone=(self)"), "got: {pp}");
        assert!(pp.contains("camera=(self)"), "got: {pp}");
        assert!(pp.contains("geolocation=()"));
    }

    #[tokio::test]
    async fn api_permissions_policy_blocks_mic_and_camera() {
        let app: Router = Router::new()
            .route("/v1/vaults", get(dummy))
            .layer(axum::middleware::from_fn(middleware));

        let req = Request::builder()
            .uri("/v1/vaults")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
        let pp = resp
            .headers()
            .get("permissions-policy")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(pp.contains("microphone=()"), "got: {pp}");
        assert!(pp.contains("camera=()"), "got: {pp}");
    }

    #[tokio::test]
    async fn api_csp_stays_strict() {
        let app: Router = Router::new()
            .route("/v1/vaults", get(dummy))
            .layer(axum::middleware::from_fn(middleware));

        let req = Request::builder()
            .uri("/v1/vaults")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
        let csp = resp
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("default-src 'none'"));
        let cc = resp
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cc.contains("no-store"));
    }
}
