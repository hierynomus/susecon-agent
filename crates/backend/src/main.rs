mod memory_bloat;
mod sessions;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use sessions::{Session, SessionCatalog};

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

fn env_parse<T: std::str::FromStr>(key: &str, default: &str) -> T
where
    T::Err: std::fmt::Display,
{
    env_or(key, default)
        .parse()
        .unwrap_or_else(|e| panic!("{key} must be a valid {}: {e}", std::any::type_name::<T>()))
}

#[derive(Clone)]
struct AppState {
    catalog: Arc<SessionCatalog>,
    memory_bloat_mb: usize,
    memory_bloat_duration: Duration,
}

#[derive(serde::Deserialize)]
struct RecommendRequest {
    topic: String,
    max_results: Option<usize>,
}

#[derive(serde::Serialize)]
struct RecommendResponse {
    sessions: Vec<Session>,
}

async fn recommend(
    State(state): State<AppState>,
    Json(request): Json<RecommendRequest>,
) -> impl IntoResponse {
    let max = request.max_results.unwrap_or(5);

    tracing::info!(
        topic = %request.topic,
        max_results = max,
        bloat_mb = state.memory_bloat_mb,
        "recommend — beginning context accumulation"
    );

    let rss = memory_bloat::bloat(state.memory_bloat_mb, state.memory_bloat_duration).await;

    tracing::info!(
        rss_mb = format!("{rss:.1}"),
        "Context accumulation complete — searching sessions"
    );

    let results: Vec<Session> = state
        .catalog
        .recommend(&request.topic, max)
        .into_iter()
        .cloned()
        .collect();

    (StatusCode::OK, Json(RecommendResponse { sessions: results }))
}

async fn health() -> StatusCode {
    StatusCode::OK
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let bind_addr = env_or("BIND_ADDRESS", "0.0.0.0:8081");
    let sessions_path = env_or("SESSIONS_PATH", "/config/sessions.yaml");
    let memory_bloat_mb: usize = env_parse("MEMORY_BLOAT_MB", "512");
    let memory_bloat_duration = Duration::from_secs(env_parse("MEMORY_BLOAT_DURATION_SECS", "12"));

    tracing::info!(
        %bind_addr,
        %sessions_path,
        memory_bloat_mb,
        ?memory_bloat_duration,
        "Starting SUSECon Recommendation Backend"
    );

    let catalog = SessionCatalog::load(Path::new(&sessions_path))?;

    let state = AppState {
        catalog: Arc::new(catalog),
        memory_bloat_mb,
        memory_bloat_duration,
    };

    let router = axum::Router::new()
        .route("/recommend", axum::routing::post(recommend))
        .route("/healthz", axum::routing::get(health))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    tracing::info!(addr = %bind_addr, "Recommendation backend listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(async { tokio::signal::ctrl_c().await.unwrap() })
        .await?;

    Ok(())
}
