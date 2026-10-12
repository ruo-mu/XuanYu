use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use tokio::sync::RwLock;
use xuanyu_protocol::SessionIsolationScope;

#[derive(Default)]
pub struct SessionIsolationRegistry {
    by_session: RwLock<BTreeMap<String, SessionIsolationScope>>,
}

impl SessionIsolationRegistry {
    pub async fn set(&self, session_id: &str, scope: SessionIsolationScope) {
        self.by_session
            .write()
            .await
            .insert(session_id.to_string(), scope);
    }

    pub async fn get(&self, session_id: &str) -> Option<SessionIsolationScope> {
        self.by_session.read().await.get(session_id).cloned()
    }

    pub async fn can_write(&self, session_id: &str, target_path: &Path) -> bool {
        let Some(scope) = self.get(session_id).await else {
            return true;
        };
        if scope.readonly {
            return false;
        }
        let normalized = normalize_path(target_path);
        let workspace = normalize_path(Path::new(&scope.workspace_root));
        if normalized.starts_with(&workspace) {
            return true;
        }
        scope.allowed_paths.iter().any(|allowed| {
            normalized.starts_with(normalize_path(Path::new(allowed)))
        })
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}
