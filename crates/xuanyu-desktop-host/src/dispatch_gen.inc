// Auto-generated — python scripts/gen-desktop-dispatch.py


async fn dispatch_permission_respond(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        request_id: String,
        allowed: bool,
        remember: Option<bool>,
        scope: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let scope = input.scope.or_else(|| input.remember.unwrap_or(false).then(|| "session".to_string()));
    let result = core.permission_respond_with_scope(input.request_id, input.allowed, scope).await;
    serde_json::to_value(result).map_err(|e| e.to_string())
}

async fn dispatch_event_replay_filtered(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        last_sequence: u64,
        filter: serde_json::Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let filter: xuanyu_eventbus::EventFilter =
        serde_json::from_value(input.filter).map_err(|err| err.to_string())?;
    let batch = core.event_replay_filtered(input.last_sequence, filter).await;
    serde_json::to_value(batch).map_err(|err| err.to_string())
}

async fn dispatch_marketplace_search(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        kinds: Option<serde_json::Value>,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        page: Option<u32>,
        #[serde(default)]
        page_size: Option<u32>,
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        source_id: Option<String>,
        #[serde(default)]
        summary_only: Option<bool>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = marketplace_search(
        core,
        MarketplaceSearchInput {
            query: input.query,
            kind: input.kind,
            kinds: input.kinds,
            request_id: input.request_id,
            page: input.page,
            page_size: input.page_size,
            category: input.category,
            source_id: input.source_id,
            summary_only: input.summary_only,
        },
    )
    .await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}


async fn dispatch_app_init(core: &AppCore) -> Result<Value, String> {
    let result: Result<AppStatus, String> = app_init(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_app_status(core: &AppCore) -> Result<Value, String> {
    let result: Result<AppStatus, String> = app_status(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_create(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        project_dir: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<SessionInfo, String> = session_create(core, input.mode, input.title, input.project_dir).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        include_archived: Option<bool>,
        #[serde(default)]
        search: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Vec<SessionInfo>, String> = session_list(core, input.include_archived, input.search, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_history(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Vec<ChatMessage>, String> = session_history(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_reset_context(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = session_reset_context(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_conversation_search(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        query: String,
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Vec<SessionInfo>, String> = conversation_search(core, input.query, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_rename(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
        name: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Option<SessionInfo>, String> = session_rename(core, input.session_id, input.name).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_archive(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
        archived: bool,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<bool, String> = session_archive(core, input.session_id, input.archived).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_delete(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<bool, String> = session_delete(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_abort(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = session_abort(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_participants_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = session_participants_list(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_threads_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = session_threads_list(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_isolation_get(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = session_isolation_get(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_send_message(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
        content: String,
        #[serde(default)]
        attachments: Option<Vec<ChatAttachment>>,
        #[serde(default)]
        deep_thinking: Option<bool>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<TurnId, String> = session_send_message(core, input.session_id, input.content, input.attachments, input.deep_thinking).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_rewind_preview(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = session_rewind_preview(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_rewind(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
        turn_id: String,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        new_content: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = session_rewind(core, input.session_id, input.turn_id, input.mode, input.new_content).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_create(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        content: String,
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        tags: Option<Vec<String>>,
        #[serde(default)]
        context: Option<String>,
        #[serde(default)]
        importance: Option<u8>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<MemoryMutationResult, String> = memory_create(core, input.content, input.category, input.tags, input.context, input.importance).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<MemoryListResult, String> = memory_list(core, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_search(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        query: String,
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<MemoryListResult, String> = memory_search(core, input.query, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_update(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        id: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        tags: Option<Vec<String>>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Option<MemoryMutationResult>, String> = memory_update(core, input.id, input.content, input.category, input.tags).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_delete(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<bool, String> = memory_delete(core, input.id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_flush(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = memory_flush(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_maintenance_run(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = memory_maintenance_run(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_snapshot_list(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = memory_snapshot_list(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_snapshot_create(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        reason: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = memory_snapshot_create(core, input.reason).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_snapshot_rollback(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        version: u64,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = memory_snapshot_rollback(core, input.version).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_pending_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = memory_pending_list(core, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_pending_approve(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = memory_pending_approve(core, input.id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_pending_reject(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = memory_pending_reject(core, input.id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_profile_list(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = memory_profile_list(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_profile_read(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        kind: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = memory_profile_read(core, input.kind).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_profile_update(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        kind: String,
        content: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = memory_profile_update(core, input.kind, input.content).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_hit_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
        #[serde(default)]
        run_id: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Vec<xuanyu_storage::MemoryHitRecord>, String> = memory_hit_list(core, input.session_id, input.run_id, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_session_summary_get(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = memory_session_summary_get(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_memory_session_summaries_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = memory_session_summaries_list(core, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_tool_catalog(core: &AppCore) -> Result<Value, String> {
    let result: Result<Vec<ToolCatalogEntry>, String> = tool_catalog(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_tool_execute(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        name: String,
        #[serde(default)]
        args: Option<serde_json::Map<String, Value>>,
        #[serde(default)]
        cwd: Option<PathBuf>,
        #[serde(default)]
        session_id: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = tool_execute(core, input.name, input.args, input.cwd, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_permission_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        session_id: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = permission_list(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_config_get(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = config_get(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_config_info(core: &AppCore) -> Result<Value, String> {
    let result: Result<ConfigInfoDto, String> = config_info(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_config_update(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        partial: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = config_update(core, input.partial).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_config_capabilities(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = config_capabilities(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_init_models_fetch(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        provider_type: String,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        protocol: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<InitModelsFetchResult, String> = init_models_fetch(core, input.provider_type, input.base_url, input.api_key, input.protocol, input.timeout_ms).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_init_postgres_status(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = init_postgres_status(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_init_postgres_start(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = init_postgres_start(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_storage_get_setup(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = storage_get_setup(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_init_channel_plugins_list(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = init_channel_plugins_list(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_init_channel_plugin_install(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        plugin_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = init_channel_plugin_install(core, input.plugin_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_system_status(core: &AppCore) -> Result<Value, String> {
    let result: Result<SystemStatusDto, String> = system_status(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_gateway_logs(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Vec<GatewayLogEntryDto>, String> = gateway_logs(core, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_diagnostics_export(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = diagnostics_export(core, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_data_backup_export(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = data_backup_export(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_data_backup_restore(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        backup_path: PathBuf,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = data_backup_restore(core, input.backup_path).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_run_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = run_list(core, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_run_timeline(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        run_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = run_timeline(core, input.run_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_trace_timeline(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = trace_timeline(core, input.id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_run_cancel(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        run_id: String,
        #[serde(default)]
        reason: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = run_cancel(core, input.run_id, input.reason).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_run_resume(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        run_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = run_resume(core, input.run_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_run_replay(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        run_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = run_replay(core, input.run_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_trace_evidence(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        run_id: String,
        #[serde(default)]
        format: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = trace_evidence(core, input.run_id, input.format).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_trace_compare(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        base_run_id: String,
        target_run_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = trace_compare(core, input.base_run_id, input.target_run_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_git_status(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        session_id: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = git_status(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_git_branches(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        session_id: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = git_branches(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_git_checkout(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        branch: String,
        #[serde(default)]
        session_id: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = git_checkout(core, input.branch, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_workspace_open(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        path: String,
        #[serde(default)]
        session_id: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = workspace_open(core, input.path, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_workspace_clear(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = workspace_clear(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_workspace_create(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        parent_dir: String,
        name: String,
        #[serde(default)]
        session_id: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = workspace_create(core, input.parent_dir, input.name, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_agent_list(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = agent_list(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_agent_create(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = agent_create(core, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_agent_update(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        id: String,
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = agent_update(core, input.id, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_agent_delete(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = agent_delete(core, input.id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_subagent_timeline(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        session_id: Option<String>,
        task_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = subagent_timeline(core, input.session_id, input.task_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_task_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = task_list(core, input.session_id, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_task_create(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        session_id: Option<String>,
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = task_create(core, input.session_id, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_subagent_delegate_many(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = subagent_delegate_many(core, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_subagent_assess_merge(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = subagent_assess_merge(core, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_task_status(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        task_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = task_status(core, input.task_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_task_control(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        task_id: String,
        action: String,
        #[serde(default)]
        reason: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = task_control(core, input.task_id, input.action, input.reason).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_event_replay(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        last_sequence: u64,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = event_replay(core, input.last_sequence).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_event_dead_letters(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = event_dead_letters(core, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_control_plane_list_tasks(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Vec<Value>, String> = control_plane_list_tasks(core, input.session_id, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_rollback_list(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = rollback_list(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_audit_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = audit_list(core, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_rollback_run(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        rollback_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = rollback_run(core, input.rollback_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_plugin_set_enabled(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        id: String,
        enabled: bool,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = plugin_set_enabled(core, input.id, input.enabled).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_plugin_install(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = plugin_install(core, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_refresh_workers(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = refresh_workers(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_extensions_list(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = extensions_list(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_stats_get(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = stats_get(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_context_status(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = context_status(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_context_compact(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
        #[serde(default)]
        strategy: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = context_compact(core, input.session_id, input.strategy).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_context_rollback(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = context_rollback(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_feedback_submit(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = feedback_submit(core, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_process_list(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = process_list(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_process_read(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        process_id: String,
        #[serde(default)]
        offset: Option<u64>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = process_read(core, input.process_id, input.offset).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_process_stop(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        process_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = process_stop(core, input.process_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_team_status(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        session_id: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = team_status(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_session_pin(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
        pinned: bool,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Option<SessionInfo>, String> = session_pin(core, input.session_id, input.pinned).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_workspace_file_reject(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
        turn_id: String,
        path: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = workspace_file_reject(core, input.session_id, input.turn_id, input.path).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_marketplace_install_plan(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        entry_id: String,
        #[serde(default)]
        scope: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = marketplace_install_plan(core, input.entry_id, input.scope).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_marketplace_install_confirm(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        plan_id: String,
        approval_digest: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = marketplace_install_confirm(core, input.plan_id, input.approval_digest).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_marketplace_install_status(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = marketplace_install_status(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_capability_installed_list(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = capability_installed_list(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_capability_installed_mutate(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        entry_id: String,
        action: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = capability_installed_mutate(core, input.entry_id, input.action).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_goal_get(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = goal_get(core, input.session_id).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_goal_action(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        session_id: String,
        action: String,
        #[serde(default)]
        objective: Option<String>,
        #[serde(default)]
        token_budget: Option<u64>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = goal_action(core, input.session_id, input.action, input.objective, input.token_budget).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_question_respond(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        request_id: String,
        answers: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = question_respond(core, input.request_id, input.answers).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_channel_adapters_list(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = channel_adapters_list(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_channel_connections_list(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = channel_connections_list(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_channel_connection_upsert(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = channel_connection_upsert(core, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_channel_connection_connect(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = channel_connection_connect(core, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_channel_connection_disconnect(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = channel_connection_disconnect(core, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_channel_connection_remove(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = channel_connection_remove(core, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_channel_qr_begin(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        payload: Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = channel_qr_begin(core, input.payload).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_channel_qr_cancel(core: &AppCore) -> Result<Value, String> {
    let result: Result<Value, String> = channel_qr_cancel(core).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
async fn dispatch_channel_delivery_list(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        limit: Option<usize>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = channel_delivery_list(core, input.limit).await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}

pub async fn invoke_command(core: Arc<AppCore>, command: &str, params: Value) -> Result<Value, String> {
    let managed = CoreState(core);
    match command {

        "app_updater_install" => Err("Electron shell handles updates".to_string()),
        "app_updater_relaunch" => Err("Electron shell handles relaunch".to_string()),
        "launch_setup_app" => Err("Electron shell handles setup".to_string()),
        "permission_respond" => dispatch_permission_respond(&managed.0, params).await,
        "event_replay_filtered" => dispatch_event_replay_filtered(&managed.0, params).await,
        "marketplace_search" => dispatch_marketplace_search(&managed.0, params).await,

        "app_init" => dispatch_app_init(&managed.0).await,
        "app_status" => dispatch_app_status(&managed.0).await,
        "session_create" => dispatch_session_create(&managed.0, params).await,
        "session_list" => dispatch_session_list(&managed.0, params).await,
        "session_history" => dispatch_session_history(&managed.0, params).await,
        "session_reset_context" => dispatch_session_reset_context(&managed.0, params).await,
        "conversation_search" => dispatch_conversation_search(&managed.0, params).await,
        "session_rename" => dispatch_session_rename(&managed.0, params).await,
        "session_archive" => dispatch_session_archive(&managed.0, params).await,
        "session_delete" => dispatch_session_delete(&managed.0, params).await,
        "session_abort" => dispatch_session_abort(&managed.0, params).await,
        "session_participants_list" => dispatch_session_participants_list(&managed.0, params).await,
        "session_threads_list" => dispatch_session_threads_list(&managed.0, params).await,
        "session_isolation_get" => dispatch_session_isolation_get(&managed.0, params).await,
        "session_send_message" => dispatch_session_send_message(&managed.0, params).await,
        "session_rewind_preview" => dispatch_session_rewind_preview(&managed.0, params).await,
        "session_rewind" => dispatch_session_rewind(&managed.0, params).await,
        "memory_create" => dispatch_memory_create(&managed.0, params).await,
        "memory_list" => dispatch_memory_list(&managed.0, params).await,
        "memory_search" => dispatch_memory_search(&managed.0, params).await,
        "memory_update" => dispatch_memory_update(&managed.0, params).await,
        "memory_delete" => dispatch_memory_delete(&managed.0, params).await,
        "memory_flush" => dispatch_memory_flush(&managed.0).await,
        "memory_maintenance_run" => dispatch_memory_maintenance_run(&managed.0).await,
        "memory_snapshot_list" => dispatch_memory_snapshot_list(&managed.0).await,
        "memory_snapshot_create" => dispatch_memory_snapshot_create(&managed.0, params).await,
        "memory_snapshot_rollback" => dispatch_memory_snapshot_rollback(&managed.0, params).await,
        "memory_pending_list" => dispatch_memory_pending_list(&managed.0, params).await,
        "memory_pending_approve" => dispatch_memory_pending_approve(&managed.0, params).await,
        "memory_pending_reject" => dispatch_memory_pending_reject(&managed.0, params).await,
        "memory_profile_list" => dispatch_memory_profile_list(&managed.0).await,
        "memory_profile_read" => dispatch_memory_profile_read(&managed.0, params).await,
        "memory_profile_update" => dispatch_memory_profile_update(&managed.0, params).await,
        "memory_hit_list" => dispatch_memory_hit_list(&managed.0, params).await,
        "memory_session_summary_get" => dispatch_memory_session_summary_get(&managed.0, params).await,
        "memory_session_summaries_list" => dispatch_memory_session_summaries_list(&managed.0, params).await,
        "tool_catalog" => dispatch_tool_catalog(&managed.0).await,
        "tool_execute" => dispatch_tool_execute(&managed.0, params).await,
        "permission_list" => dispatch_permission_list(&managed.0, params).await,
        "config_get" => dispatch_config_get(&managed.0).await,
        "config_info" => dispatch_config_info(&managed.0).await,
        "config_update" => dispatch_config_update(&managed.0, params).await,
        "config_capabilities" => dispatch_config_capabilities(&managed.0).await,
        "init_models_fetch" => dispatch_init_models_fetch(&managed.0, params).await,
        "init_postgres_status" => dispatch_init_postgres_status(&managed.0).await,
        "init_postgres_start" => dispatch_init_postgres_start(&managed.0).await,
        "storage_get_setup" => dispatch_storage_get_setup(&managed.0).await,
        "init_channel_plugins_list" => dispatch_init_channel_plugins_list(&managed.0).await,
        "init_channel_plugin_install" => dispatch_init_channel_plugin_install(&managed.0, params).await,
        "system_status" => dispatch_system_status(&managed.0).await,
        "gateway_logs" => dispatch_gateway_logs(&managed.0, params).await,
        "diagnostics_export" => dispatch_diagnostics_export(&managed.0, params).await,
        "data_backup_export" => dispatch_data_backup_export(&managed.0).await,
        "data_backup_restore" => dispatch_data_backup_restore(&managed.0, params).await,
        "run_list" => dispatch_run_list(&managed.0, params).await,
        "run_timeline" => dispatch_run_timeline(&managed.0, params).await,
        "trace_timeline" => dispatch_trace_timeline(&managed.0, params).await,
        "run_cancel" => dispatch_run_cancel(&managed.0, params).await,
        "run_resume" => dispatch_run_resume(&managed.0, params).await,
        "run_replay" => dispatch_run_replay(&managed.0, params).await,
        "trace_evidence" => dispatch_trace_evidence(&managed.0, params).await,
        "trace_compare" => dispatch_trace_compare(&managed.0, params).await,
        "git_status" => dispatch_git_status(&managed.0, params).await,
        "git_branches" => dispatch_git_branches(&managed.0, params).await,
        "git_checkout" => dispatch_git_checkout(&managed.0, params).await,
        "workspace_open" => dispatch_workspace_open(&managed.0, params).await,
        "workspace_clear" => dispatch_workspace_clear(&managed.0, params).await,
        "workspace_create" => dispatch_workspace_create(&managed.0, params).await,
        "agent_list" => dispatch_agent_list(&managed.0).await,
        "agent_create" => dispatch_agent_create(&managed.0, params).await,
        "agent_update" => dispatch_agent_update(&managed.0, params).await,
        "agent_delete" => dispatch_agent_delete(&managed.0, params).await,
        "subagent_timeline" => dispatch_subagent_timeline(&managed.0, params).await,
        "task_list" => dispatch_task_list(&managed.0, params).await,
        "task_create" => dispatch_task_create(&managed.0, params).await,
        "subagent_delegate_many" => dispatch_subagent_delegate_many(&managed.0, params).await,
        "subagent_assess_merge" => dispatch_subagent_assess_merge(&managed.0, params).await,
        "task_status" => dispatch_task_status(&managed.0, params).await,
        "task_control" => dispatch_task_control(&managed.0, params).await,
        "event_replay" => dispatch_event_replay(&managed.0, params).await,
        "event_dead_letters" => dispatch_event_dead_letters(&managed.0, params).await,
        "control_plane_list_tasks" => dispatch_control_plane_list_tasks(&managed.0, params).await,
        "rollback_list" => dispatch_rollback_list(&managed.0).await,
        "audit_list" => dispatch_audit_list(&managed.0, params).await,
        "rollback_run" => dispatch_rollback_run(&managed.0, params).await,
        "plugin_set_enabled" => dispatch_plugin_set_enabled(&managed.0, params).await,
        "plugin_install" => dispatch_plugin_install(&managed.0, params).await,
        "refresh_workers" => dispatch_refresh_workers(&managed.0).await,
        "extensions_list" => dispatch_extensions_list(&managed.0).await,
        "stats_get" => dispatch_stats_get(&managed.0).await,
        "context_status" => dispatch_context_status(&managed.0, params).await,
        "context_compact" => dispatch_context_compact(&managed.0, params).await,
        "context_rollback" => dispatch_context_rollback(&managed.0, params).await,
        "feedback_submit" => dispatch_feedback_submit(&managed.0, params).await,
        "process_list" => dispatch_process_list(&managed.0).await,
        "process_read" => dispatch_process_read(&managed.0, params).await,
        "process_stop" => dispatch_process_stop(&managed.0, params).await,
        "team_status" => dispatch_team_status(&managed.0, params).await,
        "session_pin" => dispatch_session_pin(&managed.0, params).await,
        "workspace_file_reject" => dispatch_workspace_file_reject(&managed.0, params).await,
        "marketplace_install_plan" => dispatch_marketplace_install_plan(&managed.0, params).await,
        "marketplace_install_confirm" => dispatch_marketplace_install_confirm(&managed.0, params).await,
        "marketplace_install_status" => dispatch_marketplace_install_status(&managed.0).await,
        "capability_installed_list" => dispatch_capability_installed_list(&managed.0).await,
        "capability_installed_mutate" => dispatch_capability_installed_mutate(&managed.0, params).await,
        "goal_get" => dispatch_goal_get(&managed.0, params).await,
        "goal_action" => dispatch_goal_action(&managed.0, params).await,
        "question_respond" => dispatch_question_respond(&managed.0, params).await,
        "channel_adapters_list" => dispatch_channel_adapters_list(&managed.0).await,
        "channel_connections_list" => dispatch_channel_connections_list(&managed.0).await,
        "channel_connection_upsert" => dispatch_channel_connection_upsert(&managed.0, params).await,
        "channel_connection_connect" => dispatch_channel_connection_connect(&managed.0, params).await,
        "channel_connection_disconnect" => dispatch_channel_connection_disconnect(&managed.0, params).await,
        "channel_connection_remove" => dispatch_channel_connection_remove(&managed.0, params).await,
        "channel_qr_begin" => dispatch_channel_qr_begin(&managed.0, params).await,
        "channel_qr_cancel" => dispatch_channel_qr_cancel(&managed.0).await,
        "channel_delivery_list" => dispatch_channel_delivery_list(&managed.0, params).await,
        other => Err(format!("unknown command: {other}")),
    }
}
