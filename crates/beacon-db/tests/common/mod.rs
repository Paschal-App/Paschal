//! Test helpers for DB integration tests.
//!
//! Connects to `TEST_DATABASE_URL` (defaulting to a local Postgres) and
//! truncates all tables before each test. Tests are NOT parallel-safe — run
//! with `--test-threads=1` if you see flakes.

use sqlx::PgPool;

pub fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://paschal:paschal@localhost:5432/paschal_test".into())
}

pub async fn setup() -> PgPool {
    let url = test_database_url();
    let pool = beacon_db::connect(&url)
        .await
        .expect("connect to TEST_DATABASE_URL — run scripts/setup.ps1 first");
    truncate_all(&pool).await;
    pool
}

pub async fn truncate_all(pool: &PgPool) {
    // Order matters because of FK cascades; using TRUNCATE ... CASCADE in
    // one statement keeps it simple.
    sqlx::query(
        "TRUNCATE TABLE
            release_claim,
            release_event,
            signal,
            signal_subscription,
            apple_shortcut_subscription,
            heartbeat,
            letter,
            vault,
            transparency_entry,
            magic_link,
            session,
            auth_identity,
            buddy,
            subscription,
            principal
         RESTART IDENTITY CASCADE",
    )
    .execute(pool)
    .await
    .expect("truncate all");
}
