//! Runtime operations: git root detection, directory validation, hashing, manifest management.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// Detect the sandbox directory.
///
/// Priority:
/// 1. CLOISTER_DIR environment variable
/// 2. Git repository root (via `git rev-parse --show-toplevel`)
/// 3. Current working directory
pub fn detect_sandbox_dir(git_path: &str) -> Result<String, String> {
    let current_dir =
        std::env::current_dir().map_err(|e| format!("cannot determine current directory: {e}"))?;
    let current_dir = fs::canonicalize(&current_dir)
        .map_err(|e| format!("cannot resolve current directory: {e}"))?;

    let sandbox_dir = if let Ok(cloister_dir) = std::env::var("CLOISTER_DIR") {
        cloister_dir
    } else if let Ok(output) = Command::new(git_path)
        .args(["rev-parse", "--show-toplevel"])
        .stderr(std::process::Stdio::null())
        .output()
    {
        if output.status.success() {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            current_dir.to_string_lossy().to_string()
        }
    } else {
        current_dir.to_string_lossy().to_string()
    };

    // Resolve to absolute path
    let resolved = fs::canonicalize(&sandbox_dir)
        .map_err(|e| format!("cannot resolve sandbox directory '{sandbox_dir}': {e}"))?;

    Ok(resolved.to_string_lossy().to_string())
}

/// Compute the start directory (prefer current dir if inside sandbox dir).
pub fn compute_start_dir(sandbox_dir: &str) -> String {
    let current_dir = std::env::current_dir()
        .and_then(fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from(sandbox_dir));
    let sandbox = Path::new(sandbox_dir);

    if current_dir == sandbox || current_dir.starts_with(sandbox) {
        current_dir.to_string_lossy().to_string()
    } else {
        sandbox_dir.to_string()
    }
}

/// Compute anonymized path remapping: if path is under $HOME, remap to $SANDBOX_HOME.
pub fn remap_path_for_anonymize(path: &str, home: &str, sandbox_home: &str) -> String {
    let path = Path::new(path);
    let home = Path::new(home);
    if let Ok(suffix) = path.strip_prefix(home) {
        if suffix.as_os_str().is_empty() {
            return sandbox_home.to_string();
        }
        let mut remapped = PathBuf::from(sandbox_home);
        remapped.push(suffix);
        remapped.to_string_lossy().to_string()
    } else {
        path.to_string_lossy().to_string()
    }
}

/// Compute a stable 20-character hex hash of a sandbox directory path.
/// Used for per-directory state isolation.
pub fn compute_dir_hash(sandbox_dir: &str) -> String {
    let hash = Sha256::digest(sandbox_dir.as_bytes());
    // Take first 10 bytes (20 hex chars) to match: sha256sum | cut -c1-20
    hex_encode(&hash[..10])
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Validate the per-directory base directory.
///
/// Must not be a symlink, must be owned by the current user,
/// and must not be group/other-writable.
pub fn validate_per_dir_base(base: &str) -> Result<(), String> {
    let path = Path::new(base);

    // Check for symlink
    match fs::symlink_metadata(base) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "perDirBase '{base}' is a symlink, which is not allowed."
                ));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Create it
            fs::create_dir_all(base)
                .map_err(|e| format!("failed to create perDirBase '{base}': {e}"))?;
            fs::set_permissions(base, fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("failed to chmod perDirBase '{base}': {e}"))?;
            return Ok(());
        }
        Err(e) => return Err(format!("stat perDirBase '{base}': {e}")),
    }

    if !path.is_dir() {
        return Err(format!("perDirBase '{base}' is not a directory."));
    }

    let meta = fs::metadata(base).map_err(|e| format!("stat perDirBase '{base}': {e}"))?;

    let owner_uid = meta.uid();
    let current_uid = crate::socket::current_uid();
    if owner_uid != current_uid {
        return Err(format!(
            "perDirBase '{base}' is not owned by the current user."
        ));
    }

    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o022 != 0 {
        return Err(format!(
            "perDirBase '{base}' is group/other-writable (mode {mode:o})."
        ));
    }

    Ok(())
}

/// Update the per-directory manifest.json with a hash→path entry.
/// Uses file locking (flock) for concurrent safety.
pub fn update_manifest(
    manifest_path: &str,
    dir_hash: &str,
    sandbox_dir: &str,
) -> Result<(), String> {
    let lock_path = format!("{manifest_path}.lock");

    // Open/create the lock file
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&lock_path)
        .map_err(|e| format!("open lock file {lock_path}: {e}"))?;

    // Acquire exclusive lock
    use std::os::unix::io::AsRawFd;
    let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
    if ret != 0 {
        return Err(format!("flock {lock_path}: {}", io::Error::last_os_error()));
    }

    // Read existing manifest or start with empty array
    let manifest_data = match fs::read_to_string(manifest_path) {
        Ok(data) => data,
        Err(e) if e.kind() == io::ErrorKind::NotFound => "[]".to_string(),
        Err(e) => return Err(format!("read manifest {manifest_path}: {e}")),
    };

    let mut entries: Vec<serde_json::Value> = serde_json::from_str(&manifest_data)
        .map_err(|e| format!("parse manifest {manifest_path}: {e}"))?;

    // Remove existing entry with the same hash, then add the new one
    entries.retain(|entry| entry.get("hash").and_then(|h| h.as_str()) != Some(dir_hash));
    entries.push(serde_json::json!({
        "hash": dir_hash,
        "path": sandbox_dir,
    }));

    // Write atomically via temp file
    let tmp_path = format!("{manifest_path}.{}", std::process::id());
    let data =
        serde_json::to_string_pretty(&entries).map_err(|e| format!("serialize manifest: {e}"))?;
    fs::write(&tmp_path, &data).map_err(|e| format!("write temp manifest {tmp_path}: {e}"))?;
    fs::rename(&tmp_path, manifest_path)
        .map_err(|e| format!("rename manifest {tmp_path} → {manifest_path}: {e}"))?;

    // Lock is released when lock_file is dropped
    Ok(())
}

/// Reject any path whose existing components include a symlink.
/// This prevents symlink-based attacks where an attacker plants a symlink
/// at an intermediate directory to redirect file operations.
fn reject_symlink_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if let Ok(meta) = fs::symlink_metadata(&current) {
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "refusing to write through symlink path component: {}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

/// Create directories on the host for volume-backed and per-dir binds.
pub fn ensure_dirs(paths: &[String]) -> Result<(), String> {
    for path in paths {
        reject_symlink_components(Path::new(path))?;
        fs::create_dir_all(path).map_err(|e| format!("mkdir -p {path}: {e}"))?;
        reject_symlink_components(Path::new(path))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("chmod 700 {path}: {e}"))?;
    }
    Ok(())
}

/// Create file (touch) at path if it doesn't exist, creating parent dirs as needed.
pub fn ensure_files(paths: &[String]) -> Result<(), String> {
    for path in paths {
        // If the path already exists (including as a dangling symlink), skip it.
        if Path::new(path).symlink_metadata().is_ok() {
            continue;
        }
        if let Some(parent) = Path::new(path).parent() {
            reject_symlink_components(parent)?;
            fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir -p {}: {e}", parent.display()))?;
            reject_symlink_components(parent)?;
        }
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("touch {path}: {e}")),
        }
    }
    Ok(())
}

/// Copy a file if the source exists and (overwrite is true or dest doesn't exist).
/// Destination must stay under `allowed_base` and symlinks are rejected.
pub fn copy_file(
    src: &str,
    dest: &str,
    mode: u32,
    overwrite: bool,
    allowed_base: &str,
) -> Result<(), String> {
    fn copy_contents(src: &Path, dest: &Path, mode: u32) -> Result<(), String> {
        let mut src_file = fs::File::open(src)
            .map_err(|e| format!("open source file '{}': {e}", src.display()))?;
        let mut dest_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dest)
            .map_err(|e| format!("open destination file '{}': {e}", dest.display()))?;
        io::copy(&mut src_file, &mut dest_file)
            .map_err(|e| format!("copy '{}' -> '{}': {e}", src.display(), dest.display()))?;
        fs::set_permissions(dest, fs::Permissions::from_mode(mode))
            .map_err(|e| format!("chmod {mode:o} {}: {e}", dest.display()))?;
        Ok(())
    }

    let src_path = Path::new(src);
    if !src_path.is_file() {
        return Ok(()); // Source doesn't exist, skip
    }

    let dest_path = Path::new(dest);
    let base_path = Path::new(allowed_base);
    if !dest_path.is_absolute() {
        return Err(format!("copy destination must be absolute: {dest}"));
    }
    if !dest_path.starts_with(base_path) {
        return Err(format!(
            "copy destination '{dest}' is outside copyFileBase '{allowed_base}'"
        ));
    }

    reject_symlink_components(base_path)?;
    reject_symlink_components(dest_path)?;

    let base_canon = fs::canonicalize(base_path)
        .or_else(|e| {
            if e.kind() == ErrorKind::NotFound {
                fs::create_dir_all(base_path)?;
                fs::set_permissions(base_path, fs::Permissions::from_mode(0o700))?;
                fs::canonicalize(base_path)
            } else {
                Err(e)
            }
        })
        .map_err(|e| format!("resolve copyFileBase '{allowed_base}': {e}"))?;

    if let Ok(meta) = fs::symlink_metadata(dest_path) {
        if meta.file_type().is_symlink() {
            return Err(format!("refusing to copy over symlink destination: {dest}"));
        }
        if !meta.file_type().is_file() {
            return Err(format!(
                "destination exists and is not a regular file: {dest}"
            ));
        }
        if !overwrite {
            return Ok(()); // Already exists and overwrite is false
        }
    }

    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir -p {}: {e}", parent.display()))?;
        reject_symlink_components(parent)?;
        let parent_canon = fs::canonicalize(parent)
            .map_err(|e| format!("resolve destination parent '{}': {e}", parent.display()))?;
        if !parent_canon.starts_with(&base_canon) {
            return Err(format!(
                "destination parent '{}' escapes copyFileBase '{}'",
                parent_canon.display(),
                base_canon.display()
            ));
        }
    }

    if !overwrite {
        if dest_path.exists() {
            return Ok(());
        }
        copy_contents(src_path, dest_path, mode)?;
        return Ok(());
    }

    let parent = dest_path
        .parent()
        .ok_or_else(|| format!("destination has no parent directory: {dest}"))?;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tmp_path = parent.join(format!(
        ".cloister-copy-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    copy_contents(src_path, &tmp_path, mode)?;
    fs::rename(&tmp_path, dest_path)
        .map_err(|e| format!("rename '{}' -> '{dest}': {e}", tmp_path.display()))?;

    Ok(())
}

/// Build the runtime variable map for path substitution.
pub fn build_runtime_vars(
    home: &str,
    sandbox_home: &str,
    sandbox_dir: &str,
    sandbox_dest: &str,
    dir_hash: &str,
    xdg_runtime_dir: &str,
) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    vars.insert("HOME".to_string(), home.to_string());
    vars.insert("SANDBOX_HOME".to_string(), sandbox_home.to_string());
    vars.insert("SANDBOX_DIR".to_string(), sandbox_dir.to_string());
    vars.insert("SANDBOX_DEST".to_string(), sandbox_dest.to_string());
    vars.insert("DIR_HASH".to_string(), dir_hash.to_string());
    vars.insert("XDG_RUNTIME_DIR".to_string(), xdg_runtime_dir.to_string());
    vars
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn dir_hash_is_20_hex_chars() {
        let hash = compute_dir_hash("/home/user/projects/myapp");
        assert_eq!(hash.len(), 20);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn dir_hash_is_deterministic() {
        let h1 = compute_dir_hash("/home/user/projects/myapp");
        let h2 = compute_dir_hash("/home/user/projects/myapp");
        assert_eq!(h1, h2);
    }

    #[test]
    fn dir_hash_differs_for_different_paths() {
        let h1 = compute_dir_hash("/home/user/projects/a");
        let h2 = compute_dir_hash("/home/user/projects/b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn start_dir_prefers_current_if_inside_sandbox() {
        let _guard = env_lock().lock().unwrap();
        let original = std::env::current_dir().unwrap();

        let base = std::env::temp_dir().join(format!("cloister-startdir-{}", std::process::id()));
        let sandbox = base.join("sandbox");
        let inside = sandbox.join("nested");
        let _ = fs::create_dir_all(&inside);

        std::env::set_current_dir(&inside).unwrap();
        let start = compute_start_dir(sandbox.to_str().unwrap());
        assert_eq!(start, inside.to_string_lossy());

        std::env::set_current_dir(&original).unwrap();
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn remap_inside_home() {
        assert_eq!(
            remap_path_for_anonymize("/home/user/projects", "/home/user", "/home/ubuntu"),
            "/home/ubuntu/projects"
        );
    }

    #[test]
    fn remap_outside_home() {
        assert_eq!(
            remap_path_for_anonymize("/opt/project", "/home/user", "/home/ubuntu"),
            "/opt/project"
        );
    }

    #[test]
    fn remap_exact_home() {
        assert_eq!(
            remap_path_for_anonymize("/home/user", "/home/user", "/home/ubuntu"),
            "/home/ubuntu"
        );
    }

    #[test]
    fn remap_does_not_match_prefix_only() {
        assert_eq!(
            remap_path_for_anonymize("/home/user2/projects", "/home/user", "/home/ubuntu"),
            "/home/user2/projects"
        );
    }

    #[test]
    fn remap_handles_trailing_slash() {
        assert_eq!(
            remap_path_for_anonymize("/home/user/projects", "/home/user/", "/home/ubuntu"),
            "/home/ubuntu/projects"
        );
        assert_eq!(
            remap_path_for_anonymize("/home/user/", "/home/user/", "/home/ubuntu"),
            "/home/ubuntu"
        );
    }

    #[test]
    fn start_dir_rejects_prefix_only() {
        let _guard = env_lock().lock().unwrap();
        let original = std::env::current_dir().unwrap();

        let base =
            std::env::temp_dir().join(format!("cloister-startdir-prefix-{}", std::process::id()));
        let sandbox = base.join("home-user");
        let not_inside = base.join("home-user2/project");
        let _ = fs::create_dir_all(&not_inside);
        let _ = fs::create_dir_all(&sandbox);

        std::env::set_current_dir(&not_inside).unwrap();
        let start = compute_start_dir(sandbox.to_str().unwrap());
        assert_eq!(start, sandbox.to_string_lossy());

        std::env::set_current_dir(&original).unwrap();
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn manifest_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("cloister-runtime-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let manifest = dir.join("manifest.json");
        let manifest_str = manifest.to_str().unwrap();

        // First entry
        update_manifest(manifest_str, "abc123", "/home/user/project-a").unwrap();
        let data: Vec<serde_json::Value> =
            serde_json::from_str(&fs::read_to_string(manifest_str).unwrap()).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["hash"], "abc123");

        // Second entry
        update_manifest(manifest_str, "def456", "/home/user/project-b").unwrap();
        let data: Vec<serde_json::Value> =
            serde_json::from_str(&fs::read_to_string(manifest_str).unwrap()).unwrap();
        assert_eq!(data.len(), 2);

        // Update existing entry
        update_manifest(manifest_str, "abc123", "/home/user/project-a-moved").unwrap();
        let data: Vec<serde_json::Value> =
            serde_json::from_str(&fs::read_to_string(manifest_str).unwrap()).unwrap();
        assert_eq!(data.len(), 2);
        let abc_entry = data.iter().find(|e| e["hash"] == "abc123").unwrap();
        assert_eq!(abc_entry["path"], "/home/user/project-a-moved");

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_files_does_not_follow_broken_symlink() {
        let dir = std::env::temp_dir().join(format!("cloister-ensure-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let dest = dir.join("dest");
        let target = dir.join("target");

        std::os::unix::fs::symlink(&target, &dest).unwrap();

        ensure_files(&[dest.to_str().unwrap().to_string()]).unwrap();

        assert!(!target.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_file_rejects_symlink_destination() {
        let dir = std::env::temp_dir().join(format!("cloister-copy-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let src = dir.join("src");
        let dest = dir.join("dest");
        let target = dir.join("target");

        fs::write(&src, "source").unwrap();
        fs::write(&target, "target").unwrap();
        std::os::unix::fs::symlink(&target, &dest).unwrap();

        let err = copy_file(
            src.to_str().unwrap(),
            dest.to_str().unwrap(),
            0o644,
            true,
            dir.to_str().unwrap(),
        )
        .unwrap_err();
        assert!(err.contains("symlink"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "target");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_file_rejects_destination_outside_base() {
        let dir =
            std::env::temp_dir().join(format!("cloister-copy-base-test-{}", std::process::id()));
        let outside =
            std::env::temp_dir().join(format!("cloister-copy-outside-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let _ = fs::create_dir_all(&outside);
        let src = dir.join("src");
        let dest = outside.join("dest");
        fs::write(&src, "source").unwrap();

        let err = copy_file(
            src.to_str().unwrap(),
            dest.to_str().unwrap(),
            0o644,
            true,
            dir.to_str().unwrap(),
        )
        .unwrap_err();
        assert!(err.contains("outside copyFileBase"));

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn ensure_files_rejects_symlink_in_parent_path() {
        let dir = std::env::temp_dir()
            .join(format!("cloister-ensure-symlink-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let real_dir = dir.join("real");
        let _ = fs::create_dir_all(&real_dir);
        let link_dir = dir.join("link");
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

        let target = link_dir.join("file.txt");
        let err = ensure_files(&[target.to_str().unwrap().to_string()]).unwrap_err();
        assert!(err.contains("symlink"), "expected symlink error, got: {err}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_dirs_rejects_symlink_component() {
        let dir = std::env::temp_dir()
            .join(format!("cloister-ensuredir-symlink-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let real_dir = dir.join("real");
        let _ = fs::create_dir_all(&real_dir);
        let link_dir = dir.join("link");
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

        let target = link_dir.join("subdir");
        let err = ensure_dirs(&[target.to_str().unwrap().to_string()]).unwrap_err();
        assert!(err.contains("symlink"), "expected symlink error, got: {err}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_file_succeeds_for_regular_destination() {
        let dir = std::env::temp_dir().join(format!("cloister-copy-ok-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let src = dir.join("src");
        let dest = dir.join("dest");

        fs::write(&src, "source").unwrap();
        copy_file(
            src.to_str().unwrap(),
            dest.to_str().unwrap(),
            0o640,
            true,
            dir.to_str().unwrap(),
        )
        .unwrap();

        assert_eq!(fs::read_to_string(&dest).unwrap(), "source");
        let mode = fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);

        let _ = fs::remove_dir_all(&dir);
    }
}
