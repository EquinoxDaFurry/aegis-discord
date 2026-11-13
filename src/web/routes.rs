use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;
use std::net::SocketAddr;
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState { db: SqlitePool }

#[derive(Serialize)]
struct HealthResponse { status: &'static str, database: &'static str }

async fn health(State(state): State<AppState>) -> Result<Json<HealthResponse>, StatusCode> {
    sqlx::query("SELECT 1").execute(&state.db).await.map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(HealthResponse { status: "ok", database: "ok" }))
}

pub fn router(db: SqlitePool) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/health", get(health))
        .nest_service("/", ServeDir::new("dashboard"))
        .with_state(AppState { db })
}

pub async fn serve(bind: &str, db: SqlitePool) -> anyhow::Result<()> {
    let address: SocketAddr = bind.parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "Aegis dashboard server listening");
    axum::serve(listener, router(db)).await?;
    Ok(())
}
