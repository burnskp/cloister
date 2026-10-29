//! Runtime operations: git root detection, directory validation, hashing, manifest management.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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

    let sandbox_dir = match std::env::var_os("CLOISTER_DIR") {
        Some(cloister_dir) => PathBuf::from(cloister_dir),
        None => match Command::new(git_path)
            .args(["rev-parse", "--show-toplevel"])
            .stderr(std::process::Stdio::null())
            .output()
        {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8(output.stdout)
                    .map_err(|_| "git repository root is not valid UTF-8".to_string())?;
                PathBuf::from(stdout.trim())
            }
            _ => current_dir,
        },
    };

    // Resolve to absolute path
    let resolved = fs::canonicalize(&sandbox_dir).map_err(|e| {
        format!(
            "cannot resolve sandbox directory '{}': {e}",
            sandbox_dir.display()
        )
    })?;

    // Path policies and bind arguments operate on strings; a lossy conversion
    // would make them inspect a different path than the one bwrap mounts.
    resolved.into_os_string().into_string().map_err(|path| {
        format!(
            "sandbox directory is not valid UTF-8: {}",
            Path::new(&path).display()
        )
    })
}

/// Compute the start directory (prefer current dir if inside sandbox dir).
pub fn compute_start_dir(sandbox_dir: &str) -> String {
    let current_dir = std::env::current_dir()
        .and_then(fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from(sandbox_dir));
    let sandbox = Path::new(sandbox_dir);

    if current_dir == sandbox || current_dir.starts_with(sandbox) {
        current_dir
            .into_os_string()
            .into_string()
            .unwrap_or_else(|_| sandbox_dir.to_string())
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

/// Validate a per-directory base directory.
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
                    "per-directory base '{base}' is a symlink, which is not allowed."
                ));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Create it
            fs::create_dir_all(base)
                .map_err(|e| format!("failed to create per-directory base '{base}': {e}"))?;
            fs::set_permissions(base, fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("failed to chmod per-directory base '{base}': {e}"))?;
            return Ok(());
        }
        Err(e) => return Err(format!("stat per-directory base '{base}': {e}")),
    }

    if !path.is_dir() {
        return Err(format!("per-directory base '{base}' is not a directory."));
    }

    let meta = fs::metadata(base).map_err(|e| format!("stat per-directory base '{base}': {e}"))?;

    let owner_uid = meta.uid();
    let current_uid = crate::socket::current_uid();
    if owner_uid != current_uid {
        return Err(format!(
            "per-directory base '{base}' is not owned by the current user."
        ));
    }

    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o022 != 0 {
        return Err(format!(
            "per-directory base '{base}' is group/other-writable (mode {mode:o})."
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
    let manifest_dir = Path::new(manifest_path)
        .parent()
        .ok_or_else(|| format!("manifest path has no parent directory: {manifest_path}"))?;
    reject_symlink_components(manifest_dir)?;

    let lock_path = format!("{manifest_path}.lock");

    // Open/create the lock file
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&lock_path)
        .map_err(|e| format!("open lock file {lock_path}: {e}"))?;

    // Acquire exclusive lock
    use std::os::unix::io::AsRawFd;
    let ret = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
    if ret != 0 {
        return Err(format!("flock {lock_path}: {}", io::Error::last_os_error()));
    }

    // Read existing manifest or start with empty array
    let manifest_data = match read_to_string_nofollow(manifest_path) {
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

    // Write atomically via an exclusively created temp file. The directory may
    // contain entries planted by sandboxed code, so never open an existing path.
    let data =
        serde_json::to_string_pretty(&entries).map_err(|e| format!("serialize manifest: {e}"))?;
    let (tmp_path, mut tmp_file) = create_exclusive_temp(manifest_path)?;
    if let Err(e) = tmp_file.write_all(data.as_bytes()) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("write temp manifest {tmp_path}: {e}"));
    }
    drop(tmp_file);
    fs::rename(&tmp_path, manifest_path)
        .map_err(|e| format!("rename manifest {tmp_path} → {manifest_path}: {e}"))?;

    // Lock is released when lock_file is dropped
    Ok(())
}

fn read_to_string_nofollow(path: &str) -> io::Result<String> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let mut data = String::new();
    io::Read::read_to_string(&mut file, &mut data)?;
    Ok(data)
}

/// Create a fresh `<prefix>.<unique>` file with O_CREAT|O_EXCL|O_NOFOLLOW.
fn create_exclusive_temp(prefix: &str) -> Result<(String, fs::File), String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    for _ in 0..16 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let tmp_path = format!(
            "{prefix}.{}-{}-{nanos:x}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp_path)
        {
            Ok(file) => return Ok((tmp_path, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create temp file {tmp_path}: {e}")),
        }
    }
    Err(format!(
        "create temp file for {prefix}: too many collisions"
    ))
}

/// Reject paths containing `.` or `..` components, so lexical prefix checks
/// against a base directory cannot be bypassed by traversal.
fn reject_traversal_components(path: &Path) -> Result<(), String> {
    use std::path::Component;
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(format!(
            "path must not contain '.' or '..' components: {}",
            path.display()
        ));
    }
    Ok(())
}

/// Open `base/relative` as an `O_PATH` directory descriptor, refusing symlinks
/// in `base` and in every component of `relative`.
///
/// The walk uses `openat(O_NOFOLLOW)` one component at a time, so a symlink
/// planted beneath `base` cannot redirect the result. FD_CLOEXEC is cleared on
/// the returned descriptor so bwrap can mount it with `--bind-fd`, which pins
/// the verified directory against later replacement of the path.
pub fn open_dir_fd_beneath(base: &str, relative: &Path) -> Result<OwnedFd, String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    const FLAGS: libc::c_int =
        libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    fn open_at(dirfd: libc::c_int, name: &CString, display: &Path) -> Result<OwnedFd, String> {
        let fd = unsafe { libc::openat(dirfd, name.as_ptr(), FLAGS) };
        if fd < 0 {
            let err = io::Error::last_os_error();
            return Err(match err.raw_os_error() {
                Some(libc::ELOOP) | Some(libc::ENOTDIR) => format!(
                    "refusing to traverse symlink or non-directory: {}",
                    display.display()
                ),
                _ => format!("open {}: {err}", display.display()),
            });
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    let base_path = Path::new(base);
    if !base_path.is_absolute() {
        return Err(format!("delegated source base must be absolute: {base}"));
    }
    reject_symlink_components(base_path)?;
    let base_c = CString::new(base_path.as_os_str().as_bytes())
        .map_err(|_| format!("path contains NUL byte: {base}"))?;
    let mut current = open_at(libc::AT_FDCWD, &base_c, base_path)?;
    let mut display = base_path.to_path_buf();

    for component in relative.components() {
        let std::path::Component::Normal(part) = component else {
            return Err(format!(
                "path must be relative and traversal-free: {}",
                relative.display()
            ));
        };
        display.push(part);
        let part_c = CString::new(part.as_bytes())
            .map_err(|_| format!("path contains NUL byte: {}", display.display()))?;
        current = open_at(current.as_raw_fd(), &part_c, &display)?;
    }

    let flags = unsafe { libc::fcntl(current.as_raw_fd(), libc::F_GETFD) };
    if flags < 0
        || unsafe {
            libc::fcntl(
                current.as_raw_fd(),
                libc::F_SETFD,
                flags & !libc::FD_CLOEXEC,
            )
        } < 0
    {
        return Err(format!(
            "clear FD_CLOEXEC on {}: {}",
            display.display(),
            io::Error::last_os_error()
        ));
    }

    Ok(current)
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
        // An existing entry may have been planted by sandboxed code with write
        // access to the backing directory: only accept a real, user-owned file
        // reached without traversing symlinks.
        if Path::new(path).symlink_metadata().is_ok() {
            if let Some(parent) = Path::new(path).parent() {
                reject_symlink_components(parent)?;
            }
            crate::socket::validate_existing_regular_file(path)
                .map_err(|e| format!("state file {path}: {e}"))?;
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

/// Copy a regular file into sandbox state.
/// Missing/non-regular sources are a hard error; destination must stay under
/// `allowed_base` and symlinks are rejected.
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
        return Err(format!(
            "source file does not exist or is not a regular file: {src}"
        ));
    }

    let dest_path = Path::new(dest);
    let base_path = Path::new(allowed_base);
    if !dest_path.is_absolute() {
        return Err(format!("copy destination must be absolute: {dest}"));
    }
    reject_traversal_components(base_path)?;
    reject_traversal_components(dest_path)?;
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
    host_xdg_runtime_dir: &str,
) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    vars.insert("HOME".to_string(), home.to_string());
    vars.insert("SANDBOX_HOME".to_string(), sandbox_home.to_string());
    vars.insert("SANDBOX_DIR".to_string(), sandbox_dir.to_string());
    vars.insert("SANDBOX_DEST".to_string(), sandbox_dest.to_string());
    vars.insert("DIR_HASH".to_string(), dir_hash.to_string());
    vars.insert("XDG_RUNTIME_DIR".to_string(), xdg_runtime_dir.to_string());
    vars.insert(
        "HOST_XDG_RUNTIME_DIR".to_string(),
        host_xdg_runtime_dir.to_string(),
    );
    vars
}

pub fn generate_flatpak_instance_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}-{:x}", std::process::id(), nanos)
}

fn write_runtime_file(path: &str, data: &[u8], mode: u32) -> Result<(), String> {
    let parent = Path::new(path)
        .parent()
        .ok_or_else(|| format!("runtime file path has no parent directory: {path}"))?;
    reject_symlink_components(parent)?;
    fs::create_dir_all(parent).map_err(|e| format!("mkdir -p {}: {e}", parent.display()))?;
    reject_symlink_components(parent)?;

    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!(
                "refusing to overwrite symlink runtime file: {path}"
            ));
        }
        Ok(meta) if meta.file_type().is_dir() => {
            return Err(format!("runtime file path is a directory: {path}"));
        }
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("stat {path}: {e}")),
    }

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tmp_path = parent.join(format!(
        ".cloister-runtime-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp_path)
        .map_err(|e| format!("create temp runtime file '{}': {e}", tmp_path.display()))?;
    if let Err(e) = file.write_all(data) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!(
            "write temp runtime file '{}': {e}",
            tmp_path.display()
        ));
    }
    drop(file);

    fs::rename(&tmp_path, path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "rename runtime file '{}' -> '{path}': {e}",
            tmp_path.display()
        )
    })
}

pub fn write_flatpak_info(path: &str, app_id: &str, instance_id: &str) -> Result<(), String> {
    let content =
        format!("[Application]\nname={app_id}\n\n[Instance]\ninstance-id={instance_id}\n");
    write_runtime_file(path, content.as_bytes(), 0o600)
}

pub fn write_flatpak_bwrapinfo(path: &str, child_pid: u32) -> Result<(), String> {
    let data = serde_json::json!({
        "child-pid": child_pid,
    });
    let rendered = serde_json::to_vec(&data).map_err(|e| format!("serialize bwrapinfo: {e}"))?;
    write_runtime_file(path, &rendered, 0o600)
}

pub fn remove_path_if_exists(path: &str) -> Result<(), String> {
    let target = Path::new(path);
    match fs::symlink_metadata(target) {
        Ok(meta) => {
            if meta.file_type().is_dir() {
                fs::remove_dir_all(target).map_err(|e| format!("remove directory {path}: {e}"))
            } else {
                fs::remove_file(target).map_err(|e| format!("remove file {path}: {e}"))
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("stat {path}: {e}")),
    }
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
    fn flatpak_instance_id_is_non_empty() {
        let id = generate_flatpak_instance_id();
        assert!(!id.is_empty());
        assert!(id.contains('-'));
    }

    #[test]
    fn write_flatpak_metadata_files() {
        let dir =
            std::env::temp_dir().join(format!("cloister-flatpak-test-{}", std::process::id()));
        let info_path = dir.join("flatpak-info");
        let bwrapinfo_path = dir.join("instance/bwrapinfo.json");

        write_flatpak_info(
            info_path.to_str().unwrap(),
            "dev.cloister.test",
            "instance-1",
        )
        .unwrap();
        write_flatpak_bwrapinfo(bwrapinfo_path.to_str().unwrap(), 1234).unwrap();

        let info = fs::read_to_string(&info_path).unwrap();
        assert!(info.contains("[Application]"));
        assert!(info.contains("name=dev.cloister.test"));
        assert!(info.contains("[Instance]"));
        assert!(info.contains("instance-id=instance-1"));

        let data: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&bwrapinfo_path).unwrap()).unwrap();
        assert_eq!(data["child-pid"], 1234);

        remove_path_if_exists(dir.to_str().unwrap()).unwrap();
    }

    #[test]
    fn write_flatpak_info_rejects_symlink_target() {
        let dir = std::env::temp_dir().join(format!(
            "cloister-flatpak-symlink-test-{}",
            std::process::id()
        ));
        let info_path = dir.join("flatpak-info");
        let outside_path = dir.join("outside");

        fs::create_dir_all(&dir).unwrap();
        fs::write(&outside_path, "existing").unwrap();
        std::os::unix::fs::symlink(&outside_path, &info_path).unwrap();

        let err = write_flatpak_info(
            info_path.to_str().unwrap(),
            "dev.cloister.test",
            "instance-1",
        )
        .unwrap_err();
        assert!(err.contains("refusing to overwrite symlink runtime file"));

        remove_path_if_exists(dir.to_str().unwrap()).unwrap();
    }

    #[test]
    fn build_runtime_vars_keeps_host_and_sandbox_runtime_dirs_distinct() {
        let vars = build_runtime_vars(
            "/home/user",
            "/home/sandbox",
            "/work/tree",
            "/work/tree",
            "dir-hash",
            "/run/user/1000",
            "/host/run/user/1000",
        );

        assert_eq!(
            vars.get("XDG_RUNTIME_DIR").map(String::as_str),
            Some("/run/user/1000")
        );
        assert_eq!(
            vars.get("HOST_XDG_RUNTIME_DIR").map(String::as_str),
            Some("/host/run/user/1000")
        );
    }

    #[test]
    fn detect_sandbox_dir_rejects_non_utf8_path() {
        use std::os::unix::ffi::OsStrExt;
        let _guard = env_lock().lock().unwrap();
        let original = std::env::var_os("CLOISTER_DIR");

        let base = std::env::temp_dir().join(format!("cloister-nonutf8-{}", std::process::id()));
        let bad = base.join(std::ffi::OsStr::from_bytes(b"proj\xff"));
        fs::create_dir_all(&bad).unwrap();

        std::env::set_var("CLOISTER_DIR", &bad);
        let err = detect_sandbox_dir("git").unwrap_err();
        assert!(err.contains("not valid UTF-8"));

        match original {
            Some(value) => std::env::set_var("CLOISTER_DIR", value),
            None => std::env::remove_var("CLOISTER_DIR"),
        }
        let _ = fs::remove_dir_all(&base);
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
    fn ensure_files_rejects_broken_symlink() {
        let dir = std::env::temp_dir().join(format!("cloister-ensure-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let dest = dir.join("dest");
        let target = dir.join("target");

        std::os::unix::fs::symlink(&target, &dest).unwrap();

        let err = ensure_files(&[dest.to_str().unwrap().to_string()]).unwrap_err();
        assert!(err.contains("symlink"));
        assert!(!target.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_files_rejects_symlink_to_existing_file() {
        let dir = std::env::temp_dir().join(format!(
            "cloister-ensure-existing-test-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let dest = dir.join("dest");
        let target = dir.join("target");
        fs::write(&target, "host").unwrap();
        std::os::unix::fs::symlink(&target, &dest).unwrap();

        let err = ensure_files(&[dest.to_str().unwrap().to_string()]).unwrap_err();
        assert!(err.contains("symlink"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_files_rejects_existing_directory() {
        let dir =
            std::env::temp_dir().join(format!("cloister-ensure-dir-test-{}", std::process::id()));
        let dest = dir.join("dest");
        fs::create_dir_all(&dest).unwrap();

        let err = ensure_files(&[dest.to_str().unwrap().to_string()]).unwrap_err();
        assert!(err.contains("not a regular file"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_files_accepts_existing_regular_file() {
        let dir = std::env::temp_dir().join(format!(
            "cloister-ensure-regular-test-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let dest = dir.join("dest");
        fs::write(&dest, "state").unwrap();

        ensure_files(&[dest.to_str().unwrap().to_string()]).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "state");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_does_not_follow_planted_temp_symlinks() {
        let dir = std::env::temp_dir().join(format!(
            "cloister-manifest-symlink-test-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let manifest = dir.join("manifest.json");
        let victim = dir.join("victim");
        fs::write(&victim, "host data").unwrap();
        // The old predictable temp name.
        std::os::unix::fs::symlink(
            &victim,
            dir.join(format!("manifest.json.{}", std::process::id())),
        )
        .unwrap();

        update_manifest(manifest.to_str().unwrap(), "abc123", "/home/user/p").unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "host data");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_rejects_symlinked_lock_and_manifest() {
        let dir = std::env::temp_dir().join(format!(
            "cloister-manifest-lock-test-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let manifest = dir.join("manifest.json");
        let victim = dir.join("victim");
        fs::write(&victim, "[]").unwrap();

        std::os::unix::fs::symlink(&victim, dir.join("manifest.json.lock")).unwrap();
        assert!(update_manifest(manifest.to_str().unwrap(), "abc123", "/p").is_err());
        fs::remove_file(dir.join("manifest.json.lock")).unwrap();

        std::os::unix::fs::symlink(&victim, &manifest).unwrap();
        assert!(update_manifest(manifest.to_str().unwrap(), "abc123", "/p").is_err());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "[]");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_file_rejects_parent_traversal_before_creating_dirs() {
        let dir = std::env::temp_dir().join(format!(
            "cloister-copy-traversal-test-{}",
            std::process::id()
        ));
        let base = dir.join("base");
        fs::create_dir_all(&base).unwrap();
        let src = dir.join("src");
        fs::write(&src, "source").unwrap();
        let dest = format!("{}/../outside/new/file", base.display());

        let err = copy_file(
            src.to_str().unwrap(),
            &dest,
            0o644,
            true,
            base.to_str().unwrap(),
        )
        .unwrap_err();
        assert!(err.contains("'..'"));
        assert!(!dir.join("outside").exists());

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
    fn copy_file_rejects_missing_source() {
        let dir =
            std::env::temp_dir().join(format!("cloister-copy-missing-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let src = dir.join("missing-src");
        let dest = dir.join("dest");

        let err = copy_file(
            src.to_str().unwrap(),
            dest.to_str().unwrap(),
            0o644,
            true,
            dir.to_str().unwrap(),
        )
        .unwrap_err();
        assert!(err.contains("source file does not exist"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_files_rejects_symlink_in_parent_path() {
        let dir =
            std::env::temp_dir().join(format!("cloister-ensure-symlink-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let real_dir = dir.join("real");
        let _ = fs::create_dir_all(&real_dir);
        let link_dir = dir.join("link");
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

        let target = link_dir.join("file.txt");
        let err = ensure_files(&[target.to_str().unwrap().to_string()]).unwrap_err();
        assert!(
            err.contains("symlink"),
            "expected symlink error, got: {err}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_dirs_rejects_symlink_component() {
        let dir =
            std::env::temp_dir().join(format!("cloister-ensuredir-symlink-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let real_dir = dir.join("real");
        let _ = fs::create_dir_all(&real_dir);
        let link_dir = dir.join("link");
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

        let target = link_dir.join("subdir");
        let err = ensure_dirs(&[target.to_str().unwrap().to_string()]).unwrap_err();
        assert!(
            err.contains("symlink"),
            "expected symlink error, got: {err}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_file_succeeds_for_regular_destination() {
        let dir =
            std::env::temp_dir().join(format!("cloister-copy-ok-test-{}", std::process::id()));
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
