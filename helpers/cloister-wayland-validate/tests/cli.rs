use std::process::Command;

/// Return a `Command` for the binary with Wayland env vars cleared so tests
/// behave identically whether or not a compositor is running.
fn cmd() -> Command {
    let path = if let Some(path) = option_env!("CARGO_BIN_EXE_cloister-wayland-validate") {
        path.to_owned()
    } else if let Ok(path) = std::env::var("CARGO_BIN_EXE_cloister-wayland-validate") {
        path
    } else {
        let fallback = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/debug/cloister-wayland-validate");
        if fallback.exists() {
            fallback.to_string_lossy().into_owned()
        } else {
            panic!("cloister-wayland-validate test binary path is unavailable");
        }
    };
    let mut c = Command::new(path);
    c.env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("XDG_RUNTIME_DIR");
    c
}

#[test]
fn without_compositor_exits_1() {
    let out = cmd().output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cloister-wayland-validate"),
        "expected error on stderr, got: {stderr}"
    );
}
