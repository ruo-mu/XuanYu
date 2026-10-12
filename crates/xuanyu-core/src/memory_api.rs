//! Memory CRUD / profile / snapshot / search AppCore APIs.

use crate::{
    find_profile_memory, kind_for_category, memory_item_from_entry, memory_profile_context,
    memory_profile_definitions, memory_snapshot_from_info, pending_memory_to_value,
    project_dir_from_config, ui_importance_to_score, AppCore, GovernanceDecision, MemoryCreateInput,
    MemoryListResult, MemoryMutationResult, MemoryProfileFileDto, MemorySnapshotCreateResult,
    MemorySnapshotDto, MemorySnapshotListResult, MemorySnapshotRollbackResult, MemoryStatsDto,
    MemoryUpdateInput,
};
use serde_json::{json, Value};
use xuanyu_memory::{
    ForgettingPolicy, MemoryEmbeddingProvider, MemoryEntry, MemorySearchQuery, MemorySearchResult,
    MemoryUpdate, NewMemoryEntry, PendingMemoryEdit,
};
use xuanyu_storage::MemoryHitRecord;

impl AppCore {
    pub async fn memory_create(
        &self,
        input: MemoryCreateInput,
    ) -> anyhow::Result<MemoryMutationResult> {
        let kind = kind_for_category(&input.category);
        let entry = self
            .memory
            .add_memory(NewMemoryEntry {
                kind,
                category: input.category,
                content: input.content,
                tags: input.tags,
                context: input.context,
                importance: ui_importance_to_score(input.importance),
            })
            .await;
        self.schedule_memory_embedding_refresh(entry.clone()).await;
        Ok(MemoryMutationResult {
            memory: memory_item_from_entry(entry),
            stats: self.memory_stats().await,
        })
    }

    pub async fn memory_update(
        &self,
        id: &str,
        updates: MemoryUpdateInput,
    ) -> anyhow::Result<Option<MemoryMutationResult>> {
        let category = updates.category.clone();
        let kind = category.as_deref().map(kind_for_category);
        let updated = self
            .memory
            .update_memory(
                id,
                MemoryUpdate {
                    kind,
                    category,
                    content: updates.content,
                    tags: updates.tags,
                    importance: updates.importance.map(ui_importance_to_score),
                },
            )
            .await;
        if let Some(entry) = updated {
            self.schedule_memory_embedding_refresh(entry.clone()).await;
            let stats = self.memory_stats().await;
            Ok(Some(MemoryMutationResult {
                memory: memory_item_from_entry(entry),
                stats,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn memory_delete(&self, id: &str) -> bool {
        self.memory.delete_memory(id).await
    }

    pub async fn memory_flush(&self) -> anyhow::Result<Value> {
        self.memory.flush().await?;
        Ok(json!({
            "ok": true,
            "path": self
                .memory
                .storage_path()
                .map(|path| path.to_string_lossy().to_string()),
            "stats": self.memory_stats().await
        }))
    }

    pub async fn memory_maintenance_run(&self) -> Value {
        let consolidation = self.memory.consolidate().await;
        let result = self
            .memory
            .apply_forgetting(ForgettingPolicy::default())
            .await;
        let embedding_provider = self.current_memory_embedding_provider().await;
        let embedding_refresh = self
            .memory
            .refresh_embeddings(&embedding_provider, 500)
            .await;
        let embedding_index = self
            .memory
            .embedding_index_stats(Some(&embedding_provider))
            .await;
        json!({
            "ok": true,
            "merged": consolidation.merged,
            "distilled": consolidation.distilled,
            "pruned": consolidation.pruned,
            "decayed": result.decayed,
            "dropped": result.dropped,
            "kept": result.kept,
            "embeddingRefresh": embedding_refresh,
            "embeddingIndex": embedding_index,
            "stats": self.memory_stats().await
        })
    }

    pub async fn memory_snapshot_create(
        &self,
        reason: Option<String>,
    ) -> anyhow::Result<MemorySnapshotCreateResult> {
        let snapshot = self
            .memory
            .save_snapshot(reason.unwrap_or_else(|| "manual_snapshot".to_string()))
            .await?;
        let _ = self
            .governance
            .rollback_registry()
            .register(
                format!("Rollback memory snapshot v{}", snapshot.version),
                "memory_snapshot",
                json!({ "version": snapshot.version }),
            )
            .await;
        let _ = self
            .governance
            .record_audit_event(
                "system",
                "memory.snapshot.create",
                Some(format!("memory:snapshot:{}", snapshot.version)),
                GovernanceDecision::Allow,
                format!(
                    "Created memory snapshot v{} with {} entries",
                    snapshot.version, snapshot.entry_count
                ),
                None,
            )
            .await;
        Ok(MemorySnapshotCreateResult {
            snapshot: memory_snapshot_from_info(snapshot),
            snapshots: self.memory_snapshots().await,
            stats: self.memory_stats().await,
        })
    }

    pub async fn memory_snapshot_list(&self) -> MemorySnapshotListResult {
        MemorySnapshotListResult {
            snapshots: self.memory_snapshots().await,
        }
    }

    pub async fn memory_snapshot_rollback(
        &self,
        version: u64,
    ) -> anyhow::Result<Option<MemorySnapshotRollbackResult>> {
        let Some(snapshot) = self.memory.rollback_snapshot(version).await? else {
            return Ok(None);
        };
        Ok(Some(MemorySnapshotRollbackResult {
            restored_count: snapshot.entry_count,
            snapshot: memory_snapshot_from_info(snapshot),
            snapshots: self.memory_snapshots().await,
            stats: self.memory_stats().await,
        }))
    }

    pub async fn memory_pending_list(&self, limit: usize) -> Value {
        json!({
            "pending": self
                .memory
                .list_pending(false, limit.max(1))
                .await
                .into_iter()
                .map(pending_memory_to_value)
                .collect::<Vec<_>>()
        })
    }

    pub async fn memory_pending_approve(&self, id: String) -> Value {
        match self
            .memory
            .approve_pending(&id, PendingMemoryEdit::default())
            .await
        {
            Some((pending, memory)) => {
                self.schedule_memory_embedding_refresh(memory.clone()).await;
                json!({
                    "id": id,
                    "pending": pending_memory_to_value(pending),
                    "memory": memory_item_from_entry(memory),
                    "stats": self.memory_stats().await
                })
            }
            None => json!({ "id": id, "error": "pending memory not found" }),
        }
    }

    pub async fn memory_pending_reject(&self, id: String) -> Value {
        match self.memory.reject_pending(&id, None).await {
            Some(pending) => json!({
                "id": id,
                "pending": pending_memory_to_value(pending)
            }),
            None => json!({ "id": id, "error": "pending memory not found" }),
        }
    }

    pub async fn memory_list(&self, limit: usize) -> MemoryListResult {
        let memories = self
            .memory
            .recent(limit.max(1))
            .await
            .into_iter()
            .map(memory_item_from_entry)
            .collect();
        MemoryListResult {
            memories,
            stats: self.memory_stats().await,
        }
    }

    pub async fn memory_search(&self, query: String, limit: usize) -> MemoryListResult {
        let limit = limit.max(1);
        let candidate_limit = limit.saturating_mul(3).min(100).max(limit);
        let embedding_provider = self.current_memory_embedding_provider().await;
        let results = self
            .memory
            .search_with_embedding_provider(
                MemorySearchQuery {
                    text: Some(query.clone()),
                    limit: candidate_limit,
                    ..MemorySearchQuery::default()
                },
                &embedding_provider,
            )
            .await;
        let memories = self
            .rerank_memory_results(&query, results, limit)
            .into_iter()
            .map(|result| memory_item_from_entry(result.entry))
            .collect();
        MemoryListResult {
            memories,
            stats: self.memory_stats().await,
        }
    }

    pub async fn memory_profile_list(&self) -> Value {
        json!({ "files": self.memory_profile_files().await })
    }

    pub async fn memory_profile_read(&self, kind: String) -> Value {
        let files = self.memory_profile_files().await;
        let file = files
            .iter()
            .find(|file| file.kind == kind)
            .cloned()
            .or_else(|| files.first().cloned());
        json!({ "file": file })
    }

    pub async fn memory_profile_update(
        &self,
        kind: String,
        content: String,
    ) -> anyhow::Result<Value> {
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let project_dir = project_dir_from_config(&config).unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| self.storage.config().data_dir.clone())
        });
        let Some(definition) = memory_profile_definitions(&project_dir)
            .into_iter()
            .find(|file| file.kind == kind)
        else {
            anyhow::bail!("unknown memory profile kind: {kind}");
        };
        let context = memory_profile_context(&definition.scope, &project_dir);
        let tags = vec![
            "profile".to_string(),
            definition.kind.clone(),
            definition.scope.clone(),
        ];
        let memories = self.memory.recent(1000).await;
        let existing =
            find_profile_memory(&memories, &definition.kind, &definition.scope, &context);
        if let Some(existing) = existing {
            self.memory
                .update_memory(
                    &existing.id,
                    MemoryUpdate {
                        kind: Some(kind_for_category(&definition.category)),
                        category: Some(definition.category.clone()),
                        content: Some(content),
                        tags: Some(tags),
                        importance: Some(definition.importance),
                    },
                )
                .await;
        } else {
            self.memory
                .add_memory(NewMemoryEntry {
                    kind: kind_for_category(&definition.category),
                    category: definition.category.clone(),
                    content,
                    tags,
                    context: Some(context),
                    importance: definition.importance,
                })
                .await;
        }
        let files = self.memory_profile_files().await;
        let fallback_file = definition.clone().into_file();
        let file = files
            .iter()
            .find(|file| file.kind == definition.kind)
            .cloned()
            .unwrap_or(fallback_file);
        Ok(json!({ "file": file, "files": files }))
    }

    pub(crate) async fn memory_profile_files(&self) -> Vec<MemoryProfileFileDto> {
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let project_dir = project_dir_from_config(&config).unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| self.storage.config().data_dir.clone())
        });
        let memories = self.memory.recent(1000).await;
        memory_profile_definitions(&project_dir)
            .into_iter()
            .map(|definition| {
                let context = memory_profile_context(&definition.scope, &project_dir);
                let existing =
                    find_profile_memory(&memories, &definition.kind, &definition.scope, &context);
                let mut file = definition.into_file();
                if let Some(memory) = existing {
                    file.content = memory.content.clone();
                    file.exists = true;
                    file.updated_at = Some(memory.updated_at.timestamp_millis());
                }
                file
            })
            .collect()
    }

    pub(crate) async fn build_memory_profile_context_text(&self) -> Option<String> {
        let files = self.memory_profile_files().await;
        let mut profile_lines = Vec::new();
        for file in files {
            if !file.exists {
                continue;
            }
            let content = file.content.trim();
            if content.is_empty() {
                continue;
            }
            profile_lines.push(format!("[{}] {}\n{}", file.kind, file.title, content));
        }
        if profile_lines.is_empty() {
            None
        } else {
            Some(profile_lines.join("\n\n"))
        }
    }

    pub async fn memory_hit_list(
        &self,
        session_id: String,
        run_id: Option<String>,
        limit: usize,
    ) -> Vec<MemoryHitRecord> {
        self.storage
            .conversation_store()
            .list_memory_hits(&session_id, run_id.as_deref(), limit)
            .await
            .unwrap_or_default()
    }

    pub async fn memory_session_summary_get(&self, session_id: String) -> Value {
        match self.memory.get_session_summary(&session_id).await {
            Some(summary) => json!({ "ok": true, "summary": summary }),
            None => json!({ "ok": false, "reason": "session_summary_not_found" }),
        }
    }

    pub async fn memory_session_summaries_list(&self, limit: usize) -> Value {
        let summaries = self.memory.recent_session_summaries(limit.max(1)).await;
        json!({ "ok": true, "summaries": summaries, "count": summaries.len() })
    }

    pub(crate) async fn memory_stats(&self) -> MemoryStatsDto {
        let stats = self.memory.stats().await;
        MemoryStatsDto {
            total: stats.total_entries,
            categories: stats.by_category,
        }
    }

    pub(crate) async fn memory_snapshots(&self) -> Vec<MemorySnapshotDto> {
        let mut snapshots = self
            .memory
            .list_snapshots()
            .await
            .into_iter()
            .map(memory_snapshot_from_info)
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|s| std::cmp::Reverse(s.version));
        snapshots
    }

    pub(crate) async fn current_memory_embedding_provider(&self) -> MemoryEmbeddingProvider {
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        MemoryEmbeddingProvider::from_app_config(&config)
    }

    pub(crate) async fn schedule_memory_embedding_refresh(&self, _entry: MemoryEntry) {}

    pub(crate) fn schedule_memory_embedding_refresh_with_provider(
        &self,
        _entry: MemoryEntry,
        _provider: MemoryEmbeddingProvider,
    ) {
    }

    pub(crate) fn rerank_memory_results(
        &self,
        _query: &str,
        results: Vec<MemorySearchResult>,
        limit: usize,
    ) -> Vec<MemorySearchResult> {
        results.into_iter().take(limit).collect()
    }
}
