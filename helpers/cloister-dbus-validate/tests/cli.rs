use std::process::Command;

fn bin_path() -> String {
    if let Some(path) = option_env!("CARGO_BIN_EXE_cloister-dbus-validate") {
        path.to_owned()
    } else if let Ok(path) = std::env::var("CARGO_BIN_EXE_cloister-dbus-validate") {
        path
    } else {
        let fallback = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/debug/cloister-dbus-validate");
        if fallback.exists() {
            fallback.to_string_lossy().into_owned()
        } else {
            panic!("cloister-dbus-validate test binary path is unavailable");
        }
    }
}

fn bin() -> Command {
    let mut cmd = Command::new(bin_path());
    // Point D-Bus at a nonexistent socket so zbus doesn't auto-discover
    // the real session bus via /run/user/<uid>/bus.
    cmd.env(
        "DBUS_SESSION_BUS_ADDRESS",
        "unix:path=/nonexistent-cloister-test",
    )
    .env_remove("XDG_RUNTIME_DIR");
    cmd
}

#[test]
fn without_bus_exits_1() {
    let status = bin().status().expect("failed to run binary");
    assert_eq!(status.code(), Some(1));
}

#[test]
fn unknown_arg_exits_2() {
    let status = bin().arg("--bogus").status().expect("failed to run binary");
    assert_eq!(status.code(), Some(2));
}

#[test]
fn missing_deny_value_exits_2() {
    let status = bin().arg("--deny").status().expect("failed to run binary");
    assert_eq!(status.code(), Some(2));
}

#[test]
fn json_without_bus_exits_1() {
    let status = bin().arg("--json").status().expect("failed to run binary");
    assert_eq!(status.code(), Some(1));
}
