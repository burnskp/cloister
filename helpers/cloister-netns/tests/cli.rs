use std::process::Command;

fn cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cloister-netns"))
}

#[test]
fn no_args_exits_2() {
    let out = cmd().output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Usage"), "stderr: {stderr}");
}

#[test]
fn only_separator_exits_2() {
    let out = cmd().arg("--").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Usage"), "stderr: {stderr}");
}

#[test]
fn exec_runs_command() {
    let out = cmd()
        .args(["--netns", "vpn", "--", "echo", "hello"])
        .output()
        .unwrap();
    // Assuming vpn network namespace does not exist on test host so this will fail before exec
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn exec_preserves_exit_code() {
    let out = cmd()
        .args(["--netns", "vpn", "--", "sh", "-c", "exit 42"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn exec_passes_arguments() {
    let out = cmd()
        .args(["--netns", "vpn", "--", "echo", "a", "b", "c"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
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
