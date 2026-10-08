use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::broker::BrokerSession;

pub fn broker_runtime_dir(host_runtime_dir: &str) -> PathBuf {
    Path::new(host_runtime_dir).join("cloister").join("broker")
}

pub fn session_store_dir(host_runtime_dir: &str) -> PathBuf {
    broker_runtime_dir(host_runtime_dir).join("sessions")
}

pub fn socket_path(host_runtime_dir: &str, token: &str) -> Result<PathBuf, String> {
    Ok(broker_runtime_dir(host_runtime_dir).join(format!("{}.sock", normalize_token(token)?)))
}

pub fn write_session_record(base: &Path, record: &BrokerSession) -> Result<(), String> {
    let path = session_record_path(base, &record.token)?;
    let parent = path.parent().ok_or_else(|| {
        format!(
            "session record path has no parent directory: {}",
            path.display()
        )
    })?;

    reject_symlink_components(parent)?;
    fs::create_dir_all(parent).map_err(|e| format!("mkdir -p {}: {e}", parent.display()))?;
    reject_symlink_components(parent)?;

    let rendered =
        serde_json::to_vec(record).map_err(|e| format!("serialize session record: {e}"))?;

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tmp_path = parent.join(format!(
        ".cloister-broker-session-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp_path)
        .map_err(|e| format!("create temp session record '{}': {e}", tmp_path.display()))?;
    if let Err(e) = file.write_all(&rendered) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!(
            "write temp session record '{}': {e}",
            tmp_path.display()
        ));
    }
    drop(file);

    fs::rename(&tmp_path, &path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "rename session record '{}' -> '{}': {e}",
            tmp_path.display(),
            path.display()
        )
    })
}

pub fn load_session_record(base: &Path, token: &str) -> Result<BrokerSession, String> {
    let path = session_record_path(base, token)?;
    let parent = path.parent().ok_or_else(|| {
        format!(
            "session record path has no parent directory: {}",
            path.display()
        )
    })?;
    reject_symlink_components(parent)?;
    let data = read_session_record(&path, token)?;

    serde_json::from_str(&data)
        .map_err(|e| format!("parse session record '{}': {e}", path.display()))
}

pub fn remove_session_record(base: &Path, token: &str) -> Result<(), String> {
    let path = session_record_path(base, token)?;
    let parent = path.parent().ok_or_else(|| {
        format!(
            "session record path has no parent directory: {}",
            path.display()
        )
    })?;
    reject_symlink_components(parent)?;

    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove session record '{}': {e}", path.display())),
    }
}

fn read_session_record(path: &Path, token: &str) -> Result<String, String> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => format!("broker session token not found: {token}"),
            _ => format!("read session record '{}': {e}", path.display()),
        })?;

    let mut data = String::new();
    std::io::Read::read_to_string(&mut file, &mut data)
        .map_err(|e| format!("read session record '{}': {e}", path.display()))?;

    Ok(data)
}

fn session_record_path(base: &Path, token: &str) -> Result<PathBuf, String> {
    Ok(base.join(normalize_token(token)?).with_extension("json"))
}

fn normalize_token(token: &str) -> Result<&str, String> {
    if token.is_empty() {
        return Err("broker session token must not be empty".to_string());
    }

    let path = Path::new(token);
    match path.components().next() {
        Some(std::path::Component::Normal(_)) if path.components().count() == 1 => Ok(token),
        _ => Err(format!(
            "broker session token must be a single safe path segment: {token}"
        )),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;

    fn temp_test_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cloister-broker-store-{label}-{}",
            std::process::id()
        ))
    }

    fn sample_session(token: &str) -> BrokerSession {
        BrokerSession {
            token: token.to_string(),
            project_root: "/workspace/project".to_string(),
            dir_hash: "abc123def456".to_string(),
            profiles: BTreeMap::new(),
        }
    }

    #[test]
    fn session_store_round_trip_persists_session_record() {
        let dir = temp_test_dir("roundtrip");
        let store = session_store_dir(dir.to_str().unwrap());
        let session = sample_session("token-1");

        write_session_record(&store, &session).unwrap();
        let loaded = load_session_record(&store, &session.token).unwrap();

        assert_eq!(loaded, session);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_session_record_rejects_missing_token() {
        let dir = temp_test_dir("missing");
        let store = session_store_dir(dir.to_str().unwrap());

        let err = load_session_record(&store, "missing-token").unwrap_err();
        assert!(err.contains("missing-token"), "unexpected error: {err}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_session_record_rejects_symlinked_session_file() {
        let dir = temp_test_dir("symlink");
        let store = session_store_dir(dir.to_str().unwrap());
        let real_dir = dir.join("real");
        let real_record = real_dir.join("token-1.json");
        let link_record = store.join("token-1.json");

        fs::create_dir_all(&real_dir).unwrap();
        fs::create_dir_all(&store).unwrap();
        fs::write(
            &real_record,
            serde_json::to_vec(&sample_session("token-1")).unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(&real_record, &link_record).unwrap();

        let err = load_session_record(&store, "token-1").unwrap_err();
        assert!(
            err.contains("read session record"),
            "unexpected error: {err}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_session_record_rejects_symlinked_parent_store_dir() {
        let dir = temp_test_dir("load-store-symlink");
        let store = session_store_dir(dir.to_str().unwrap());
        let cloister_dir = dir.join("cloister");
        let redirect_dir = dir.join("redirected-cloister");
        let redirected_store = redirect_dir.join("broker").join("sessions");

        fs::create_dir_all(&redirected_store).unwrap();
        fs::write(
            redirected_store.join("token-1.json"),
            serde_json::to_vec(&sample_session("token-1")).unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(&redirect_dir, &cloister_dir).unwrap();

        let err = load_session_record(&store, "token-1").unwrap_err();
        assert!(
            err.contains("symlink path component"),
            "unexpected error: {err}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_session_record_rejects_symlinked_intermediate_store_dir() {
        let dir = temp_test_dir("store-symlink");
        let store = session_store_dir(dir.to_str().unwrap());
        let cloister_dir = dir.join("cloister");
        let redirect_dir = dir.join("redirected-cloister");

        fs::create_dir_all(&redirect_dir).unwrap();
        std::os::unix::fs::symlink(&redirect_dir, &cloister_dir).unwrap();

        let err = write_session_record(&store, &sample_session("token-1")).unwrap_err();
        assert!(
            err.contains("symlink path component"),
            "unexpected error: {err}"
        );
        assert!(!redirect_dir.join("broker").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_session_record_deletes_existing_record() {
        let dir = temp_test_dir("remove-existing");
        let store = session_store_dir(dir.to_str().unwrap());
        let session = sample_session("token-1");

        write_session_record(&store, &session).unwrap();
        let record_path = store.join("token-1.json");

        remove_session_record(&store, "token-1").unwrap();

        assert!(!record_path.exists(), "record should be removed");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_session_record_allows_missing_record() {
        let dir = temp_test_dir("remove-missing");
        let store = session_store_dir(dir.to_str().unwrap());
        let record_path = store.join("token-1.json");

        fs::create_dir_all(&store).unwrap();

        remove_session_record(&store, "token-1").unwrap();

        assert!(!record_path.exists(), "record should stay absent");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_session_record_rejects_symlinked_parent_store_dir() {
        let dir = temp_test_dir("remove-store-symlink");
        let store = session_store_dir(dir.to_str().unwrap());
        let cloister_dir = dir.join("cloister");
        let redirect_dir = dir.join("redirected-cloister");
        let redirected_store = redirect_dir.join("broker").join("sessions");
        let redirected_record = redirected_store.join("token-1.json");

        fs::create_dir_all(&redirected_store).unwrap();
        fs::write(
            &redirected_record,
            serde_json::to_vec(&sample_session("token-1")).unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(&redirect_dir, &cloister_dir).unwrap();

        let err = remove_session_record(&store, "token-1").unwrap_err();
        assert!(
            err.contains("symlink path component"),
            "unexpected error: {err}"
        );
        assert!(redirected_record.exists(), "record should remain untouched");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_store_dir_nests_under_host_runtime_dir() {
        assert_eq!(
            session_store_dir("/run/user/1000"),
            PathBuf::from("/run/user/1000/cloister/broker/sessions")
        );
    }

    #[test]
    fn socket_path_nests_under_host_runtime_dir() {
        assert_eq!(
            socket_path("/run/user/1000", "token-123").unwrap(),
            PathBuf::from("/run/user/1000/cloister/broker/token-123.sock")
        );
    }
}
