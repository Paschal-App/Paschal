//! Beacon library entry — public API used by `main.rs` and integration tests.

pub mod app;
pub mod auth;
pub mod errors;
pub mod metrics;
pub mod notifications;
pub mod passkey;
pub mod rate_limit;
pub mod routes;
pub mod scheduler;
pub mod security_headers;
pub mod spa;
pub mod state;
pub mod tracing_setup;

pub mod dotenv {
    use std::{
        env,
        fs::File,
        io::{BufRead, BufReader},
    };

    pub fn dotenv() -> Result<(), std::io::Error> {
        let file = File::open(".env")?;
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                if env::var_os(k).is_none() {
                    env::set_var(k, v);
                }
            }
        }
        Ok(())
    }
}
