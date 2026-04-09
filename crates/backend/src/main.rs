mod session_loader;
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
    catalog_index_mb: usize,
    catalog_index_duration: Duration,
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
        index_mb = state.catalog_index_mb,
        "recommend — loading session catalog"
    );

    let rss = session_loader::load_and_index(state.catalog_index_mb, state.catalog_index_duration).await;

    tracing::info!(
        rss_mb = format!("{rss:.1}"),
        "Session catalog indexed — searching sessions"
    );

    let results: Vec<Session> = state
        .catalog
        .recommend(&request.topic, max)
        .into_iter()
        .cloned()
        .collect();

    (StatusCode::OK, Json(RecommendResponse { sessions: results }))
}

#[derive(serde::Deserialize)]
struct SessionDetailPath {
    id: String,
}

async fn session_detail(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<SessionDetailPath>,
) -> impl IntoResponse {
    tracing::info!(session_id = %path.id, "session_detail — looking up session");

    match state.catalog.find_by_id(&path.id) {
        Some(session) => (StatusCode::OK, Json(serde_json::json!({ "session": session }))).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": format!("Session '{}' not found", path.id) }))).into_response(),
    }
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
    let catalog_index_mb: usize = env_parse("CATALOG_INDEX_MB", "512");
    let catalog_index_duration = Duration::from_secs(env_parse("CATALOG_INDEX_DURATION_SECS", "12"));

    tracing::info!(
        %bind_addr,
        %sessions_path,
        catalog_index_mb,
        ?catalog_index_duration,
        "Starting SUSECon Recommendation Backend"
    );

    let catalog = SessionCatalog::load(Path::new(&sessions_path))?;

    let state = AppState {
        catalog: Arc::new(catalog),
        catalog_index_mb,
        catalog_index_duration,
    };

    let router = axum::Router::new()
        .route("/recommend", axum::routing::post(recommend))
        .route("/session/{id}", axum::routing::get(session_detail))
        .route("/healthz", axum::routing::get(health))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    tracing::info!(addr = %bind_addr, "Recommendation backend listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(async { tokio::signal::ctrl_c().await.unwrap() })
        .await?;

    Ok(())
}
