//! Session list / rewind / participant APIs for AppCore.

use crate::{
    compact_context_line, mark_lazy_sections_for_refresh, normalize_rewind_mode,
    parse_lazy_context_from_snapshot, user_turn_order, AppCore,
};
use chrono::Utc;
use serde_json::{json, Value};
use xuanyu_protocol::{ChatMessage, SessionId, SessionInfo};
use xuanyu_governance::RestoreWorkspaceInput;

impl AppCore {
    pub async fn session_history(&self, session_id: SessionId) -> anyhow::Result<Vec<ChatMessage>> {
        self.sessions.history(&session_id).await
    }

    pub async fn session_reset_context(&self, session_id: SessionId) -> anyhow::Result<Value> {
        let history = self
            .session_history(session_id.clone())
            .await
            .unwrap_or_default();
        let store = self.storage.conversation_store();
        let segments = store.list_segments(&session_id.0).await.unwrap_or_default();
        let compact = if history.is_empty() {
            None
        } else {
            Some(
                self.context_compact(session_id.clone(), Some("autocompact".to_string()))
                    .await,
            )
        };
        let session = self
            .sessions
            .replace_history_records(&session_id, Vec::new())
            .await?;
        store.replace_segments(&session_id.0, Vec::new()).await?;
        let status = self.context_status(session_id.clone()).await;
        Ok(json!({
            "ok": true,
            "sessionId": session_id.0,
            "messageCountBefore": history.len(),
            "segmentCountBefore": segments.len(),
            "compact": compact,
            "status": status,
            "session": session
        }))
    }

    pub async fn session_rename(
        &self,
        session_id: SessionId,
        name: String,
    ) -> anyhow::Result<Option<SessionInfo>> {
        self.sessions.rename_session(&session_id, name).await
    }

    pub async fn session_pin(
        &self,
        session_id: SessionId,
        pinned: bool,
    ) -> anyhow::Result<Option<SessionInfo>> {
        self.sessions.pin_session(&session_id, pinned).await
    }

    pub async fn session_archive(
        &self,
        session_id: SessionId,
        archived: bool,
    ) -> anyhow::Result<bool> {
        self.sessions.archive_session(&session_id, archived).await
    }

    pub async fn session_delete(&self, session_id: SessionId) -> anyhow::Result<bool> {
        let deleted = self.sessions.delete_session(&session_id).await?;
        
        Ok(deleted)
    }

    pub async fn session_abort(&self, session_id: SessionId) -> Value {
        self.reject_pending_questions_for_session(&session_id.0, "session aborted")
            .await;
        let session = match self.sessions.abort_session(&session_id).await {
            Ok(session) => session,
            Err(err) => {
                return json!({
                    "ok": false,
                    "sessionId": session_id.0,
                    "error": err.to_string()
                });
            }
        };
        let cancelled_run = if session.is_some() {
            match self
                .runs
                .latest_running_run_for_session(&session_id.0)
                .await
            {
                Some(run) => match self
                    .runs
                    .cancel_run(&run.id, Some("User aborted the run".to_string()))
                    .await
                {
                    Ok(cancelled) => Some(cancelled),
                    Err(err) => {
                        return json!({
                            "ok": false,
                            "sessionId": session_id.0,
                            "session": session,
                            "runId": run.id,
                            "error": err.to_string()
                        });
                    }
                },
                None => None,
            }
        } else {
            None
        };
        json!({
            "ok": session.is_some(),
            "sessionId": session_id.0,
            "session": session,
            "cancelledRun": cancelled_run
        })
    }

    pub async fn session_participants_list(&self, session_id: SessionId) -> Value {
        json!({
            "sessionId": session_id.0,
            "participants": self.sessions.list_participants(&session_id).await
        })
    }

    pub async fn session_threads_list(&self, session_id: SessionId) -> Value {
        json!({
            "sessionId": session_id.0,
            "threads": self.sessions.list_threads(&session_id).await
        })
    }

    pub async fn session_isolation_get(&self, session_id: SessionId) -> Value {
        json!({
            "sessionId": session_id.0,
            "isolation": self.sessions.isolation_scope(&session_id).await
        })
    }

    pub async fn session_rewind_preview(&self, session_id: SessionId) -> Value {
        let messages = self
            .storage
            .conversation_store()
            .list_messages(&session_id.0)
            .await
            .unwrap_or_default();
        let snapshots = self
            .storage
            .conversation_store()
            .list_snapshots(&session_id.0)
            .await
            .unwrap_or_default();
        let turn_order = user_turn_order(&messages);
        let targets = turn_order
            .iter()
            .map(|turn_id| {
                let checkpoint = snapshots
                    .iter()
                    .find(|snapshot| snapshot.checkpoint_id.as_deref() == Some(turn_id.as_str()));
                let user_message = messages.iter().find(|message| {
                    message.role == "user" && message.turn_id.as_deref() == Some(turn_id)
                });
                let checkpoint_workspace = self
                    .turn_ledger
                    .load_workspace_checkpoint(&session_id.0, turn_id)
                    .ok()
                    .flatten();
                let files = self
                    .turn_ledger
                    .files_changed_after_turn(&session_id.0, &turn_order, turn_id)
                    .unwrap_or_default()
                    .into_iter()
                    .chain(
                        checkpoint_workspace
                            .as_ref()
                            .map(|checkpoint| checkpoint.files.clone())
                            .unwrap_or_default(),
                    )
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let has_workspace_changes = !files.is_empty()
                    || checkpoint_workspace
                        .as_ref()
                        .map(|checkpoint| checkpoint.has_changes)
                        .unwrap_or(false);
                json!({
                    "turnId": turn_id,
                    "userMessagePreview": user_message
                        .map(|message| compact_context_line(&message.content, 240))
                        .or_else(|| checkpoint.as_ref().and_then(|snapshot| snapshot.summary.clone()))
                        .unwrap_or_else(|| turn_id.clone()),
                    "createdAt": user_message
                        .map(|message| message.created_at.timestamp_millis())
                        .or_else(|| checkpoint.as_ref().map(|snapshot| snapshot.created_at.timestamp_millis()))
                        .unwrap_or_else(|| Utc::now().timestamp_millis()),
                    "filesToRevert": files,
                    "hasWorkspaceChanges": has_workspace_changes
                })
            })
            .collect::<Vec<_>>();
        json!({ "sessionId": session_id.0, "targets": targets })
    }

    pub async fn session_rewind(
        &self,
        session_id: SessionId,
        turn_id: String,
        mode: Option<String>,
        new_content: Option<String>,
    ) -> anyhow::Result<Value> {
        let mode = normalize_rewind_mode(mode.as_deref());
        let store = self.storage.conversation_store();
        let messages = store.list_messages(&session_id.0).await?;
        let Some(target_message) = messages
            .iter()
            .find(|message| message.role == "user" && message.turn_id.as_deref() == Some(&turn_id))
            .cloned()
        else {
            anyhow::bail!("rewind target turn not found: {turn_id}");
        };
        let turn_order = user_turn_order(&messages);
        let snapshots = store
            .list_snapshots(&session_id.0)
            .await
            .unwrap_or_default();
        let checkpoint = snapshots
            .iter()
            .find(|snapshot| snapshot.checkpoint_id.as_deref() == Some(turn_id.as_str()))
            .cloned();
        let rewind_cutoff_ms = checkpoint
            .as_ref()
            .map(|snapshot| snapshot.created_at.timestamp_millis())
            .unwrap_or_else(|| target_message.created_at.timestamp_millis());
        let cancelled_tasks = self
            .cancel_subagent_tasks_after(&session_id.0, rewind_cutoff_ms)
            .await;

        let mut workspace_restored = false;
        let mut files_reverted = Vec::<String>::new();
        let mut conflicts = Vec::<Value>::new();
        if mode == "both" || mode == "workspace" {
            // 安全回退：只用 turn ledger 逐文件恢复（带冲突检测，绝不改动用户未提交但与
            // 本 turn 无关的文件，也绝不用 git reset/restore）。checkpoint 属于兜底/审计，
            // 不作为「编辑重发」的直接回滚路径。
            let ledger_result =
                self.turn_ledger
                    .restore_workspace_from_ledger(RestoreWorkspaceInput {
                        session_id: &session_id.0,
                        target_turn_id: &turn_id,
                        turn_order: &turn_order,
                    })?;
            workspace_restored = !ledger_result.restored.is_empty()
                || !ledger_result.deleted.is_empty()
                || !ledger_result.conflicts.is_empty();
            files_reverted = ledger_result
                .restored
                .into_iter()
                .chain(ledger_result.deleted)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            conflicts = ledger_result
                .conflicts
                .into_iter()
                .map(|conflict| json!({ "path": conflict.path, "reason": conflict.reason }))
                .collect();
        }

        let mut removed_message_count = 0usize;
        let mut removed_segment_count = 0usize;
        if mode == "both" || mode == "conversation" {
            // 按 turn 时间裁剪活表；不从 snapshot 嵌套全量 messages 恢复。
            let kept_messages = messages
                .iter()
                .filter(|message| message.created_at < target_message.created_at)
                .cloned()
                .collect::<Vec<_>>();
            removed_message_count = messages.len().saturating_sub(kept_messages.len());
            self.sessions
                .replace_history_records(&session_id, kept_messages)
                .await?;

            let segments = store.list_segments(&session_id.0).await.unwrap_or_default();
            let kept_segments = segments
                .iter()
                .filter(|segment| segment.created_at < target_message.created_at)
                .cloned()
                .collect::<Vec<_>>();
            removed_segment_count = segments.len().saturating_sub(kept_segments.len());
            store.replace_segments(&session_id.0, kept_segments).await?;
        }

        if mode == "both" || mode == "conversation" || mode == "workspace" {
            let _ = self
                .turn_ledger
                .delete_after(&session_id.0, &turn_id, &turn_order);
            let _ = self.turn_ledger.delete_workspace_checkpoints_after(
                &session_id.0,
                &turn_id,
                &turn_order,
            );
            if let Some(start) = turn_order.iter().position(|item| item == &turn_id) {
                let _ = store
                    .delete_snapshots_for_turns(&session_id.0, &turn_order[start..])
                    .await;
            }
        }

        if let Some(ref checkpoint) = checkpoint {
            if let Some(mut lazy) = parse_lazy_context_from_snapshot(checkpoint) {
                mark_lazy_sections_for_refresh(&mut lazy);
                self.set_lazy_context_override(&session_id, lazy).await;
            }
        }

        let resent_turn_id = if mode != "workspace" {
            let content = new_content.unwrap_or_default();
            if content.trim().is_empty() {
                None
            } else {
                Some(
                    self.send_message(session_id.clone(), content.trim().to_string())
                        .await?
                        .0,
                )
            }
        } else {
            None
        };

        Ok(json!({
            "sessionId": session_id.0,
            "rewound": {
                "turnId": turn_id,
                "mode": mode,
                "removedMessageCount": removed_message_count,
                "removedSegmentCount": removed_segment_count,
                "workspaceRestored": workspace_restored,
                "filesReverted": files_reverted,
                "conflicts": conflicts,
                "cancelledTasks": cancelled_tasks,
                "resentTurnId": resent_turn_id
            }
        }))
    }
}
