//! Tracing initialization.
//!
//! Honors `BEACON_LOG_FORMAT`:
//!   * `pretty` (default) — coloured human-readable for `cargo run`
//!   * `json`             — line-delimited JSON for log aggregators

use tracing_subscriber::EnvFilter;

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let format = std::env::var("BEACON_LOG_FORMAT").unwrap_or_else(|_| "pretty".into());

    if format == "json" {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .with_current_span(true)
            .with_target(true)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}
