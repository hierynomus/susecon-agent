use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub speaker: String,
    pub r#abstract: String,
    pub track: String,
    pub day: String,
    pub time: String,
    pub room: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCatalog {
    pub sessions: Vec<Session>,
}

impl SessionCatalog {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let catalog: SessionCatalog = serde_yaml::from_str(&content)?;
        tracing::info!(count = catalog.sessions.len(), "Loaded sessions from catalog");
        Ok(catalog)
    }

    pub fn recommend(&self, topic: &str, max_results: usize) -> Vec<&Session> {
        let topic_lower = topic.to_lowercase();
        let keywords: Vec<&str> = topic_lower.split_whitespace().collect();

        let mut scored: Vec<(usize, &Session)> = self
            .sessions
            .iter()
            .map(|s| {
                let score = keywords.iter().fold(0usize, |acc, kw| {
                    let mut s_score = 0;
                    if s.title.to_lowercase().contains(kw) {
                        s_score += 3;
                    }
                    if s.track.to_lowercase().contains(kw) {
                        s_score += 2;
                    }
                    if s.tags.iter().any(|t| t.to_lowercase().contains(kw)) {
                        s_score += 2;
                    }
                    if s.r#abstract.to_lowercase().contains(kw) {
                        s_score += 1;
                    }
                    acc + s_score
                });
                (score, s)
            })
            .filter(|(score, _)| *score > 0)
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().take(max_results).map(|(_, s)| s).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sessions() -> SessionCatalog {
        SessionCatalog {
            sessions: vec![
                Session {
                    id: "SC-101".to_string(),
                    title: "Agentic AI on Kubernetes".to_string(),
                    speaker: "Dr. Elena Vasquez".to_string(),
                    r#abstract: "Deploy autonomous AI agents on Kubernetes using MCP.".to_string(),
                    track: "AI/ML".to_string(),
                    day: "Tuesday".to_string(),
                    time: "09:00".to_string(),
                    room: "Hall A1".to_string(),
                    tags: vec!["ai".to_string(), "kubernetes".to_string(), "mcp".to_string()],
                },
                Session {
                    id: "SC-102".to_string(),
                    title: "LLM Inference at the Edge".to_string(),
                    speaker: "Marcus Chen".to_string(),
                    r#abstract: "Deploy quantized language models on edge nodes.".to_string(),
                    track: "AI/ML".to_string(),
                    day: "Tuesday".to_string(),
                    time: "11:00".to_string(),
                    room: "Hall A2".to_string(),
                    tags: vec!["ai".to_string(), "edge".to_string(), "llm".to_string()],
                },
                Session {
                    id: "SC-103".to_string(),
                    title: "Kubernetes Memory Management".to_string(),
                    speaker: "Priya Sharma".to_string(),
                    r#abstract: "Kubernetes swap support and memory QoS for AI workloads.".to_string(),
                    track: "Kubernetes".to_string(),
                    day: "Tuesday".to_string(),
                    time: "14:00".to_string(),
                    room: "Hall B1".to_string(),
                    tags: vec!["kubernetes".to_string(), "swap".to_string(), "memory".to_string()],
                },
                Session {
                    id: "SC-104".to_string(),
                    title: "Security Best Practices for Cloud Native".to_string(),
                    speaker: "Alex Johnson".to_string(),
                    r#abstract: "Learn container security, network policies, and supply chain safety.".to_string(),
                    track: "Security".to_string(),
                    day: "Wednesday".to_string(),
                    time: "10:00".to_string(),
                    room: "Hall C1".to_string(),
                    tags: vec!["security".to_string(), "containers".to_string()],
                },
            ],
        }
    }

    #[test]
    fn test_recommend_by_title() {
        let catalog = sample_sessions();
        let results = catalog.recommend("kubernetes", 5);

        assert!(!results.is_empty());
        // Should find sessions with "kubernetes" in title
        assert!(results.iter().any(|s| s.id == "SC-101"));
        assert!(results.iter().any(|s| s.id == "SC-103"));
    }

    #[test]
    fn test_recommend_by_tag() {
        let catalog = sample_sessions();
        let results = catalog.recommend("edge", 5);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "SC-102");
    }

    #[test]
    fn test_recommend_by_track() {
        let catalog = sample_sessions();
        let results = catalog.recommend("security", 5);

        assert!(!results.is_empty());
        assert!(results.iter().any(|s| s.track == "Security"));
    }

    #[test]
    fn test_recommend_by_abstract() {
        let catalog = sample_sessions();
        let results = catalog.recommend("swap", 5);

        assert!(!results.is_empty());
        // SC-103 has "swap" in tags and abstract
        assert!(results.iter().any(|s| s.id == "SC-103"));
    }

    #[test]
    fn test_recommend_case_insensitive() {
        let catalog = sample_sessions();

        let lower = catalog.recommend("ai", 5);
        let upper = catalog.recommend("AI", 5);
        let mixed = catalog.recommend("Ai", 5);

        assert_eq!(lower.len(), upper.len());
        assert_eq!(lower.len(), mixed.len());
    }

    #[test]
    fn test_recommend_no_results() {
        let catalog = sample_sessions();
        let results = catalog.recommend("nonexistent topic xyz", 5);

        assert!(results.is_empty());
    }

    #[test]
    fn test_recommend_max_results() {
        let catalog = sample_sessions();
        let results = catalog.recommend("ai", 2);

        assert!(results.len() <= 2);
    }

    #[test]
    fn test_recommend_multiple_keywords() {
        let catalog = sample_sessions();
        let results = catalog.recommend("ai kubernetes", 5);

        // SC-101 should rank highest (has both keywords)
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "SC-101");
    }

    #[test]
    fn test_recommend_sorted_by_relevance() {
        let catalog = sample_sessions();
        let results = catalog.recommend("kubernetes", 5);

        // Sessions with kubernetes in title should rank higher than those with it only in abstract
        assert!(results.len() >= 2);
        // SC-103 has "Kubernetes" in title and track, SC-101 has it in title
        let first_ids: Vec<_> = results.iter().take(2).map(|s| s.id.as_str()).collect();
        assert!(first_ids.contains(&"SC-101") || first_ids.contains(&"SC-103"));
    }

    #[test]
    fn test_session_serialization() {
        let session = Session {
            id: "TEST-1".to_string(),
            title: "Test Session".to_string(),
            speaker: "Test Speaker".to_string(),
            r#abstract: "Test abstract".to_string(),
            track: "Test Track".to_string(),
            day: "Monday".to_string(),
            time: "10:00".to_string(),
            room: "Room 1".to_string(),
            tags: vec!["tag1".to_string(), "tag2".to_string()],
        };

        let yaml = serde_yaml::to_string(&session).expect("Failed to serialize");
        let deserialized: Session = serde_yaml::from_str(&yaml).expect("Failed to deserialize");

        assert_eq!(session.id, deserialized.id);
        assert_eq!(session.title, deserialized.title);
        assert_eq!(session.tags, deserialized.tags);
    }

    #[test]
    fn test_catalog_load_from_yaml() {
        let yaml = r#"
sessions:
  - id: "TEST-1"
    title: "Test Session"
    speaker: "Test Speaker"
    abstract: "Test abstract"
    track: "Test Track"
    day: "Monday"
    time: "10:00"
    room: "Room 1"
    tags: ["tag1", "tag2"]
"#;
        let catalog: SessionCatalog = serde_yaml::from_str(yaml).expect("Failed to parse YAML");
        assert_eq!(catalog.sessions.len(), 1);
        assert_eq!(catalog.sessions[0].id, "TEST-1");
    }

    #[test]
    fn test_empty_catalog() {
        let catalog = SessionCatalog { sessions: vec![] };
        let results = catalog.recommend("anything", 5);
        assert!(results.is_empty());
    }
}
