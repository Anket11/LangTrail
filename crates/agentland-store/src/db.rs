//! Database connection pool management.
//!
//! Creates and manages a `sqlx::PgPool` configured from `agentland-common::Config`.

use agentland_common::config::DatabaseConfig;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

/// Type alias for the shared connection pool.
pub type StorePool = PgPool;

/// Create a new PostgreSQL connection pool from the given `DatabaseConfig`.
///
/// This function should be called once at startup and the pool shared via `Arc` or
/// injected into handler state.
pub async fn connect(cfg: &DatabaseConfig) -> Result<StorePool, sqlx::Error> {
    tracing::info!(
        url = %cfg.url.replace(
            // Redact password in log output
            cfg.url.split('@').next().unwrap_or(""),
            "[redacted]"
        ),
        max_connections = cfg.max_connections,
        "connecting to PostgreSQL"
    );

    let pool = PgPoolOptions::new()
        .max_connections(cfg.max_connections)
        .min_connections(cfg.min_connections)
        .acquire_timeout(Duration::from_secs(cfg.connect_timeout_secs))
        .connect(&cfg.url)
        .await?;

    tracing::info!("PostgreSQL connection pool established");
    Ok(pool)
}

/// Apply bundled SQL migrations.
///
/// Hosted databases start empty, so deployment should not depend on a separate
/// manual `psql` step. A transaction-scoped advisory lock prevents duplicate
/// migration work when multiple services boot against the same database.
pub async fn run_migrations(pool: &StorePool) -> Result<(), sqlx::Error> {
    const MIGRATIONS: &[(&str, &str)] = &[
        ("001_create_events", include_str!("../../../init/001_create_events.sql")),
        ("002_create_agents", include_str!("../../../init/002_create_agents.sql")),
        ("003_create_costs", include_str!("../../../init/003_create_costs.sql")),
        (
            "004_create_hypertables",
            include_str!("../../../init/004_create_hypertables.sql"),
        ),
        ("005_create_indexes", include_str!("../../../init/005_create_indexes.sql")),
        ("006_budget_daily", include_str!("../../../init/006_budget_daily.sql")),
        ("007_budget_config", include_str!("../../../init/007_budget_config.sql")),
        ("008_create_projects", include_str!("../../../init/008_create_projects.sql")),
        (
            "009_create_trajectory_reviews",
            include_str!("../../../init/009_create_trajectory_reviews.sql"),
        ),
    ];

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(728519540001)")
        .execute(&mut *tx)
        .await?;

    for (name, sql) in MIGRATIONS {
        tracing::info!(migration = *name, "applying database migration");
        sqlx::raw_sql(sql).execute(&mut *tx).await?;
    }

    tx.commit().await?;
    tracing::info!("database migrations applied");
    Ok(())
}

/// Run a connectivity check against the database.
pub async fn health_check(pool: &StorePool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}
