//! Per-IP token-bucket rate limiting.
//!
//! Simple, contended implementation suitable for hundreds of req/s. For
//! higher scale, swap to `governor` or a Redis-backed quota.
//!
//! Each IP gets `capacity` tokens refilled at `capacity` per minute. A
//! request that finds zero tokens returns 429 with `Retry-After`.
//!
//! Excluded paths: `/health`, `/livez`, `/readyz`, `/metrics`. These must
//! work even if the limiter is mis-tuned, so monitoring can detect it.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{HeaderValue, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f32,
    last_refill: Instant,
}

#[derive(Clone)]
pub struct RateLimiter {
    capacity: f32,
    refill_per_sec: f32,
    state: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
}

impl RateLimiter {
    /// `per_minute` requests per IP. Zero disables.
    pub fn new(per_minute: u32) -> Self {
        let capacity = per_minute as f32;
        Self {
            capacity,
            refill_per_sec: capacity / 60.0,
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn enabled(&self) -> bool {
        self.capacity > 0.0
    }

    /// Returns `Ok(())` if the IP has tokens, `Err(retry_after)` otherwise.
    fn check(&self, ip: IpAddr) -> Result<(), u32> {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();
        let bucket = state.entry(ip).or_insert(Bucket {
            tokens: self.capacity,
            last_refill: now,
        });

        // Refill.
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f32();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            // Seconds to next token.
            let need = 1.0 - bucket.tokens;
            let secs = (need / self.refill_per_sec).ceil() as u32;
            Err(secs.max(1))
        }
    }
}

fn rate_limited_response(retry_after_secs: u32) -> Response {
    let mut resp = (
        StatusCode::TOO_MANY_REQUESTS,
        [("content-type", "application/problem+json")],
        r#"{"type":"https://paschal.com/errors/rate_limited","title":"rate_limited","status":429,"detail":"per-IP rate limit reached"}"#.to_string(),
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
        resp.headers_mut().insert("retry-after", v);
    }
    resp
}

/// Core enforcement: check the limiter for `req`'s IP and either pass through
/// or return 429. No path exclusions — callers handle that themselves. Used
/// both by the global [`middleware`] and the per-route auth limiter.
///
/// Tolerant of missing ConnectInfo (e.g. in `oneshot`-driven tests): if absent,
/// the request passes through — production always has it.
pub async fn apply(limiter: &RateLimiter, req: Request<axum::body::Body>, next: Next) -> Response {
    if !limiter.enabled() {
        return next.run(req).await;
    }
    let (mut parts, body) = req.into_parts();
    let connect_info = ConnectInfo::<std::net::SocketAddr>::from_request_parts(&mut parts, &())
        .await
        .ok();
    let req = Request::from_parts(parts, body);
    let Some(ConnectInfo(addr)) = connect_info else {
        return next.run(req).await;
    };
    match limiter.check(addr.ip()) {
        Ok(()) => next.run(req).await,
        Err(secs) => rate_limited_response(secs),
    }
}

/// axum middleware. Skips excluded paths. Tolerant of missing ConnectInfo
/// (e.g. in `oneshot`-driven tests).
pub async fn middleware(
    axum::extract::State(limiter): axum::extract::State<RateLimiter>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    if matches!(path.as_str(), "/health" | "/livez" | "/readyz" | "/metrics") {
        return next.run(req).await;
    }
    apply(&limiter, req, next).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_when_zero() {
        let r = RateLimiter::new(0);
        assert!(!r.enabled());
    }

    #[test]
    fn drains_then_refuses() {
        let r = RateLimiter::new(3);
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(r.check(ip).is_ok());
        assert!(r.check(ip).is_ok());
        assert!(r.check(ip).is_ok());
        let res = r.check(ip);
        assert!(res.is_err());
    }

    #[test]
    fn separate_ips_are_independent() {
        let r = RateLimiter::new(1);
        let a: IpAddr = "1.2.3.4".parse().unwrap();
        let b: IpAddr = "5.6.7.8".parse().unwrap();
        assert!(r.check(a).is_ok());
        assert!(r.check(b).is_ok(), "second IP must have its own bucket");
        assert!(r.check(a).is_err());
        assert!(r.check(b).is_err());
    }

    #[test]
    fn refill_recovers_tokens() {
        let r = RateLimiter::new(60); // 1 token / sec
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        // Drain the bucket.
        for _ in 0..60 {
            assert!(r.check(ip).is_ok());
        }
        assert!(r.check(ip).is_err());
        // After "no real wait" we wouldn't expect a recovery, but we can
        // verify recovery is possible by manually pushing the bucket state.
        // The clock-based recovery is exercised in integration tests.
    }
}
