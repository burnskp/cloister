use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cloister-seccomp-validate"))
}

#[test]
fn runs_without_crashing() {
    // Exit code depends on the environment: 0 (PASS) if syscalls are blocked
    // (e.g. running inside a seccomp sandbox), 1 (FAIL) if they're allowed.
    // Either outcome is valid — the test just verifies the binary runs cleanly.
    let status = bin().status().expect("failed to run binary");
    assert!(
        status.code() == Some(0) || status.code() == Some(1),
        "should exit 0 (PASS) or 1 (FAIL), got {:?}",
        status.code()
    );
}

#[test]
fn json_output_has_expected_structure() {
    let output = bin().arg("--json").output().expect("failed to run binary");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify basic JSON structure
    assert!(
        stdout.contains("\"results\""),
        "JSON should contain results array"
    );
    assert!(
        stdout.contains("\"pass\""),
        "JSON should contain pass field"
    );
    assert!(
        stdout.contains("\"name\""),
        "JSON should contain name fields"
    );
    assert!(
        stdout.contains("\"category\""),
        "JSON should contain category fields"
    );
    assert!(
        stdout.contains("\"status\""),
        "JSON should contain status fields"
    );

    // pass can be true or false depending on whether we're inside a sandbox
    assert!(
        stdout.contains("\"pass\": true") || stdout.contains("\"pass\": false"),
        "JSON should contain a pass field with a boolean value"
    );
}

#[test]
fn allow_chromium_sandbox_flag_accepted() {
    let output = bin()
        .arg("--allow-chromium-sandbox")
        .output()
        .expect("failed to run binary");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // chroot, unshare, setns, clone3, clone(CLONE_NEWUSER) should show as SKIPPED
    assert!(
        stdout.contains("SKIPPED (--allow-chromium-sandbox)"),
        "chromium sandbox syscalls should be skipped with --allow-chromium-sandbox"
    );
}

#[test]
fn unknown_arg_exits_2() {
    let status = bin().arg("--bogus").status().expect("failed to run binary");

    assert_eq!(
        status.code(),
        Some(2),
        "should exit 2 for unknown arguments"
    );
}
