use anyhow::Result;
use sqlx::{sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous}, SqlitePool};
use std::{path::Path, time::Duration};

pub async fn connect(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new().max_connections(5).connect_with(options).await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS guild_configs (
        guild_id TEXT PRIMARY KEY NOT NULL, config_json TEXT NOT NULL DEFAULT '{}', updated_at INTEGER NOT NULL
    )").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS moderation_cases (
        case_id TEXT PRIMARY KEY NOT NULL, guild_id TEXT NOT NULL, member_id TEXT NOT NULL,
        moderator_id TEXT, reason TEXT NOT NULL, source TEXT NOT NULL, timeout_until INTEGER,
        status TEXT NOT NULL CHECK(status IN ('active', 'expired', 'reversed', 'failed')),
        created_at INTEGER NOT NULL, original_message_url TEXT
    )").execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_moderation_cases_created ON moderation_cases(created_at DESC)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_moderation_cases_guild ON moderation_cases(guild_id, created_at DESC)")
        .execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS moderation_reversals (
        reversal_id INTEGER PRIMARY KEY AUTOINCREMENT, case_id TEXT NOT NULL UNIQUE REFERENCES moderation_cases(case_id),
        reversed_by TEXT NOT NULL, reversal_reason TEXT NOT NULL, reversed_at INTEGER NOT NULL
    )").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS false_positive_reports (
        report_id INTEGER PRIMARY KEY AUTOINCREMENT, case_id TEXT NOT NULL REFERENCES moderation_cases(case_id),
        reporter_id TEXT NOT NULL, report_reason TEXT NOT NULL, created_at INTEGER NOT NULL,
        status TEXT NOT NULL DEFAULT 'pending'
    )").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS dashboard_sessions (
        token_hash TEXT PRIMARY KEY NOT NULL, discord_user_id TEXT NOT NULL,
        expires_at INTEGER NOT NULL, created_at INTEGER NOT NULL,
        access_token TEXT NOT NULL DEFAULT '', username TEXT NOT NULL DEFAULT '',
        avatar TEXT
    )").execute(pool).await?;
    // Upgrade databases created by the initial dashboard preview.
    let _ = sqlx::query("ALTER TABLE dashboard_sessions ADD COLUMN access_token TEXT NOT NULL DEFAULT ''").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE dashboard_sessions ADD COLUMN username TEXT NOT NULL DEFAULT ''").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE dashboard_sessions ADD COLUMN avatar TEXT").execute(pool).await;
    sqlx::query("CREATE TABLE IF NOT EXISTS dashboard_oauth_states (
        state_hash TEXT PRIMARY KEY NOT NULL, expires_at INTEGER NOT NULL
    )").execute(pool).await?;
    Ok(())
}
