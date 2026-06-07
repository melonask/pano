use anyhow::{Context, Result};
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;

/// Open a SQLite connection pool with WAL and synchronous=NORMAL pragmas.
pub async fn open_sqlite_pool(path: &str, max_connections: u32) -> Result<SqlitePool> {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true),
        )
        .await
        .with_context(|| format!("failed to open sqlite database at {path}"))?;

    sqlx::query("PRAGMA journal_mode = WAL;")
        .execute(&pool)
        .await
        .context("failed to enable sqlite WAL mode")?;
    sqlx::query("PRAGMA synchronous = NORMAL;")
        .execute(&pool)
        .await
        .context("failed to set sqlite synchronous=NORMAL")?;

    Ok(pool)
}

/// Connect to a PostgreSQL database, returning a connection pool.
pub async fn connect_pg(url: &str) -> Result<sqlx::PgPool> {
    sqlx::PgPool::connect(url)
        .await
        .with_context(|| format!("failed to connect to postgres at {url}"))
}
