//! Marketplace / capability / session-goal AppCore APIs.

use crate::marketplace;
use crate::session_goal;
use crate::{project_dir_from_config, AppCore, ExtensionLoadConfig};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use xuanyu_storage::project_data_root;

pub struct MarketplaceSearchInput {
    pub query: Option<String>,
    pub kind: Option<String>,
    pub kinds: Option<Value>,
    pub request_id: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub category: Option<String>,
    pub source_id: Option<String>,
    pub summary_only: Option<bool>,
}

impl AppCore {
    pub async fn marketplace_search(&self, input: MarketplaceSearchInput) -> Value {
        let MarketplaceSearchInput {
            query,
            kind,
            kinds,
            request_id,
            page,
            page_size,
            category,
            source_id,
            summary_only,
        } = input;
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let project_dir = project_dir_from_config(&config)
            .unwrap_or_else(|| self.storage.config().data_dir.clone());
        let kinds = kinds.and_then(|value| {
            value.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        });
        marketplace::marketplace_search_async(marketplace::MarketplaceSearchParams {
            project_dir,
            config: &config,
            query: query.as_deref(),
            kind: kind.as_deref(),
            kinds,
            request_id,
            page: page.map(|v| v as usize),
            page_size: page_size.map(|v| v as usize),
            category: category.as_deref(),
            source_id: source_id.as_deref(),
            summary_only: summary_only.unwrap_or(false),
        })
        .await
    }

    pub async fn marketplace_install_plan(
        &self,
        entry_id: String,
        scope: Option<String>,
    ) -> Value {
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let project_dir = project_dir_from_config(&config)
            .unwrap_or_else(|| self.storage.config().data_dir.clone());
        match marketplace::marketplace_install_plan(
            project_dir,
            &config,
            &entry_id,
            scope.as_deref(),
        )
        .await
        {
            Ok(plan) => plan,
            Err(err) => json!({ "ok": false, "error": err.to_string() }),
        }
    }

    pub async fn marketplace_install_confirm(
        &self,
        plan_id: String,
        approval_digest: String,
    ) -> Value {
        let plan = match marketplace::marketplace_install_confirm_plan(&plan_id, &approval_digest) {
            Ok(plan) => plan,
            Err(err) => {
                return marketplace::failed_operation("install", &plan_id, &err);
            }
        };
        let entry = plan.get("entry").cloned().unwrap_or_else(|| json!({}));
        let install_result = self.plugin_install(entry.clone()).await;
        if install_result.get("ok").and_then(Value::as_bool) == Some(true) {
            let config = self.config_get().await.unwrap_or_else(|_| json!({}));
            let project_dir = project_dir_from_config(&config)
                .unwrap_or_else(|| self.storage.config().data_dir.clone());
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("plugin");
            let kind = entry
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("plugin");
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(id);
            let version = entry
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("0.0.0");
            let digest = entry
                .pointer("/provenance/digest")
                .and_then(Value::as_str)
                .unwrap_or(id);
            let path = install_result
                .get("installedDir")
                .and_then(Value::as_str)
                .unwrap_or("");
            let scope = plan
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("user");
            let _ = marketplace::upsert_installed_capability(
                marketplace::UpsertInstalledCapabilityInput {
                    project_dir: &project_dir,
                    id,
                    kind,
                    name,
                    version,
                    digest,
                    enabled: true,
                    scope,
                    path,
                    previous_artifact_path: None,
                },
            );
        }
        marketplace::marketplace_install_record(&plan, &install_result)
    }

    pub async fn marketplace_install_status(&self) -> Value {
        marketplace::marketplace_install_status()
    }

    pub async fn capability_installed_list(&self) -> Value {
        let extensions = self.extensions_list().await;
        marketplace::capability_installed_list(&extensions)
    }

    pub async fn capability_installed_mutate(
        &self,
        entry_id: String,
        action: String,
    ) -> Value {
        match action.as_str() {
            "enable" => match self.plugin_set_enabled(entry_id.clone(), true).await {
                Ok(_) => {
                    let extensions = self.extensions_list().await;
                    marketplace::capability_installed_mutate(
                        &extensions,
                        &entry_id,
                        "enable",
                        true,
                        None,
                    )
                }
                Err(err) => marketplace::failed_operation("enable", &entry_id, &err.to_string()),
            },
            "disable" => match self.plugin_set_enabled(entry_id.clone(), false).await {
                Ok(_) => {
                    let extensions = self.extensions_list().await;
                    marketplace::capability_installed_mutate(
                        &extensions,
                        &entry_id,
                        "disable",
                        true,
                        None,
                    )
                }
                Err(err) => marketplace::failed_operation("disable", &entry_id, &err.to_string()),
            },
            "uninstall" => {
                let config = self.config_get().await.unwrap_or_else(|_| json!({}));
                let project_dir = project_dir_from_config(&config)
                    .unwrap_or_else(|| self.storage.config().data_dir.clone());
                match self.uninstall_capability(&project_dir, &entry_id).await {
                    Ok(()) => {
                        let extensions = self.extensions_list().await;
                        marketplace::capability_installed_mutate(
                            &extensions,
                            &entry_id,
                            "uninstall",
                            true,
                            None,
                        )
                    }
                    Err(err) => marketplace::failed_operation("uninstall", &entry_id, &err.to_string()),
                }
            }
            "rollback" => {
                let config = self.config_get().await.unwrap_or_else(|_| json!({}));
                let project_dir = project_dir_from_config(&config)
                    .unwrap_or_else(|| self.storage.config().data_dir.clone());
                match self.rollback_capability(&project_dir, &entry_id).await {
                    Ok(()) => {
                        let extensions = self.extensions_list().await;
                        marketplace::capability_installed_mutate(
                            &extensions,
                            &entry_id,
                            "rollback",
                            true,
                            None,
                        )
                    }
                    Err(err) => marketplace::failed_operation("rollback", &entry_id, &err.to_string()),
                }
            }
            other => marketplace::failed_operation(other, &entry_id, "Unknown action"),
        }
    }

    pub(crate) async fn uninstall_capability(&self, project_dir: &Path, entry_id: &str) -> anyhow::Result<()> {
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let extension_config = ExtensionLoadConfig::from_app_config(project_dir.to_path_buf(), &config);
        let target = extension_config
            .plugin_directories
            .iter()
            .map(|root| root.join(entry_id))
            .find(|path| path.is_dir())
            .or_else(|| {
                let fallback = project_data_root(project_dir).join("plugins").join(entry_id);
                fallback.is_dir().then_some(fallback)
            })
            .or_else(|| {
                let fallback = self.storage.config().data_dir.join("plugins").join(entry_id);
                fallback.is_dir().then_some(fallback)
            });
        let Some(target) = target else {
            anyhow::bail!("Installed plugin directory not found for {entry_id}");
        };
        let state_dir = project_data_root(project_dir);
        let data_plugins = self.storage.config().data_dir.join("plugins");
        if !(marketplace::is_path_inside(&target, &state_dir)
            || marketplace::is_path_inside(&target, &data_plugins)
            || extension_config
                .plugin_directories
                .iter()
                .any(|root| marketplace::is_path_inside(&target, root)))
        {
            anyhow::bail!("Refusing to uninstall plugin outside managed directories");
        }
        let _ = marketplace::backup_installed_directory(
            project_dir,
            entry_id,
            "pre-uninstall",
            &target,
        );
        fs::remove_dir_all(&target)?;
        let _ = marketplace::remove_lock_record(project_dir, entry_id);
        Ok(())
    }

    pub(crate) async fn rollback_capability(&self, project_dir: &Path, entry_id: &str) -> anyhow::Result<()> {
        let mut lock = marketplace::read_capabilities_lock(project_dir);
        let (record, previous) =
            marketplace::extract_previous_from_lock(&mut lock, entry_id)?;
        let previous_path = previous
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Rollback artifact path missing"))?;
        let active_path = record
            .get("path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| project_data_root(project_dir).join("plugins").join(entry_id));
        marketplace::restore_installed_directory(&active_path, Path::new(previous_path))?;
        let version = previous
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("0.0.0");
        let digest = previous
            .get("digest")
            .and_then(Value::as_str)
            .unwrap_or(entry_id);
        let enabled = previous
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        marketplace::apply_lock_rollback_record_in_memory(
            &mut lock,
            entry_id,
            version,
            digest,
            enabled,
        )?;
        marketplace::write_capabilities_lock(project_dir, &lock)?;
        Ok(())
    }

    pub async fn goal_get(&self, session_id: String) -> Value {
        let root = self.resolve_workspace_dir(Some(session_id.as_str())).await;
        session_goal::SessionGoalStore::new(root).wire_snapshot(&session_id)
    }

    pub async fn goal_action(
        &self,
        session_id: String,
        action: String,
        objective: Option<String>,
        token_budget: Option<u64>,
    ) -> Value {
        let root = self.resolve_workspace_dir(Some(session_id.as_str())).await;
        let store = session_goal::SessionGoalStore::new(root);
        if action == "create" || action == "set" {
            let Some(text) = objective.as_deref().map(str::trim).filter(|value| !value.is_empty()) else {
                return json!({ "ok": false, "error": "objective required" });
            };
            let goal = session_goal::create_session_goal(&session_id, text);
            return match store.save(&goal) {
                Ok(()) => json!({
                    "ok": true,
                    "snapshot": store.wire_snapshot(&session_id),
                }),
                Err(err) => json!({ "ok": false, "error": err.to_string() }),
            };
        }
        let result = store.apply_action(
            &session_id,
            &action,
            objective.as_deref(),
            token_budget,
        );
        json!({
            "ok": result.ok,
            "error": result.error,
            "continueMessage": result.continue_message,
            "snapshot": store.wire_snapshot(&session_id),
        })
    }
}
