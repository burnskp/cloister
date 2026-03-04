//! Sandbox directory validation: strict home policy, disallowed paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Enforce the strict home directory policy (Policy A).
///
/// Rejects:
/// - The home directory itself
/// - The home parent directory (e.g. /home)
/// - Any user home directory under the parent
/// - Dot-directories directly inside a user home
pub fn validate_strict_home_policy(sandbox_dir: &str, home_dir: &str) -> Result<(), String> {
    let sandbox = Path::new(sandbox_dir);
    let home = Path::new(home_dir);

    // Canonicalize for comparison (these should already be resolved by the caller)
    if sandbox == home {
        return Err(
            "Cannot sandbox your home directory — it contains sensitive files.\n\
             cd to the directory you want to work in, or set CLOISTER_DIR"
                .to_string(),
        );
    }

    let home_parent = match home.parent() {
        Some(p) if p != Path::new("/") => p,
        _ => return Ok(()),
    };

    if sandbox == home_parent {
        return Err(format!(
            "Cannot sandbox home parent directory ({}).",
            home_parent.display()
        ));
    }

    if let Ok(relative) = sandbox.strip_prefix(home_parent) {
        let components: Vec<_> = relative.components().collect();
        if components.len() == 1 {
            // This is a user home directory (e.g. /home/otheruser)
            return Err(format!(
                "Cannot sandbox a user home directory ({sandbox_dir})."
            ));
        }
        if components.len() >= 2 {
            // Check if the path after the user home component starts with a dot
            let nested = &components[1..];
            if let Some(first) = nested.first() {
                let first_str = first.as_os_str().to_string_lossy();
                if first_str.starts_with('.') {
                    return Err(format!(
                        "Cannot sandbox a dot-directory within a user home ({sandbox_dir})."
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Check if the sandbox directory matches any disallowed path or is a subdirectory of one.
///
/// Matches the bash logic exactly:
/// - For "/": only the root itself is blocked (since "/"+"/..." = "//..." which matches nothing)
/// - For other paths: trailing slashes are stripped, then exact match or prefix+/ match
pub fn validate_disallowed_paths(sandbox_dir: &str, disallowed: &[String]) -> Result<(), String> {
    for disallowed_path in disallowed {
        // Try to resolve symlinks in the disallowed path (e.g. /var/run → /run).
        // Fall back to normalized string if the path doesn't exist on disk.
        let resolved = std::fs::canonicalize(disallowed_path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| {
                if disallowed_path == "/" {
                    "/".to_string()
                } else {
                    disallowed_path.trim_end_matches('/').to_string()
                }
            });

        // Match bash: $SANDBOX_DIR == "$_disallowed" || $SANDBOX_DIR == "$_disallowed"/*
        // For "/": checks sandbox_dir == "/" || sandbox_dir starts with "//"
        //   → only "/" itself is blocked (no real path starts with "//")
        // For "/root": checks sandbox_dir == "/root" || sandbox_dir starts with "/root/"
        if sandbox_dir == resolved || sandbox_dir.starts_with(&format!("{resolved}/")) {
            return Err(format!(
                "Sandboxing \"{disallowed_path}\" or its subdirectories is disallowed by configuration."
            ));
        }
    }

    Ok(())
}

/// Validate that the sandbox directory exists.
pub fn validate_sandbox_dir_exists(sandbox_dir: &str) -> Result<(), String> {
    if !Path::new(sandbox_dir).is_dir() {
        return Err(format!("Directory '{sandbox_dir}' does not exist"));
    }
    Ok(())
}

fn paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
}

fn normalize_entry(entry: &str) -> String {
    if entry == "/" {
        return "/".to_string();
    }
    entry.trim_end_matches('/').to_string()
}

fn matches_entry(canon: &Path, home: &Path, entry: &str) -> bool {
    if entry.is_empty() {
        return false;
    }
    if entry.starts_with('/') {
        let canon_str = canon.to_string_lossy();
        return paths_overlap(&canon_str, entry);
    }
    if let Ok(rel) = canon.strip_prefix(home) {
        let rel_str = rel.to_string_lossy();
        let rel_str = rel_str.trim_start_matches('/');
        return paths_overlap(rel_str, entry);
    }
    false
}

/// Validate bind sources against dangerous paths, resolving symlinks at runtime.
pub fn validate_dangerous_binds(
    bind_sources: &[String],
    runtime_vars: &HashMap<String, String>,
    home_dir: &str,
    dangerous_paths: &[String],
    allow_dangerous_paths: &[String],
) -> Result<(), String> {
    if bind_sources.is_empty() || dangerous_paths.is_empty() {
        return Ok(());
    }

    let home = std::fs::canonicalize(home_dir).unwrap_or_else(|_| PathBuf::from(home_dir));
    let dangerous: Vec<String> = dangerous_paths.iter().map(|p| normalize_entry(p)).collect();
    let allowed: Vec<String> = allow_dangerous_paths
        .iter()
        .map(|p| normalize_entry(p))
        .collect();

    let mut matches: Vec<String> = Vec::new();

    for src in bind_sources {
        let resolved = crate::vars::expand_vars(src, runtime_vars);
        let path = Path::new(&resolved);
        if !path.exists() {
            continue;
        }

        let canon = match std::fs::canonicalize(path) {
            Ok(p) => p,
            Err(e) => return Err(format!("cannot resolve bind source '{resolved}': {e}")),
        };

        if allowed.iter().any(|a| matches_entry(&canon, &home, a)) {
            continue;
        }

        if dangerous.iter().any(|d| matches_entry(&canon, &home, d)) {
            matches.push(format!("{} (resolved from {})", canon.display(), resolved));
        }
    }

    if matches.is_empty() {
        return Ok(());
    }

    Err(format!(
        "binds resolve to dangerous paths:\n{}",
        matches
            .into_iter()
            .map(|m| format!("  - {m}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_home_rejects_home_dir() {
        let result = validate_strict_home_policy("/home/user", "/home/user");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("home directory"));
    }

    #[test]
    fn strict_home_rejects_home_parent() {
        let result = validate_strict_home_policy("/home", "/home/user");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("home parent"));
    }

    #[test]
    fn strict_home_rejects_other_user_home() {
        let result = validate_strict_home_policy("/home/otheruser", "/home/user");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("user home directory"));
    }

    #[test]
    fn strict_home_rejects_dot_dir_in_user_home() {
        let result = validate_strict_home_policy("/home/user/.ssh", "/home/user");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("dot-directory"));
    }

    #[test]
    fn strict_home_allows_normal_subdir() {
        let result = validate_strict_home_policy("/home/user/projects", "/home/user");
        assert!(result.is_ok());
    }

    #[test]
    fn strict_home_allows_deep_subdir() {
        let result = validate_strict_home_policy("/home/user/projects/myapp", "/home/user");
        assert!(result.is_ok());
    }

    #[test]
    fn strict_home_allows_outside_home() {
        let result = validate_strict_home_policy("/opt/project", "/home/user");
        assert!(result.is_ok());
    }

    #[test]
    fn strict_home_root_home_parent() {
        // If home is /root, parent is /, which we skip
        let result = validate_strict_home_policy("/opt/project", "/root");
        assert!(result.is_ok());
    }

    #[test]
    fn disallowed_rejects_exact_match() {
        let result = validate_disallowed_paths("/root", &["/root".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn disallowed_rejects_subdirectory() {
        let result = validate_disallowed_paths("/root/subdir", &["/root".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn disallowed_allows_non_matching() {
        let result = validate_disallowed_paths("/home/user/projects", &["/root".to_string()]);
        assert!(result.is_ok());
    }

    #[test]
    fn disallowed_root_only_blocks_root_itself() {
        // "/" in the disallowed list only blocks "/" itself, not everything under it.
        // This matches the bash behavior: "/" + "/" = "//" prefix which matches nothing.
        let result = validate_disallowed_paths("/", &["/".to_string()]);
        assert!(result.is_err());

        let result = validate_disallowed_paths("/home/user/projects", &["/".to_string()]);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_exists_rejects_nonexistent() {
        let result = validate_sandbox_dir_exists("/nonexistent/path/xyz");
        assert!(result.is_err());
    }

    #[test]
    fn validate_exists_accepts_tmp() {
        let result = validate_sandbox_dir_exists("/tmp");
        assert!(result.is_ok());
    }

    #[test]
    fn disallowed_resolves_symlinks() {
        // Create a tmpdir with a symlink: link → target_dir
        let tmp = std::env::temp_dir().join("cloister-test-disallowed-symlink");
        let target = tmp.join("target_dir");
        let link = tmp.join("link");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let target_str = target.to_string_lossy().to_string();
        let link_str = link.to_string_lossy().to_string();

        // Disallowing via the symlink should block the resolved (canonical) target
        let result = validate_disallowed_paths(&target_str, &[link_str]);
        assert!(result.is_err(), "symlink disallowed path should match its canonical target");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
