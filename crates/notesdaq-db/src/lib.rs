pub mod models;
pub mod queries;

use anyhow::Result;
use sqlx::PgPool;

/// Run all migrations in the migrations/ directory.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    // We use raw SQL migration files rather than sqlx's built-in migrator
    // because we need CREATE EXTENSION and create_hypertable which are
    // PostgreSQL/TimescaleDB-specific.
    let migration = include_str!("../migrations/001_initial_schema.sql");
    sqlx::raw_sql(migration).execute(pool).await?;
    Ok(())
}

/// Connect to the database and return a connection pool.
pub async fn connect(database_url: &str) -> Result<PgPool> {
    let pool = PgPool::connect(database_url).await?;
    Ok(pool)
}
