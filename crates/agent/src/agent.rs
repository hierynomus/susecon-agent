use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    schemars::JsonSchema,
    tool, tool_handler, tool_router,
};

const DEFAULT_MAX_RESULTS: usize = 5;

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct RecommendSessionsRequest {
    /// The topic or interest area to search for, e.g. "AI", "Kubernetes security", "edge observability"
    #[schemars(description = "Topic or interest area to get session recommendations for")]
    pub topic: String,

    /// Maximum number of sessions to return (default: 5)
    #[schemars(description = "Maximum number of session recommendations to return")]
    pub max_results: Option<usize>,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct SessionDetailsRequest {
    /// The session ID to look up, e.g. "TUTORIAL-1137", "CUSTOMER-1232"
    #[schemars(description = "Session ID to retrieve details for (e.g. TUTORIAL-1137)")]
    pub session_id: String,
}

#[derive(Debug, serde::Deserialize)]
struct BackendSession {
    id: String,
    title: String,
    speaker: String,
    r#abstract: String,
    track: String,
    day: String,
    time: String,
    room: String,
    tags: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct BackendResponse {
    sessions: Vec<BackendSession>,
}

#[derive(Debug, serde::Deserialize)]
struct BackendSessionDetailResponse {
    session: BackendSession,
}

#[derive(Clone)]
pub struct SuseConAgent {
    backend_url: String,
    client: reqwest::Client,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl SuseConAgent {
    pub fn new(backend_url: String, client: reqwest::Client) -> Self {
        Self {
            backend_url,
            client,
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
        let max = request.max_results.unwrap_or(DEFAULT_MAX_RESULTS);

        tracing::info!(
            topic = %request.topic,
            max_results = max,
            "recommend_sessions — calling recommendation backend"
        );

        let url = format!("{}/recommend", self.backend_url);

        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "topic": request.topic,
                "max_results": max,
            }))
            .send()
            .await;

        let response = match response {
            Ok(resp) => resp,
            Err(e) => {
                tracing::error!(error = %e, "Failed to reach recommendation backend");
                let message = if e.is_connect() {
                    "Recommendation backend is unavailable (connection refused). \
                     The backend pod may have been OOM-killed. Try again shortly."
                } else if e.is_timeout() {
                    "Recommendation backend timed out. \
                     The backend pod may be under memory pressure or OOM-killed. Try again shortly."
                } else {
                    "Recommendation backend request failed unexpectedly."
                };
                return Ok(CallToolResult::error(vec![Content::text(message)]));
            }
        };

        if !response.status().is_success() {
            tracing::error!(
                status = %response.status(),
                "Recommendation backend returned an error"
            );
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Recommendation backend returned HTTP {}. \
                 The backend may be unhealthy or restarting.",
                response.status()
            ))]));
        }

        let body: BackendResponse = match response.json().await {
            Ok(body) => body,
            Err(e) => {
                tracing::error!(error = %e, "Failed to parse backend response");
                return Ok(CallToolResult::error(vec![Content::text(
                    "Failed to parse response from recommendation backend. \
                     The backend may have been killed mid-response.",
                )]));
            }
        };

        if body.sessions.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "No sessions found matching '{}'. \
                 Try broader keywords like 'AI', 'Kubernetes', 'security', or 'edge'.",
                request.topic
            ))]));
        }

        let mut output = format!(
            "Found {} SUSECon session(s) matching '{}':\n\n",
            body.sessions.len(),
            request.topic,
        );

        for session in &body.sessions {
            use std::fmt::Write;
            let _ = write!(
                output,
                "ID: {}\n Title: **{}**\n Speaker: {}\n Track: {} | {} @ {} | Room: {}\n Tags: {}\n\n",
                session.id,
                session.title,
                session.speaker,
                session.track,
                session.day,
                session.time,
                session.room,
                session.tags.join(", "),
            );
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(
        description = "Get full details of a specific SUSECon session by its ID. Returns title, speaker(s), full abstract, schedule, room, and tags."
    )]
    async fn session_details(
        &self,
        Parameters(request): Parameters<SessionDetailsRequest>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            session_id = %request.session_id,
            "session_details — looking up session"
        );

        let url = format!("{}/session/{}", self.backend_url, request.session_id);

        let response = self.client.get(&url).send().await;

        let response = match response {
            Ok(resp) => resp,
            Err(e) => {
                tracing::error!(error = %e, "Failed to reach recommendation backend");
                let message = if e.is_connect() {
                    "Recommendation backend is unavailable (connection refused). \
                     The backend pod may have been OOM-killed. Try again shortly."
                } else if e.is_timeout() {
                    "Recommendation backend timed out. Try again shortly."
                } else {
                    "Recommendation backend request failed unexpectedly."
                };
                return Ok(CallToolResult::error(vec![Content::text(message)]));
            }
        };

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "No session found with ID '{}'. \
                 Use recommend_sessions to search by topic first.",
                request.session_id
            ))]));
        }

        if !response.status().is_success() {
            tracing::error!(
                status = %response.status(),
                "Recommendation backend returned an error"
            );
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Recommendation backend returned HTTP {}.",
                response.status()
            ))]));
        }

        let body: BackendSessionDetailResponse = match response.json().await {
            Ok(body) => body,
            Err(e) => {
                tracing::error!(error = %e, "Failed to parse backend response");
                return Ok(CallToolResult::error(vec![Content::text(
                    "Failed to parse response from recommendation backend.",
                )]));
            }
        };

        let s = &body.session;
        let output = format!(
            "ID: {}\nTitle: {}\nSpeaker: {}\nTrack: {}\nSchedule: {} @ {} | Room: {}\nTags: {}\n\nAbstract:\n{}",
            s.id,
            s.title,
            s.speaker,
            s.track,
            s.day,
            s.time,
            s.room,
            s.tags.join(", "),
            s.r#abstract,
        );

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

#[tool_handler]
impl ServerHandler for SuseConAgent {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_03_26;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("susecon-agent", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(
            "SUSECon Session Recommender Agent. Use the recommend_sessions tool to find \
             sessions at SUSECon based on your interests. Use the session_details tool to \
             get full details of a specific session by its ID. Provide a topic like 'AI', \
             'Kubernetes', 'security', 'edge', or 'observability'."
                .into(),
        );
        info
    }
}
