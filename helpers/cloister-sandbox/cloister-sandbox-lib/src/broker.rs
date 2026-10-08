use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::{DelegatedPerDirMount, WorkspaceMode};
use crate::runtime;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BrokerSession {
    pub token: String,
    pub project_root: String,
    pub dir_hash: String,
    pub profiles: BTreeMap<String, BrokerSpawnableProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerParentCapability {
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BrokerSpawnableProfile {
    pub sandbox: String,
    pub workspace_mode: WorkspaceMode,
    pub delegated_per_dir_mounts: BTreeMap<String, DelegatedPerDirMount>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BrokerLaunchRequest {
    pub profile: String,
    pub sandbox: String,
    pub argv: Vec<String>,
    pub cwd_rel: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BrokerLaunchResponse {
    pub exit_code: i32,
    pub error: Option<String>,
}

pub fn resolve_delegated_per_dir_source(
    base_path: &str,
    dir_hash: &str,
    sub_path: Option<&str>,
) -> Result<String, String> {
    let mut resolved = PathBuf::from(base_path);
    resolved.push(normalize_safe_path_segment("dir_hash", dir_hash)?);

    if let Some(sub_path) = sub_path.filter(|sub_path| !sub_path.is_empty()) {
        let relative_sub_path = normalize_relative_sub_path(sub_path)?;
        resolved.push(relative_sub_path);
    }

    Ok(resolved.to_string_lossy().to_string())
}

pub fn validate_project_binding(session: &BrokerSession, project_root: &str) -> Result<(), String> {
    validate_project_root_binding(&session.project_root, project_root)
}

pub fn validate_capability_session_identity(
    capability: &BrokerParentCapability,
    session: &BrokerSession,
    project_root: &str,
) -> Result<(), String> {
    if capability.token != session.token {
        return Err(format!(
            "broker session token not found: {}",
            capability.token
        ));
    }

    validate_project_identity(&session.project_root, project_root, &session.dir_hash)
}

pub fn validate_project_identity(
    session_project_root: &str,
    requested_project_root: &str,
    dir_hash: &str,
) -> Result<(), String> {
    if dir_hash.is_empty() {
        return Err("broker project identity requires a non-empty dir hash".to_string());
    }

    let session_root = normalize_absolute_path(session_project_root)?;
    let requested_root = normalize_absolute_path(requested_project_root)?;
    let requested_dir_hash =
        runtime::compute_dir_hash(requested_root.to_str().ok_or_else(|| {
            format!("project root must be valid UTF-8: {requested_project_root}")
        })?);

    if session_root == requested_root || requested_dir_hash == dir_hash {
        Ok(())
    } else {
        Err(format!(
            "broker project identity does not match requested project root '{}'",
            requested_project_root
        ))
    }
}

fn validate_project_root_binding(
    bound_project_root: &str,
    project_root: &str,
) -> Result<(), String> {
    let session_root = normalize_absolute_path(bound_project_root)?;
    let requested_root = normalize_absolute_path(project_root)?;

    if session_root == requested_root {
        Ok(())
    } else {
        Err(format!(
            "broker capability project root '{}' does not match requested project root '{}'",
            bound_project_root, project_root
        ))
    }
}

impl From<&BrokerSession> for BrokerParentCapability {
    fn from(session: &BrokerSession) -> Self {
        Self {
            token: session.token.clone(),
        }
    }
}

pub fn resolve_relative_launch_dir(project_root: &str, cwd_rel: &str) -> Result<PathBuf, String> {
    let project_root = normalize_absolute_path(project_root)?;
    if cwd_rel.is_empty() {
        return Ok(project_root);
    }
    Ok(project_root.join(normalize_relative_sub_path(cwd_rel)?))
}

fn normalize_relative_sub_path(sub_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(sub_path);
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => normalized.push(part),
            _ => {
                return Err(format!(
                    "delegated per-dir sub_path must be a relative traversal-free path: {sub_path}"
                ));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err("delegated per-dir sub_path must not be empty".to_string());
    }

    Ok(normalized)
}

fn normalize_safe_path_segment(label: &str, value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => normalized.push(part),
            _ => {
                return Err(format!(
                    "{label} must be a single safe path segment: {value}"
                ));
            }
        }
    }

    if normalized.as_os_str().is_empty() || normalized.components().count() != 1 {
        return Err(format!(
            "{label} must be a single safe path segment: {value}"
        ));
    }

    Ok(normalized)
}

fn normalize_absolute_path(path: &str) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(format!("project root must be an absolute path: {path:?}"));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::ParentDir => {
                return Err(format!(
                    "project root must not contain parent directory traversal: {path:?}"
                ));
            }
        }
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_delegated_per_dir_source_joins_hash_and_optional_sub_path() {
        let resolved = resolve_delegated_per_dir_source(
            "/var/lib/cloister/delegated",
            "abc123def456",
            Some("cache/npm"),
        )
        .unwrap();
        assert_eq!(
            resolved,
            "/var/lib/cloister/delegated/abc123def456/cache/npm"
        );

        let without_sub_path =
            resolve_delegated_per_dir_source("/var/lib/cloister/delegated", "abc123def456", None)
                .unwrap();
        assert_eq!(without_sub_path, "/var/lib/cloister/delegated/abc123def456");
    }

    #[test]
    fn resolve_relative_launch_dir_accepts_empty_relative_path() {
        assert_eq!(
            resolve_relative_launch_dir("/workspace/project", "").unwrap(),
            PathBuf::from("/workspace/project")
        );
    }

    #[test]
    fn resolve_relative_launch_dir_joins_safe_relative_path() {
        assert_eq!(
            resolve_relative_launch_dir("/workspace/project", "nested/src").unwrap(),
            PathBuf::from("/workspace/project/nested/src")
        );
    }

    #[test]
    fn resolve_relative_launch_dir_rejects_traversal() {
        let err = resolve_relative_launch_dir("/workspace/project", "../escape").unwrap_err();
        assert!(err.contains("delegated per-dir sub_path must be a relative traversal-free path"));
    }

    #[test]
    fn resolve_delegated_per_dir_source_rejects_traversal_sub_path() {
        let err = resolve_delegated_per_dir_source(
            "/var/lib/cloister/delegated",
            "abc123def456",
            Some("../escape"),
        )
        .unwrap_err();

        assert!(
            err.contains("relative"),
            "expected relative path error, got: {err}"
        );
    }

    #[test]
    fn resolve_delegated_per_dir_source_rejects_absolute_sub_path() {
        let err = resolve_delegated_per_dir_source(
            "/var/lib/cloister/delegated",
            "abc123def456",
            Some("/escape"),
        )
        .unwrap_err();

        assert!(
            err.contains("relative"),
            "expected relative path error, got: {err}"
        );
    }

    #[test]
    fn resolve_delegated_per_dir_source_rejects_empty_dir_hash() {
        let err =
            resolve_delegated_per_dir_source("/var/lib/cloister/delegated", "", Some("cache/npm"))
                .unwrap_err();

        assert!(
            err.contains("dir_hash"),
            "expected dir_hash error, got: {err}"
        );
    }

    #[test]
    fn resolve_delegated_per_dir_source_rejects_traversal_dir_hash() {
        let err = resolve_delegated_per_dir_source(
            "/var/lib/cloister/delegated",
            "../escape",
            Some("cache/npm"),
        )
        .unwrap_err();

        assert!(
            err.contains("dir_hash"),
            "expected dir_hash error, got: {err}"
        );
    }

    #[test]
    fn validate_project_binding_rejects_different_project_root() {
        let session = BrokerSession {
            token: "token-1".to_string(),
            project_root: "/workspace/project-a".to_string(),
            dir_hash: "abc123def456".to_string(),
            profiles: BTreeMap::new(),
        };

        let err = validate_project_binding(&session, "/workspace/project-b").unwrap_err();
        assert!(
            err.contains("project root"),
            "expected project root error, got: {err}"
        );
    }

    #[test]
    fn validate_project_binding_accepts_equivalent_normalized_project_root() {
        let session = BrokerSession {
            token: "token-1".to_string(),
            project_root: "/workspace/project".to_string(),
            dir_hash: "abc123def456".to_string(),
            profiles: BTreeMap::new(),
        };

        validate_project_binding(&session, "/workspace/project/./").unwrap();
    }
}
