//! Thin IPC facade for Electron ↔ Rust engine (no Tauri).
//! Command dispatch is generated into `dispatch_gen.inc` by
//! `scripts/gen-desktop-dispatch.py`.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use xuanyu_core::{
    AppCore, ConfigInfoDto, GatewayLogEntryDto, InitModelsFetchInput, InitModelsFetchResult,
    MarketplaceSearchInput, MemoryCreateInput, MemoryListResult, MemoryMutationResult,
    MemoryUpdateInput, SystemStatusDto, ToolCatalogEntry,
};
use xuanyu_protocol::{
    AppStatus, ChatAttachment, ChatMessage, SessionId, SessionInfo, TurnId,
};
use xuanyu_storage::StorageConfig;

pub struct CoreState(pub Arc<AppCore>);

pub fn create_app_core(data_dir: PathBuf) -> Arc<AppCore> {
    Arc::new(AppCore::new(StorageConfig::for_data_dir(data_dir)))
}

async fn app_init(core: &AppCore) -> Result<AppStatus, String> {
    core.initialize().await.map_err(|err| err.to_string())
}

async fn app_status(core: &AppCore) -> Result<AppStatus, String> {
    Ok(core.status().await)
}

async fn session_create(
    core: &AppCore,
    mode: Option<String>,
    title: Option<String>,
    project_dir: Option<String>,
) -> Result<SessionInfo, String> {
    core.create_session(mode, title, project_dir)
        .await
        .map_err(|err| err.to_string())
}

async fn session_list(
    core: &AppCore,
    include_archived: Option<bool>,
    search: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SessionInfo>, String> {
    Ok(core
        .query_sessions(
            include_archived.unwrap_or(false),
            search,
            limit.unwrap_or(500),
        )
        .await)
}

async fn session_history(core: &AppCore, session_id: String) -> Result<Vec<ChatMessage>, String> {
    core.session_history(SessionId(session_id))
        .await
        .map_err(|err| err.to_string())
}

async fn session_reset_context(core: &AppCore, session_id: String) -> Result<Value, String> {
    core.session_reset_context(SessionId(session_id))
        .await
        .map_err(|err| err.to_string())
}

async fn conversation_search(
    core: &AppCore,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<SessionInfo>, String> {
    Ok(core.conversation_search(query, limit.unwrap_or(50)).await)
}

async fn session_rename(
    core: &AppCore,
    session_id: String,
    name: String,
) -> Result<Option<SessionInfo>, String> {
    core.session_rename(SessionId(session_id), name)
        .await
        .map_err(|err| err.to_string())
}

async fn session_archive(
    core: &AppCore,
    session_id: String,
    archived: bool,
) -> Result<bool, String> {
    core.session_archive(SessionId(session_id), archived)
        .await
        .map_err(|err| err.to_string())
}

async fn session_delete(core: &AppCore, session_id: String) -> Result<bool, String> {
    core.session_delete(SessionId(session_id))
        .await
        .map_err(|err| err.to_string())
}

async fn session_abort(core: &AppCore, session_id: String) -> Result<Value, String> {
    Ok(core.session_abort(SessionId(session_id)).await)
}

async fn session_participants_list(core: &AppCore, session_id: String) -> Result<Value, String> {
    Ok(core
        .session_participants_list(SessionId(session_id))
        .await)
}

async fn session_threads_list(core: &AppCore, session_id: String) -> Result<Value, String> {
    Ok(core.session_threads_list(SessionId(session_id)).await)
}

async fn session_isolation_get(core: &AppCore, session_id: String) -> Result<Value, String> {
    Ok(core.session_isolation_get(SessionId(session_id)).await)
}

async fn session_send_message(
    core: &AppCore,
    session_id: String,
    content: String,
    attachments: Option<Vec<ChatAttachment>>,
    deep_thinking: Option<bool>,
) -> Result<TurnId, String> {
    core.send_message_with_attachments(
        SessionId(session_id),
        content,
        attachments,
        deep_thinking.unwrap_or(false),
    )
    .await
    .map_err(|err| err.to_string())
}

async fn session_rewind_preview(core: &AppCore, session_id: String) -> Result<Value, String> {
    Ok(core.session_rewind_preview(SessionId(session_id)).await)
}

async fn session_rewind(
    core: &AppCore,
    session_id: String,
    turn_id: String,
    mode: Option<String>,
    new_content: Option<String>,
) -> Result<Value, String> {
    core.session_rewind(SessionId(session_id), turn_id, mode, new_content)
        .await
        .map_err(|err| err.to_string())
}

async fn memory_create(
    core: &AppCore,
    content: String,
    category: Option<String>,
    tags: Option<Vec<String>>,
    context: Option<String>,
    importance: Option<u8>,
) -> Result<MemoryMutationResult, String> {
    core.memory_create(MemoryCreateInput {
        content,
        category: category.unwrap_or_else(|| "note".to_string()),
        tags: tags.unwrap_or_default(),
        context,
        importance: importance.unwrap_or(3),
    })
    .await
    .map_err(|err| err.to_string())
}

async fn memory_list(core: &AppCore, limit: Option<usize>) -> Result<MemoryListResult, String> {
    Ok(core.memory_list(limit.unwrap_or(100)).await)
}

async fn memory_search(
    core: &AppCore,
    query: String,
    limit: Option<usize>,
) -> Result<MemoryListResult, String> {
    Ok(core.memory_search(query, limit.unwrap_or(20)).await)
}

async fn memory_update(
    core: &AppCore,
    id: String,
    content: Option<String>,
    category: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<Option<MemoryMutationResult>, String> {
    core.memory_update(
        &id,
        MemoryUpdateInput {
            content,
            category,
            tags,
            importance: None,
        },
    )
    .await
    .map_err(|err| err.to_string())
}

async fn memory_delete(core: &AppCore, id: String) -> Result<bool, String> {
    Ok(core.memory_delete(&id).await)
}

async fn memory_flush(core: &AppCore) -> Result<Value, String> {
    core.memory_flush().await.map_err(|err| err.to_string())
}

async fn memory_maintenance_run(core: &AppCore) -> Result<Value, String> {
    Ok(core.memory_maintenance_run().await)
}

async fn memory_snapshot_list(core: &AppCore) -> Result<Value, String> {
    serde_json::to_value(core.memory_snapshot_list().await).map_err(|e| e.to_string())
}

async fn memory_snapshot_create(
    core: &AppCore,
    reason: Option<String>,
) -> Result<Value, String> {
    core.memory_snapshot_create(reason)
        .await
        .map_err(|err| err.to_string())
        .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

async fn memory_snapshot_rollback(core: &AppCore, version: u64) -> Result<Value, String> {
    core.memory_snapshot_rollback(version)
        .await
        .map_err(|err| err.to_string())
        .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

async fn memory_pending_list(core: &AppCore, limit: Option<usize>) -> Result<Value, String> {
    Ok(core.memory_pending_list(limit.unwrap_or(100)).await)
}

async fn memory_pending_approve(core: &AppCore, id: String) -> Result<Value, String> {
    Ok(core.memory_pending_approve(id).await)
}

async fn memory_pending_reject(core: &AppCore, id: String) -> Result<Value, String> {
    Ok(core.memory_pending_reject(id).await)
}

async fn memory_profile_list(core: &AppCore) -> Result<Value, String> {
    Ok(core.memory_profile_list().await)
}

async fn memory_profile_read(core: &AppCore, kind: String) -> Result<Value, String> {
    Ok(core.memory_profile_read(kind).await)
}

async fn memory_profile_update(
    core: &AppCore,
    kind: String,
    content: String,
) -> Result<Value, String> {
    core.memory_profile_update(kind, content)
        .await
        .map_err(|err| err.to_string())
}

async fn memory_hit_list(
    core: &AppCore,
    session_id: String,
    run_id: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<xuanyu_storage::MemoryHitRecord>, String> {
    Ok(core
        .memory_hit_list(session_id, run_id, limit.unwrap_or(50))
        .await)
}

async fn memory_session_summary_get(core: &AppCore, session_id: String) -> Result<Value, String> {
    Ok(core.memory_session_summary_get(session_id).await)
}

async fn memory_session_summaries_list(
    core: &AppCore,
    limit: Option<usize>,
) -> Result<Value, String> {
    Ok(core
        .memory_session_summaries_list(limit.unwrap_or(20))
        .await)
}

async fn tool_catalog(core: &AppCore) -> Result<Vec<ToolCatalogEntry>, String> {
    Ok(core.tool_catalog().await)
}

async fn tool_execute(
    core: &AppCore,
    name: String,
    args: Option<serde_json::Map<String, Value>>,
    cwd: Option<PathBuf>,
    session_id: Option<String>,
) -> Result<Value, String> {
    let mapped_args = args
        .unwrap_or_default()
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let output = core
        .tool_execute(xuanyu_core::ToolExecuteInput {
            name,
            args: mapped_args,
            cwd,
            session_id,
        })
        .await;
    serde_json::to_value(output).map_err(|e| e.to_string())
}

async fn permission_list(
    core: &AppCore,
    session_id: Option<String>,
) -> Result<Value, String> {
    Ok(core.permission_list(session_id).await)
}

async fn config_get(core: &AppCore) -> Result<Value, String> {
    // 本机桌面端需要完整配置（含密钥），否则设置页无法查看/修改。
    core.config_get().await.map_err(|err| err.to_string())
}

async fn config_info(core: &AppCore) -> Result<ConfigInfoDto, String> {
    Ok(core.config_info().await)
}

async fn config_update(core: &AppCore, partial: Value) -> Result<Value, String> {
    core.config_update(partial)
        .await
        .map_err(|err| err.to_string())
}

async fn config_capabilities(core: &AppCore) -> Result<Value, String> {
    Ok(core.config_capabilities().await)
}

async fn init_models_fetch(
    core: &AppCore,
    provider_type: String,
    base_url: Option<String>,
    api_key: Option<String>,
    protocol: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<InitModelsFetchResult, String> {
    Ok(core
        .init_models_fetch(InitModelsFetchInput {
            provider_type,
            base_url,
            api_key,
            protocol,
            timeout_ms,
        })
        .await)
}

async fn init_postgres_status(
    core: &AppCore,
    connection_string: Option<String>,
) -> Result<Value, String> {
    Ok(core.init_postgres_status(connection_string).await)
}

async fn init_postgres_test_connection(
    core: &AppCore,
    connection_string: String,
) -> Result<Value, String> {
    Ok(core
        .init_postgres_test_connection(connection_string)
        .await)
}

async fn init_postgres_start(core: &AppCore) -> Result<Value, String> {
    core.init_postgres_start()
        .await
        .map_err(|err| err.to_string())
}

async fn storage_get_setup(core: &AppCore) -> Result<Value, String> {
    Ok(core.storage_get_setup().await)
}

async fn storage_test_external_connection(
    core: &AppCore,
    connection_string: String,
) -> Result<Value, String> {
    Ok(core.storage_test_external_connection(connection_string).await)
}

async fn storage_save_external(
    core: &AppCore,
    connection_string: String,
) -> Result<Value, String> {
    Ok(core.storage_save_external(connection_string).await)
}

async fn init_channel_plugins_list(core: &AppCore) -> Result<Value, String> {
    Ok(core.init_channel_plugins_list())
}

async fn init_channel_plugin_install(core: &AppCore, plugin_id: String) -> Result<Value, String> {
    Ok(core.init_channel_plugin_install(plugin_id).await)
}

async fn system_status(core: &AppCore) -> Result<SystemStatusDto, String> {
    Ok(core.system_status().await)
}

async fn gateway_logs(core: &AppCore, limit: Option<usize>) -> Result<Vec<GatewayLogEntryDto>, String> {
    Ok(core.gateway_logs(limit.unwrap_or(200)).await)
}

async fn diagnostics_export(core: &AppCore, limit: Option<usize>) -> Result<Value, String> {
    core.diagnostics_export(limit.unwrap_or(500))
        .await
        .map_err(|err| err.to_string())
}

async fn data_backup_export(core: &AppCore) -> Result<Value, String> {
    core.data_backup_export()
        .await
        .map_err(|err| err.to_string())
}

async fn data_backup_restore(core: &AppCore, backup_path: PathBuf) -> Result<Value, String> {
    core.data_backup_restore(backup_path)
        .await
        .map_err(|err| err.to_string())
}

async fn run_list(core: &AppCore, limit: Option<usize>) -> Result<Value, String> {
    Ok(core.run_list(limit.unwrap_or(50)).await)
}

async fn run_timeline(core: &AppCore, run_id: String) -> Result<Value, String> {
    Ok(core.run_timeline(run_id).await)
}

async fn trace_timeline(core: &AppCore, id: String) -> Result<Value, String> {
    Ok(core.trace_timeline(id).await)
}

async fn run_cancel(
    core: &AppCore,
    run_id: String,
    reason: Option<String>,
) -> Result<Value, String> {
    Ok(core.run_cancel(run_id, reason).await)
}

async fn run_resume(core: &AppCore, run_id: String) -> Result<Value, String> {
    Ok(core.run_resume(run_id).await)
}

async fn run_replay(core: &AppCore, run_id: String) -> Result<Value, String> {
    Ok(core.run_replay(run_id).await)
}

async fn trace_evidence(
    core: &AppCore,
    run_id: String,
    format: Option<String>,
) -> Result<Value, String> {
    Ok(core.trace_evidence(run_id, format).await)
}

async fn trace_compare(
    core: &AppCore,
    base_run_id: String,
    target_run_id: String,
) -> Result<Value, String> {
    Ok(core.trace_compare(base_run_id, target_run_id).await)
}

async fn git_status(core: &AppCore, session_id: Option<String>) -> Result<Value, String> {
    Ok(core.git_status(session_id).await)
}

async fn git_branches(core: &AppCore, session_id: Option<String>) -> Result<Value, String> {
    Ok(core.git_branches(session_id).await)
}

async fn git_checkout(
    core: &AppCore,
    branch: String,
    session_id: Option<String>,
) -> Result<Value, String> {
    Ok(core.git_checkout(branch, session_id).await)
}

async fn workspace_open(
    core: &AppCore,
    path: String,
    session_id: Option<String>,
) -> Result<Value, String> {
    core.open_workspace(path, session_id)
        .await
        .map_err(|err| err.to_string())
}

async fn workspace_clear(core: &AppCore, session_id: String) -> Result<Value, String> {
    core.clear_session_workspace(session_id)
        .await
        .map_err(|err| err.to_string())
}

async fn workspace_create(
    core: &AppCore,
    parent_dir: String,
    name: String,
    session_id: Option<String>,
) -> Result<Value, String> {
    core.create_workspace(parent_dir, name, session_id)
        .await
        .map_err(|err| err.to_string())
}

async fn agent_list(core: &AppCore) -> Result<Value, String> {
    Ok(core.agent_list().await)
}

async fn agent_create(core: &AppCore, payload: Value) -> Result<Value, String> {
    Ok(core.agent_create(payload).await)
}

async fn agent_update(core: &AppCore, id: String, payload: Value) -> Result<Value, String> {
    Ok(core.agent_update(id, payload).await)
}

async fn agent_delete(core: &AppCore, id: String) -> Result<Value, String> {
    Ok(core.agent_delete(id).await)
}

async fn subagent_timeline(
    core: &AppCore,
    session_id: Option<String>,
    task_id: String,
) -> Result<Value, String> {
    Ok(core.subagent_timeline(session_id, task_id).await)
}

async fn task_list(
    core: &AppCore,
    session_id: Option<String>,
    limit: Option<usize>,
) -> Result<Value, String> {
    Ok(core.task_list(session_id, limit.unwrap_or(50)).await)
}

async fn task_create(
    core: &AppCore,
    session_id: Option<String>,
    payload: Value,
) -> Result<Value, String> {
    Ok(core.task_create(session_id, payload).await)
}

async fn subagent_delegate_many(core: &AppCore, payload: Value) -> Result<Value, String> {
    Ok(core.subagent_delegate_many(payload).await)
}

async fn subagent_assess_merge(core: &AppCore, payload: Value) -> Result<Value, String> {
    Ok(core.subagent_assess_merge(payload).await)
}

async fn task_status(core: &AppCore, task_id: String) -> Result<Value, String> {
    Ok(core.task_status(task_id).await)
}

async fn task_control(
    core: &AppCore,
    task_id: String,
    action: String,
    reason: Option<String>,
) -> Result<Value, String> {
    Ok(core.task_control(task_id, action, reason).await)
}

async fn event_replay(core: &AppCore, last_sequence: u64) -> Result<Value, String> {
    serde_json::to_value(core.event_replay(last_sequence).await).map_err(|e| e.to_string())
}

async fn event_dead_letters(core: &AppCore, limit: Option<usize>) -> Result<Value, String> {
    serde_json::to_value(core.event_dead_letters(limit.unwrap_or(50)).await)
        .map_err(|e| e.to_string())
}

async fn control_plane_list_tasks(
    core: &AppCore,
    session_id: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<Value>, String> {
    core.control_plane_list_tasks(session_id, limit.unwrap_or(50))
        .await
        .map_err(|err| err.to_string())
}

async fn rollback_list(core: &AppCore) -> Result<Value, String> {
    Ok(core.rollback_list().await)
}

async fn audit_list(core: &AppCore, limit: Option<usize>) -> Result<Value, String> {
    Ok(core.audit_list(limit).await)
}

async fn rollback_run(core: &AppCore, rollback_id: String) -> Result<Value, String> {
    Ok(core.rollback_run(rollback_id).await)
}

async fn plugin_set_enabled(
    core: &AppCore,
    id: String,
    enabled: bool,
) -> Result<Value, String> {
    core.plugin_set_enabled(id, enabled)
        .await
        .map_err(|err| err.to_string())
}

async fn plugin_install(core: &AppCore, payload: Value) -> Result<Value, String> {
    Ok(core.plugin_install(payload).await)
}

async fn refresh_workers(core: &AppCore) -> Result<Value, String> {
    serde_json::to_value(core.refresh_workers_and_publish()).map_err(|e| e.to_string())
}

async fn extensions_list(core: &AppCore) -> Result<Value, String> {
    Ok(core.extensions_list().await)
}

async fn stats_get(core: &AppCore) -> Result<Value, String> {
    Ok(core.stats_snapshot().await)
}

async fn context_status(core: &AppCore, session_id: String) -> Result<Value, String> {
    Ok(core.context_status(SessionId(session_id)).await)
}

async fn context_compact(
    core: &AppCore,
    session_id: String,
    strategy: Option<String>,
) -> Result<Value, String> {
    Ok(core
        .context_compact(SessionId(session_id), strategy)
        .await)
}

async fn context_rollback(core: &AppCore, session_id: String) -> Result<Value, String> {
    Ok(core.context_rollback(SessionId(session_id)).await)
}

async fn feedback_submit(core: &AppCore, payload: Value) -> Result<Value, String> {
    Ok(core.feedback_submit(payload).await)
}

async fn process_list(core: &AppCore) -> Result<Value, String> {
    Ok(core.process_list().await)
}

async fn process_read(
    core: &AppCore,
    process_id: String,
    offset: Option<u64>,
) -> Result<Value, String> {
    Ok(core.process_read(process_id, offset.unwrap_or(0)).await)
}

async fn process_stop(core: &AppCore, process_id: String) -> Result<Value, String> {
    Ok(core.process_stop(process_id).await)
}

async fn team_status(core: &AppCore, session_id: Option<String>) -> Result<Value, String> {
    Ok(core.team_status(session_id).await)
}

async fn session_pin(
    core: &AppCore,
    session_id: String,
    pinned: bool,
) -> Result<Option<SessionInfo>, String> {
    core.session_pin(SessionId(session_id), pinned)
        .await
        .map_err(|err| err.to_string())
}

async fn workspace_file_reject(
    core: &AppCore,
    session_id: String,
    turn_id: String,
    path: String,
) -> Result<Value, String> {
    Ok(core
        .workspace_file_reject(session_id, turn_id, path)
        .await)
}

async fn marketplace_search(
    core: &AppCore,
    input: MarketplaceSearchInput,
) -> Result<Value, String> {
    Ok(core.marketplace_search(input).await)
}

async fn marketplace_install_plan(
    core: &AppCore,
    entry_id: String,
    scope: Option<String>,
) -> Result<Value, String> {
    Ok(core.marketplace_install_plan(entry_id, scope).await)
}

async fn marketplace_install_confirm(
    core: &AppCore,
    plan_id: String,
    approval_digest: String,
) -> Result<Value, String> {
    Ok(core
        .marketplace_install_confirm(plan_id, approval_digest)
        .await)
}

async fn marketplace_install_status(core: &AppCore) -> Result<Value, String> {
    Ok(core.marketplace_install_status().await)
}

async fn capability_installed_list(core: &AppCore) -> Result<Value, String> {
    Ok(core.capability_installed_list().await)
}

async fn capability_installed_mutate(
    core: &AppCore,
    entry_id: String,
    action: String,
) -> Result<Value, String> {
    Ok(core.capability_installed_mutate(entry_id, action).await)
}

async fn goal_get(core: &AppCore, session_id: String) -> Result<Value, String> {
    Ok(core.goal_get(session_id).await)
}

async fn goal_action(
    core: &AppCore,
    session_id: String,
    action: String,
    objective: Option<String>,
    token_budget: Option<u64>,
) -> Result<Value, String> {
    Ok(core
        .goal_action(session_id, action, objective, token_budget)
        .await)
}

async fn question_respond(
    core: &AppCore,
    request_id: String,
    answers: Value,
) -> Result<Value, String> {
    Ok(core.question_respond(request_id, answers).await)
}

async fn channel_adapters_list(core: &AppCore) -> Result<Value, String> {
    Ok(core.channel_adapters_list().await)
}

async fn channel_connections_list(core: &AppCore) -> Result<Value, String> {
    Ok(core.channel_connections_list().await)
}

async fn channel_connection_upsert(core: &AppCore, payload: Value) -> Result<Value, String> {
    Ok(core.channel_connection_upsert(payload).await)
}

async fn channel_connection_connect(core: &AppCore, payload: Value) -> Result<Value, String> {
    Ok(core.channel_connection_connect(payload).await)
}

async fn channel_connection_disconnect(core: &AppCore, payload: Value) -> Result<Value, String> {
    Ok(core.channel_connection_disconnect(payload).await)
}

async fn channel_connection_remove(core: &AppCore, payload: Value) -> Result<Value, String> {
    Ok(core.channel_connection_remove(payload).await)
}

async fn channel_qr_begin(core: &AppCore, payload: Value) -> Result<Value, String> {
    Ok(core.channel_qr_begin(payload).await)
}

async fn channel_qr_cancel(core: &AppCore) -> Result<Value, String> {
    Ok(core.channel_qr_cancel().await)
}

async fn channel_delivery_list(core: &AppCore, limit: Option<usize>) -> Result<Value, String> {
    Ok(core.channel_delivery_list(limit).await)
}

include!("dispatch_gen.inc");
