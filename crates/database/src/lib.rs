use parakeet_application::*;
use parakeet_domain::*;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::Path, time::Duration};
mod library;
mod meetings;
mod runs;
mod settings;
mod speakers;

#[derive(Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}
impl SqliteStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}
pub async fn create_empty(path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "database already exists",
        )
        .into());
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    drop(file);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false)
                .pragma("application_id", APPLICATION_ID.to_string()),
        )
        .await?;
    pool.close().await;
    Ok(())
}
pub async fn open_existing(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    check_ownership(path).await?;
    check_schema(path).await?;
    open_writable(path).await
}
/// Refuse a library whose applied migrations differ from the embedded ones; never migrates.
async fn check_schema(path: &Path) -> Result<(), sqlx::Error> {
    let pool = raw_read_only(path).await?;
    let applied: Result<i64, sqlx::Error> = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version),0) FROM _sqlx_migrations WHERE success=1",
    )
    .fetch_one(&pool)
    .await;
    pool.close().await;
    let applied = applied.unwrap_or(0);
    let expected = sqlx::migrate!("./migrations")
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(0);
    if applied != expected {
        return Err(sqlx::Error::Protocol(format!(
            "library schema is version {applied} but this build expects {expected}; run `parakeet db migrate`"
        )));
    }
    Ok(())
}
async fn open_writable(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
}
async fn raw_read_only(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
}
const APPLICATION_ID: i64 = 0x504b5431;

/// Inspect ownership before opening a writable connection or enabling WAL.
/// A zero marker is accepted only for databases created by the exact initial migration.
async fn check_ownership(path: &Path) -> Result<(), sqlx::Error> {
    let pool = raw_read_only(path).await?;
    let marker: i64 = sqlx::query_scalar("PRAGMA application_id")
        .fetch_one(&pool)
        .await?;
    let mut owned = marker == APPLICATION_ID;
    if marker == 0 {
        let has_migrations: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='_sqlx_migrations'",
        )
        .fetch_one(&pool)
        .await?;
        if has_migrations == 1 {
            let checksum: Option<Vec<u8>> = sqlx::query_scalar(
                "SELECT checksum FROM _sqlx_migrations WHERE version=1 AND success=1",
            )
            .fetch_optional(&pool)
            .await?;
            let migrations = sqlx::migrate!("./migrations");
            owned = checksum.as_deref() == migrations.iter().next().map(|m| m.checksum.as_ref());
        }
    }
    pool.close().await;
    if !owned {
        return Err(sqlx::Error::Protocol("refusing foreign database: expected a Parakeet Local database; use meetings import-source-db for MacParakeet".into()));
    }
    Ok(())
}
pub async fn open_read_only(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    check_ownership(path).await?;
    check_schema(path).await?;
    raw_read_only(path).await
}
pub async fn migrate(path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    check_ownership(path).await?;
    let pool = open_writable(path).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    sqlx::query(&format!("PRAGMA application_id={APPLICATION_ID}"))
        .execute(&pool)
        .await?;
    pool.close().await;
    Ok(())
}
pub async fn verify(
    path: &Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    check_ownership(path).await?;
    let pool = raw_read_only(path).await?;
    // Operational introspection exception: PRAGMA/schema information is not application SQL.
    let integrity: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&pool)
        .await?;
    let version: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version),0) FROM _sqlx_migrations WHERE success=1")
            .fetch_one(&pool)
            .await?;
    let sqlite_version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&pool)
        .await?;
    pool.close().await;
    Ok(
        serde_json::json!({"integrity":integrity,"schema_version":version,"sqlite_version":sqlite_version,"read_only":true}),
    )
}
fn encode<T: serde::Serialize>(value: &T) -> Result<String, PersistenceError> {
    serde_json::to_string(value).map_err(PersistenceError::new)
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, PersistenceError> {
    serde_json::from_str(value).map_err(PersistenceError::new)
}

mod source;
pub use source::MacParakeetCatalog;

mod storage;

pub mod cleanup;

mod analysis;
