use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use chrono::{DateTime, Utc};

use xuanyu_memory::{
    classify_memory_write, extract_memories_from_messages, kind_for_category,
    with_memory_session_tag, ExtractionMessage, ExtractionRole, ForgettingPolicy,
    MemoryEmbeddingConfig, MemoryEmbeddingProvider, MemoryEmbeddingProviderKind, MemoryEntry,
    MemoryIngestInput, MemoryIngestStatus, MemoryKind, MemoryConflictResolution,
    MemorySearchQuery, MemoryService, MemoryWriteDecision, NewMemoryEntry, PendingMemoryEdit,
    PendingMemoryInput, PendingMemoryStatus,
};

fn start_embedding_fixture_server(max_requests: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let addr = listener.local_addr().expect("fixture address");
    thread::spawn(move || {
        for stream in listener.incoming().take(max_requests) {
            let Ok(mut stream) = stream else {
                continue;
            };
            handle_embedding_fixture_request(&mut stream);
        }
    });
    format!("http://{addr}")
}

fn handle_embedding_fixture_request(stream: &mut TcpStream) {
    let request = read_http_request(stream);
    let body = request
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or_default()
        .trim_end_matches(char::from(0));
    let payload = serde_json::from_str::<serde_json::Value>(body).unwrap_or_default();
    let data = payload
        .get("input")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let text = item.as_str().unwrap_or_default().to_ascii_lowercase();
                    let embedding = if text.contains("needle") || text.contains("target") {
                        vec![1.0, 0.0, 0.0]
                    } else {
                        vec![0.0, 1.0, 0.0]
                    };
                    serde_json::json!({ "index": index, "embedding": embedding })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let body = serde_json::json!({ "data": data }).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .expect("write fixture response");
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        let read = stream.read(&mut chunk).expect("read fixture request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let request = String::from_utf8_lossy(&buffer);
        let Some(header_end) = request.find("\r\n\r\n") else {
            continue;
        };
        let content_length = request[..header_end]
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        if buffer.len() >= header_end + 4 + content_length {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

fn test_data_dir(name: &str) -> std::path::PathBuf {
    let stamp = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    std::env::temp_dir().join(format!(
        "xuanyu-memory-{name}-{}-{stamp}",
        std::process::id()
    ))
}

#[tokio::test]
async fn search_ranks_content_and_tags() {
    let service = MemoryService::new();
    service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::Semantic,
            category: "code_knowledge".to_string(),
            content: "Rust core owns the state truth".to_string(),
            tags: vec!["architecture".to_string()],
            context: None,
            importance: 0.9,
        })
        .await;
    service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::Episodic,
            category: "session".to_string(),
            content: "Lunch notes".to_string(),
            tags: vec!["personal".to_string()],
            context: None,
            importance: 0.8,
        })
        .await;

    let results = service
        .search(MemorySearchQuery {
            text: Some("rust core".to_string()),
            tags: vec!["architecture".to_string()],
            limit: 5,
            ..MemorySearchQuery::default()
        })
        .await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entry.kind, MemoryKind::Semantic);
}

#[tokio::test]
async fn search_can_recall_by_query_keywords() {
    let service = MemoryService::new();
    service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Rust core owns the state truth".to_string(),
            tags: vec!["architecture".to_string()],
            context: Some("desktop migration".to_string()),
            importance: 0.9,
        })
        .await;

    let results = service
        .search(MemorySearchQuery {
            text: Some("who owns architecture state?".to_string()),
            limit: 5,
            ..MemorySearchQuery::default()
        })
        .await;

    assert_eq!(results.len(), 1);
    assert!(results[0]
        .reasons
        .iter()
        .any(|reason| reason == "keyword_match"));
    assert_eq!(results[0].entry.access_count, 1);
    assert!(results[0].entry.last_accessed_at.is_some());
}

#[tokio::test]
async fn search_supports_cjk_keyword_terms() {
    let service = MemoryService::new();
    service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Rust Core 是桌面应用的状态真相源".to_string(),
            tags: vec!["架构".to_string()],
            context: Some("重构计划".to_string()),
            importance: 0.8,
        })
        .await;

    let results = service
        .search(MemorySearchQuery {
            text: Some("谁负责状态 真相".to_string()),
            limit: 5,
            ..MemorySearchQuery::default()
        })
        .await;

    assert_eq!(results.len(), 1);
    assert!(results[0]
        .reasons
        .iter()
        .any(|reason| reason == "keyword_match"));
}

#[tokio::test]
async fn search_uses_hash_vector_for_short_semantic_terms() {
    let service = MemoryService::new();
    service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "React UI uses stateful panels and compact controls".to_string(),
            tags: vec!["frontend".to_string()],
            context: Some("desktop shell".to_string()),
            importance: 0.6,
        })
        .await;

    let results = service
        .search(MemorySearchQuery {
            text: Some("js ui".to_string()),
            limit: 5,
            ..MemorySearchQuery::default()
        })
        .await;

    assert_eq!(results.len(), 1);
    assert!(results[0]
        .reasons
        .iter()
        .any(|reason| reason == "semantic_match"));
}

#[tokio::test]
async fn search_with_unconfigured_local_embedding_falls_back_to_hash() {
    let service = MemoryService::new();
    service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "React UI uses stateful panels and compact controls".to_string(),
            tags: vec!["frontend".to_string()],
            context: Some("desktop shell".to_string()),
            importance: 0.6,
        })
        .await;
    let provider = MemoryEmbeddingProvider::new(MemoryEmbeddingConfig {
        provider: MemoryEmbeddingProviderKind::Local,
        model: "bge-small-zh-v1.5-onnx".to_string(),
        dimensions: 1536,
        api_key: None,
        base_url: None,
        timeout_ms: 1_000,
        batch_size: 4,
    });

    let results = service
        .search_with_embedding_provider(
            MemorySearchQuery {
                text: Some("js ui".to_string()),
                limit: 5,
                ..MemorySearchQuery::default()
            },
            &provider,
        )
        .await;

    assert_eq!(results.len(), 1);
    assert!(results[0]
        .reasons
        .iter()
        .any(|reason| reason == "semantic_match"));
}

#[tokio::test]
async fn embedding_refresh_persists_sidecar_index_and_search_uses_it() {
    let base_url = start_embedding_fixture_server(4);
    let provider = MemoryEmbeddingProvider::new(MemoryEmbeddingConfig {
        provider: MemoryEmbeddingProviderKind::Local,
        model: "fixture-embedding".to_string(),
        dimensions: 3,
        api_key: None,
        base_url: Some(base_url),
        timeout_ms: 2_000,
        batch_size: 8,
    });
    let root = test_data_dir("embedding-index");
    let path = root.join("memories.json");
    let service = MemoryService::persistent(path.clone());
    service.initialize().await.expect("init");
    let target = service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Target memory carries the semantic needle".to_string(),
            tags: vec!["target".to_string()],
            context: None,
            importance: 0.8,
        })
        .await;
    service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Unrelated memory points elsewhere".to_string(),
            tags: vec!["other".to_string()],
            context: None,
            importance: 0.8,
        })
        .await;

    let refreshed = service.refresh_embeddings(&provider, 10).await;
    assert_eq!(refreshed.attempted, 2);
    assert_eq!(refreshed.refreshed, 2);
    let stats = service.embedding_index_stats(Some(&provider)).await;
    assert_eq!(stats.ready, 2);
    assert_eq!(stats.stale, 0);

    let results = service
        .search_with_embedding_provider(
            MemorySearchQuery {
                text: Some("needle".to_string()),
                limit: 10,
                ..MemorySearchQuery::default()
            },
            &provider,
        )
        .await;
    assert_eq!(
        results.first().map(|result| result.entry.id.as_str()),
        Some(target.id.as_str())
    );
    assert!(results[0]
        .reasons
        .iter()
        .any(|reason| reason == "semantic_match"));

    let restored = MemoryService::persistent(path);
    restored.initialize().await.expect("restore");
    let restored_stats = restored.embedding_index_stats(Some(&provider)).await;
    assert_eq!(restored_stats.ready, 2);
}

#[tokio::test]
async fn search_blends_recency_importance_and_frequency() {
    let service = MemoryService::new();
    let now = Utc::now();
    let stale = now - chrono::Duration::days(90);
    service
        .import_memory_entry(MemoryEntry {
            id: "stale-high-importance".to_string(),
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Recall planner stores owner architecture signal from last quarter"
                .to_string(),
            tags: vec!["architecture".to_string()],
            context: Some("recall".to_string()),
            importance: 0.95,
            created_at: stale,
            updated_at: stale,
            last_accessed_at: None,
            access_count: 0,
        })
        .await;
    service
        .import_memory_entry(MemoryEntry {
            id: "fresh-frequent".to_string(),
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Recall planner stores owner architecture signal from this session"
                .to_string(),
            tags: vec!["architecture".to_string()],
            context: Some("recall".to_string()),
            importance: 0.6,
            created_at: now,
            updated_at: now,
            last_accessed_at: Some(now),
            access_count: 8,
        })
        .await;

    let results = service
        .search(MemorySearchQuery {
            text: Some("owner architecture signal".to_string()),
            limit: 2,
            ..MemorySearchQuery::default()
        })
        .await;

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].entry.id, "fresh-frequent");
    assert!(results[0].reasons.iter().any(|reason| reason == "recent"));
    assert!(results[0].reasons.iter().any(|reason| reason == "frequent"));
    assert!(results[0].score > results[1].score);
}

#[tokio::test]
async fn search_respects_session_scope_tags() {
    let service = MemoryService::new();
    let now = Utc::now();
    for (id, session) in [("a", "session-a"), ("b", "session-b")] {
        service
            .import_memory_entry(MemoryEntry {
                id: format!("memory-{id}"),
                kind: MemoryKind::ProjectProfile,
                category: "project_fact".to_string(),
                content: format!("Scoped alpha memory for {session}"),
                tags: with_memory_session_tag(Vec::new(), Some(session)),
                context: Some(session.to_string()),
                importance: 0.8,
                created_at: now,
                updated_at: now,
                last_accessed_at: None,
                access_count: 0,
            })
            .await;
    }
    service
        .import_memory_entry(MemoryEntry {
            id: "global".to_string(),
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Scoped alpha global memory".to_string(),
            tags: Vec::new(),
            context: None,
            importance: 0.8,
            created_at: now,
            updated_at: now,
            last_accessed_at: None,
            access_count: 0,
        })
        .await;

    let scoped = service
        .search(MemorySearchQuery {
            text: Some("scoped alpha memory".to_string()),
            session_id: Some("session-a".to_string()),
            limit: 10,
            ..MemorySearchQuery::default()
        })
        .await;
    let ids = scoped
        .iter()
        .map(|result| result.entry.id.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"memory-a"));
    assert!(ids.contains(&"global"));
    assert!(!ids.contains(&"memory-b"));

    let cross_session = service
        .search(MemorySearchQuery {
            text: Some("scoped alpha memory".to_string()),
            session_id: Some("session-a".to_string()),
            include_cross_session: true,
            limit: 10,
            ..MemorySearchQuery::default()
        })
        .await;
    assert!(cross_session
        .iter()
        .any(|result| result.entry.id == "memory-b"));
}

#[tokio::test]
async fn add_memory_normalizes_category_synonyms() {
    let service = MemoryService::new();
    let entry = service
        .add_memory(NewMemoryEntry {
            kind: kind_for_category("project-context"),
            category: "project-context".to_string(),
            content: "  Project uses Rust core for state ownership.  ".to_string(),
            tags: vec![" Rust ".to_string()],
            context: None,
            importance: 0.7,
        })
        .await;

    assert_eq!(entry.category, "project_fact");
    assert_eq!(entry.kind, MemoryKind::ProjectProfile);
    assert_eq!(entry.content, "Project uses Rust core for state ownership.");
    assert_eq!(entry.tags, vec!["rust".to_string()]);

    let identity = service
        .add_memory(NewMemoryEntry {
            kind: kind_for_category("soul"),
            category: "soul".to_string(),
            content: "Assistant identity stays concise and verification-first.".to_string(),
            tags: vec![],
            context: None,
            importance: 1.0,
        })
        .await;
    assert_eq!(identity.category, "identity");
    assert_eq!(identity.kind, MemoryKind::SelfArchive);
}

#[tokio::test]
async fn add_memory_merges_duplicate_content_with_better_metadata() {
    let service = MemoryService::new();
    let first = service
        .add_memory(NewMemoryEntry {
            kind: kind_for_category("preference"),
            category: "preference".to_string(),
            content: "User prefers concise engineering summaries with verification notes"
                .to_string(),
            tags: vec!["style".to_string()],
            context: Some("profile".to_string()),
            importance: 0.5,
        })
        .await;
    let second = service
        .add_memory(NewMemoryEntry {
            kind: kind_for_category("user_preference"),
            category: "user_preference".to_string(),
            content: "User prefers concise engineering summaries with verification notes today"
                .to_string(),
            tags: vec!["style".to_string(), "summary".to_string()],
            context: Some("profile".to_string()),
            importance: 0.8,
        })
        .await;
    let entries = service.recent(10).await;

    assert_eq!(first.id, second.id);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].importance, 0.8);
    assert_eq!(
        entries[0].content,
        "User prefers concise engineering summaries with verification notes today"
    );
    assert_eq!(
        entries[0].tags,
        vec!["style".to_string(), "summary".to_string()]
    );
    assert!(entries[0].last_accessed_at.is_some());
}

#[test]
fn rule_extractor_collects_conversation_memory_candidates() {
    let result = extract_memories_from_messages(&[
        ExtractionMessage {
            role: ExtractionRole::User,
            content: "Please refactor crates/xuanyu-memory/src/lib.rs and remember the approach: keep Rust as state owner.".to_string(),
            tool_names: Vec::new(),
        },
        ExtractionMessage {
            role: ExtractionRole::Assistant,
            content: "Decided to use rule-based extraction.\n```rust\nfn main() {}\n```\nerror: vector index failed during migration".to_string(),
            tool_names: vec!["file_edit".to_string(), "cargo_test".to_string()],
        },
    ]);

    assert!(result
        .files_modified
        .iter()
        .any(|file| file == "crates/xuanyu-memory/src/lib.rs"));
    assert_eq!(
        result.tools_used,
        vec!["cargo_test".to_string(), "file_edit".to_string()]
    );
    assert!(result.summary.contains("User requested"));
    assert!(result
        .memories
        .iter()
        .any(|memory| memory.category == "task_state"));
    assert!(result
        .memories
        .iter()
        .any(|memory| memory.category == "code_knowledge"));
    assert!(result
        .memories
        .iter()
        .any(|memory| memory.category == "decision"));
    assert!(result
        .memories
        .iter()
        .any(|memory| memory.category == "error_experience"));
    assert!(result
        .memories
        .iter()
        .any(|memory| memory.category == "project_fact"));
}

#[test]
fn memory_write_policy_routes_sensitive_and_inferred_profile_data() {
    assert_eq!(
        classify_memory_write("api_key=secret", "project_fact", "context_compact"),
        MemoryWriteDecision::Deny
    );
    assert_eq!(
        classify_memory_write("User likes short answers", "preference", "context_compact"),
        MemoryWriteDecision::Pending
    );
    assert_eq!(
        classify_memory_write(
            "User likes short answers",
            "preference",
            "explicit:file:user.md"
        ),
        MemoryWriteDecision::Direct
    );
    assert_eq!(
        classify_memory_write(
            "Files modified: src/lib.rs",
            "project_fact",
            "context_compact"
        ),
        MemoryWriteDecision::Direct
    );
}

#[tokio::test]
async fn ingest_memory_candidate_sends_moderate_conflicts_to_pending() {
    let service = MemoryService::new();
    service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "alpha beta gamma delta".to_string(),
            tags: vec!["first".to_string()],
            context: None,
            importance: 0.7,
        })
        .await;

    let result = service
        .ingest_memory_candidate(MemoryIngestInput {
            entry: NewMemoryEntry {
                kind: MemoryKind::ProjectProfile,
                category: "project_fact".to_string(),
                content: "alpha beta gamma epsilon".to_string(),
                tags: vec!["second".to_string()],
                context: None,
                importance: 0.8,
            },
            source: "context_compact".to_string(),
            confidence: Some(0.7),
            reason: "candidate".to_string(),
            session_id: Some("session-1".to_string()),
            task_id: None,
            run_id: None,
            trace_id: None,
        })
        .await;

    assert_eq!(result.status, MemoryIngestStatus::Pending);
    assert_eq!(service.recent(10).await.len(), 1);
    assert_eq!(service.list_pending(false, 10).await.len(), 1);
    assert_eq!(
        result.conflicts[0].resolution,
        MemoryConflictResolution::Pending
    );
    assert!(result.conflicts[0].reason.contains("moderate_similarity"));
}

#[tokio::test]
async fn ingest_memory_candidate_merges_high_similarity_candidates() {
    let service = MemoryService::new();
    let existing = service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "alpha beta gamma delta epsilon zeta eta theta".to_string(),
            tags: vec!["first".to_string()],
            context: None,
            importance: 0.5,
        })
        .await;

    let result = service
        .ingest_memory_candidate(MemoryIngestInput {
            entry: NewMemoryEntry {
                kind: MemoryKind::ProjectProfile,
                category: "project_fact".to_string(),
                content: "alpha beta gamma delta epsilon zeta eta iota".to_string(),
                tags: vec!["second".to_string()],
                context: None,
                importance: 0.8,
            },
            source: "context_compact".to_string(),
            confidence: Some(0.7),
            reason: "candidate".to_string(),
            session_id: None,
            task_id: None,
            run_id: None,
            trace_id: None,
        })
        .await;
    let entries = service.recent(10).await;

    assert_eq!(result.status, MemoryIngestStatus::Stored);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, existing.id);
    assert_eq!(entries[0].importance, 0.8);
    assert_eq!(
        entries[0].tags,
        vec!["first".to_string(), "second".to_string()]
    );
    assert_eq!(
        result.conflicts[0].resolution,
        MemoryConflictResolution::Merge
    );
}

#[tokio::test]
async fn ingest_memory_candidate_keeps_stronger_near_duplicate() {
    let service = MemoryService::new();
    let existing = service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Rust core owns state truth".to_string(),
            tags: vec!["architecture".to_string()],
            context: None,
            importance: 0.9,
        })
        .await;

    let result = service
        .ingest_memory_candidate(MemoryIngestInput {
            entry: NewMemoryEntry {
                kind: MemoryKind::ProjectProfile,
                category: "project_fact".to_string(),
                content: "Rust core owns state truth".to_string(),
                tags: vec!["duplicate".to_string()],
                context: None,
                importance: 0.4,
            },
            source: "context_compact".to_string(),
            confidence: Some(0.7),
            reason: "candidate".to_string(),
            session_id: None,
            task_id: None,
            run_id: None,
            trace_id: None,
        })
        .await;
    let entries = service.recent(10).await;

    assert_eq!(result.status, MemoryIngestStatus::Skipped);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, existing.id);
    assert_eq!(entries[0].importance, 0.9);
    assert_eq!(entries[0].tags, vec!["architecture".to_string()]);
    assert_eq!(
        result.conflicts[0].resolution,
        MemoryConflictResolution::KeepExisting
    );
}

#[tokio::test]
async fn repeated_searches_raise_frequency_reason() {
    let service = MemoryService::new();
    service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::Preference,
            category: "user_preference".to_string(),
            content: "User prefers concise engineering summaries".to_string(),
            tags: vec!["style".to_string()],
            context: None,
            importance: 0.6,
        })
        .await;

    let first = service
        .search(MemorySearchQuery {
            text: Some("concise summaries".to_string()),
            limit: 5,
            ..MemorySearchQuery::default()
        })
        .await;
    assert_eq!(first[0].entry.access_count, 1);

    let second = service
        .search(MemorySearchQuery {
            text: Some("concise summaries".to_string()),
            limit: 5,
            ..MemorySearchQuery::default()
        })
        .await;
    assert_eq!(second[0].entry.access_count, 2);
    assert!(second[0].reasons.iter().any(|reason| reason == "frequent"));
}

#[tokio::test]
async fn consolidate_merges_similar_entries_within_same_scope() {
    let service = MemoryService::new();
    let now = Utc::now();
    service
        .import_memory_entry(MemoryEntry {
            id: "short".to_string(),
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Rust core owns the state truth for desktop app".to_string(),
            tags: vec!["architecture".to_string()],
            context: Some("project".to_string()),
            importance: 0.5,
            created_at: now,
            updated_at: now,
            last_accessed_at: None,
            access_count: 1,
        })
        .await;
    service
        .import_memory_entry(MemoryEntry {
            id: "long".to_string(),
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Rust core owns the state truth for desktop application".to_string(),
            tags: vec!["state".to_string()],
            context: Some("project".to_string()),
            importance: 0.9,
            created_at: now + chrono::Duration::seconds(10),
            updated_at: now + chrono::Duration::seconds(10),
            last_accessed_at: Some(now + chrono::Duration::seconds(20)),
            access_count: 2,
        })
        .await;

    let result = service.consolidate().await;
    let entries = service.recent(10).await;

    assert_eq!(result.merged, 1);
    assert_eq!(result.pruned, 1);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].content,
        "Rust core owns the state truth for desktop application"
    );
    assert_eq!(entries[0].importance, 0.9);
    assert_eq!(
        entries[0].tags,
        vec!["architecture".to_string(), "state".to_string()]
    );
    assert_eq!(entries[0].access_count, 3);
}

#[tokio::test]
async fn consolidate_keeps_similar_entries_in_different_contexts() {
    let service = MemoryService::new();
    let now = Utc::now();
    for context in ["session-a", "session-b"] {
        service
            .import_memory_entry(MemoryEntry {
                id: format!("memory-{context}"),
                kind: MemoryKind::Episodic,
                category: "session".to_string(),
                content: "User asked to test the memory consolidation path".to_string(),
                tags: Vec::new(),
                context: Some(context.to_string()),
                importance: 0.6,
                created_at: now,
                updated_at: now,
                last_accessed_at: None,
                access_count: 0,
            })
            .await;
    }

    let result = service.consolidate().await;

    assert_eq!(result.merged, 0);
    assert_eq!(service.recent(10).await.len(), 2);
}

#[tokio::test]
async fn forgetting_decays_idle_low_value_memory() {
    let service = MemoryService::new();
    let now = Utc::now();
    let old = now - chrono::Duration::days(20);
    let imported = service
        .import_memory_entry(MemoryEntry {
            id: "idle-low-value".to_string(),
            kind: MemoryKind::Episodic,
            category: "task_state".to_string(),
            content: "Temporary task detail".to_string(),
            tags: Vec::new(),
            context: None,
            importance: 0.3,
            created_at: old,
            updated_at: old,
            last_accessed_at: None,
            access_count: 0,
        })
        .await;

    let result = service.apply_forgetting(ForgettingPolicy::default()).await;
    let entry = service.get_memory(&imported.id).await.expect("kept memory");

    assert_eq!(result.decayed, vec!["idle-low-value".to_string()]);
    assert!(result.dropped.is_empty());
    assert!(entry.importance < 0.3);
}

#[tokio::test]
async fn forgetting_drops_stale_low_importance_memory() {
    let service = MemoryService::new();
    let old = Utc::now() - chrono::Duration::days(120);
    service
        .import_memory_entry(MemoryEntry {
            id: "stale-task-state".to_string(),
            kind: MemoryKind::Episodic,
            category: "task_state".to_string(),
            content: "Very old temporary task state".to_string(),
            tags: Vec::new(),
            context: None,
            importance: 0.5,
            created_at: old,
            updated_at: old,
            last_accessed_at: None,
            access_count: 0,
        })
        .await;

    let result = service.apply_forgetting(ForgettingPolicy::default()).await;
    let entries = service.recent(10).await;

    assert_eq!(result.dropped, vec!["stale-task-state".to_string()]);
    assert!(result.decayed.is_empty());
    assert!(entries.is_empty());
    assert_eq!(result.kept, 0);
}

#[tokio::test]
async fn persistent_service_restores_entries() {
    let root = test_data_dir("persistent");
    let path = root.join("memories.json");
    let service = MemoryService::persistent(path.clone());
    service.initialize().await.expect("init");
    let created = service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Memory persists across desktop restarts".to_string(),
            tags: vec!["Rust".to_string(), "rust".to_string()],
            context: Some("test".to_string()),
            importance: 0.8,
        })
        .await;

    let restored = MemoryService::persistent(path);
    restored.initialize().await.expect("restore");
    let entries = restored.recent(10).await;

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, created.id);
    assert_eq!(entries[0].tags, vec!["rust".to_string()]);
}

#[tokio::test]
async fn snapshots_persist_and_restore_entries() {
    let root = test_data_dir("snapshots");
    let path = root.join("memories.json");
    let service = MemoryService::persistent(path.clone());
    service.initialize().await.expect("init");
    let first = service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::ProjectProfile,
            category: "project_fact".to_string(),
            content: "Snapshot baseline memory".to_string(),
            tags: vec!["snapshot".to_string()],
            context: None,
            importance: 0.8,
        })
        .await;
    let snapshot = service
        .save_snapshot("baseline".to_string())
        .await
        .expect("snapshot");

    service
        .add_memory(NewMemoryEntry {
            kind: MemoryKind::Semantic,
            category: "code_knowledge".to_string(),
            content: "Temporary memory after snapshot".to_string(),
            tags: Vec::new(),
            context: None,
            importance: 0.5,
        })
        .await;
    assert_eq!(service.recent(10).await.len(), 2);

    let restored = service
        .rollback_snapshot(snapshot.version)
        .await
        .expect("rollback")
        .expect("snapshot exists");
    assert_eq!(restored.entry_count, 1);
    let entries = service.recent(10).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, first.id);

    let restarted = MemoryService::persistent(path);
    restarted.initialize().await.expect("restore");
    let snapshots = restarted.list_snapshots().await;
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].version, snapshot.version);
    assert_eq!(restarted.recent(10).await[0].id, first.id);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn snapshots_keep_latest_ten_versions() {
    let service = MemoryService::new();
    for index in 0..12 {
        service
            .save_snapshot(format!("snapshot {index}"))
            .await
            .expect("snapshot");
    }

    let snapshots = service.list_snapshots().await;
    assert_eq!(snapshots.len(), 10);
    assert_eq!(snapshots[0].version, 3);
    assert_eq!(snapshots.last().map(|snapshot| snapshot.version), Some(12));
}

#[tokio::test]
async fn import_memory_entry_preserves_legacy_identity_and_timestamps() {
    let root = test_data_dir("import");
    let path = root.join("memories.json");
    let service = MemoryService::persistent(path.clone());
    service.initialize().await.expect("init");
    let created_at = DateTime::<Utc>::from_timestamp_millis(1_700_000_000_000).unwrap();

    let imported = service
        .import_memory_entry(MemoryEntry {
            id: "legacy-memory-1".to_string(),
            kind: MemoryKind::ProjectProfile,
            category: "PROJECT_FACT".to_string(),
            content: "Legacy project memory".to_string(),
            tags: vec![" Migration ".to_string(), "migration".to_string()],
            context: Some("legacy".to_string()),
            importance: 8.0,
            created_at,
            updated_at: created_at - chrono::Duration::seconds(60),
            last_accessed_at: Some(created_at),
            access_count: 7,
        })
        .await;

    assert_eq!(imported.id, "legacy-memory-1");
    assert_eq!(imported.category, "project_fact");
    assert_eq!(imported.importance, 1.0);
    assert_eq!(imported.tags, vec!["migration".to_string()]);
    assert_eq!(imported.updated_at, created_at);

    let restored = MemoryService::persistent(path);
    restored.initialize().await.expect("restore");
    let entries = restored.recent(10).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "legacy-memory-1");
    assert_eq!(entries[0].access_count, 7);
}

#[tokio::test]
async fn pending_memory_can_be_approved_rejected_and_restored() {
    let root = test_data_dir("pending");
    let path = root.join("memories.json");
    let service = MemoryService::persistent(path.clone());
    service.initialize().await.expect("init");

    let pending = service
        .propose_pending(PendingMemoryInput {
            content: "User prefers concise answers".to_string(),
            category: "user_preference".to_string(),
            importance: 8,
            source: "test".to_string(),
            tags: vec!["User".to_string()],
            confidence: Some(0.8),
            reason: "personal preference".to_string(),
            session_id: Some("session-1".to_string()),
            task_id: None,
            run_id: None,
            trace_id: None,
        })
        .await
        .expect("pending");
    assert_eq!(service.list_pending(false, 10).await.len(), 1);
    assert!(service
        .propose_pending(PendingMemoryInput {
            content: "api_key=secret".to_string(),
            category: "semantic".to_string(),
            importance: 5,
            source: "test".to_string(),
            tags: Vec::new(),
            confidence: None,
            reason: "sensitive".to_string(),
            session_id: None,
            task_id: None,
            run_id: None,
            trace_id: None,
        })
        .await
        .is_none());

    let (_approved, memory) = service
        .approve_pending(&pending.id, PendingMemoryEdit::default())
        .await
        .expect("approved");
    assert_eq!(memory.category, "user_preference");
    assert!(service.list_pending(false, 10).await.is_empty());

    let rejected = service
        .propose_pending(PendingMemoryInput {
            content: "Temporary note".to_string(),
            category: "semantic".to_string(),
            importance: 5,
            source: "test".to_string(),
            tags: Vec::new(),
            confidence: None,
            reason: "candidate".to_string(),
            session_id: None,
            task_id: None,
            run_id: None,
            trace_id: None,
        })
        .await
        .expect("second pending");
    service
        .reject_pending(&rejected.id, Some(60_000))
        .await
        .expect("rejected");

    let restored = MemoryService::persistent(path);
    restored.initialize().await.expect("restore");
    assert_eq!(restored.recent(10).await.len(), 1);
    assert_eq!(restored.list_pending(true, 10).await.len(), 2);
    assert!(restored
        .list_pending(true, 10)
        .await
        .iter()
        .any(|item| item.status == PendingMemoryStatus::Rejected));
}
