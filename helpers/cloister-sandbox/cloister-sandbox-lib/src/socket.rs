use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::Path;

/// Return the effective UID of the calling process.
pub fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

/// Validate that the parent directory of a Unix socket path is safe to use:
/// - Path must be absolute
/// - Parent must exist and be a real directory (not a symlink)
/// - Parent must be owned by the current user
/// - Parent must not be group/other-writable
pub fn validate_socket_parent(path: &str) -> Result<(), String> {
    let socket_path = Path::new(path);
    if !socket_path.is_absolute() {
        return Err(format!("socket path must be absolute: {path}"));
    }
    let parent = socket_path
        .parent()
        .ok_or_else(|| format!("socket path has no parent: {path}"))?;
    let meta = std::fs::symlink_metadata(parent)
        .map_err(|e| format!("stat parent dir {parent:?}: {e}"))?;
    if meta.file_type().is_symlink() {
        return Err(format!("socket parent is a symlink: {parent:?}"));
    }
    if !meta.is_dir() {
        return Err(format!("socket parent is not a directory: {parent:?}"));
    }
    if meta.uid() != current_uid() {
        return Err(format!(
            "socket parent is not owned by current user: {parent:?}"
        ));
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o022 != 0 {
        return Err(format!(
            "socket parent is group/other-writable: {parent:?} (mode {:o})",
            mode
        ));
    }
    Ok(())
}

/// Validate an existing Unix socket path controlled by the current user.
/// Rejects non-absolute paths, symlinks, non-sockets, and unsafe parent dirs.
pub fn validate_existing_socket(path: &str) -> Result<(), String> {
    let socket_path = Path::new(path);
    if !socket_path.is_absolute() {
        return Err(format!("socket path must be absolute: {path}"));
    }

    let meta =
        std::fs::symlink_metadata(path).map_err(|e| format!("stat socket path {path}: {e}"))?;
    if meta.file_type().is_symlink() {
        return Err(format!("socket path must not be a symlink: {path}"));
    }
    if !meta.file_type().is_socket() {
        return Err(format!("path is not a Unix socket: {path}"));
    }
    if meta.uid() != current_uid() {
        return Err(format!("socket path is not owned by current user: {path}"));
    }

    validate_socket_parent(path)
}

/// Validate an existing regular file path controlled by the current user.
/// Rejects non-absolute paths, symlinks, and non-regular files.
pub fn validate_existing_regular_file(path: &str) -> Result<(), String> {
    let file_path = Path::new(path);
    if !file_path.is_absolute() {
        return Err(format!("file path must be absolute: {path}"));
    }

    let meta =
        std::fs::symlink_metadata(path).map_err(|e| format!("stat file path {path}: {e}"))?;
    if meta.file_type().is_symlink() {
        return Err(format!("file path must not be a symlink: {path}"));
    }
    if !meta.file_type().is_file() {
        return Err(format!("path is not a regular file: {path}"));
    }
    if meta.uid() != current_uid() {
        return Err(format!("file path is not owned by current user: {path}"));
    }

    Ok(())
}

/// Safely remove a stale Unix socket at the given path.
/// Refuses to unlink symlinks, non-sockets, or files not owned by the current user.
pub fn remove_stale_socket(path: &str) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(format!("refusing to unlink symlink: {path}"));
            }
            if !meta.file_type().is_socket() {
                return Err(format!("refusing to unlink non-socket path: {path}"));
            }
            if meta.uid() != current_uid() {
                return Err(format!(
                    "refusing to unlink socket not owned by current user: {path}"
                ));
            }
            std::fs::remove_file(path)
                .map_err(|e| format!("failed to remove stale socket {path}: {e}"))?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("stat socket path {path}: {e}")),
    }
    Ok(())
}

/// Bind a Unix listening socket with restrictive permissions (0o600).
/// Removes any stale socket first, then sets permissions on the path.
/// Thread-safe: no process-global state (umask) is touched.
/// The brief window between bind and chmod is mitigated by
/// validate_socket_parent (parent dir is user-owned, mode 0700).
pub fn bind_socket_restricted(path: &str) -> io::Result<std::os::unix::net::UnixListener> {
    validate_socket_parent(path).map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    remove_stale_socket(path).map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    let listener = std::os::unix::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;

    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir();
        for i in 0..100 {
            let p = base.join(format!("{}-{}-{}", prefix, std::process::id(), i));
            if std::fs::create_dir(&p).is_ok() {
                return p;
            }
        }
        panic!("failed to create temp dir");
    }

    #[test]
    fn validate_socket_parent_accepts_private_dir() {
        let dir = make_temp_dir("cloister-socket");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let sock = dir.join("sock");
        let res = validate_socket_parent(sock.to_str().unwrap());
        let _ = std::fs::remove_dir(&dir);
        assert!(res.is_ok());
    }

    #[test]
    fn validate_socket_parent_rejects_group_writable() {
        let dir = make_temp_dir("cloister-socket");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o770)).unwrap();
        let sock = dir.join("sock");
        let res = validate_socket_parent(sock.to_str().unwrap());
        let _ = std::fs::remove_dir(&dir);
        assert!(res.is_err());
    }

    #[test]
    fn validate_socket_parent_rejects_relative_path() {
        let res = validate_socket_parent("relative/path/sock");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("must be absolute"));
    }

    #[test]
    fn remove_stale_socket_rejects_non_socket() {
        let dir = make_temp_dir("cloister-socket");
        let path = dir.join("not-a-socket");
        std::fs::write(&path, b"nope").unwrap();
        let res = remove_stale_socket(path.to_str().unwrap());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
        assert!(res.is_err());
    }

    #[test]
    fn remove_stale_socket_allows_socket() {
        let dir = make_temp_dir("cloister-socket");
        let path = dir.join("sock");
        let listener = UnixListener::bind(&path).unwrap();
        drop(listener);
        let res = remove_stale_socket(path.to_str().unwrap());
        let _ = std::fs::remove_dir(&dir);
        assert!(res.is_ok());
        assert!(!path.exists());
    }

    #[test]
    fn remove_stale_socket_nonexistent_ok() {
        let res = remove_stale_socket("/tmp/nonexistent-cloister-socket-test");
        assert!(res.is_ok());
    }

    #[test]
    fn bind_socket_restricted_creates_socket() {
        let dir = make_temp_dir("cloister-socket");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let sock_path = dir.join("test.sock");
        let listener = bind_socket_restricted(sock_path.to_str().unwrap()).unwrap();
        let meta = std::fs::metadata(&sock_path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        drop(listener);
        let _ = std::fs::remove_file(&sock_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn validate_existing_socket_accepts_valid_socket() {
        let dir = make_temp_dir("cloister-socket");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let sock_path = dir.join("ok.sock");
        let listener = UnixListener::bind(&sock_path).unwrap();

        let res = validate_existing_socket(sock_path.to_str().unwrap());
        drop(listener);
        let _ = std::fs::remove_file(&sock_path);
        let _ = std::fs::remove_dir(&dir);
        assert!(res.is_ok());
    }

    #[test]
    fn validate_existing_regular_file_rejects_symlink() {
        let dir = make_temp_dir("cloister-file");
        let target = dir.join("target");
        let link = dir.join("link");
        std::fs::write(&target, b"data").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let res = validate_existing_regular_file(link.to_str().unwrap());
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_dir(&dir);
        assert!(res.is_err());
    }
}
