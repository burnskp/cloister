use std::process::Command;

fn cmd() -> Command {
    let path = if let Some(path) = option_env!("CARGO_BIN_EXE_cloister-pipewire-validate") {
        path.to_owned()
    } else if let Ok(path) = std::env::var("CARGO_BIN_EXE_cloister-pipewire-validate") {
        path
    } else {
        let fallback = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/debug/cloister-pipewire-validate");
        if fallback.exists() {
            fallback.to_string_lossy().into_owned()
        } else {
            panic!("cloister-pipewire-validate test binary path is unavailable");
        }
    };
    Command::new(path)
}

#[test]
fn help_exits_zero() {
    let out = cmd().arg("--help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Usage:"), "stderr: {stderr}");
}

#[test]
fn unknown_argument_exits_nonzero() {
    let out = cmd().arg("--definitely-invalid").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Unknown argument"), "stderr: {stderr}");
}

#[test]
fn invalid_timeout_exits_nonzero() {
    let out = cmd().args(["--timeout-ms", "abc"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Invalid timeout"), "stderr: {stderr}");
}
