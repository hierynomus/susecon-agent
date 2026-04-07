mod memory_bloat;
mod sessions;

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    schemars::JsonSchema,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpService,
        session::local::LocalSessionManager,
    },
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use sessions::SessionCatalog;

// ---------------------------------------------------------------------------
// Tool input schema
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct RecommendSessionsRequest {
    /// The topic or interest area to search for, e.g. "AI", "Kubernetes security", "edge observability"
    #[schemars(description = "Topic or interest area to get session recommendations for")]
    pub topic: String,

    /// Maximum number of sessions to return (default: 5)
    #[schemars(description = "Maximum number of session recommendations to return")]
    pub max_results: Option<u32>,
}

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SuseConAgent {
    catalog: Arc<SessionCatalog>,
    memory_bloat_mb: usize,
    memory_bloat_duration_secs: u64,
    tool_router: ToolRouter<SuseConAgent>,
}

#[tool_router]
impl SuseConAgent {
    pub fn new(catalog: SessionCatalog, memory_bloat_mb: usize, memory_bloat_duration_secs: u64) -> Self {
        Self {
            catalog: Arc::new(catalog),
            memory_bloat_mb,
            memory_bloat_duration_secs,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Recommend SUSECon sessions based on a topic or interest area. Returns session titles, speakers, abstracts, and schedule information."
    )]
    async fn recommend_sessions(
        &self,
        Parameters(request): Parameters<RecommendSessionsRequest>,
    ) -> Result<CallToolResult, McpError> {
        let topic = request.topic;
        let max = request.max_results.unwrap_or(5) as usize;

        tracing::info!(
            topic = %topic,
            max_results = max,
            bloat_mb = self.memory_bloat_mb,
            "Tool call: recommend_sessions — beginning context accumulation"
        );

        // Simulate bursty agent memory growth (context accumulation)
        let rss = memory_bloat::bloat(self.memory_bloat_mb, self.memory_bloat_duration_secs).await;

        tracing::info!(
            rss_mb = format!("{:.1}", rss),
            "Context accumulation complete — searching sessions"
        );

        // Search sessions
        let results = self.catalog.recommend(&topic, max);

        if results.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "No sessions found matching '{}'. Try broader keywords like 'AI', 'Kubernetes', 'security', or 'edge'.",
                topic
            ))]));
        }

        let mut output = format!(
            "Found {} SUSECon session(s) matching '{}':\n\n",
            results.len(),
            topic
        );

        for (i, session) in results.iter().enumerate() {
            output.push_str(&format!(
                "{}. **{}**\n   Speaker: {}\n   Track: {} | {} @ {} | Room: {}\n   {}\n   Tags: {}\n\n",
                i + 1,
                session.title,
                session.speaker,
                session.track,
                session.day,
                session.time,
                session.room,
                session.r#abstract,
                session.tags.join(", ")
            ));
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

#[tool_handler]
impl ServerHandler for SuseConAgent {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_03_26;
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .build();
        info.server_info = Implementation::new(
            "susecon-agent",
            env!("CARGO_PKG_VERSION"),
        );
        info.instructions = Some(
            "SUSECon Session Recommender Agent. Use the recommend_sessions tool to find \
             sessions at SUSECon based on your interests. Provide a topic like 'AI', \
             'Kubernetes', 'security', 'edge', or 'observability'."
                .to_string(),
        );
        info
    }
}

// ---------------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".to_string().into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Configuration via environment variables
    let bind_addr = std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let sessions_path =
        std::env::var("SESSIONS_PATH").unwrap_or_else(|_| "/config/sessions.yaml".to_string());
    let memory_bloat_mb: usize = std::env::var("MEMORY_BLOAT_MB")
        .unwrap_or_else(|_| "512".to_string())
        .parse()
        .expect("MEMORY_BLOAT_MB must be a number");
    let memory_bloat_duration_secs: u64 = std::env::var("MEMORY_BLOAT_DURATION_SECS")
        .unwrap_or_else(|_| "12".to_string())
        .parse()
        .expect("MEMORY_BLOAT_DURATION_SECS must be a number");

    tracing::info!(
        bind_addr = %bind_addr,
        sessions_path = %sessions_path,
        memory_bloat_mb,
        memory_bloat_duration_secs,
        "Starting SUSECon Agent MCP Server"
    );

    // Load session catalog
    let catalog = SessionCatalog::load(std::path::Path::new(&sessions_path))?;

    // Create the MCP service
    let bloat_mb = memory_bloat_mb;
    let bloat_secs = memory_bloat_duration_secs;
    let catalog_clone = catalog.clone();

    let service = StreamableHttpService::new(
        move || Ok(SuseConAgent::new(catalog_clone.clone(), bloat_mb, bloat_secs)),
        LocalSessionManager::default().into(),
        Default::default(),
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let tcp_listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    tracing::info!(addr = %bind_addr, "MCP server listening");

    axum::serve(tcp_listener, router)
        .with_graceful_shutdown(async { tokio::signal::ctrl_c().await.unwrap() })
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::RawContent;
    use sessions::SessionCatalog;

    /// Build a minimal agent with zero memory bloat for tests.
    fn test_agent() -> SuseConAgent {
        let catalog = SessionCatalog {
            sessions: vec![
                sessions::Session {
                    id: "T-1".into(),
                    title: "AI on Kubernetes".into(),
                    speaker: "Alice".into(),
                    r#abstract: "Deploy agents on k8s.".into(),
                    track: "AI/ML".into(),
                    day: "Tuesday".into(),
                    time: "09:00".into(),
                    room: "A1".into(),
                    tags: vec!["ai".into(), "kubernetes".into()],
                },
                sessions::Session {
                    id: "T-2".into(),
                    title: "Zero-Trust Security".into(),
                    speaker: "Bob".into(),
                    r#abstract: "Network policies and NeuVector.".into(),
                    track: "Security".into(),
                    day: "Wednesday".into(),
                    time: "10:00".into(),
                    room: "B1".into(),
                    tags: vec!["security".into(), "neuvector".into()],
                },
            ],
        };
        SuseConAgent::new(catalog, 0, 0)
    }

    // -- ServerInfo ---------------------------------------------------------

    #[test]
    fn server_info_protocol_version() {
        let agent = test_agent();
        let info = agent.get_info();
        assert_eq!(info.protocol_version, ProtocolVersion::V_2025_03_26);
    }

    #[test]
    fn server_info_name_and_version() {
        let agent = test_agent();
        let info = agent.get_info();
        assert_eq!(info.server_info.name, "susecon-agent");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn server_info_has_instructions() {
        let agent = test_agent();
        let info = agent.get_info();
        let instructions = info.instructions.expect("instructions should be set");
        assert!(instructions.contains("SUSECon"));
        assert!(instructions.contains("recommend_sessions"));
    }

    #[test]
    fn server_info_enables_tools_capability() {
        let agent = test_agent();
        let info = agent.get_info();
        assert!(
            info.capabilities.tools.is_some(),
            "tools capability should be enabled"
        );
    }

    // -- Tool registration --------------------------------------------------

    #[test]
    fn tool_router_has_recommend_sessions() {
        let agent = test_agent();
        assert!(
            agent.tool_router.has_route("recommend_sessions"),
            "tool router should contain recommend_sessions"
        );
    }

    #[test]
    fn tool_router_lists_exactly_one_tool() {
        let agent = test_agent();
        let tools = agent.tool_router.list_all();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "recommend_sessions");
    }

    #[test]
    fn tool_description_is_present() {
        let agent = test_agent();
        let tool = agent
            .tool_router
            .get("recommend_sessions")
            .expect("tool should exist");
        let desc = tool.description.as_deref().expect("description should be set");
        assert!(desc.to_lowercase().contains("recommend"));
        assert!(desc.to_lowercase().contains("susecon"));
    }

    #[test]
    fn tool_input_schema_has_topic_field() {
        let agent = test_agent();
        let tool = agent.tool_router.get("recommend_sessions").unwrap();
        let schema_json = serde_json::to_string(&tool.input_schema).unwrap();
        assert!(schema_json.contains("topic"), "schema must include 'topic' field");
    }

    #[test]
    fn tool_input_schema_has_max_results_field() {
        let agent = test_agent();
        let tool = agent.tool_router.get("recommend_sessions").unwrap();
        let schema_json = serde_json::to_string(&tool.input_schema).unwrap();
        assert!(
            schema_json.contains("max_results"),
            "schema must include 'max_results' field"
        );
    }

    // -- Tool invocation (async) --------------------------------------------

    fn extract_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| match &c.raw {
                RawContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    #[tokio::test]
    async fn recommend_sessions_returns_matching_results() {
        let agent = test_agent();
        let request = RecommendSessionsRequest {
            topic: "AI".into(),
            max_results: Some(5),
        };
        let result = agent
            .recommend_sessions(Parameters(request))
            .await
            .expect("tool call should succeed");

        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        assert!(text.contains("AI on Kubernetes"));
    }

    #[tokio::test]
    async fn recommend_sessions_no_match() {
        let agent = test_agent();
        let request = RecommendSessionsRequest {
            topic: "nonexistent_topic_xyz".into(),
            max_results: Some(5),
        };
        let result = agent
            .recommend_sessions(Parameters(request))
            .await
            .expect("tool call should succeed");

        let text = extract_text(&result);
        assert!(text.contains("No sessions found"));
    }

    #[tokio::test]
    async fn recommend_sessions_respects_max_results() {
        let agent = test_agent();
        let request = RecommendSessionsRequest {
            topic: "ai kubernetes security".into(),
            max_results: Some(1),
        };
        let result = agent
            .recommend_sessions(Parameters(request))
            .await
            .expect("tool call should succeed");

        let text = extract_text(&result);
        // With max_results=1, should only see one numbered result
        assert!(text.contains("1."));
        assert!(!text.contains("2."));
    }

    #[tokio::test]
    async fn recommend_sessions_default_max_results() {
        let agent = test_agent();
        let request = RecommendSessionsRequest {
            topic: "ai".into(),
            max_results: None,
        };
        let result = agent
            .recommend_sessions(Parameters(request))
            .await
            .expect("tool call should succeed with None max_results");
        assert!(!result.is_error.unwrap_or(false));
    }
}
