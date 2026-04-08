mod agent;

use std::time::Duration;

use rmcp::transport::streamable_http_server::{
    StreamableHttpService,
    session::local::LocalSessionManager,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use agent::SuseConAgent;

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let bind_addr = env_or("BIND_ADDRESS", "0.0.0.0:8080");
    let backend_url = env_or("BACKEND_URL", "http://localhost:8081");
    let request_timeout: u64 = env_parse("BACKEND_TIMEOUT_SECS", "30");

    tracing::info!(
        %bind_addr,
        %backend_url,
        request_timeout,
        "Starting SUSECon Agent MCP Server"
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(request_timeout))
        .build()?;

    let service = StreamableHttpService::new(
        move || Ok(SuseConAgent::new(backend_url.clone(), client.clone())),
        LocalSessionManager::default().into(),
        Default::default(),
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    tracing::info!(addr = %bind_addr, "MCP server listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(async { tokio::signal::ctrl_c().await.unwrap() })
        .await?;

    Ok(())
}
