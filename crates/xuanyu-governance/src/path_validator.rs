use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PathValidationResult {
    pub valid: bool,
    pub reason: Option<String>,
}

/** 工具路径边界：只允许落在项目根、应用数据目录、系统临时目录。 */
pub struct SecurePathValidator {
    allowed_roots: Vec<PathBuf>,
}

impl SecurePathValidator {
    pub fn new(project_root: impl AsRef<Path>) -> Self {
        let project = normalize_path(project_root.as_ref());
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        let app_data = normalize_path(&home.join(".xuanyu"));
        let tmp = normalize_path(&std::env::temp_dir());
        Self {
            allowed_roots: vec![project, app_data, tmp],
        }
    }

    pub fn validate(&self, file_path: &Path) -> PathValidationResult {
        let resolved = normalize_path(file_path);
        let in_root = self
            .allowed_roots
            .iter()
            .any(|root| is_within_root(&resolved, root));
        if !in_root {
            return PathValidationResult {
                valid: false,
                reason: Some("Path outside allowed roots".to_string()),
            };
        }
        if is_blocked_sensitive_path(&resolved) {
            return PathValidationResult {
                valid: false,
                reason: Some("Blocked pattern matched".to_string()),
            };
        }
        PathValidationResult {
            valid: true,
            reason: None,
        }
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    let trimmed = raw.trim();
    let stripped = if let Some(rest) = trimmed.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = trimmed.strip_prefix(r"\\?\") {
        rest.to_string()
    } else if let Some(rest) = trimmed.strip_prefix("//?/") {
        rest.replace('/', "\\")
    } else {
        trimmed.to_string()
    };
    let candidate = PathBuf::from(stripped);
    candidate
        .canonicalize()
        .unwrap_or_else(|_| {
            // 目标尚不存在时按字面组件规范化（去掉 . / ..）
            let mut out = PathBuf::new();
            for component in candidate.components() {
                match component {
                    Component::CurDir => {}
                    Component::ParentDir => {
                        out.pop();
                    }
                    other => out.push(other.as_os_str()),
                }
            }
            if out.as_os_str().is_empty() {
                candidate
            } else {
                out
            }
        })
}

fn is_within_root(resolved: &Path, root: &Path) -> bool {
    if paths_equal(resolved, root) {
        return true;
    }
    let Ok(rel) = resolved.strip_prefix(root) else {
        // Windows 大小写不一致时再比一次
        let resolved_key = path_key(resolved);
        let root_key = path_key(root);
        if resolved_key == root_key {
            return true;
        }
        if let Some(rest) = resolved_key.strip_prefix(&root_key) {
            return rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\');
        }
        return false;
    };
    !rel.as_os_str().is_empty()
        && !rel
            .components()
            .any(|component| matches!(component, Component::ParentDir))
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_ascii_lowercase()
}

fn is_blocked_sensitive_path(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    let lower = text.to_ascii_lowercase();
    if lower.contains("/../") || lower.ends_with("/..") || lower.starts_with("../") {
        return true;
    }
    if lower.ends_with("/.env") || lower.ends_with(".env") {
        return true;
    }
    if lower.ends_with(".pem") || lower.contains("id_rsa") {
        return true;
    }
    if lower.contains("/.aws/credentials") || lower.contains("/.ssh/") {
        return true;
    }
    false
}
