use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cloister-seccomp-filter"))
}

#[test]
fn missing_output_fails() {
    let status = bin().status().expect("failed to run binary");
    assert!(!status.success(), "should fail without --output");
}

#[test]
fn writes_bpf_file() {
    let dir = std::env::temp_dir();
    let path = dir.join("cli-test-output.bpf");

    let status = bin()
        .arg("--output")
        .arg(&path)
        .status()
        .expect("failed to run binary");

    assert!(status.success(), "should succeed with --output");

    let meta = std::fs::metadata(&path).expect("output file should exist");
    assert!(meta.len() > 0, "output file should be non-empty");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn allow_chromium_sandbox_flag() {
    let dir = std::env::temp_dir();
    let path = dir.join("cli-test-chromium.bpf");

    let status = bin()
        .arg("--output")
        .arg(&path)
        .arg("--allow-chromium-sandbox")
        .status()
        .expect("failed to run binary");

    assert!(
        status.success(),
        "should succeed with --allow-chromium-sandbox"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn unknown_arg_fails() {
    let dir = std::env::temp_dir();
    let path = dir.join("cli-test-unknown.bpf");

    let status = bin()
        .arg("--output")
        .arg(&path)
        .arg("--bogus")
        .status()
        .expect("failed to run binary");

    assert!(!status.success(), "should fail with unknown argument");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn default_and_chromium_produce_different_output() {
    let dir = std::env::temp_dir();
    let path_default = dir.join("cli-test-diff-default.bpf");
    let path_chromium = dir.join("cli-test-diff-chromium.bpf");

    let s1 = bin()
        .arg("--output")
        .arg(&path_default)
        .status()
        .expect("failed to run binary");
    assert!(s1.success());

    let s2 = bin()
        .arg("--output")
        .arg(&path_chromium)
        .arg("--allow-chromium-sandbox")
        .status()
        .expect("failed to run binary");
    assert!(s2.success());

    let buf1 = std::fs::read(&path_default).expect("read default");
    let buf2 = std::fs::read(&path_chromium).expect("read chromium");

    let _ = std::fs::remove_file(&path_default);
    let _ = std::fs::remove_file(&path_chromium);

    assert_ne!(
        buf1, buf2,
        "default and allow-chromium-sandbox BPF should differ"
    );
}
