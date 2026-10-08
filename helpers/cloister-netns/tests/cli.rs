use std::process::Command;

fn cmd() -> Command {
    let path = if let Some(path) = option_env!("CARGO_BIN_EXE_cloister-netns") {
        path.to_owned()
    } else if let Ok(path) = std::env::var("CARGO_BIN_EXE_cloister-netns") {
        path
    } else {
        let fallback =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/cloister-netns");
        if fallback.exists() {
            fallback.to_string_lossy().into_owned()
        } else {
            panic!("cloister-netns test binary path is unavailable");
        }
    };
    Command::new(path)
}

#[test]
fn no_args_exits_2() {
    let out = cmd().output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Usage"), "stderr: {stderr}");
}

#[test]
fn required_group_exits_1_before_namespace_work() {
    let out = cmd()
        .args(["--netns", "vpn", "--", "echo", "hello"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("required group") || stderr.contains("not in the allowed list"),
        "stderr: {stderr}"
    );
}

#[test]
fn only_separator_exits_2() {
    let out = cmd().arg("--").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Usage"), "stderr: {stderr}");
}

#[test]
fn disallowed_namespace_exits_1() {
    let out = cmd()
        .args([
            "--netns",
            "definitely-not-allowlisted",
            "--",
            "echo",
            "hello",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not in the allowed list"),
        "stderr: {stderr}"
    );
}

#[test]
fn nonexistent_namespace_exits_1() {
    let out = cmd()
        .args([
            "--netns",
            "this-ns-does-not-exist-12345",
            "--",
            "echo",
            "hi",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cloister-netns"), "stderr: {stderr}");
}
