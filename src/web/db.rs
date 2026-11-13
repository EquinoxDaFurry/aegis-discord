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
        case_id TEXT PRIMARY KEY NOT NULL, guild_id TEXT NOT NULL, guild_name TEXT NOT NULL DEFAULT '', member_id TEXT NOT NULL,
        moderator_id TEXT, reason TEXT NOT NULL, source TEXT NOT NULL, timeout_until INTEGER,
        status TEXT NOT NULL CHECK(status IN ('active', 'expired', 'reversed', 'failed')),
        created_at INTEGER NOT NULL, original_message_url TEXT
    )").execute(pool).await?;
    let _ = sqlx::query("ALTER TABLE moderation_cases ADD COLUMN guild_name TEXT NOT NULL DEFAULT ''").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE moderation_cases ADD COLUMN flagged_content TEXT NOT NULL DEFAULT ''").execute(pool).await;
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
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_false_positive_reports_case ON false_positive_reports(case_id)")
        .execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS dashboard_sessions (
        token_hash TEXT PRIMARY KEY NOT NULL, discord_user_id TEXT NOT NULL,
        expires_at INTEGER NOT NULL, created_at INTEGER NOT NULL,
        access_token TEXT NOT NULL DEFAULT '', username TEXT NOT NULL DEFAULT '',
        avatar TEXT, guilds_json TEXT NOT NULL DEFAULT '[]'
    )").execute(pool).await?;
    // Upgrade databases created by the initial dashboard preview.
    let _ = sqlx::query("ALTER TABLE dashboard_sessions ADD COLUMN access_token TEXT NOT NULL DEFAULT ''").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE dashboard_sessions ADD COLUMN username TEXT NOT NULL DEFAULT ''").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE dashboard_sessions ADD COLUMN avatar TEXT").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE dashboard_sessions ADD COLUMN guilds_json TEXT NOT NULL DEFAULT '[]'").execute(pool).await;
    sqlx::query("CREATE TABLE IF NOT EXISTS dashboard_oauth_states (
        state_hash TEXT PRIMARY KEY NOT NULL, expires_at INTEGER NOT NULL
    )").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS analytics_hourly (
        guild_id TEXT NOT NULL, hour_start INTEGER NOT NULL,
        messages INTEGER NOT NULL DEFAULT 0, scam_detections INTEGER NOT NULL DEFAULT 0,
        confidence_sum REAL NOT NULL DEFAULT 0, confidence_count INTEGER NOT NULL DEFAULT 0,
        deleted_messages INTEGER NOT NULL DEFAULT 0, timeouts INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY(guild_id, hour_start)
    )").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS cross_server_actions (
        source_guild_id TEXT NOT NULL, target_guild_id TEXT NOT NULL, member_id TEXT NOT NULL,
        source_message_id TEXT NOT NULL, created_at INTEGER NOT NULL,
        PRIMARY KEY(source_guild_id, target_guild_id, member_id, source_message_id)
    )").execute(pool).await?;
    // Link each network action to its owner-console moderation case.
    let _ = sqlx::query("ALTER TABLE cross_server_actions ADD COLUMN case_id TEXT").execute(pool).await;
    // Backfill links for network actions created before the owner-console integration.
    sqlx::query("UPDATE cross_server_actions SET case_id = (
        SELECT c.case_id FROM moderation_cases c
        WHERE c.guild_id = cross_server_actions.target_guild_id
          AND c.member_id = cross_server_actions.member_id
          AND c.source = 'cross_server_intelligence'
          AND c.original_message_url LIKE '%' || '/' || cross_server_actions.source_message_id
        ORDER BY c.created_at DESC LIMIT 1
    ) WHERE case_id IS NULL").execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_cross_server_actions_case ON cross_server_actions(case_id)")
        .execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_analytics_hourly_guild_hour ON analytics_hourly(guild_id, hour_start)")
        .execute(pool).await?;
    Ok(())
}
