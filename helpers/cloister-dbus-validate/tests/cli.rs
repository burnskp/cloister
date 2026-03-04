use std::process::Command;

fn bin() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cloister-dbus-validate"));
    // Point D-Bus at a nonexistent socket so zbus doesn't auto-discover
    // the real session bus via /run/user/<uid>/bus.
    cmd.env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent-cloister-test")
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
fn quiet_flag_does_not_panic() {
    let status = bin().arg("--quiet").status().expect("failed to run binary");
    assert!(status.code().is_some());
}
