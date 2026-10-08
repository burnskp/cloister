use std::collections::HashSet;
use std::io;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{self, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::thread;
use std::time::Duration;

use cloister_sandbox_lib::broker;
use cloister_sandbox_lib::broker_store;
use cloister_sandbox_lib::bwrap;
use cloister_sandbox_lib::config::{SandboxConfig, StoreMode, WorkspaceMode};
use cloister_sandbox_lib::env;
use cloister_sandbox_lib::features;
use cloister_sandbox_lib::runtime;
use cloister_sandbox_lib::seccomp;
use cloister_sandbox_lib::socket;
use cloister_sandbox_lib::ssh_filter;
use cloister_sandbox_lib::validate;
use cloister_sandbox_lib::vars;
use cloister_sandbox_lib::wayland;

/// PID of the active child process. 0 means no child is running.
static CHILD_PID: AtomicI32 = AtomicI32::new(0);

/// Host PID that should receive graceful signals (SIGINT/SIGTERM).
static GRACEFUL_PID: AtomicI32 = AtomicI32::new(0);

/// Host PID that should receive forced teardown signals (SIGKILL).
static FORCE_PID: AtomicI32 = AtomicI32::new(0);

/// Number of rapid consecutive SIGINTs (resets after [`SIGINT_ESCALATION_WINDOW_SECS`]).
static SIGINT_COUNT: AtomicI32 = AtomicI32::new(0);

/// Monotonic timestamp (seconds) of the last SIGINT, for escalation windowing.
static LAST_SIGINT_SEC: AtomicI64 = AtomicI64::new(0);

/// Whether the sandbox is running an interactive shell (no `--new-session`).
/// When true, the shell receives SIGINT directly from the terminal, so we
/// must not forward the first Ctrl-C ourselves.
static INTERACTIVE_MODE: AtomicBool = AtomicBool::new(false);

/// If consecutive Ctrl-C presses are more than this many seconds apart, the
/// escalation counter resets and the next press is treated as a fresh first press.
const SIGINT_ESCALATION_WINDOW_SECS: i64 = 2;

/// After forwarding SIGTERM, wait this many seconds before sending SIGKILL.
const SIGTERM_GRACE_SECS: libc::c_uint = 10;
const BROKER_PARENT_CAPABILITY_ENV: &str = "CLOISTER_BROKER_PARENT_CAPABILITY";
const BROKER_CHILD_PROFILE_ENV: &str = "CLOISTER_BROKER_CHILD_PROFILE";
const CLOISTER_CONFIG_PATH_ENV: &str = "CLOISTER_CONFIG_PATH";
const BROKER_LAUNCHER_PACKAGE_ENV: &str = "CLOISTER_WORKER_BROKER_LAUNCHER_PACKAGE";
const BROKER_TRUSTED_RECORD_ENV: &str = "CLOISTER_BROKER_TRUSTED_RECORD";
const BROKER_SESSION_RECORD_CHILD_PATH: &str = "/run/cloister/broker/session.json";
const BROKER_SOCKET_CHILD_PATH: &str = "/run/cloister/broker/socket";

fn broker_parent_capability_token() -> String {
    let mut bytes = [0_u8; 16];
    let mut random = std::fs::File::open("/dev/urandom")
        .expect("open /dev/urandom for broker parent capability token");
    random
        .read_exact(&mut bytes)
        .expect("read broker parent capability token bytes");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn forwarded_signal_set() -> libc::sigset_t {
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGHUP);
        libc::sigaddset(&mut set, libc::SIGALRM);
    }
    set
}

fn set_signal_mask(how: libc::c_int, set: &libc::sigset_t, old_set: Option<&mut libc::sigset_t>) {
    unsafe {
        libc::sigprocmask(
            how,
            set,
            old_set.map_or(std::ptr::null_mut(), |mask| mask as *mut libc::sigset_t),
        );
    }
}

fn clear_signal_targets() {
    CHILD_PID.store(0, Ordering::Release);
    GRACEFUL_PID.store(0, Ordering::Release);
    FORCE_PID.store(0, Ordering::Release);
}

fn clear_signal_targets_blocking_signals() {
    let block_set = forwarded_signal_set();
    let mut old_set: libc::sigset_t = unsafe { std::mem::zeroed() };
    set_signal_mask(libc::SIG_BLOCK, &block_set, Some(&mut old_set));
    clear_signal_targets();
    unsafe {
        libc::alarm(0);
    }
    set_signal_mask(libc::SIG_SETMASK, &old_set, None);
}

fn graceful_signal_target(pid: libc::pid_t) -> libc::pid_t {
    pid
}

fn force_signal_target(pid: libc::pid_t, interactive: bool) -> libc::pid_t {
    if interactive { pid } else { -pid }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrokerLaunchSelector {
    profile: String,
    sandbox: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliArgs {
    config_path: Option<String>,
    after_netns: bool,
    broker_launch: Option<BrokerLaunchSelector>,
    broker_trusted_record_path: Option<String>,
    sandbox_args: Vec<String>,
}

fn set_config_path(cli: &mut CliArgs, path: String) -> Result<(), String> {
    if cli.config_path.replace(path).is_some() {
        return Err("cloister-sandbox: --config may only be passed once".to_string());
    }
    Ok(())
}

fn set_after_netns(cli: &mut CliArgs) -> Result<(), String> {
    if cli.after_netns {
        return Err("cloister-sandbox: --after-netns may only be passed once".to_string());
    }
    cli.after_netns = true;
    Ok(())
}

fn set_broker_launch_profile(cli: &mut CliArgs, profile: String) -> Result<(), String> {
    let selector = cli
        .broker_launch
        .get_or_insert_with(|| BrokerLaunchSelector {
            profile: String::new(),
            sandbox: String::new(),
        });
    if !selector.profile.is_empty() {
        return Err(
            "cloister-sandbox: --broker-launch-profile may only be passed once".to_string(),
        );
    }
    selector.profile = profile;
    Ok(())
}

fn set_broker_launch_sandbox(cli: &mut CliArgs, sandbox: String) -> Result<(), String> {
    let selector = cli
        .broker_launch
        .get_or_insert_with(|| BrokerLaunchSelector {
            profile: String::new(),
            sandbox: String::new(),
        });
    if !selector.sandbox.is_empty() {
        return Err(
            "cloister-sandbox: --broker-launch-sandbox may only be passed once".to_string(),
        );
    }
    selector.sandbox = sandbox;
    Ok(())
}

fn set_broker_trusted_record_path(cli: &mut CliArgs, path: String) -> Result<(), String> {
    if cli.broker_trusted_record_path.replace(path).is_some() {
        return Err(
            "cloister-sandbox: --broker-trusted-record may only be passed once".to_string(),
        );
    }
    Ok(())
}

fn sandbox_signal_pid_with_reader<R, N>(
    root_pid: u32,
    mut read_children: R,
    mut read_nspid: N,
) -> Result<(u32, u32), String>
where
    R: FnMut(u32) -> Result<Vec<u32>, String> + Copy,
    N: FnMut(u32) -> Result<Vec<u32>, String> + Copy,
{
    let root_nspid = read_nspid(root_pid)?;
    let expected_depth = root_nspid.len() + 1;
    let sandbox_pid1 = find_descendant_pid(root_pid, read_children, |pid| {
        let nspid = read_nspid(pid)?;
        Ok(is_sandbox_pid1_nspid(&nspid, expected_depth))
    })?;

    let graceful_pid = read_children(sandbox_pid1)
        .unwrap_or_default()
        .into_iter()
        .find(|&pid| {
            read_nspid(pid)
                .map(|nspid| nspid.last() == Some(&2))
                .unwrap_or(false)
        })
        .unwrap_or(sandbox_pid1);

    Ok((graceful_pid, sandbox_pid1))
}

fn find_descendant_pid<F, R>(
    root_pid: u32,
    mut read_children: R,
    mut predicate: F,
) -> Result<u32, String>
where
    F: FnMut(u32) -> Result<bool, String>,
    R: FnMut(u32) -> Result<Vec<u32>, String>,
{
    let mut pending = read_children(root_pid)?;
    let mut visited = HashSet::new();

    while let Some(pid) = pending.pop() {
        if !visited.insert(pid) {
            continue;
        }

        if predicate(pid)? {
            return Ok(pid);
        }

        match read_children(pid) {
            Ok(children) => pending.extend(children),
            Err(_) => continue,
        }
    }

    Err(format!("failed to find matching child for pid {root_pid}"))
}

fn is_sandbox_pid1_nspid(nspid: &[u32], expected_depth: usize) -> bool {
    nspid.len() == expected_depth && nspid.last() == Some(&1)
}

fn parse_proc_nspid(content: &str) -> Result<Vec<u32>, String> {
    let line = content
        .lines()
        .find(|line| line.starts_with("NSpid:"))
        .ok_or_else(|| "missing NSpid field".to_string())?;
    let nspid = line
        .strip_prefix("NSpid:")
        .unwrap_or_default()
        .split_ascii_whitespace()
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|e| format!("invalid NSpid value '{value}': {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if nspid.is_empty() {
        return Err("empty NSpid field".to_string());
    }

    Ok(nspid)
}

fn read_proc_nspid(pid: u32) -> Result<Vec<u32>, String> {
    let path = format!("/proc/{pid}/status");
    let content = std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?;
    parse_proc_nspid(&content).map_err(|e| format!("parse {path}: {e}"))
}

fn sandbox_signal_pid(root_pid: u32) -> Result<u32, String> {
    sandbox_signal_pid_with_reader(root_pid, read_proc_children, read_proc_nspid)
        .map(|(pid, _)| pid)
}

fn find_signal_pids(root_pid: u32, interactive: bool) -> Result<(u32, u32), String> {
    if interactive {
        return Ok((root_pid, root_pid));
    }

    sandbox_signal_pid(root_pid).map(|graceful_pid| (graceful_pid, root_pid))
}

fn wait_for_signal_pids(root_pid: u32, interactive: bool) -> Result<(u32, u32), String> {
    if interactive {
        return Ok((root_pid, root_pid));
    }

    let poll = Duration::from_millis(FLATPAK_CHILD_PID_POLL_MS);
    let attempts = FLATPAK_CHILD_PID_TIMEOUT_MS / FLATPAK_CHILD_PID_POLL_MS;
    let mut last_err = None;

    for _ in 0..attempts {
        match find_signal_pids(root_pid, interactive) {
            Ok(pids) => return Ok(pids),
            Err(err) => last_err = Some(err),
        }
        thread::sleep(poll);
    }

    Err(last_err.unwrap_or_else(|| {
        format!("timed out waiting for signal target for bubblewrap pid {root_pid}")
    }))
}

const FLATPAK_CHILD_PID_TIMEOUT_MS: u64 = 500;
const FLATPAK_CHILD_PID_POLL_MS: u64 = 10;
const DBUS_PROXY_SOCKET_TIMEOUT_MS: u64 = 3_000;
const DBUS_PROXY_SOCKET_POLL_MS: u64 = 20;

struct PulseOnlyBridge {
    child: process::Child,
    runtime_dir: String,
    socket_path: String,
    anonymize_file_paths: Vec<String>,
}

struct ProcPrivacyState {
    file_paths: Vec<String>,
}

struct FlatpakPortalState {
    flatpak_info_path: String,
    instance_dir: String,
}

struct DbusProxyState {
    child: process::Child,
    socket_path: String,
}

struct CleanupState {
    ssh_handle: Option<ssh_filter::SshFilterHandle>,
    dbus_proxy: Option<DbusProxyState>,
    pulse_bridge: Option<PulseOnlyBridge>,
    wayland_socket: Option<String>,
    machine_id_path: Option<String>,
    proc_privacy_state: Option<ProcPrivacyState>,
    anonymize_file_paths: Vec<String>,
    flatpak_portal_state: Option<FlatpakPortalState>,
    broker_session_record_path: Option<PathBuf>,
    broker_listener: Option<BrokerListenerHandle>,
}

struct ParentBrokerLaunchRegistration {
    env_args: Vec<String>,
    broker_session_record_path: Option<PathBuf>,
}

struct BrokerListenerHandle {
    socket_path: PathBuf,
    stop: Arc<AtomicBool>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl BrokerListenerHandle {
    fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[derive(Debug, serde::Deserialize)]
struct ImageStoreMeta {
    version: u64,
    mode: String,
    #[serde(rename = "storeId")]
    store_id: String,
}

fn anonymized_identity(config: &SandboxConfig) -> Result<&str, String> {
    config.anonymized_identity().ok_or_else(|| {
        "anonymized sandbox identity is missing; sandbox_home must end with a username".to_string()
    })
}

fn pulse_proxy_identity(config: &SandboxConfig) -> Result<&str, String> {
    anonymized_identity(config)
}

struct PulseProxyCommand<'a> {
    runtime_dir: &'a str,
    pipewire_remote: &'a str,
    identity: &'a str,
    passwd_path: Option<&'a str>,
    group_path: Option<&'a str>,
    pipewire_pulse_bin: &'a str,
    pipewire_pulse_conf: &'a str,
}

fn build_pulse_only_proxy_command(
    config: &SandboxConfig,
    proxy: PulseProxyCommand<'_>,
) -> Result<process::Command, String> {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    let uid_str = uid.to_string();
    let gid_str = gid.to_string();
    let proxy_home = format!("/home/{}", proxy.identity);
    let store_bind_args = match config.store_mode {
        StoreMode::Host => host_store_bind_args(),
        StoreMode::ImageStore => {
            image_store_bind_args(config.store_mount_path.as_deref().ok_or_else(|| {
                "missing store_mount_path for image-store pulse proxy".to_string()
            })?)
        }
    };

    let mut cmd = process::Command::new(&config.bwrap_path);
    cmd.args([
        "--die-with-parent",
        "--new-session",
        "--unshare-all",
        "--hostname",
        proxy.identity,
        "--uid",
        &uid_str,
        "--gid",
        &gid_str,
        "--clearenv",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/nix",
        "--dir",
        "/nix/store",
        "--dir",
        "/home",
        "--dir",
        &proxy_home,
        "--bind",
        proxy.runtime_dir,
        proxy.runtime_dir,
        "--ro-bind",
        proxy.pipewire_remote,
        proxy.pipewire_remote,
        "--ro-bind",
        proxy.pipewire_pulse_conf,
        proxy.pipewire_pulse_conf,
    ]);
    cmd.args(&store_bind_args);

    if let Some(path) = proxy.passwd_path {
        cmd.args(["--ro-bind", path, "/etc/passwd"]);
    }
    if let Some(path) = proxy.group_path {
        cmd.args(["--ro-bind", path, "/etc/group"]);
    }

    cmd.args([
        "--setenv",
        "HOME",
        &proxy_home,
        "--setenv",
        "USER",
        proxy.identity,
        "--setenv",
        "LOGNAME",
        proxy.identity,
        "--setenv",
        "XDG_RUNTIME_DIR",
        proxy.runtime_dir,
        "--setenv",
        "PULSE_RUNTIME_PATH",
        proxy.runtime_dir,
        "--setenv",
        "PIPEWIRE_REMOTE",
        proxy.pipewire_remote,
        "--chdir",
        "/",
        "--",
        proxy.pipewire_pulse_bin,
        "-c",
        proxy.pipewire_pulse_conf,
    ]);

    Ok(cmd)
}

fn host_store_bind_args() -> Vec<String> {
    vec![
        "--ro-bind".to_string(),
        "/nix/store".to_string(),
        "/nix/store".to_string(),
    ]
}

fn image_store_bind_args(store_mount_path: &str) -> Vec<String> {
    let store_path = format!("{store_mount_path}/nix/store");
    vec![
        "--ro-bind".to_string(),
        store_path,
        "/nix/store".to_string(),
    ]
}

fn mount_flags_indicate_read_only(flags: libc::c_ulong) -> bool {
    flags & libc::ST_RDONLY == libc::ST_RDONLY
}

fn mount_is_read_only(path: &Path) -> Result<bool, String> {
    let path_bytes = path.as_os_str().as_bytes();
    let path_cstr = std::ffi::CString::new(path_bytes)
        .map_err(|_| format!("mount path '{}' contains interior NUL byte", path.display()))?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(path_cstr.as_ptr(), &mut stat) };
    if rc != 0 {
        return Err(format!(
            "failed to statvfs '{}' for mount flags: {}",
            path.display(),
            io::Error::last_os_error()
        ));
    }
    Ok(mount_flags_indicate_read_only(stat.f_flag))
}

fn validate_image_store_meta(
    meta: &ImageStoreMeta,
    meta_path: &Path,
    expected_store_id: &str,
    prefix: &str,
) -> Result<(), String> {
    if meta.version != 1 {
        return Err(format!(
            "{prefix}: image-store metadata '{}' has unsupported version {}",
            meta_path.display(),
            meta.version
        ));
    }
    if meta.mode != "image-store" {
        return Err(format!(
            "{prefix}: image-store metadata '{}' has unexpected mode '{}'",
            meta_path.display(),
            meta.mode
        ));
    }
    if meta.store_id != expected_store_id {
        return Err(format!(
            "{prefix}: image-store metadata '{}' has store ID '{}' but expected '{}'",
            meta_path.display(),
            meta.store_id,
            expected_store_id
        ));
    }

    Ok(())
}

fn ensure_image_store_mounted(config: &SandboxConfig, prefix: &str) -> Result<String, String> {
    let store_id = config
        .store_id
        .as_deref()
        .ok_or_else(|| format!("{prefix}: missing store_id for image-store mode"))?;
    let store_mount_path = config
        .store_mount_path
        .as_deref()
        .ok_or_else(|| format!("{prefix}: missing store_mount_path for image-store mode"))?;
    let store_image_path = config
        .store_image_path
        .as_deref()
        .ok_or_else(|| format!("{prefix}: missing store_image_path for image-store mode"))?;

    let mount_path = Path::new(store_mount_path);
    if !mount_path.is_dir() {
        return Err(format!(
            "{prefix}: image-store mount '{}' is missing; ensure the cloister-image-store NixOS module is enabled and system mounts are active for store '{}' ({})",
            store_mount_path, store_id, store_image_path
        ));
    }

    let store_base = mount_path.join("nix/store");
    if !store_base.is_dir() {
        return Err(format!(
            "{prefix}: image-store mount '{}' does not contain /nix/store",
            store_mount_path
        ));
    }

    if !is_mountpoint(mount_path)? {
        return Err(format!(
            "{prefix}: image-store mount '{}' is not an active mountpoint",
            store_mount_path
        ));
    }
    if !mount_is_read_only(mount_path)? {
        return Err(format!(
            "{prefix}: image-store mount '{}' is writable; expected a read-only squashfs mount for store '{}' ({})",
            store_mount_path, store_id, store_image_path
        ));
    }

    let meta_path = mount_path.join("meta.json");
    let meta = read_image_store_meta(&meta_path, prefix)?;
    validate_image_store_meta(&meta, &meta_path, store_id, prefix)?;

    Ok(store_mount_path.to_string())
}

fn is_mountpoint(path: &Path) -> Result<bool, String> {
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("failed to stat '{}': {e}", path.display()))?;
    let Some(parent) = path.parent() else {
        return Ok(true);
    };
    let parent_metadata = std::fs::metadata(parent)
        .map_err(|e| format!("failed to stat parent '{}': {e}", parent.display()))?;

    Ok(metadata.dev() != parent_metadata.dev() || metadata.ino() == parent_metadata.ino())
}

fn read_image_store_meta(path: &Path, prefix: &str) -> Result<ImageStoreMeta, String> {
    let data = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "{prefix}: image-store metadata '{}' is unreadable: {e}",
            path.display()
        )
    })?;
    serde_json::from_str(&data).map_err(|e| {
        format!(
            "{prefix}: image-store metadata '{}' is invalid JSON: {e}",
            path.display()
        )
    })
}

/// Write to stderr from a signal handler (async-signal-safe).
fn signal_write_stderr(msg: &[u8]) {
    unsafe {
        libc::write(
            libc::STDERR_FILENO,
            msg.as_ptr() as *const libc::c_void,
            msg.len(),
        );
    }
}

/// Get the current monotonic time in seconds (async-signal-safe).
fn monotonic_seconds() -> i64 {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts.tv_sec
}

/// Signal handler that forwards signals to the child process with escalation.
///
/// - **SIGINT (non-interactive)**: Rapid consecutive presses escalate:
///   SIGINT → SIGTERM → SIGKILL. Presses spaced more than
///   [`SIGINT_ESCALATION_WINDOW_SECS`] apart reset the counter, so normal
///   interactive Ctrl-C usage (e.g. cancelling a shell command) is unaffected.
/// - **SIGINT (interactive)**: The first press is **not forwarded** because the
///   shell already receives SIGINT directly from the terminal (no `--new-session`).
///   The 2nd and 3rd rapid presses still escalate to SIGTERM / SIGKILL.
/// - **SIGTERM**: Forwarded to the child; a [`SIGTERM_GRACE_SECS`] alarm is set to
///   send SIGKILL if the child hasn't exited by then.
/// - **SIGALRM**: Sends SIGKILL to the child (grace period expired).
/// - **SIGHUP**: Forwarded directly.
///
/// Only uses async-signal-safe operations (atomics, `kill`, `write`, `clock_gettime`, `alarm`).
extern "C" fn forward_signal(sig: libc::c_int) {
    let graceful_pid = GRACEFUL_PID.load(Ordering::Acquire);
    let force_pid = FORCE_PID
        .load(Ordering::Acquire)
        .max(CHILD_PID.load(Ordering::Acquire));
    if graceful_pid <= 0 && force_pid <= 0 {
        return;
    }

    let interactive = INTERACTIVE_MODE.load(Ordering::Acquire);
    let graceful_target = if graceful_pid > 0 {
        graceful_signal_target(graceful_pid)
    } else {
        0
    };
    let force_target = if force_pid > 0 {
        force_signal_target(force_pid, interactive)
    } else {
        0
    };

    match sig {
        libc::SIGINT => {
            let now = monotonic_seconds();
            let last = LAST_SIGINT_SEC.swap(now, Ordering::AcqRel);
            let count = if last == 0 || (now - last) > SIGINT_ESCALATION_WINDOW_SECS {
                SIGINT_COUNT.store(1, Ordering::Release);
                1
            } else {
                SIGINT_COUNT.fetch_add(1, Ordering::AcqRel) + 1
            };

            match count {
                1 => {
                    if !interactive && graceful_target != 0 {
                        unsafe {
                            libc::kill(graceful_target, libc::SIGINT);
                        }
                    }
                    // In interactive mode the shell gets SIGINT from the terminal directly.
                }
                2 => {
                    signal_write_stderr(
                        b"\ncloister-sandbox: requesting sandbox shutdown (Ctrl-C again to force)...\n",
                    );
                    if graceful_target != 0 {
                        unsafe {
                            libc::kill(graceful_target, libc::SIGTERM);
                        }
                    }
                }
                _ => {
                    signal_write_stderr(b"\ncloister-sandbox: force-killing sandbox.\n");
                    if force_target != 0 {
                        unsafe {
                            libc::kill(force_target, libc::SIGKILL);
                        }
                    }
                }
            }
        }
        libc::SIGTERM => unsafe {
            if graceful_target != 0 {
                libc::kill(graceful_target, libc::SIGTERM);
            }
            libc::alarm(SIGTERM_GRACE_SECS);
        },
        libc::SIGALRM => {
            signal_write_stderr(
                b"cloister-sandbox: graceful shutdown timed out, force-killing sandbox.\n",
            );
            if force_target != 0 {
                unsafe {
                    libc::kill(force_target, libc::SIGKILL);
                }
            }
        }
        _ => unsafe {
            if graceful_target != 0 {
                libc::kill(graceful_target, sig);
            }
        },
    }
}

/// Install signal handlers for SIGTERM, SIGINT, SIGHUP, and SIGALRM.
fn install_signal_handlers() {
    for &sig in &[libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGALRM] {
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = forward_signal as *const () as usize;
            sa.sa_flags = libc::SA_RESTART;
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

/// Spawn a child process and wait for it, storing its PID so signal handlers
/// can forward signals to it.
///
/// When `interactive` is true, a `pre_exec` hook is installed that:
/// 1. Sets SIGINT to `SIG_IGN` so the bwrap outer process ignores Ctrl-C
///    (the shell inside handles it via the terminal).
/// 2. Resets the signal mask to empty so the child starts with clean signals.
///
/// Blocks SIGTERM/SIGINT/SIGHUP/SIGALRM around the spawn→store window to
/// prevent signals from being dropped when CHILD_PID is still 0.
fn spawn_and_wait(
    cmd: &mut process::Command,
    interactive: bool,
    flatpak_state: Option<&FlatpakPortalState>,
) -> io::Result<ExitStatus> {
    // Reset escalation state for new child
    SIGINT_COUNT.store(0, Ordering::Release);
    LAST_SIGINT_SEC.store(0, Ordering::Release);

    // Block forwarded signals before spawn so none are lost in the race window
    let mut old_set: libc::sigset_t = unsafe { std::mem::zeroed() };
    let block_set = forwarded_signal_set();
    set_signal_mask(libc::SIG_BLOCK, &block_set, Some(&mut old_set));

    // Safety: only calls async-signal-safe functions (signal, sigemptyset,
    // sigprocmask). Runs between fork and exec in the child process.
    unsafe {
        cmd.pre_exec(move || {
            if interactive {
                // Ignore SIGINT in bwrap's outer process so Ctrl-C doesn't kill it.
                libc::signal(libc::SIGINT, libc::SIG_IGN);
            }

            // Reset signal mask so the child starts with no blocked signals.
            let mut empty_set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut empty_set);
            libc::sigprocmask(libc::SIG_SETMASK, &empty_set, std::ptr::null_mut());

            Ok(())
        });
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            // Restore signal mask before returning error
            unsafe {
                libc::sigprocmask(libc::SIG_SETMASK, &old_set, std::ptr::null_mut());
            }
            return Err(e);
        }
    };
    CHILD_PID.store(child.id() as i32, Ordering::Release);
    let (graceful_pid, force_pid) =
        wait_for_signal_pids(child.id(), interactive).unwrap_or((child.id(), child.id()));
    GRACEFUL_PID.store(graceful_pid as i32, Ordering::Release);
    FORCE_PID.store(force_pid as i32, Ordering::Release);

    if let Some(state) = flatpak_state {
        let bwrapinfo_path = format!("{}/bwrapinfo.json", state.instance_dir);
        let setup_result = (|| {
            let sandbox_pid = wait_for_flatpak_sandbox_pid(child.id())?;
            runtime::write_flatpak_bwrapinfo(&bwrapinfo_path, sandbox_pid)
        })();
        if let Err(err) = setup_result {
            let _ = child.kill();
            let _ = child.wait();
            clear_signal_targets();
            unsafe { libc::alarm(0) };
            set_signal_mask(libc::SIG_SETMASK, &old_set, None);
            return Err(io::Error::other(err));
        }
    }

    // Restore old signal mask — any pending signals are delivered now
    set_signal_mask(libc::SIG_SETMASK, &old_set, None);

    let status = child.wait();
    // Block the forwarded signals while clearing stale targets so a late
    // delivery cannot hit a recycled PID or process group.
    clear_signal_targets_blocking_signals();
    status
}

fn parse_proc_children(content: &str) -> Vec<u32> {
    content
        .split_ascii_whitespace()
        .filter_map(|value| value.parse::<u32>().ok())
        .collect()
}

fn read_proc_children(pid: u32) -> Result<Vec<u32>, String> {
    let path = format!("/proc/{pid}/task/{pid}/children");
    let content = std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?;
    Ok(parse_proc_children(&content))
}

fn has_flatpak_info(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}/root/.flatpak-info")).exists()
}

fn find_flatpak_sandbox_pid(root_pid: u32) -> Result<u32, String> {
    find_descendant_pid(root_pid, read_proc_children, |pid| {
        Ok(has_flatpak_info(pid))
    })
    .map_err(|e| format!("discover sandbox child for bubblewrap pid {root_pid}: {e}"))
}

fn wait_for_flatpak_sandbox_pid(root_pid: u32) -> Result<u32, String> {
    let poll = Duration::from_millis(FLATPAK_CHILD_PID_POLL_MS);
    let attempts = FLATPAK_CHILD_PID_TIMEOUT_MS / FLATPAK_CHILD_PID_POLL_MS;
    let mut last_err = None;

    for _ in 0..attempts {
        if has_flatpak_info(root_pid) {
            return Ok(root_pid);
        }
        match find_flatpak_sandbox_pid(root_pid) {
            Ok(pid) => return Ok(pid),
            Err(err) => last_err = Some(err),
        }
        thread::sleep(poll);
    }

    Err(last_err.unwrap_or_else(|| {
        format!("timed out waiting for sandbox child for bubblewrap pid {root_pid}")
    }))
}

fn wait_for_dbus_proxy_socket(path: &str, child: &mut process::Child) -> Result<(), String> {
    let poll = Duration::from_millis(DBUS_PROXY_SOCKET_POLL_MS);
    let attempts = DBUS_PROXY_SOCKET_TIMEOUT_MS / DBUS_PROXY_SOCKET_POLL_MS;

    for _ in 0..attempts {
        if socket::validate_existing_socket(path).is_ok() {
            return Ok(());
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!(
                    "dbus proxy exited before socket became ready: {}",
                    status
                ));
            }
            Ok(None) => {}
            Err(e) => return Err(format!("check dbus proxy status: {e}")),
        }

        thread::sleep(poll);
    }

    socket::validate_existing_socket(path)
        .map(|_| ())
        .map_err(|e| format!("dbus proxy socket not ready at {path}: {e}"))
}

fn start_pulse_only_bridge(
    config: &SandboxConfig,
    xdg_runtime_dir: &str,
    prefix: &str,
) -> Result<Option<PulseOnlyBridge>, String> {
    let (Some(backend_socket_name), Some(pipewire_pulse_bin), Some(pipewire_pulse_conf)) = (
        &config.pipewire_backend_socket_name,
        &config.pipewire_pulse_binary_path,
        &config.pipewire_pulse_config_path,
    ) else {
        return Ok(None);
    };

    let runtime_dir = format!(
        "{xdg_runtime_dir}/cloister/pulse/{}-{}",
        config.name,
        process::id()
    );
    let socket_path = format!("{runtime_dir}/native");
    let pipewire_remote = format!("{xdg_runtime_dir}/{backend_socket_name}");

    if let Err(e) = socket::validate_existing_socket(&pipewire_remote) {
        return Err(format!(
            "{prefix}: pulse-only backend socket '{pipewire_remote}': {e}"
        ));
    }

    std::fs::create_dir_all(&runtime_dir)
        .map_err(|e| format!("{prefix}: pulse-only runtime dir '{runtime_dir}': {e}"))?;
    std::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(0o700)).map_err(
        |e| format!("{prefix}: pulse-only runtime dir permissions '{runtime_dir}': {e}"),
    )?;
    socket::validate_socket_parent(&socket_path)
        .map_err(|e| format!("{prefix}: pulse-only socket path '{socket_path}': {e}"))?;
    socket::remove_stale_socket(&socket_path)
        .map_err(|e| format!("{prefix}: pulse-only socket cleanup '{socket_path}': {e}"))?;

    let (anonymize_file_paths, mut child) = if config.anonymize {
        let identity = pulse_proxy_identity(config)?;
        let proxy_home = format!("/home/{identity}");
        let overlays = features::anonymize_identity_args(
            identity,
            &config.shell_bin,
            &proxy_home,
            config.network_namespace.as_deref(),
        );

        let passwd_path = overlays.file_paths.first().map(String::as_str);
        let group_path = overlays.file_paths.get(1).map(String::as_str);

        let mut cmd = build_pulse_only_proxy_command(
            config,
            PulseProxyCommand {
                runtime_dir: &runtime_dir,
                pipewire_remote: &pipewire_remote,
                identity,
                passwd_path,
                group_path,
                pipewire_pulse_bin,
                pipewire_pulse_conf,
            },
        )?;

        let child = cmd
            .spawn()
            .map_err(|e| format!("{prefix}: start anonymized pulse-only proxy: {e}"))?;
        (overlays.file_paths, child)
    } else {
        let child = process::Command::new(pipewire_pulse_bin)
            .arg("-c")
            .arg(pipewire_pulse_conf)
            .env("PIPEWIRE_REMOTE", &pipewire_remote)
            .env("PULSE_RUNTIME_PATH", &runtime_dir)
            .spawn()
            .map_err(|e| format!("{prefix}: start pulse-only pipewire-pulse: {e}"))?;
        (Vec::new(), child)
    };

    for _ in 0..50 {
        if Path::new(&socket_path)
            .metadata()
            .map(|meta| meta.file_type().is_socket())
            .unwrap_or(false)
        {
            if let Err(e) = socket::validate_existing_socket(&socket_path) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = socket::remove_stale_socket(&socket_path);
                for path in &anonymize_file_paths {
                    let _ = std::fs::remove_file(path);
                }
                return Err(format!(
                    "{prefix}: pulse-only bridge produced invalid socket '{socket_path}': {e}"
                ));
            }
            return Ok(Some(PulseOnlyBridge {
                child,
                runtime_dir,
                socket_path,
                anonymize_file_paths,
            }));
        }

        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("{prefix}: poll pulse-only pipewire-pulse: {e}"))?
        {
            for path in &anonymize_file_paths {
                let _ = std::fs::remove_file(path);
            }
            let _ = socket::remove_stale_socket(&socket_path);
            return Err(format!(
                "{prefix}: pulse-only pipewire-pulse exited before creating {socket_path} (status: {status})"
            ));
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = socket::remove_stale_socket(&socket_path);
    for path in &anonymize_file_paths {
        let _ = std::fs::remove_file(path);
    }
    Err(format!(
        "{prefix}: timed out waiting for pulse-only socket {socket_path}"
    ))
}

fn main() {
    let exit_code = run();
    process::exit(exit_code);
}

fn err_prefix(name: &str) -> String {
    format!("cloister-sandbox[{name}]")
}

fn prepare_run_cmd(
    config: &SandboxConfig,
    sandbox_args: &[String],
    broker_launch: Option<&BrokerLaunchSelector>,
) -> Result<(Vec<String>, bool), &'static str> {
    if broker_launch.is_some() {
        if sandbox_args.is_empty() {
            return Err("broker launcher requires a command argv");
        }
        if matches!(sandbox_args.first().map(String::as_str), Some("-c")) {
            return Err("broker launcher does not support -c; pass the command argv directly");
        }
        return Ok((sandbox_args.to_vec(), false));
    }

    let parsed_args = env::parse_sandbox_args(sandbox_args)?;
    let run_cmd = env::build_run_cmd(
        &config.shell_bin,
        &config.shell_interactive_args,
        &config.wrapped_command_shell_args,
        config.default_command.as_deref(),
        &parsed_args,
    );
    let is_interactive = env::is_interactive(&parsed_args, config.default_command.as_deref());

    Ok((run_cmd, is_interactive))
}

pub fn create_parent_broker_session(
    config: &SandboxConfig,
    project_root: &str,
    dir_hash: &str,
) -> Result<Option<broker::BrokerSession>, String> {
    if config.worker_broker.profiles.is_empty() {
        return Ok(None);
    }

    if project_root.is_empty() || dir_hash.is_empty() {
        return Err(
            "worker broker session requires a non-empty project root and dir hash".to_string(),
        );
    }

    let child_visible_project_root = if config.anonymize {
        runtime::remap_path_for_anonymize(
            project_root,
            &config.home_directory,
            &config.sandbox_home,
        )
    } else {
        project_root.to_string()
    };

    Ok(Some(broker::BrokerSession {
        token: broker_parent_capability_token(),
        project_root: child_visible_project_root,
        dir_hash: dir_hash.to_string(),
        profiles: config
            .worker_broker
            .profiles
            .iter()
            .map(|(name, profile)| {
                (
                    name.clone(),
                    broker::BrokerSpawnableProfile {
                        sandbox: profile.sandbox.clone(),
                        workspace_mode: profile.workspace.mode,
                        delegated_per_dir_mounts: profile.delegated_per_dir_mounts.clone(),
                    },
                )
            })
            .collect(),
    }))
}

pub fn parent_broker_env_args(session: &broker::BrokerSession) -> Result<Vec<String>, String> {
    let payload = serde_json::to_string(&broker::BrokerParentCapability::from(session))
        .map_err(|e| format!("serialize parent broker capability: {e}"))?;
    Ok(vec![
        "--setenv".to_string(),
        BROKER_PARENT_CAPABILITY_ENV.to_string(),
        payload,
    ])
}

fn register_parent_broker_launch(
    config: &SandboxConfig,
    project_root: &str,
    dir_hash: &str,
    host_runtime_dir: &str,
) -> Result<Option<ParentBrokerLaunchRegistration>, String> {
    let Some(session) = create_parent_broker_session(config, project_root, dir_hash)? else {
        return Ok(None);
    };

    if host_runtime_dir.is_empty() {
        return Err("worker broker parent launch requires XDG_RUNTIME_DIR".to_string());
    }

    let store = broker_store::session_store_dir(host_runtime_dir);
    broker_store::write_session_record(&store, &session)?;
    let record_path = store.join(format!("{}.json", session.token));

    let socket_path = broker_client_socket_path(host_runtime_dir, &session.token)?;

    let mut args = vec![
        "--ro-bind".to_string(),
        record_path.to_string_lossy().to_string(),
        BROKER_SESSION_RECORD_CHILD_PATH.to_string(),
        "--bind".to_string(),
        socket_path.to_string_lossy().to_string(),
        BROKER_SOCKET_CHILD_PATH.to_string(),
    ];
    args.extend(parent_broker_env_args(&session)?);
    Ok(Some(ParentBrokerLaunchRegistration {
        env_args: args,
        broker_session_record_path: Some(record_path),
    }))
}

fn broker_client_socket_path(host_runtime_dir: &str, token: &str) -> Result<PathBuf, String> {
    broker_store::socket_path(host_runtime_dir, token)
}

fn broker_launcher_path(
    generated_launchers: &std::collections::BTreeMap<
        String,
        cloister_sandbox_lib::config::GeneratedLauncher,
    >,
    profile_name: &str,
    sandbox_name: &str,
) -> Result<PathBuf, String> {
    let launcher_name = generated_launchers
        .iter()
        .find_map(|(launcher_name, launcher)| {
            (launcher.profile == profile_name && launcher.sandbox == sandbox_name)
                .then_some(launcher_name)
        })
        .ok_or_else(|| {
            format!(
                "no generated worker broker launcher metadata for profile '{}' and sandbox '{}'",
                profile_name, sandbox_name
            )
        })?;
    let package = std::env::var(BROKER_LAUNCHER_PACKAGE_ENV)
        .map_err(|_| format!("{BROKER_LAUNCHER_PACKAGE_ENV} must be set"))?;
    Ok(Path::new(&package).join("bin").join(launcher_name))
}

fn send_broker_response(
    stream: &mut UnixStream,
    exit_code: i32,
    error: Option<String>,
) -> Result<(), String> {
    let rendered = serde_json::to_vec(&broker::BrokerLaunchResponse { exit_code, error })
        .map_err(|e| format!("serialize broker launch response: {e}"))?;
    stream
        .write_all(&rendered)
        .map_err(|e| format!("write broker launch response: {e}"))
}

const BROKER_STDIO_FD_COUNT: usize = 3;
const BROKER_REQUEST_FD_PREFIX_LEN: usize = 1;

type BrokerStdioRawFds = [RawFd; BROKER_STDIO_FD_COUNT];

fn cmsg_align(len: usize) -> usize {
    let align = std::mem::size_of::<usize>();
    (len + align - 1) & !(align - 1)
}

fn cmsg_space(payload_len: usize) -> usize {
    cmsg_align(std::mem::size_of::<libc::cmsghdr>()) + cmsg_align(payload_len)
}

fn cmsg_len(payload_len: usize) -> usize {
    cmsg_align(std::mem::size_of::<libc::cmsghdr>()) + payload_len
}

struct CmsgBuffer {
    storage: Vec<libc::cmsghdr>,
    len: usize,
}

impl CmsgBuffer {
    fn new(len: usize) -> Self {
        let elem_size = std::mem::size_of::<libc::cmsghdr>();
        let elem_count = len.div_ceil(elem_size).max(1);
        Self {
            storage: vec![unsafe { std::mem::zeroed() }; elem_count],
            len,
        }
    }

    fn as_mut_ptr(&mut self) -> *mut libc::c_void {
        self.storage.as_mut_ptr() as *mut libc::c_void
    }

    fn len(&self) -> usize {
        self.len
    }
}

fn fd_is_open(fd: RawFd) -> bool {
    if unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0 {
        true
    } else {
        io::Error::last_os_error().raw_os_error() != Some(libc::EBADF)
    }
}

fn open_dev_null_stdio(for_output: bool) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(!for_output).write(for_output);
    let file = options
        .open("/dev/null")
        .map_err(|e| format!("open /dev/null for broker stdio fallback: {e}"))?;
    let fd = file.as_raw_fd();
    if fd > libc::STDERR_FILENO {
        return Ok(file);
    }

    let dup_fd = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, libc::STDERR_FILENO + 1) };
    if dup_fd < 0 {
        return Err(format!(
            "duplicate /dev/null broker stdio fallback fd: {}",
            io::Error::last_os_error()
        ));
    }
    drop(file);
    Ok(unsafe { std::fs::File::from_raw_fd(dup_fd) })
}

fn broker_stdio_fds_for_send() -> Result<(BrokerStdioRawFds, Vec<std::fs::File>), String> {
    let mut fallbacks = Vec::new();
    let mut fds = [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO];
    for (index, fd) in fds.iter_mut().enumerate() {
        if !fd_is_open(*fd) {
            let fallback = open_dev_null_stdio(index != 0)?;
            *fd = fallback.as_raw_fd();
            fallbacks.push(fallback);
        }
    }
    Ok((fds, fallbacks))
}

fn attach_stdio_fds(msg: &mut libc::msghdr, stdio_fds: &BrokerStdioRawFds) {
    unsafe {
        let cmsg = msg.msg_control as *mut libc::cmsghdr;
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = cmsg_len(std::mem::size_of_val(stdio_fds));
        std::ptr::copy_nonoverlapping(
            stdio_fds.as_ptr(),
            libc::CMSG_DATA(cmsg) as *mut RawFd,
            stdio_fds.len(),
        );
        msg.msg_controllen = (*cmsg).cmsg_len;
    }
}

fn send_broker_request_with_stdio(
    stream: &UnixStream,
    payload: &[u8],
    stdio_fds: &BrokerStdioRawFds,
    fallback_files: Vec<std::fs::File>,
) -> Result<(), String> {
    if payload.is_empty() {
        return Err("broker launch request payload is empty".to_string());
    }

    let mut iov = libc::iovec {
        iov_base: payload.as_ptr() as *mut libc::c_void,
        iov_len: BROKER_REQUEST_FD_PREFIX_LEN,
    };
    let mut control = CmsgBuffer::new(cmsg_space(std::mem::size_of_val(stdio_fds)));
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr();
    msg.msg_controllen = control.len();
    attach_stdio_fds(&mut msg, stdio_fds);

    let sent = loop {
        let result = unsafe { libc::sendmsg(stream.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) };
        if result >= 0 {
            break result as usize;
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(format!("send broker launch request with stdio fds: {err}"));
        }
    };

    if sent != BROKER_REQUEST_FD_PREFIX_LEN {
        return Err(format!(
            "short broker launch request fd write: sent {sent} of {BROKER_REQUEST_FD_PREFIX_LEN} bytes"
        ));
    }

    // Keep fallback /dev/null descriptors alive until sendmsg has handed them to
    // the kernel for delivery to the receiver.
    drop(fallback_files);

    let mut stream_ref = stream;
    stream_ref
        .write_all(&payload[sent..])
        .map_err(|e| format!("write broker launch request: {e}"))
}

fn close_raw_fds(fds: &[RawFd]) {
    for &fd in fds {
        unsafe {
            libc::close(fd);
        }
    }
}

fn close_received_scm_rights(msg: &libc::msghdr) {
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let data_len = (*cmsg).cmsg_len.saturating_sub(cmsg_len(0));
                let fd_count = data_len / std::mem::size_of::<RawFd>();
                let data = libc::CMSG_DATA(cmsg) as *const RawFd;
                let received_fds = std::slice::from_raw_parts(data, fd_count);
                close_raw_fds(received_fds);
            }
            cmsg = libc::CMSG_NXTHDR(msg, cmsg);
        }
    }
}

struct BrokerStdioFds {
    fds: [RawFd; 3],
    consumed: bool,
}

impl BrokerStdioFds {
    fn new(fds: [RawFd; 3]) -> Self {
        Self {
            fds,
            consumed: false,
        }
    }

    fn into_stdio(mut self) -> [Stdio; 3] {
        self.consumed = true;
        unsafe {
            [
                Stdio::from(std::fs::File::from_raw_fd(self.fds[0])),
                Stdio::from(std::fs::File::from_raw_fd(self.fds[1])),
                Stdio::from(std::fs::File::from_raw_fd(self.fds[2])),
            ]
        }
    }
}

impl Drop for BrokerStdioFds {
    fn drop(&mut self) {
        if !self.consumed {
            close_raw_fds(&self.fds);
        }
    }
}

fn recv_broker_request_with_stdio(
    stream: &mut UnixStream,
) -> Result<(Vec<u8>, BrokerStdioFds), String> {
    let mut first_payload = vec![0_u8; 64 * 1024];
    let mut iov = libc::iovec {
        iov_base: first_payload.as_mut_ptr() as *mut libc::c_void,
        iov_len: first_payload.len(),
    };
    let mut control = CmsgBuffer::new(cmsg_space(
        BROKER_STDIO_FD_COUNT * std::mem::size_of::<RawFd>(),
    ));
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr();
    msg.msg_controllen = control.len();

    let received = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if received < 0 {
        return Err(format!(
            "read broker launch request: {}",
            io::Error::last_os_error()
        ));
    }
    if received == 0 {
        return Err("read broker launch request: empty request".to_string());
    }
    if (msg.msg_flags & libc::MSG_CTRUNC) != 0 {
        close_received_scm_rights(&msg);
        return Err("broker launch request stdio fd control data was truncated".to_string());
    }

    let mut stdio_fds = None;
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let data_len = (*cmsg).cmsg_len.saturating_sub(cmsg_len(0));
                let fd_count = data_len / std::mem::size_of::<RawFd>();
                let data = libc::CMSG_DATA(cmsg) as *const RawFd;
                let received_fds = std::slice::from_raw_parts(data, fd_count);
                if fd_count != BROKER_STDIO_FD_COUNT {
                    close_raw_fds(received_fds);
                    return Err(format!(
                        "broker launch request sent {fd_count} stdio fds, expected {BROKER_STDIO_FD_COUNT}"
                    ));
                }
                if stdio_fds.is_some() {
                    close_raw_fds(received_fds);
                    return Err("broker launch request sent multiple stdio fd sets".to_string());
                }
                stdio_fds = Some(BrokerStdioFds::new([
                    received_fds[0],
                    received_fds[1],
                    received_fds[2],
                ]));
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }

    let stdio_fds =
        stdio_fds.ok_or_else(|| "broker launch request did not include stdio fds".to_string())?;

    first_payload.truncate(received as usize);
    stream
        .read_to_end(&mut first_payload)
        .map_err(|e| format!("read broker launch request: {e}"))?;
    Ok((first_payload, stdio_fds))
}

fn apply_broker_child_stdio(cmd: &mut process::Command, stdio_fds: BrokerStdioFds) {
    let [stdin, stdout, stderr] = stdio_fds.into_stdio();
    cmd.stdin(stdin);
    cmd.stdout(stdout);
    cmd.stderr(stderr);
}

fn handle_broker_launch_request(
    request: broker::BrokerLaunchRequest,
    stdio_fds: BrokerStdioFds,
    generated_launchers: &std::collections::BTreeMap<
        String,
        cloister_sandbox_lib::config::GeneratedLauncher,
    >,
    session_record_path: &Path,
    host_project_root: &str,
    capability_payload: &str,
) -> Result<i32, String> {
    let store = session_record_path.parent().ok_or_else(|| {
        format!(
            "session record path has no parent directory: {}",
            session_record_path.display()
        )
    })?;
    let token = session_record_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "session record path has invalid token filename: {}",
                session_record_path.display()
            )
        })?;
    let session = broker_store::load_session_record(store, token)?;
    let profile = lookup_child_profile(&session, &request.profile)?;
    if profile.sandbox != request.sandbox {
        return Err(format!(
            "broker child profile '{}' targets sandbox '{}' but request targeted '{}'",
            request.profile, profile.sandbox, request.sandbox
        ));
    }

    let launcher_path =
        broker_launcher_path(generated_launchers, &request.profile, &request.sandbox)?;

    let mut cmd = process::Command::new(&launcher_path);
    cmd.current_dir(broker::resolve_relative_launch_dir(
        host_project_root,
        &request.cwd_rel,
    )?);
    cmd.args(&request.argv);
    cmd.env(BROKER_PARENT_CAPABILITY_ENV, capability_payload);
    cmd.env(
        BROKER_TRUSTED_RECORD_ENV,
        session_record_path.to_string_lossy().to_string(),
    );
    apply_broker_child_stdio(&mut cmd, stdio_fds);

    let status = cmd
        .status()
        .map_err(|e| format!("spawn broker child sandbox '{}': {e}", request.sandbox))?;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}

fn start_broker_listener(
    config: &SandboxConfig,
    host_runtime_dir: &str,
    host_project_root: &str,
    session_record_path: &Path,
) -> Result<BrokerListenerHandle, String> {
    let token = session_record_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "session record path has invalid token filename: {}",
                session_record_path.display()
            )
        })?;
    let socket_path = broker_client_socket_path(host_runtime_dir, token)?;
    let parent_dir = socket_path.parent().ok_or_else(|| {
        format!(
            "broker socket path has no parent directory: {}",
            socket_path.display()
        )
    })?;
    std::fs::create_dir_all(parent_dir)
        .map_err(|e| format!("mkdir -p {}: {e}", parent_dir.display()))?;
    if socket_path.exists() {
        std::fs::remove_file(&socket_path).map_err(|e| {
            format!(
                "remove stale broker socket '{}': {e}",
                socket_path.display()
            )
        })?;
    }

    let listener = UnixListener::bind(&socket_path)
        .map_err(|e| format!("bind broker socket '{}': {e}", socket_path.display()))?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let session_record_path = session_record_path.to_path_buf();
    let host_project_root = host_project_root.to_string();
    let capability_payload = serde_json::to_string(&broker::BrokerParentCapability {
        token: token.to_string(),
    })
    .map_err(|e| format!("serialize broker parent capability: {e}"))?;
    let generated_launchers = config.worker_broker.generated_launchers.clone();
    let join_handle = thread::spawn(move || {
        while !stop_thread.load(Ordering::Acquire) {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) => {
                    if stop_thread.load(Ordering::Acquire) {
                        break;
                    }
                    eprintln!("cloister-sandbox: accept broker launch request: {e}");
                    continue;
                }
            };

            if stop_thread.load(Ordering::Acquire) {
                break;
            }

            let (payload, stdio_fds) = match recv_broker_request_with_stdio(&mut stream) {
                Ok(result) => result,
                Err(e) => {
                    let _ = send_broker_response(&mut stream, 1, Some(e));
                    continue;
                }
            };

            let request: broker::BrokerLaunchRequest = match serde_json::from_slice(&payload) {
                Ok(request) => request,
                Err(e) => {
                    let _ = send_broker_response(
                        &mut stream,
                        1,
                        Some(format!("parse broker launch request: {e}")),
                    );
                    continue;
                }
            };

            let result = handle_broker_launch_request(
                request,
                stdio_fds,
                &generated_launchers,
                &session_record_path,
                &host_project_root,
                &capability_payload,
            );
            match result {
                Ok(exit_code) => {
                    let _ = send_broker_response(&mut stream, exit_code, None);
                }
                Err(e) => {
                    let _ = send_broker_response(&mut stream, 1, Some(e));
                }
            }
        }
    });

    Ok(BrokerListenerHandle {
        socket_path,
        stop,
        join_handle: Some(join_handle),
    })
}

fn run_broker_client(
    _config: &SandboxConfig,
    selector: &BrokerLaunchSelector,
    sandbox_args: &[String],
    stdio_fds: &BrokerStdioRawFds,
    fallback_files: Vec<std::fs::File>,
) -> Result<i32, String> {
    let record_path = child_broker_record_path_with_override(None)?;
    let store = record_path.parent().ok_or_else(|| {
        format!(
            "session record path has no parent directory: {}",
            record_path.display()
        )
    })?;
    let token = record_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "session record path has invalid token filename: {}",
                record_path.display()
            )
        })?;
    let session = broker_store::load_session_record(store, token)?;
    let project_root = session.project_root;
    let current_dir = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|e| format!("resolve current dir: {e}"))?;
    let cwd_rel = current_dir
        .strip_prefix(&project_root)
        .map_err(|_| {
            format!(
                "worker broker launch cwd '{}' is outside project root '{}'",
                current_dir.display(),
                project_root
            )
        })?
        .to_string_lossy()
        .to_string();

    let request = broker::BrokerLaunchRequest {
        profile: selector.profile.clone(),
        sandbox: selector.sandbox.clone(),
        argv: sandbox_args.to_vec(),
        cwd_rel,
    };

    let payload = serde_json::to_vec(&request)
        .map_err(|e| format!("serialize broker launch request: {e}"))?;
    let mut stream = UnixStream::connect(BROKER_SOCKET_CHILD_PATH)
        .map_err(|e| format!("connect broker socket '{BROKER_SOCKET_CHILD_PATH}': {e}"))?;
    send_broker_request_with_stdio(&stream, &payload, stdio_fds, fallback_files)?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("shutdown broker socket write end: {e}"))?;

    let mut response_payload = Vec::new();
    stream
        .read_to_end(&mut response_payload)
        .map_err(|e| format!("read broker launch response: {e}"))?;
    let response: broker::BrokerLaunchResponse = serde_json::from_slice(&response_payload)
        .map_err(|e| format!("parse broker launch response: {e}"))?;
    if let Some(error) = response.error {
        return Err(error);
    }
    Ok(response.exit_code)
}

fn child_broker_record_path_with_override(
    trusted_record_override: Option<&std::path::Path>,
) -> Result<std::path::PathBuf, String> {
    let trusted_record = trusted_record_override
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from(BROKER_SESSION_RECORD_CHILD_PATH));

    if trusted_record.is_file() {
        Ok(trusted_record)
    } else {
        Err("trusted broker session record mount is unavailable".to_string())
    }
}

pub fn lookup_child_profile<'a>(
    session: &'a broker::BrokerSession,
    profile_name: &str,
) -> Result<&'a broker::BrokerSpawnableProfile, String> {
    session
        .profiles
        .get(profile_name)
        .ok_or_else(|| format!("undefined child profile '{profile_name}'"))
}

pub fn workspace_mode_args(
    profile: &broker::BrokerSpawnableProfile,
    project_root: &str,
    overlay_lower_source: Option<&str>,
) -> Vec<String> {
    match profile.workspace_mode {
        WorkspaceMode::ProjectRw => Vec::new(),
        WorkspaceMode::ProjectOverlay => bwrap::project_overlay_args(
            overlay_lower_source.expect("project-overlay requires overlay lower source"),
            project_root,
        ),
    }
}

pub fn delegated_mount_args(
    session: &broker::BrokerSession,
    profile: &broker::BrokerSpawnableProfile,
    project_root: &str,
) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    for (dest, mount) in &profile.delegated_per_dir_mounts {
        let source = broker::resolve_delegated_per_dir_source(
            &mount.path,
            &session.dir_hash,
            mount.sub_path.as_deref(),
        )?;
        let dest_path = format!("{project_root}/{dest}");
        args.extend(bwrap::delegated_per_dir_args(
            &source, &dest_path, mount.mode,
        ));
    }
    Ok(args)
}

fn load_parent_broker_capability() -> Result<Option<broker::BrokerParentCapability>, String> {
    let Some(payload) = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV) else {
        return Ok(None);
    };
    let payload = payload
        .into_string()
        .map_err(|_| format!("{BROKER_PARENT_CAPABILITY_ENV} must be valid UTF-8"))?;
    let capability: broker::BrokerParentCapability = serde_json::from_str(&payload)
        .map_err(|e| format!("parse {BROKER_PARENT_CAPABILITY_ENV}: {e}"))?;
    if capability.token.is_empty() {
        return Err("broker parent capability token must not be empty".to_string());
    }
    Ok(Some(capability))
}

fn child_broker_args(
    sandbox_name: &str,
    selector_override: Option<&BrokerLaunchSelector>,
    host_project_root: &str,
    child_project_root: &str,
    host_runtime_dir: &str,
    trusted_record_override: Option<&std::path::Path>,
    overlay_lower_source: Option<&str>,
) -> Result<Option<Vec<String>>, String> {
    child_broker_args_with_store_dir(
        sandbox_name,
        selector_override,
        host_project_root,
        child_project_root,
        host_runtime_dir,
        trusted_record_override,
        overlay_lower_source,
    )
}

fn child_broker_args_with_store_dir(
    sandbox_name: &str,
    selector_override: Option<&BrokerLaunchSelector>,
    host_project_root: &str,
    child_project_root: &str,
    _host_runtime_dir: &str,
    trusted_record_override: Option<&std::path::Path>,
    overlay_lower_source: Option<&str>,
) -> Result<Option<Vec<String>>, String> {
    let Some(capability) = load_parent_broker_capability()? else {
        return Ok(None);
    };
    let profile_name = if let Some(selector) = selector_override {
        if selector.sandbox != sandbox_name {
            return Err(format!(
                "broker launcher targets sandbox '{}' but current sandbox is '{}'",
                selector.sandbox, sandbox_name
            ));
        }
        selector.profile.clone()
    } else {
        let Some(profile_name) = std::env::var_os(BROKER_CHILD_PROFILE_ENV) else {
            return Ok(None);
        };
        profile_name
            .into_string()
            .map_err(|_| format!("{BROKER_CHILD_PROFILE_ENV} must be valid UTF-8"))?
    };

    let record_path = child_broker_record_path_with_override(trusted_record_override)?;
    let store = record_path.parent().ok_or_else(|| {
        format!(
            "session record path has no parent directory: {}",
            record_path.display()
        )
    })?;
    let token = record_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "session record path has invalid token filename: {}",
                record_path.display()
            )
        })?;
    let session = broker_store::load_session_record(store, token)?;
    broker::validate_capability_session_identity(&capability, &session, host_project_root)?;

    let profile = lookup_child_profile(&session, &profile_name)?;
    if profile.sandbox != sandbox_name {
        return Err(format!(
            "broker child profile '{profile_name}' targets sandbox '{}' but current sandbox is '{}'",
            profile.sandbox, sandbox_name
        ));
    }

    let mut args = workspace_mode_args(profile, child_project_root, overlay_lower_source);
    args.extend(delegated_mount_args(&session, profile, child_project_root)?);
    Ok(Some(args))
}

fn load_trusted_child_profile(
    host_project_root: &str,
    selector_override: Option<&BrokerLaunchSelector>,
    trusted_record_override: Option<&std::path::Path>,
) -> Result<Option<broker::BrokerSpawnableProfile>, String> {
    let Some(capability) = load_parent_broker_capability()? else {
        return Ok(None);
    };
    let profile_name = if let Some(selector) = selector_override {
        selector.profile.clone()
    } else {
        let Some(profile_name) = std::env::var_os(BROKER_CHILD_PROFILE_ENV) else {
            return Ok(None);
        };
        profile_name
            .into_string()
            .map_err(|_| format!("{BROKER_CHILD_PROFILE_ENV} must be valid UTF-8"))?
    };
    let record_path = child_broker_record_path_with_override(trusted_record_override)?;
    let store = record_path.parent().ok_or_else(|| {
        format!(
            "session record path has no parent directory: {}",
            record_path.display()
        )
    })?;
    let token = record_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "session record path has invalid token filename: {}",
                record_path.display()
            )
        })?;
    let session = broker_store::load_session_record(store, token)?;
    broker::validate_capability_session_identity(&capability, &session, host_project_root)?;

    Ok(Some(lookup_child_profile(&session, &profile_name)?.clone()))
}

fn dir_hash_for_launch(config: &SandboxConfig, sandbox_dir: &str) -> String {
    if !sandbox_dir.is_empty()
        && (!config.per_dir.is_empty() || !config.worker_broker.profiles.is_empty())
    {
        runtime::compute_dir_hash(sandbox_dir)
    } else {
        String::new()
    }
}

fn filter_child_overlay_project_bind(
    dynamic_binds: &[cloister_sandbox_lib::config::DynamicBind],
    runtime_vars: &std::collections::HashMap<String, String>,
    profile: Option<&broker::BrokerSpawnableProfile>,
) -> Vec<cloister_sandbox_lib::config::DynamicBind> {
    let Some(profile) = profile else {
        return dynamic_binds.to_vec();
    };
    if profile.workspace_mode != WorkspaceMode::ProjectOverlay {
        return dynamic_binds.to_vec();
    }

    let project_dest = runtime_vars
        .get("SANDBOX_DEST")
        .cloned()
        .unwrap_or_else(|| runtime_vars.get("SANDBOX_DIR").cloned().unwrap_or_default());

    dynamic_binds
        .iter()
        .filter(|bind| {
            let src = vars::expand_vars(&bind.src, runtime_vars);
            let dest = bind
                .dest
                .as_ref()
                .map(|d| vars::expand_vars(d, runtime_vars))
                .unwrap_or_else(|| src.clone());
            dest != project_dest
        })
        .cloned()
        .collect()
}

fn run() -> i32 {
    // --- 1. Parse CLI args ---
    let args: Vec<String> = std::env::args().collect();
    let cli = parse_cli_args(&args).unwrap_or_else(|e| {
        eprintln!("{e}");
        process::exit(2);
    });
    let broker_client_stdio = if cli.broker_launch.is_some() {
        Some(broker_stdio_fds_for_send().unwrap_or_else(|e| {
            eprintln!("cloister-sandbox: {e}");
            process::exit(1);
        }))
    } else {
        None
    };

    if matches!(cli.sandbox_args.as_slice(), [arg] if arg == "--version" || arg == "--build-info")
        || (cli.config_path.is_none()
            && matches!(
                args.get(1).map(String::as_str),
                Some("--version") | Some("--build-info")
            ))
    {
        print_build_info(cli.config_path.as_deref());
        return 0;
    }

    let config_path = cli.config_path.unwrap_or_else(|| {
        eprintln!("cloister-sandbox: --config <path> is required");
        process::exit(2);
    });

    // --- 2. Load config ---
    let config = SandboxConfig::load(&config_path).unwrap_or_else(|e| {
        eprintln!("cloister-sandbox: {e}");
        process::exit(1);
    });

    let prefix = err_prefix(&config.name);

    if let Err(e) = config.validate() {
        eprintln!("{prefix}: {e}");
        process::exit(1);
    }

    let broker_trusted_record_path = cli.broker_trusted_record_path.as_deref().map(Path::new);
    let host_xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    if let Err(e) = validate_xdg_runtime_dir(&config, &host_xdg_runtime_dir) {
        eprintln!("{prefix}: {e}");
        process::exit(1);
    }
    let sandbox_xdg_runtime_dir = if host_xdg_runtime_dir.is_empty() {
        String::new()
    } else {
        format!("/run/user/{}", cloister_sandbox_lib::socket::current_uid())
    };

    if let Some(selector) = cli.broker_launch.as_ref() {
        if broker_trusted_record_path.is_none()
            && Path::new(BROKER_SOCKET_CHILD_PATH).exists()
            && child_broker_record_path_with_override(None).is_ok()
        {
            if let Err(e) = prepare_run_cmd(&config, &cli.sandbox_args, cli.broker_launch.as_ref())
            {
                eprintln!("{prefix}: {e}");
                process::exit(2);
            }
            let (stdio_fds, fallback_files) = broker_client_stdio
                .expect("broker launch stdio fds are prepared when broker launch is requested");
            let broker_status = run_broker_client(
                &config,
                selector,
                &cli.sandbox_args,
                &stdio_fds,
                fallback_files,
            )
            .unwrap_or_else(|e| {
                eprintln!("{prefix}: {e}");
                process::exit(1);
            });
            return broker_status;
        }
    }
    // --- 2b. Install signal handlers so SIGTERM/SIGINT/SIGHUP forward to children ---
    install_signal_handlers();

    // --- 3. Netns re-exec ---
    if let Some(ref netns_helper) = config.netns_helper_path {
        if let (Some(ns), false) = (&config.network_namespace, cli.after_netns) {
            // Re-exec through netns helper: netns_helper --netns <name> -- <self> --after-netns --config <path> [original args]
            let self_exe = std::env::current_exe().unwrap_or_else(|e| {
                eprintln!("{prefix}: cannot determine self path: {e}");
                process::exit(1);
            });

            let mut cmd = process::Command::new(netns_helper);
            cmd.args(["--netns", ns, "--"]);
            cmd.arg(&self_exe);
            cmd.arg("--after-netns");
            cmd.args(["--config", &config_path]);
            if let Some(selector) = cli.broker_launch.as_ref() {
                cmd.args(["--broker-launch-profile", &selector.profile]);
                cmd.args(["--broker-launch-sandbox", &selector.sandbox]);
            }
            if let Some(trusted_record_path) = cli.broker_trusted_record_path.as_deref() {
                cmd.args(["--broker-trusted-record", trusted_record_path]);
            }
            cmd.args(&cli.sandbox_args);

            let status = spawn_and_wait(&mut cmd, false, None).unwrap_or_else(|e| {
                eprintln!("{prefix}: exec netns helper: {e}");
                process::exit(127);
            });

            return status
                .code()
                .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
        }
    }

    // --- 4-7. Sandbox directory, validation, start dir, per-dir setup ---
    let configured_home = config.home_directory.clone();
    let configured_home_resolved = std::fs::canonicalize(&configured_home)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| configured_home.clone());

    let (sandbox_dir, sandbox_dest, dir_hash, effective_start_dir) = if config
        .bind_working_directory
    {
        // --- 4. Determine sandbox directory ---
        let sandbox_dir = runtime::detect_sandbox_dir(&config.git_path).unwrap_or_else(|e| {
            eprintln!("{prefix}: {e}");
            process::exit(1);
        });

        // --- 5. Validate sandbox directory ---
        if config.enforce_strict_home_policy {
            if let Err(e) =
                validate::validate_strict_home_policy(&sandbox_dir, &configured_home_resolved)
            {
                eprintln!("{prefix}: {e}");
                process::exit(1);
            }
        }

        if let Err(e) = validate::validate_disallowed_paths(&sandbox_dir, &config.disallowed_paths)
        {
            eprintln!("{prefix}: {e}");
            process::exit(1);
        }

        if let Err(e) = validate::validate_sandbox_dir_exists(&sandbox_dir) {
            eprintln!("{prefix}: {e}");
            process::exit(1);
        }

        // --- 6. Compute start dir and anonymization ---
        let start_dir = runtime::compute_start_dir(&sandbox_dir);
        let sandbox_dest = if config.anonymize {
            runtime::remap_path_for_anonymize(&sandbox_dir, &configured_home, &config.sandbox_home)
        } else {
            sandbox_dir.clone()
        };
        let effective_start_dir = if config.anonymize {
            runtime::remap_path_for_anonymize(&start_dir, &configured_home, &config.sandbox_home)
        } else {
            start_dir
        };

        // --- 7. Per-dir setup ---
        let dir_hash = dir_hash_for_launch(&config, &sandbox_dir);

        if !config.per_dir.is_empty() {
            let hash = dir_hash.clone();

            for (base, paths) in &config.per_dir {
                if let Err(e) = runtime::validate_per_dir_base(base) {
                    eprintln!("{prefix}: {e}");
                    process::exit(1);
                }

                let per_dir_mkdirs: Vec<String> = paths
                    .iter()
                    .map(|p| format!("{}/{}/{}", base, hash, p))
                    .collect();
                if let Err(e) = runtime::ensure_dirs(&per_dir_mkdirs) {
                    eprintln!("{prefix}: {e}");
                    process::exit(1);
                }

                let manifest_path = format!("{}/manifest.json", base);
                if let Err(e) = runtime::update_manifest(&manifest_path, &hash, &sandbox_dir) {
                    eprintln!("{prefix}: {e}");
                    process::exit(1);
                }
            }
        }

        (sandbox_dir, sandbox_dest, dir_hash, effective_start_dir)
    } else {
        // No working directory needed
        (
            String::new(),
            String::new(),
            String::new(),
            config.sandbox_home.clone(),
        )
    };

    // --- 8. Volume-backed dir/file creation, copy-on-first-use files ---
    let dir_paths: Vec<String> = config.dir_mkdirs.iter().map(|s| s.path.clone()).collect();
    if let Err(e) = runtime::ensure_dirs(&dir_paths) {
        eprintln!("{prefix}: {e}");
        process::exit(1);
    }

    // Host-side dirs for managed files inside dir-backed mounts
    if let Err(e) = runtime::ensure_dirs(&config.managed_file_host_mkdirs) {
        eprintln!("{prefix}: {e}");
        process::exit(1);
    }

    let file_paths: Vec<String> = config.file_mkdirs.iter().map(|s| s.path.clone()).collect();
    if let Err(e) = runtime::ensure_files(&file_paths) {
        eprintln!("{prefix}: {e}");
        process::exit(1);
    }

    // Copy files into writable sandbox state. Missing sources are fatal.
    for cf in &config.copy_files {
        let mode = match u32::from_str_radix(&cf.mode, 8) {
            Ok(m) if m <= 0o777 => m,
            _ => {
                eprintln!(
                    "{prefix}: invalid mode '{}' for copy_file '{}'",
                    cf.mode, cf.host_dest
                );
                process::exit(1);
            }
        };
        if let Err(e) = runtime::copy_file(
            &cf.src,
            &cf.host_dest,
            mode,
            cf.overwrite,
            &config.copy_file_base,
        ) {
            eprintln!("{prefix}: {e}");
            process::exit(1);
        }
    }

    // --- 9. Build bwrap args ---
    let mut runtime_vars = runtime::build_runtime_vars(
        &configured_home,
        &config.sandbox_home,
        &sandbox_dir,
        &sandbox_dest,
        &dir_hash,
        &sandbox_xdg_runtime_dir,
        &host_xdg_runtime_dir,
    );

    let launch_instance_id = if config.dbus_enable || config.flatpak_app_id.is_some() {
        Some(runtime::generate_flatpak_instance_id())
    } else {
        None
    };

    if let (Some(socket_name), Some(instance_id)) = (
        config.dbus_proxy_socket_name.as_deref(),
        launch_instance_id.as_deref(),
    ) {
        runtime_vars.insert(
            "DBUS_PROXY_SOCKET".to_string(),
            features::dbus_runtime_socket_path(&host_xdg_runtime_dir, socket_name, instance_id),
        );
    }

    let flatpak_portal_state = if let Some(app_id) = config.flatpak_app_id.as_deref() {
        let instance_id = launch_instance_id
            .clone()
            .expect("launch instance id must exist when portal integration is enabled");
        let flatpak_info_path =
            format!("{host_xdg_runtime_dir}/cloister/flatpak-info/{instance_id}.ini");
        let instance_dir = format!("{host_xdg_runtime_dir}/.flatpak/{instance_id}");

        if let Err(e) = runtime::write_flatpak_info(&flatpak_info_path, app_id, &instance_id) {
            eprintln!("{prefix}: {e}");
            process::exit(1);
        }
        if let Err(e) = runtime::ensure_dirs(std::slice::from_ref(&instance_dir)) {
            eprintln!("{prefix}: {e}");
            let _ = runtime::remove_path_if_exists(&flatpak_info_path);
            process::exit(1);
        }

        runtime_vars.insert("FLATPAK_INFO_PATH".to_string(), flatpak_info_path.clone());

        Some(FlatpakPortalState {
            flatpak_info_path,
            instance_dir,
        })
    } else {
        None
    };

    let image_store_mount_path = if config.store_mode == StoreMode::ImageStore {
        match ensure_image_store_mounted(&config, &prefix) {
            Ok(path) => Some(path),
            Err(e) => {
                eprintln!("{e}");
                process::exit(1);
            }
        }
    } else {
        None
    };

    let pulse_bridge = match start_pulse_only_bridge(&config, &host_xdg_runtime_dir, &prefix) {
        Ok(bridge) => bridge,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };

    let mut extra_args = Vec::new();
    let mut config_dynamic_binds = config.dynamic_binds.clone();
    let broker_parent_capability_present = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV).is_some();
    let broker_profile_present =
        cli.broker_launch.is_some() || std::env::var_os(BROKER_CHILD_PROFILE_ENV).is_some();
    if broker_profile_present {
        match child_broker_args(
            &config.name,
            cli.broker_launch.as_ref(),
            &sandbox_dir,
            &sandbox_dest,
            &host_xdg_runtime_dir,
            broker_trusted_record_path,
            Some(&sandbox_dir),
        ) {
            Ok(Some(args)) => {
                let profile = load_trusted_child_profile(
                    &sandbox_dir,
                    cli.broker_launch.as_ref(),
                    broker_trusted_record_path,
                )
                .ok()
                .flatten();
                config_dynamic_binds = filter_child_overlay_project_bind(
                    &config.dynamic_binds,
                    &runtime_vars,
                    profile.as_ref(),
                );
                extra_args.extend(args)
            }
            Err(e) => {
                eprintln!("{prefix}: {e}");
                process::exit(1);
            }
            Ok(None) => {
                eprintln!("{prefix}: broker child launch requires {BROKER_PARENT_CAPABILITY_ENV}");
                process::exit(1);
            }
        }
    } else if broker_parent_capability_present {
        // Parent capabilities propagate through nested launches but stay inert until a child profile is selected.
    }

    let broker_registration = match register_parent_broker_launch(
        &config,
        &sandbox_dir,
        &dir_hash,
        &host_xdg_runtime_dir,
    ) {
        Ok(registration) => registration,
        Err(e) => {
            eprintln!("{prefix}: {e}");
            process::exit(1);
        }
    };
    if let Some(registration) = broker_registration.as_ref() {
        extra_args.extend(registration.env_args.clone());
    }

    match config.store_mode {
        StoreMode::Host => extra_args.extend(host_store_bind_args()),
        StoreMode::ImageStore => {
            if let Some(store_mount_path) = image_store_mount_path.as_deref() {
                extra_args.extend(image_store_bind_args(store_mount_path));
            }
        }
    }

    let broker_session_record_path = broker_registration
        .as_ref()
        .and_then(|registration| registration.broker_session_record_path.clone());
    let broker_listener = match broker_session_record_path.as_deref() {
        Some(record_path) => {
            match start_broker_listener(&config, &host_xdg_runtime_dir, &sandbox_dir, record_path) {
                Ok(listener) => Some(listener),
                Err(e) => {
                    eprintln!("{prefix}: {e}");
                    cleanup(CleanupState {
                        ssh_handle: None,
                        dbus_proxy: None,
                        pulse_bridge: None,
                        wayland_socket: None,
                        machine_id_path: None,
                        proc_privacy_state: None,
                        anonymize_file_paths: Vec::new(),
                        flatpak_portal_state: None,
                        broker_session_record_path,
                        broker_listener: None,
                    });
                    process::exit(1);
                }
            }
        }
        None => None,
    };

    // Passthrough env
    extra_args.extend(bwrap::passthrough_env_args(&config.passthrough_env));

    if !sandbox_xdg_runtime_dir.is_empty() {
        extra_args.extend([
            "--setenv".to_string(),
            "XDG_RUNTIME_DIR".to_string(),
            sandbox_xdg_runtime_dir.clone(),
        ]);
    }

    extra_args.extend([
        "--setenv".to_string(),
        CLOISTER_CONFIG_PATH_ENV.to_string(),
        config_path.clone(),
    ]);

    // ZDOTDIR (only forward host ZDOTDIR when host shell config is enabled)
    if config.shell_name == "zsh" && config.shell_host_config {
        extra_args.extend(bwrap::zdotdir_args(&configured_home, &config.sandbox_home));
    }

    // SSH
    let ssh_filter_handle;
    if config.ssh_filter_enabled() {
        if let Ok(auth_sock) = std::env::var("SSH_AUTH_SOCK") {
            if !auth_sock.is_empty() {
                match socket::validate_existing_socket(&auth_sock) {
                    Err(e) => {
                        eprintln!("{prefix}: invalid SSH_AUTH_SOCK '{auth_sock}': {e}");
                        ssh_filter_handle = None;
                    }
                    Ok(()) => {
                        let filter_socket =
                            ssh_filter_socket_path(&host_xdg_runtime_dir, process::id());
                        let filter_socket_ready = Path::new(&filter_socket)
                            .parent()
                            .map(|parent| parent.to_string_lossy().to_string())
                            .map_or(Ok(()), |parent| {
                                runtime::ensure_dirs(std::slice::from_ref(&parent))
                            });
                        if let Err(e) = filter_socket_ready {
                            eprintln!("{prefix}: ssh filter setup failed: {e}");
                            ssh_filter_handle = None;
                        } else {
                            let (sandbox_auth_sock, sandbox_auth_sock_parent) =
                                sandbox_ssh_auth_sock(
                                    &auth_sock,
                                    &host_xdg_runtime_dir,
                                    &sandbox_xdg_runtime_dir,
                                );
                            match ssh_filter::start_listener(
                                &filter_socket,
                                &auth_sock,
                                config.ssh_allow_fingerprints.clone(),
                                config.ssh_filter_timeout_seconds,
                            ) {
                                Ok(handle) => {
                                    if let Some(parent) = sandbox_auth_sock_parent {
                                        extra_args.extend(["--dir".to_string(), parent]);
                                    }
                                    extra_args.extend([
                                        "--bind".to_string(),
                                        filter_socket.clone(),
                                        sandbox_auth_sock.clone(),
                                        "--setenv".to_string(),
                                        "SSH_AUTH_SOCK".to_string(),
                                        sandbox_auth_sock,
                                    ]);
                                    ssh_filter_handle = Some(handle);
                                }
                                Err(e) => {
                                    eprintln!("{prefix}: ssh filter setup failed: {e}");
                                    ssh_filter_handle = None;
                                }
                            }
                        }
                    }
                }
            } else {
                ssh_filter_handle = None;
            }
        } else {
            ssh_filter_handle = None;
        }
    } else if config.ssh_enable {
        if let Ok(auth_sock) = std::env::var("SSH_AUTH_SOCK") {
            if !auth_sock.is_empty() {
                if let Err(e) = socket::validate_existing_socket(&auth_sock) {
                    eprintln!("{prefix}: invalid SSH_AUTH_SOCK '{auth_sock}': {e}");
                } else {
                    let (sandbox_auth_sock, sandbox_auth_sock_parent) = sandbox_ssh_auth_sock(
                        &auth_sock,
                        &host_xdg_runtime_dir,
                        &sandbox_xdg_runtime_dir,
                    );
                    if let Some(parent) = sandbox_auth_sock_parent {
                        extra_args.extend(["--dir".to_string(), parent]);
                    }
                    extra_args.extend([
                        "--bind".to_string(),
                        auth_sock.clone(),
                        sandbox_auth_sock.clone(),
                        "--setenv".to_string(),
                        "SSH_AUTH_SOCK".to_string(),
                        sandbox_auth_sock,
                    ]);
                }
            }
        }
        ssh_filter_handle = None;
    } else {
        ssh_filter_handle = None;
    }

    // Pulse-only proxy
    if let Some(bridge) = &pulse_bridge {
        let sandbox_socket = format!("{sandbox_xdg_runtime_dir}/pulse/native");
        extra_args.extend(features::pulseaudio_args_with_source(
            &bridge.socket_path,
            &sandbox_socket,
        ));
    }

    // PipeWire
    if let Some(socket_name) = &config.pipewire_socket_name {
        extra_args.extend(features::pipewire_args_with_dest(
            &host_xdg_runtime_dir,
            socket_name,
            &sandbox_xdg_runtime_dir,
            "pipewire-0",
        ));
    }

    // Wayland
    let _wayland_keep_alive;
    let wayland_socket_path;
    if config.gui_enable {
        if std::env::var("WAYLAND_DISPLAY")
            .map(|d| !d.is_empty())
            .unwrap_or(false)
        {
            let wayland_dir = format!("{host_xdg_runtime_dir}/cloister/wayland");
            if let Err(e) = std::fs::create_dir_all(&wayland_dir) {
                eprintln!("{prefix}: wayland runtime dir: {e}");
                process::exit(cleanup_and_exit(CleanupState {
                    ssh_handle: ssh_filter_handle,
                    dbus_proxy: None,
                    pulse_bridge,
                    wayland_socket: None,
                    machine_id_path: None,
                    proc_privacy_state: None,
                    anonymize_file_paths: Vec::new(),
                    flatpak_portal_state,
                    broker_session_record_path,
                    broker_listener,
                }));
            }
            let socket = format!("{wayland_dir}/{}", process::id());
            if !wayland::probe() {
                eprintln!("{prefix}: compositor does not support wp-security-context-v1.");
                eprintln!("Use a supported compositor (sway 1.9+, Hyprland, niri, labwc 0.8.2+).");
                process::exit(cleanup_and_exit(CleanupState {
                    ssh_handle: ssh_filter_handle,
                    dbus_proxy: None,
                    pulse_bridge,
                    wayland_socket: None,
                    machine_id_path: None,
                    proc_privacy_state: None,
                    anonymize_file_paths: Vec::new(),
                    flatpak_portal_state,
                    broker_session_record_path,
                    broker_listener,
                }));
            }
            let app_id = format!("cloister-{}", config.name);
            match wayland::setup_context(&socket, "cloister", &app_id) {
                Ok(fd) => {
                    extra_args.extend([
                        "--ro-bind".to_string(),
                        socket.clone(),
                        format!("{sandbox_xdg_runtime_dir}/wayland-1"),
                        "--setenv".to_string(),
                        "WAYLAND_DISPLAY".to_string(),
                        "wayland-1".to_string(),
                    ]);
                    _wayland_keep_alive = Some(fd);
                    wayland_socket_path = Some(socket);
                }
                Err(e) => {
                    eprintln!("{prefix}: wayland setup: {e}");
                    process::exit(cleanup_and_exit(CleanupState {
                        ssh_handle: ssh_filter_handle,
                        dbus_proxy: None,
                        pulse_bridge,
                        wayland_socket: None,
                        machine_id_path: None,
                        proc_privacy_state: None,
                        anonymize_file_paths: Vec::new(),
                        flatpak_portal_state,
                        broker_session_record_path,
                        broker_listener,
                    }));
                }
            }
            // Electron/Chromium apps need this to use native Wayland via Ozone
            extra_args.extend([
                "--setenv".to_string(),
                "NIXOS_OZONE_WL".to_string(),
                "1".to_string(),
            ]);
        } else {
            _wayland_keep_alive = None;
            wayland_socket_path = None;
        }
    } else {
        _wayland_keep_alive = None;
        wayland_socket_path = None;
    }

    // GPU
    if config.gui_enable {
        extra_args.extend(features::gpu_args());
    }

    // Device binds
    extra_args.extend(features::dev_bind_args(&config.dev_binds));

    // Anonymized identity (synthetic /etc/passwd + /etc/group with real UID/GID)
    let anonymize_file_paths = if config.anonymize {
        let identity = match anonymized_identity(&config) {
            Ok(identity) => identity,
            Err(e) => {
                eprintln!("{prefix}: {e}");
                process::exit(cleanup_and_exit(CleanupState {
                    ssh_handle: ssh_filter_handle,
                    dbus_proxy: None,
                    pulse_bridge,
                    wayland_socket: wayland_socket_path,
                    machine_id_path: None,
                    proc_privacy_state: None,
                    anonymize_file_paths: Vec::new(),
                    flatpak_portal_state,
                    broker_session_record_path,
                    broker_listener,
                }));
            }
        };
        let overlays = features::anonymize_identity_args(
            identity,
            &config.shell_bin,
            &config.sandbox_home,
            config.network_namespace.as_deref(),
        );
        extra_args.extend(overlays.args);
        overlays.file_paths
    } else {
        Vec::new()
    };

    // Machine ID (random per invocation — avoids host fingerprinting)
    let (machine_id_bwrap_args, machine_id_path) = features::machine_id_args();
    extra_args.extend(machine_id_bwrap_args);

    let proc_privacy_state = if config.anonymize {
        let overlays = features::proc_privacy_args();
        extra_args.extend(overlays.args.clone());
        Some(ProcPrivacyState {
            file_paths: overlays.file_paths,
        })
    } else {
        None
    };

    // Seccomp
    let _seccomp_file; // Keep alive until bwrap finishes
    if let Some(ref filter_path) = config.seccomp_filter_path {
        if config.seccomp_enable {
            match seccomp::open_seccomp_fd(filter_path) {
                Ok((file, fd)) => {
                    extra_args.extend(seccomp::seccomp_args(fd));
                    _seccomp_file = Some(file);
                }
                Err(e) => {
                    eprintln!("{prefix}: seccomp filter open: {e}");
                    process::exit(cleanup_and_exit(CleanupState {
                        ssh_handle: ssh_filter_handle,
                        dbus_proxy: None,
                        pulse_bridge,
                        wayland_socket: wayland_socket_path,
                        machine_id_path,
                        proc_privacy_state,
                        anonymize_file_paths,
                        flatpak_portal_state,
                        broker_session_record_path,
                        broker_listener,
                    }));
                }
            }
        } else {
            _seccomp_file = None;
        }
    } else {
        _seccomp_file = None;
    }

    // D-Bus
    let mut dbus_proxy = None;
    let _dbus_keepalive;
    if config.dbus_enable {
        let host_bus_socket = format!("{host_xdg_runtime_dir}/bus");
        if socket::validate_existing_socket(&host_bus_socket).is_ok() {
            if let (Some(socket_name), Some(proxy_path), Some(instance_id)) = (
                config.dbus_proxy_socket_name.as_deref(),
                config.dbus_proxy_path.as_deref(),
                launch_instance_id.as_deref(),
            ) {
                let runtime_socket = features::dbus_runtime_socket_path(
                    &host_xdg_runtime_dir,
                    socket_name,
                    instance_id,
                );
                let _ = socket::remove_stale_socket(&runtime_socket);

                let mut proxy_cmd = process::Command::new(proxy_path);
                proxy_cmd.env("XDG_RUNTIME_DIR", &host_xdg_runtime_dir);
                proxy_cmd.env("CLOISTER_DBUS_PROXY_SOCKET", &runtime_socket);
                proxy_cmd.env("CLOISTER_DBUS_PROXY_INSTANCE_ID", instance_id);

                let mut child = match proxy_cmd.spawn() {
                    Ok(child) => child,
                    Err(e) => {
                        eprintln!("{prefix}: start dbus proxy: {e}");
                        cleanup(CleanupState {
                            ssh_handle: ssh_filter_handle,
                            dbus_proxy: None,
                            pulse_bridge,
                            wayland_socket: wayland_socket_path,
                            machine_id_path,
                            proc_privacy_state,
                            anonymize_file_paths,
                            flatpak_portal_state,
                            broker_session_record_path,
                            broker_listener,
                        });
                        process::exit(1);
                    }
                };
                if let Err(e) = wait_for_dbus_proxy_socket(&runtime_socket, &mut child) {
                    eprintln!("{prefix}: {e}");
                    cleanup(CleanupState {
                        ssh_handle: ssh_filter_handle,
                        dbus_proxy: Some(DbusProxyState {
                            child,
                            socket_path: runtime_socket,
                        }),
                        pulse_bridge,
                        wayland_socket: wayland_socket_path,
                        machine_id_path,
                        proc_privacy_state,
                        anonymize_file_paths,
                        flatpak_portal_state,
                        broker_session_record_path,
                        broker_listener,
                    });
                    process::exit(1);
                }
                dbus_proxy = Some(DbusProxyState {
                    child,
                    socket_path: runtime_socket,
                });

                // Best-effort prewarm held until the sandbox exits to prevent proxy shutdown.
                let keepalive_socket_name = format!("{socket_name}-{instance_id}");
                _dbus_keepalive =
                    features::warm_dbus_proxy(&host_xdg_runtime_dir, &keepalive_socket_name);
            } else {
                _dbus_keepalive = None;
            }
        } else {
            _dbus_keepalive = None;
        }
        if !sandbox_xdg_runtime_dir.is_empty() {
            extra_args.extend([
                "--setenv".to_string(),
                "DBUS_SESSION_BUS_ADDRESS".to_string(),
                format!("unix:path={sandbox_xdg_runtime_dir}/bus"),
            ]);
        }
    } else {
        _dbus_keepalive = None;
    }

    // --- 10. Parse command args and build run_cmd ---
    let (run_cmd, is_interactive) =
        match prepare_run_cmd(&config, &cli.sandbox_args, cli.broker_launch.as_ref()) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("{prefix}: {e}");
                process::exit(cleanup_and_return_exit_code(
                    CleanupState {
                        ssh_handle: ssh_filter_handle,
                        dbus_proxy,
                        pulse_bridge,
                        wayland_socket: wayland_socket_path,
                        machine_id_path,
                        proc_privacy_state,
                        anonymize_file_paths,
                        flatpak_portal_state,
                        broker_session_record_path,
                        broker_listener,
                    },
                    2,
                ));
            }
        };

    // --- 11. Spawn bwrap ---
    INTERACTIVE_MODE.store(is_interactive, Ordering::Release);

    let config_for_bwrap = SandboxConfig {
        dynamic_binds: config_dynamic_binds,
        ..config
    };

    let (mut cmd, _args_fd) = match bwrap::build_bwrap_command(
        &config_for_bwrap,
        &runtime_vars,
        extra_args,
        &run_cmd,
        &effective_start_dir,
        is_interactive,
    ) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{prefix}: args pipe setup: {e}");
            cleanup(CleanupState {
                ssh_handle: ssh_filter_handle,
                dbus_proxy,
                pulse_bridge,
                wayland_socket: wayland_socket_path,
                machine_id_path,
                proc_privacy_state,
                anonymize_file_paths,
                flatpak_portal_state,
                broker_session_record_path,
                broker_listener,
            });
            process::exit(1);
        }
    };

    let status = match spawn_and_wait(&mut cmd, is_interactive, flatpak_portal_state.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{prefix}: exec bwrap: {e}");
            cleanup(CleanupState {
                ssh_handle: ssh_filter_handle,
                dbus_proxy,
                pulse_bridge,
                wayland_socket: wayland_socket_path,
                machine_id_path,
                proc_privacy_state,
                anonymize_file_paths,
                flatpak_portal_state,
                broker_session_record_path,
                broker_listener,
            });
            return 127;
        }
    };

    // --- 12. Cleanup ---
    cleanup(CleanupState {
        ssh_handle: ssh_filter_handle,
        dbus_proxy,
        pulse_bridge,
        wayland_socket: wayland_socket_path,
        machine_id_path,
        proc_privacy_state,
        anonymize_file_paths,
        flatpak_portal_state,
        broker_session_record_path,
        broker_listener,
    });

    // --- 13. Exit with bwrap's exit code ---
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

fn print_build_info(config_path: Option<&str>) {
    let env_revision = std::env::var("CLOISTER_BUILD_REV").ok();

    if let Some(config_path) = config_path {
        match SandboxConfig::load(config_path) {
            Ok(config) => {
                let revision = env_revision
                    .or(config.build_revision.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                println!("cloister-sandbox {}", revision);
                println!("config={}", config_path);
                println!("sandbox={}", config.name);
                return;
            }
            Err(e) => {
                let revision = env_revision.unwrap_or_else(|| "unknown".to_string());
                println!("cloister-sandbox {}", revision);
                println!("config={}", config_path);
                println!("config_error={}", e);
                return;
            }
        }
    }

    let revision = env_revision.unwrap_or_else(|| "unknown".to_string());
    println!("cloister-sandbox {}", revision);
}

fn cleanup(state: CleanupState) {
    let CleanupState {
        ssh_handle,
        mut dbus_proxy,
        mut pulse_bridge,
        wayland_socket,
        machine_id_path,
        proc_privacy_state,
        anonymize_file_paths,
        flatpak_portal_state,
        broker_session_record_path,
        broker_listener,
    } = state;
    // SSH filter cleanup (SshFilterHandle::drop handles this)
    drop(ssh_handle);

    if let Some(mut proxy) = dbus_proxy.take() {
        let _ = proxy.child.kill();
        let _ = proxy.child.wait();
        let _ = socket::remove_stale_socket(&proxy.socket_path);
    }

    if let Some(mut bridge) = pulse_bridge.take() {
        let _ = bridge.child.kill();
        let _ = bridge.child.wait();
        let _ = socket::remove_stale_socket(&bridge.socket_path);
        let _ = std::fs::remove_dir(&bridge.runtime_dir);
        for path in bridge.anonymize_file_paths.drain(..) {
            let _ = std::fs::remove_file(&path);
        }
    }

    // Wayland socket cleanup
    if let Some(path) = wayland_socket {
        let _ = cloister_sandbox_lib::socket::remove_stale_socket(&path);
    }

    // Machine-id temp file cleanup
    if let Some(path) = machine_id_path {
        let _ = std::fs::remove_file(&path);
    }

    if let Some(state) = proc_privacy_state {
        for path in state.file_paths {
            let _ = std::fs::remove_file(&path);
        }
    }

    // Anonymization temp file cleanup
    for path in anonymize_file_paths {
        let _ = std::fs::remove_file(&path);
    }

    if let Some(state) = flatpak_portal_state {
        let _ = runtime::remove_path_if_exists(&state.flatpak_info_path);
        let _ = runtime::remove_path_if_exists(&state.instance_dir);
    }

    if let Some(listener) = broker_listener {
        listener.stop();
    }

    if let Some(path) = broker_session_record_path {
        let removal = (|| {
            let base = path.parent().ok_or_else(|| {
                format!(
                    "session record path has no parent directory: {}",
                    path.display()
                )
            })?;
            let token = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    format!(
                        "session record path has invalid token filename: {}",
                        path.display()
                    )
                })?;
            broker_store::remove_session_record(base, token)
        })();
        if let Err(e) = removal {
            eprintln!("cleanup: {e}");
        }
    }
}

fn cleanup_and_return_exit_code(state: CleanupState, exit_code: i32) -> i32 {
    cleanup(state);
    exit_code
}

fn cleanup_and_exit(state: CleanupState) -> i32 {
    cleanup_and_return_exit_code(state, 1)
}

fn requires_xdg_runtime_dir(config: &SandboxConfig) -> bool {
    config.dbus_enable
        || config.gui_enable
        || config.pipewire_pulse_config_path.is_some()
        || config.pipewire_socket_name.is_some()
        || !config.worker_broker.profiles.is_empty()
}

fn validate_xdg_runtime_dir(config: &SandboxConfig, xdg_runtime_dir: &str) -> Result<(), String> {
    if requires_xdg_runtime_dir(config) && xdg_runtime_dir.is_empty() {
        return Err(
            "XDG_RUNTIME_DIR must be set when using D-Bus, GUI, audio, or worker broker features."
                .to_string(),
        );
    }
    Ok(())
}

fn sandbox_ssh_auth_sock(
    auth_sock: &str,
    host_xdg_runtime_dir: &str,
    sandbox_xdg_runtime_dir: &str,
) -> (String, Option<String>) {
    if !host_xdg_runtime_dir.is_empty() && !sandbox_xdg_runtime_dir.is_empty() {
        if let Ok(relative) = Path::new(auth_sock).strip_prefix(host_xdg_runtime_dir) {
            let sandbox_auth_sock = Path::new(sandbox_xdg_runtime_dir).join(relative);
            let sandbox_auth_sock = sandbox_auth_sock.to_string_lossy().to_string();
            let sandbox_parent = Path::new(&sandbox_auth_sock)
                .parent()
                .filter(|parent| *parent != Path::new(sandbox_xdg_runtime_dir))
                .map(|parent| parent.to_string_lossy().to_string());
            return (sandbox_auth_sock, sandbox_parent);
        }
    }

    ("/ssh-agent".to_string(), None)
}

fn ssh_filter_socket_path(host_xdg_runtime_dir: &str, pid: u32) -> String {
    if host_xdg_runtime_dir.is_empty() {
        format!("/tmp/cloister-ssh-filter-{pid}/agent.sock")
    } else {
        format!("{host_xdg_runtime_dir}/cloister-ssh-filter-{pid}")
    }
}

/// Parse Cloister-owned flags and remaining sandbox arguments.
fn parse_cli_args(args: &[String]) -> Result<CliArgs, String> {
    let mut parsed = CliArgs {
        config_path: None,
        after_netns: false,
        broker_launch: None,
        broker_trusted_record_path: None,
        sandbox_args: Vec::new(),
    };
    let mut i = 1;
    let mut past_separator = false;
    let mut shell_mode = false;

    while i < args.len() {
        if past_separator {
            parsed.sandbox_args.push(args[i].clone());
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "--config" => {
                i += 1;
                if i >= args.len() {
                    return Err("cloister-sandbox: --config requires a path".to_string());
                }
                set_config_path(&mut parsed, args[i].clone())?;
            }
            "--after-netns" => {
                set_after_netns(&mut parsed)?;
            }
            "--broker-launch-profile" => {
                i += 1;
                if i >= args.len() {
                    return Err(
                        "cloister-sandbox: --broker-launch-profile requires a profile".to_string(),
                    );
                }
                set_broker_launch_profile(&mut parsed, args[i].clone())?;
            }
            "--broker-launch-sandbox" => {
                i += 1;
                if i >= args.len() {
                    return Err(
                        "cloister-sandbox: --broker-launch-sandbox requires a sandbox".to_string(),
                    );
                }
                set_broker_launch_sandbox(&mut parsed, args[i].clone())?;
            }
            "--broker-trusted-record" => {
                i += 1;
                if i >= args.len() {
                    return Err(
                        "cloister-sandbox: --broker-trusted-record requires a path".to_string()
                    );
                }
                set_broker_trusted_record_path(&mut parsed, args[i].clone())?;
            }
            "--" => {
                past_separator = true;
            }
            "--shell" => {
                shell_mode = true;
                parsed.sandbox_args.push(args[i].clone());
            }
            _ => {
                if shell_mode {
                    parsed.sandbox_args.push(args[i].clone());
                } else {
                    parsed.sandbox_args.extend(args[i..].iter().cloned());
                    break;
                }
            }
        }
        i += 1;
    }

    if let Some(selector) = parsed.broker_launch.as_ref() {
        if selector.profile.is_empty() || selector.sandbox.is_empty() {
            return Err(
                "cloister-sandbox: --broker-launch-profile and --broker-launch-sandbox must be passed together"
                    .to_string(),
            );
        }
    }

    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use cloister_sandbox_lib::config::{DelegatedAccessMode, DelegatedPerDirMount};

    /// Serialize tests that touch shared signal statics (CHILD_PID, SIGINT_COUNT, etc.).
    fn signal_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn temp_test_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", process::id()))
    }

    fn config_with_flags(
        dbus_enable: bool,
        gui_enable: bool,
        pipewire_socket_name: Option<String>,
        pipewire_pulse_config_path: Option<String>,
        ssh_enable: bool,
    ) -> SandboxConfig {
        serde_json::from_value(serde_json::json!({
            "name": "test",
            "bwrap_path": "/nix/store/xxx-bubblewrap/bin/bwrap",
            "shell_bin": "/nix/store/xxx-zsh/bin/zsh",
            "shell_interactive_args": ["-i"],
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/user",
            "store_mode": "host",
            "store_roots": ["/nix/store/aaa-shell"],
            "per_dir": {},
            "copy_file_base": "/home/user/.local/state/cloister",
            "git_path": "/nix/store/xxx-git/bin/git",
            "dbus_enable": dbus_enable,
            "gui_enable": gui_enable,
            "pipewire_socket_name": pipewire_socket_name,
            "pipewire_pulse_config_path": pipewire_pulse_config_path,
            "ssh_enable": ssh_enable
        }))
        .expect("valid config")
    }

    fn broker_session_fixture() -> broker::BrokerSession {
        broker::BrokerSession {
            token: "token-1".to_string(),
            project_root: "/workspace/project".to_string(),
            dir_hash: "abc123def456".to_string(),
            profiles: BTreeMap::from([
                (
                    "overlay".to_string(),
                    broker::BrokerSpawnableProfile {
                        sandbox: "worker".to_string(),
                        workspace_mode: WorkspaceMode::ProjectOverlay,
                        delegated_per_dir_mounts: BTreeMap::from([
                            (
                                "worktrees".to_string(),
                                DelegatedPerDirMount {
                                    mode: DelegatedAccessMode::Rw,
                                    path: "/local/worktrees/dev".to_string(),
                                    sub_path: None,
                                },
                            ),
                            (
                                ".cache/pre-commit".to_string(),
                                DelegatedPerDirMount {
                                    mode: DelegatedAccessMode::Ro,
                                    path: "/local/ephemeral/dev".to_string(),
                                    sub_path: Some(".cache/pre-commit".to_string()),
                                },
                            ),
                        ]),
                    },
                ),
                (
                    "project".to_string(),
                    broker::BrokerSpawnableProfile {
                        sandbox: "worker".to_string(),
                        workspace_mode: WorkspaceMode::ProjectRw,
                        delegated_per_dir_mounts: BTreeMap::new(),
                    },
                ),
            ]),
        }
    }

    fn config_with_worker_broker() -> SandboxConfig {
        serde_json::from_value(serde_json::json!({
            "name": "dev",
            "bwrap_path": "/nix/store/xxx-bubblewrap/bin/bwrap",
            "shell_bin": "/nix/store/xxx-zsh/bin/zsh",
            "shell_interactive_args": ["-i"],
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/user",
            "store_mode": "host",
            "store_roots": ["/nix/store/aaa-shell"],
            "per_dir": {},
            "copy_file_base": "/home/user/.local/state/cloister",
            "git_path": "/nix/store/xxx-git/bin/git",
            "worker_broker": {
                "profiles": {
                    "overlay": {
                        "sandbox": "worker",
                        "workspace": {
                            "mode": "project-overlay"
                        },
                        "delegated_per_dir_mounts": {
                            "worktrees": {
                                "mode": "rw",
                                "path": "/local/worktrees/dev",
                                "sub_path": null
                            },
                            ".cache/pre-commit": {
                                "mode": "ro",
                                "path": "/local/ephemeral/dev",
                                "sub_path": ".cache/pre-commit"
                            }
                        }
                    },
                    "project": {
                        "sandbox": "worker",
                        "workspace": {
                            "mode": "project-rw"
                        }
                    }
                },
                "generated_launchers": {
                    "clb-overlay": {
                        "profile": "overlay",
                        "sandbox": "worker"
                    },
                    "clb-project": {
                        "profile": "project",
                        "sandbox": "worker"
                    }
                }
            }
        }))
        .expect("valid worker broker config")
    }

    fn env_lock_guard() -> std::sync::MutexGuard<'static, ()> {
        env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn broker_child_profile_lookup_rejects_unknown_profile() {
        let session = broker_session_fixture();

        let err = lookup_child_profile(&session, "missing").unwrap_err();
        assert!(err.contains("undefined child profile 'missing'"));
    }

    #[test]
    fn broker_workspace_mode_selects_overlay_mounts() {
        let session = broker_session_fixture();
        let profile = lookup_child_profile(&session, "overlay").unwrap();

        assert_eq!(
            workspace_mode_args(profile, &session.project_root, Some("/workspace/project")),
            vec![
                "--overlay-src",
                "/workspace/project",
                "--tmp-overlay",
                "/workspace/project",
            ]
        );
    }

    #[test]
    fn broker_workspace_mode_skips_extra_args_for_project_rw() {
        let session = broker_session_fixture();
        let profile = lookup_child_profile(&session, "project").unwrap();

        assert!(workspace_mode_args(profile, &session.project_root, None).is_empty());
    }

    #[test]
    fn broker_delegated_mount_args_use_stored_session_config() {
        let session = broker_session_fixture();
        let profile = lookup_child_profile(&session, "overlay").unwrap();

        assert_eq!(
            delegated_mount_args(&session, profile, &session.project_root).unwrap(),
            vec![
                "--ro-bind",
                "/local/ephemeral/dev/abc123def456/.cache/pre-commit",
                "/workspace/project/.cache/pre-commit",
                "--bind",
                "/local/worktrees/dev/abc123def456",
                "/workspace/project/worktrees",
            ]
        );
    }

    #[test]
    fn parent_broker_session_captures_current_project_root_and_capabilities() {
        let config = config_with_worker_broker();

        let session = create_parent_broker_session(&config, "/workspace/project", "abc123def456")
            .unwrap()
            .expect("parent session");

        assert_eq!(session.project_root, "/workspace/project");
        assert_eq!(session.dir_hash, "abc123def456");
        assert_eq!(
            lookup_child_profile(&session, "overlay")
                .unwrap()
                .workspace_mode,
            WorkspaceMode::ProjectOverlay
        );
        assert_eq!(
            lookup_child_profile(&session, "overlay")
                .unwrap()
                .delegated_per_dir_mounts
                .get(".cache/pre-commit")
                .and_then(|mount| mount.sub_path.as_deref()),
            Some(".cache/pre-commit")
        );
        assert!(!session.token.is_empty());
    }

    #[test]
    fn parent_broker_session_uses_child_visible_project_root_under_anonymization() {
        let mut config = config_with_worker_broker();
        config.anonymize = true;
        config.home_directory = "/home/alice".to_string();
        config.sandbox_home = "/home/ubuntu".to_string();

        let session =
            create_parent_broker_session(&config, "/home/alice/src/project", "abc123def456")
                .unwrap()
                .expect("parent session");

        assert_eq!(session.project_root, "/home/ubuntu/src/project");
    }

    #[test]
    fn parent_broker_session_generates_opaque_per_launch_token() {
        let config = config_with_worker_broker();

        let first = create_parent_broker_session(&config, "/workspace/project", "abc123def456")
            .unwrap()
            .expect("first parent session");
        let second = create_parent_broker_session(&config, "/workspace/project", "abc123def456")
            .unwrap()
            .expect("second parent session");

        assert_ne!(first.token, second.token);
        assert!(!first.token.contains("/workspace/project"));
        assert!(!first.token.contains("abc123def456"));
        assert_eq!(first.token.len(), 32);
        assert!(first.token.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn parent_broker_capability_env_args_export_serialized_capability() {
        let session = broker_session_fixture();

        let args = parent_broker_env_args(&session).unwrap();
        let payload = args
            .windows(3)
            .find(|window| {
                window[0] == "--setenv" && window[1] == "CLOISTER_BROKER_PARENT_CAPABILITY"
            })
            .map(|window| window[2].clone())
            .expect("broker parent capability env payload");
        let restored: broker::BrokerParentCapability = serde_json::from_str(&payload).unwrap();

        assert_eq!(restored.token, session.token);
        assert!(args.windows(2).all(|window| {
            !(window[0] == "--setenv" && window[1] == "CLOISTER_BROKER_SESSION")
        }));
    }

    #[test]
    fn parent_broker_capability_env_args_do_not_embed_session_policy() {
        let session = broker_session_fixture();

        let args = parent_broker_env_args(&session).unwrap();
        let payload = args
            .windows(3)
            .find(|window| window[0] == "--setenv" && window[1] == BROKER_PARENT_CAPABILITY_ENV)
            .map(|window| window[2].clone())
            .expect("broker parent capability env payload");
        let payload_json: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert!(payload_json.get("token").is_some());
        assert_eq!(payload_json.as_object().map(|value| value.len()), Some(1));
        assert!(payload_json.get("project_root").is_none());
        assert!(payload_json.get("dir_hash").is_none());
        assert!(payload_json.get("profiles").is_none());
    }

    #[test]
    fn register_parent_broker_launch_writes_session_record_to_host_runtime_store() {
        let config = config_with_worker_broker();
        let host_runtime_dir = temp_test_dir("register-parent");

        let registration = register_parent_broker_launch(
            &config,
            "/workspace/project",
            "abc123def456",
            host_runtime_dir.to_str().unwrap(),
        )
        .unwrap()
        .expect("parent session registration");

        let env_args = registration.env_args;

        let payload = env_args
            .windows(3)
            .find(|window| window[0] == "--setenv" && window[1] == BROKER_PARENT_CAPABILITY_ENV)
            .map(|window| window[2].clone())
            .expect("broker parent capability env payload");
        let capability: broker::BrokerParentCapability = serde_json::from_str(&payload).unwrap();
        let record_path = host_runtime_dir
            .join("cloister")
            .join("broker")
            .join("sessions")
            .join(format!("{}.json", capability.token));

        assert!(
            record_path.is_file(),
            "missing session record: {}",
            record_path.display()
        );

        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        assert_eq!(
            record.get("token").and_then(|value| value.as_str()),
            Some(capability.token.as_str())
        );
        assert_eq!(
            record.get("project_root").and_then(|value| value.as_str()),
            Some("/workspace/project")
        );
        assert_eq!(
            record.get("dir_hash").and_then(|value| value.as_str()),
            Some("abc123def456")
        );
        assert!(record.get("profiles").is_some());
        assert!(env_args.windows(3).any(|window| {
            window[0] == "--bind"
                && window[1]
                    == broker_store::socket_path(
                        host_runtime_dir.to_str().unwrap(),
                        &capability.token,
                    )
                    .unwrap()
                    .to_string_lossy()
                && window[2] == BROKER_SOCKET_CHILD_PATH
        }));

        let _ = std::fs::remove_dir_all(&host_runtime_dir);
    }

    #[test]
    fn register_parent_broker_launch_exports_only_opaque_capability() {
        let config = config_with_worker_broker();
        let host_runtime_dir = temp_test_dir("register-capability");

        let registration = register_parent_broker_launch(
            &config,
            "/workspace/project",
            "abc123def456",
            host_runtime_dir.to_str().unwrap(),
        )
        .unwrap()
        .expect("parent session registration");

        let env_args = registration.env_args;

        let payload = env_args
            .windows(3)
            .find(|window| window[0] == "--setenv" && window[1] == BROKER_PARENT_CAPABILITY_ENV)
            .map(|window| window[2].clone())
            .expect("broker parent capability env payload");
        let payload_json: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert!(payload_json.get("token").is_some());
        assert_eq!(payload_json.as_object().map(|value| value.len()), Some(1));
        assert!(payload_json.get("project_root").is_none());
        assert!(payload_json.get("dir_hash").is_none());
        assert!(payload_json.get("profiles").is_none());

        let _ = std::fs::remove_dir_all(&host_runtime_dir);
    }

    #[test]
    fn parent_broker_session_exports_with_empty_per_dir_when_worker_broker_enabled() {
        let config = config_with_worker_broker();

        let session = create_parent_broker_session(&config, "/workspace/project", "abc123def456")
            .unwrap()
            .expect("parent session");

        assert_eq!(session.dir_hash, "abc123def456");
        assert_eq!(session.project_root, "/workspace/project");
    }

    #[test]
    fn parent_broker_session_rejects_missing_dir_hash_when_enabled() {
        let config = config_with_worker_broker();

        let err = create_parent_broker_session(&config, "/workspace/project", "").unwrap_err();

        assert!(err.contains("non-empty project root and dir hash"));
    }

    #[test]
    fn apply_child_broker_profile_loads_session_policy_from_trusted_record_mount() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let session = broker_session_fixture();
        let trusted_record = temp_test_dir("child-broker-load-session").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        assert!(
            child_broker_args_with_store_dir(
                "worker",
                None,
                "/workspace/project",
                "/workspace/project",
                "/run/user/1000-child",
                Some(trusted_record.as_path()),
                Some("/workspace/project")
            )
            .unwrap()
            .is_some()
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_accepts_cli_selector_without_legacy_profile_env() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let session = broker_session_fixture();
        let trusted_record = temp_test_dir("child-broker-cli-selector").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::remove_var(BROKER_CHILD_PROFILE_ENV);

        let selector = BrokerLaunchSelector {
            profile: "overlay".to_string(),
            sandbox: "worker".to_string(),
        };

        assert!(
            child_broker_args_with_store_dir(
                "worker",
                Some(&selector),
                "/workspace/project",
                "/workspace/project",
                "/run/user/1000-child",
                Some(trusted_record.as_path()),
                Some("/workspace/project"),
            )
            .unwrap()
            .is_some()
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_rejects_cli_selector_for_other_sandbox() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let session = broker_session_fixture();
        let trusted_record =
            temp_test_dir("child-broker-cli-selector-sandbox").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::remove_var(BROKER_CHILD_PROFILE_ENV);

        let selector = BrokerLaunchSelector {
            profile: "overlay".to_string(),
            sandbox: "dev".to_string(),
        };

        let err = child_broker_args_with_store_dir(
            "worker",
            Some(&selector),
            "/workspace/project",
            "/workspace/project",
            "/run/user/1000-child",
            Some(trusted_record.as_path()),
            Some("/workspace/project"),
        )
        .unwrap_err();
        assert!(err.contains("broker launcher targets sandbox 'dev'"));
        assert!(err.contains("current sandbox is 'worker'"));

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn register_parent_broker_launch_exports_child_visible_session_record_path() {
        let config = config_with_worker_broker();
        let host_runtime_dir = temp_test_dir("register-broker-store-path");

        let registration = register_parent_broker_launch(
            &config,
            "/workspace/project",
            "abc123def456",
            host_runtime_dir.to_str().unwrap(),
        )
        .unwrap()
        .expect("parent session registration");

        let env_args = registration.env_args;

        let payload = env_args
            .windows(3)
            .find(|window| window[0] == "--setenv" && window[1] == BROKER_PARENT_CAPABILITY_ENV)
            .map(|window| window[2].clone())
            .expect("broker parent capability env payload");
        let capability: broker::BrokerParentCapability = serde_json::from_str(&payload).unwrap();

        assert!(env_args.windows(3).any(|window| {
            window[0] == "--ro-bind"
                && window[1]
                    == broker_store::session_store_dir(host_runtime_dir.to_str().unwrap())
                        .join(format!("{}.json", capability.token))
                        .to_string_lossy()
                && window[2] == "/run/cloister/broker/session.json"
        }));
        assert!(
            !env_args
                .iter()
                .any(|arg| arg == "CLOISTER_BROKER_SESSION_STORE")
        );

        let _ = std::fs::remove_dir_all(&host_runtime_dir);
    }

    #[test]
    fn cleanup_removes_registered_parent_broker_session_record() {
        let config = config_with_worker_broker();
        let host_runtime_dir = temp_test_dir("cleanup-parent-record");

        let registration = register_parent_broker_launch(
            &config,
            "/workspace/project",
            "abc123def456",
            host_runtime_dir.to_str().unwrap(),
        )
        .unwrap()
        .expect("parent session registration");

        let payload = registration
            .env_args
            .windows(3)
            .find(|window| window[0] == "--setenv" && window[1] == BROKER_PARENT_CAPABILITY_ENV)
            .map(|window| window[2].clone())
            .expect("broker parent capability env payload");
        let capability: broker::BrokerParentCapability = serde_json::from_str(&payload).unwrap();
        let record_path = host_runtime_dir
            .join("cloister")
            .join("broker")
            .join("sessions")
            .join(format!("{}.json", capability.token));

        assert!(
            record_path.is_file(),
            "session record should exist before cleanup"
        );

        cleanup(CleanupState {
            ssh_handle: None,
            dbus_proxy: None,
            pulse_bridge: None,
            wayland_socket: None,
            machine_id_path: None,
            proc_privacy_state: None,
            anonymize_file_paths: Vec::new(),
            flatpak_portal_state: None,
            broker_session_record_path: registration.broker_session_record_path,
            broker_listener: None,
        });

        assert!(
            !record_path.exists(),
            "session record should be removed during cleanup"
        );

        let _ = std::fs::remove_dir_all(&host_runtime_dir);
    }

    #[test]
    fn cleanup_and_exit_removes_registered_parent_broker_session_record() {
        let config = config_with_worker_broker();
        let host_runtime_dir = temp_test_dir("cleanup-parent-record-exit");

        let registration = register_parent_broker_launch(
            &config,
            "/workspace/project",
            "abc123def456",
            host_runtime_dir.to_str().unwrap(),
        )
        .unwrap()
        .expect("parent session registration");

        let payload = registration
            .env_args
            .windows(3)
            .find(|window| window[0] == "--setenv" && window[1] == BROKER_PARENT_CAPABILITY_ENV)
            .map(|window| window[2].clone())
            .expect("broker parent capability env payload");
        let capability: broker::BrokerParentCapability = serde_json::from_str(&payload).unwrap();
        let record_path = host_runtime_dir
            .join("cloister")
            .join("broker")
            .join("sessions")
            .join(format!("{}.json", capability.token));

        assert!(
            record_path.is_file(),
            "session record should exist before cleanup"
        );

        let exit_code = cleanup_and_exit(CleanupState {
            ssh_handle: None,
            dbus_proxy: None,
            pulse_bridge: None,
            wayland_socket: None,
            machine_id_path: None,
            proc_privacy_state: None,
            anonymize_file_paths: Vec::new(),
            flatpak_portal_state: None,
            broker_session_record_path: registration.broker_session_record_path,
            broker_listener: None,
        });

        assert_eq!(exit_code, 1);
        assert!(
            !record_path.exists(),
            "session record should be removed during cleanup"
        );

        let _ = std::fs::remove_dir_all(&host_runtime_dir);
    }

    #[test]
    fn cleanup_and_return_exit_code_preserves_requested_exit_code() {
        let config = config_with_worker_broker();
        let host_runtime_dir = temp_test_dir("cleanup-parent-record-exit-code");

        let registration = register_parent_broker_launch(
            &config,
            "/workspace/project",
            "abc123def456",
            host_runtime_dir.to_str().unwrap(),
        )
        .unwrap()
        .expect("parent session registration");

        let payload = registration
            .env_args
            .windows(3)
            .find(|window| window[0] == "--setenv" && window[1] == BROKER_PARENT_CAPABILITY_ENV)
            .map(|window| window[2].clone())
            .expect("broker parent capability env payload");
        let capability: broker::BrokerParentCapability = serde_json::from_str(&payload).unwrap();
        let record_path = host_runtime_dir
            .join("cloister")
            .join("broker")
            .join("sessions")
            .join(format!("{}.json", capability.token));

        assert!(
            record_path.is_file(),
            "session record should exist before cleanup"
        );

        let exit_code = cleanup_and_return_exit_code(
            CleanupState {
                ssh_handle: None,
                dbus_proxy: None,
                pulse_bridge: None,
                wayland_socket: None,
                machine_id_path: None,
                proc_privacy_state: None,
                anonymize_file_paths: Vec::new(),
                flatpak_portal_state: None,
                broker_session_record_path: registration.broker_session_record_path,
                broker_listener: None,
            },
            2,
        );

        assert_eq!(exit_code, 2);
        assert!(
            !record_path.exists(),
            "session record should be removed during cleanup"
        );

        let _ = std::fs::remove_dir_all(&host_runtime_dir);
    }

    #[test]
    fn child_broker_record_path_ignores_env_override_and_uses_injected_trusted_path() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let session = broker_session_fixture();
        let trusted_record = temp_test_dir("child-broker-trusted-store").join("session.json");
        let attacker_root = temp_test_dir("child-broker-attacker-store");

        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();

        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");
        std::env::set_var("CLOISTER_BROKER_SESSION_STORE", &attacker_root);

        assert!(
            child_broker_args_with_store_dir(
                "worker",
                None,
                "/workspace/project",
                "/workspace/project",
                "/run/user/1000-child",
                Some(trusted_record.as_path()),
                Some("/workspace/project")
            )
            .unwrap()
            .is_some()
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }
        std::env::remove_var("CLOISTER_BROKER_SESSION_STORE");

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
        let _ = std::fs::remove_dir_all(&attacker_root);
    }

    #[test]
    fn child_broker_record_path_fails_closed_without_trusted_record_mount() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let session = broker_session_fixture();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        let err = child_broker_args_with_store_dir(
            "worker",
            None,
            "/workspace/project",
            "/workspace/project",
            "/run/user/1000-child",
            Some(Path::new("/definitely/missing/trusted-session.json")),
            Some("/workspace/project"),
        )
        .unwrap_err();
        assert!(err.contains("trusted broker session record mount is unavailable"));

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }
    }

    #[test]
    fn apply_child_broker_profile_rejects_legacy_full_session_env_payload() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let session = broker_session_fixture();
        let trusted_record =
            temp_test_dir("child-broker-legacy-full-session-env").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&session).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        let err = child_broker_args_with_store_dir(
            "worker",
            None,
            "/workspace/project",
            "/workspace/project",
            "/run/user/1000-child",
            Some(trusted_record.as_path()),
            Some("/workspace/project"),
        )
        .unwrap_err();
        assert!(
            err.contains("unknown field")
                || err.contains("parse CLOISTER_BROKER_PARENT_CAPABILITY"),
            "expected strict capability parse failure, got: {err}"
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_rejects_unknown_token_from_host_runtime_store() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let session = broker_session_fixture();
        let trusted_record = temp_test_dir("child-broker-unknown-token").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        let mut mismatched_session = session.clone();
        mismatched_session.token = "different-token".to_string();
        std::fs::write(
            &trusted_record,
            serde_json::to_vec(&mismatched_session).unwrap(),
        )
        .unwrap();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        let err = child_broker_args_with_store_dir(
            "worker",
            None,
            "/workspace/project",
            "/workspace/project",
            "/run/user/1000-child",
            Some(trusted_record.as_path()),
            Some("/workspace/project"),
        )
        .unwrap_err();
        assert!(err.contains("broker session token not found: token-1"));

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_applies_overlay_and_delegated_mount_args_from_stored_session_policy()
     {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let session = broker_session_fixture();
        let trusted_record = temp_test_dir("child-broker-apply-args").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        let args = child_broker_args_with_store_dir(
            "worker",
            None,
            "/workspace/project",
            "/workspace/project",
            "/run/user/1000-child",
            Some(trusted_record.as_path()),
            Some("/workspace/project"),
        )
        .unwrap()
        .expect("child broker args");
        assert_eq!(
            args,
            s(&[
                "--overlay-src",
                "/workspace/project",
                "--tmp-overlay",
                "/workspace/project",
                "--ro-bind",
                "/local/ephemeral/dev/abc123def456/.cache/pre-commit",
                "/workspace/project/.cache/pre-commit",
                "--bind",
                "/local/worktrees/dev/abc123def456",
                "/workspace/project/worktrees",
            ])
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_rejects_empty_capability_token() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let mut session = broker_session_fixture();
        session.token.clear();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");

        let err = child_broker_args(
            "worker",
            None,
            "/workspace/project",
            "/workspace/project",
            "/run/user/1000",
            None,
            Some("/workspace/project"),
        )
        .unwrap_err();
        assert!(err.contains("broker parent capability token must not be empty"));

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
    }

    #[test]
    fn apply_child_broker_profile_rejects_project_root_mismatch_before_session_lookup() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let session = broker_session_fixture();
        let trusted_record = temp_test_dir("child-broker-project-mismatch").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        let err = child_broker_args_with_store_dir(
            "worker",
            None,
            "/workspace/other-project",
            "/workspace/other-project",
            "/run/user/1000",
            Some(trusted_record.as_path()),
            Some("/workspace/project"),
        )
        .unwrap_err();
        assert!(err.contains("broker project identity does not match"));

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_accepts_mixed_parent_child_path_views_when_dir_hash_matches() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let mut session = broker_session_fixture();
        session.project_root = "/home/ubuntu/src/project".to_string();
        session.dir_hash = runtime::compute_dir_hash("/home/alice/src/project");
        let trusted_record = temp_test_dir("child-broker-mixed-view").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        let capability = broker::BrokerParentCapability {
            token: session.token.clone(),
        };
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&capability).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        assert!(
            child_broker_args_with_store_dir(
                "worker",
                None,
                "/home/alice/src/project",
                "/home/ubuntu/src/project",
                "/run/user/1000-child",
                Some(trusted_record.as_path()),
                Some("/home/ubuntu/src/project")
            )
            .unwrap()
            .is_some()
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_rejects_unrelated_requested_root_when_views_differ() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let mut session = broker_session_fixture();
        session.project_root = "/home/ubuntu/src/project".to_string();
        session.dir_hash = "abc123def456".to_string();
        let trusted_record = temp_test_dir("child-broker-unrelated-root").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        let capability = broker::BrokerParentCapability {
            token: session.token.clone(),
        };
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&capability).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        let err = child_broker_args_with_store_dir(
            "worker",
            None,
            "/tmp/other-project",
            "/home/ubuntu/src/project",
            "/run/user/1000-child",
            Some(trusted_record.as_path()),
            Some("/home/ubuntu/src/project"),
        )
        .unwrap_err();
        assert!(err.contains("broker project identity does not match"));

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_rejects_requested_root_when_session_dir_hash_targets_other_view()
    {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let mut session = broker_session_fixture();
        session.project_root = "/home/ubuntu/src/project".to_string();
        session.dir_hash = runtime::compute_dir_hash("/home/alice/src/project");
        let trusted_record = temp_test_dir("child-broker-hash-mismatch").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        let capability = broker::BrokerParentCapability {
            token: session.token.clone(),
        };
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&capability).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        let err = child_broker_args_with_store_dir(
            "worker",
            None,
            "/tmp/unrelated-project",
            "/home/ubuntu/src/project",
            "/run/user/1000-child",
            Some(trusted_record.as_path()),
            Some("/home/ubuntu/src/project"),
        )
        .unwrap_err();
        assert!(err.contains("broker project identity does not match"));

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_rejects_mixed_views_when_requested_root_hash_differs() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let mut session = broker_session_fixture();
        session.project_root = "/home/ubuntu/src/project".to_string();
        session.dir_hash = runtime::compute_dir_hash("/home/alice/src/project");
        let trusted_record =
            temp_test_dir("child-broker-capability-path-match").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        let capability = broker::BrokerParentCapability {
            token: session.token.clone(),
        };
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&capability).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        let err = child_broker_args_with_store_dir(
            "worker",
            None,
            "/home/alice/src/other-project",
            "/home/ubuntu/src/project",
            "/run/user/1000-child",
            Some(trusted_record.as_path()),
            Some("/home/ubuntu/src/project"),
        )
        .unwrap_err();
        assert!(err.contains("broker project identity does not match"));

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_accepts_exact_project_root_match_without_env_identity_fields() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let session = broker_session_fixture();
        let trusted_record = temp_test_dir("child-broker-exact-match").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        let capability = broker::BrokerParentCapability {
            token: session.token.clone(),
        };
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&capability).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        assert!(
            child_broker_args_with_store_dir(
                "worker",
                None,
                "/workspace/project",
                "/workspace/project",
                "/run/user/1000-child",
                Some(trusted_record.as_path()),
                Some("/workspace/project"),
            )
            .unwrap()
            .is_some()
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_uses_project_root_overlay_source() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let session = broker_session_fixture();
        let trusted_record = temp_test_dir("child-broker-overlay-lower").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        let capability = broker::BrokerParentCapability {
            token: session.token.clone(),
        };
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&capability).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        assert_eq!(
            child_broker_args_with_store_dir(
                "worker",
                None,
                "/workspace/project",
                "/workspace/project",
                "/run/user/1000-child",
                Some(trusted_record.as_path()),
                Some("/workspace/project"),
            )
            .unwrap()
            .unwrap(),
            vec![
                "--overlay-src",
                "/workspace/project",
                "--tmp-overlay",
                "/workspace/project",
                "--ro-bind",
                "/local/ephemeral/dev/abc123def456/.cache/pre-commit",
                "/workspace/project/.cache/pre-commit",
                "--bind",
                "/local/worktrees/dev/abc123def456",
                "/workspace/project/worktrees",
            ]
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn child_overlay_launch_filters_generated_project_bind_after_trusted_profile_resolution() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");

        let session = broker_session_fixture();
        let trusted_record = temp_test_dir("child-broker-overlay-build-bwrap").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        let mut config = config_with_worker_broker();
        config
            .dynamic_binds
            .push(cloister_sandbox_lib::config::DynamicBind {
                src: "$SANDBOX_DIR".to_string(),
                dest: Some("$SANDBOX_DEST".to_string()),
                mode: cloister_sandbox_lib::config::BindMode::Rw,
                try_bind: false,
            });

        let mut runtime_vars = std::collections::HashMap::new();
        runtime_vars.insert("SANDBOX_DIR".to_string(), "/workspace/project".to_string());
        runtime_vars.insert("SANDBOX_DEST".to_string(), "/workspace/project".to_string());

        let profile = lookup_child_profile(&session, "overlay").unwrap();
        let filtered =
            filter_child_overlay_project_bind(&config.dynamic_binds, &runtime_vars, Some(profile));

        assert!(filtered.is_empty());

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn apply_child_broker_profile_returns_false_without_broker_env() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        std::env::remove_var(BROKER_CHILD_PROFILE_ENV);

        assert!(
            child_broker_args(
                "worker",
                None,
                "/workspace/project",
                "/workspace/project",
                "/run/user/1000",
                None,
                None
            )
            .unwrap()
            .is_none()
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        }
    }

    #[test]
    fn apply_child_broker_profile_returns_false_with_capability_without_profile_selector() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let session = broker_session_fixture();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::remove_var(BROKER_CHILD_PROFILE_ENV);

        assert!(
            child_broker_args(
                "worker",
                None,
                "/workspace/project",
                "/workspace/project",
                "/run/user/1000",
                None,
                None
            )
            .unwrap()
            .is_none()
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        }
    }

    #[test]
    fn apply_child_broker_profile_returns_false_with_profile_selector_without_capability() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");

        assert!(
            child_broker_args(
                "worker",
                None,
                "/workspace/project",
                "/workspace/project",
                "/run/user/1000",
                None,
                None
            )
            .unwrap()
            .is_none()
        );

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
    }

    #[test]
    fn child_broker_args_rejects_profile_for_different_sandbox() {
        let _guard = env_lock_guard();
        let original_capability = std::env::var_os(BROKER_PARENT_CAPABILITY_ENV);
        let original_profile = std::env::var_os(BROKER_CHILD_PROFILE_ENV);
        let original_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        let session = broker_session_fixture();
        let trusted_record = temp_test_dir("child-broker-sandbox-mismatch").join("session.json");
        std::fs::create_dir_all(trusted_record.parent().unwrap()).unwrap();
        std::fs::write(&trusted_record, serde_json::to_vec(&session).unwrap()).unwrap();
        std::env::set_var(
            BROKER_PARENT_CAPABILITY_ENV,
            serde_json::to_string(&broker::BrokerParentCapability::from(&session)).unwrap(),
        );
        std::env::set_var(BROKER_CHILD_PROFILE_ENV, "overlay");
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000-child");

        let err = child_broker_args_with_store_dir(
            "dev",
            None,
            "/workspace/project",
            "/workspace/project",
            "/run/user/1000-child",
            Some(trusted_record.as_path()),
            Some("/workspace/project"),
        )
        .unwrap_err();
        assert!(err.contains("targets sandbox 'worker'"));
        assert!(err.contains("current sandbox is 'dev'"));

        if let Some(value) = original_capability {
            std::env::set_var(BROKER_PARENT_CAPABILITY_ENV, value);
        } else {
            std::env::remove_var(BROKER_PARENT_CAPABILITY_ENV);
        }
        if let Some(value) = original_profile {
            std::env::set_var(BROKER_CHILD_PROFILE_ENV, value);
        } else {
            std::env::remove_var(BROKER_CHILD_PROFILE_ENV);
        }
        if let Some(value) = original_runtime_dir {
            std::env::set_var("XDG_RUNTIME_DIR", value);
        } else {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let _ = std::fs::remove_dir_all(trusted_record.parent().unwrap());
    }

    #[test]
    fn pulse_proxy_identity_uses_sandbox_home_leaf() {
        let mut config = config_with_flags(false, false, None, None, false);
        config.anonymize = true;
        config.sandbox_home = "/home/ubuntu".to_string();

        assert_eq!(pulse_proxy_identity(&config).unwrap(), "ubuntu");
    }

    #[test]
    fn pulse_proxy_identity_rejects_missing_leaf() {
        let mut config = config_with_flags(false, false, None, None, false);
        config.anonymize = true;
        config.sandbox_home = "/home/".to_string();

        let err = pulse_proxy_identity(&config).unwrap_err();
        assert!(err.contains("sandbox_home must end with a username"));
    }

    #[test]
    fn parse_proc_children_handles_multiple_pids() {
        assert_eq!(parse_proc_children("4724 4725\n"), vec![4724, 4725]);
        assert_eq!(parse_proc_children("\n"), Vec::<u32>::new());
    }

    #[test]
    fn pulse_proxy_command_sets_anonymous_identity() {
        let mut config = config_with_flags(false, false, None, None, false);
        config.anonymize = true;
        config.bwrap_path = "/nix/store/xxx-bubblewrap-subset-pid/bin/bwrap".to_string();
        config.sandbox_home = "/home/ubuntu".to_string();
        let cmd = build_pulse_only_proxy_command(
            &config,
            PulseProxyCommand {
                runtime_dir: "/run/user/1000/cloister/pulse/test-1",
                pipewire_remote: "/run/user/1000/cloister/pipewire/test",
                identity: "ubuntu",
                passwd_path: Some("/tmp/passwd"),
                group_path: Some("/tmp/group"),
                pipewire_pulse_bin: "/nix/store/xxx-pipewire/bin/pipewire-pulse",
                pipewire_pulse_conf: "/nix/store/xxx-pulse.conf",
            },
        )
        .unwrap();

        let argv: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        assert_eq!(
            cmd.get_program().to_string_lossy(),
            "/nix/store/xxx-bubblewrap-subset-pid/bin/bwrap"
        );
        assert!(argv.windows(2).any(|w| w == ["--hostname", "ubuntu"]));
        assert!(argv.windows(2).any(|w| w == ["--proc", "/proc"]));
        assert!(
            argv.windows(3)
                .any(|w| w == ["--ro-bind", "/tmp/passwd", "/etc/passwd"])
        );
        assert!(
            argv.windows(3)
                .any(|w| w == ["--ro-bind", "/tmp/group", "/etc/group"])
        );
        assert!(
            argv.windows(3)
                .any(|w| w == ["--setenv", "HOME", "/home/ubuntu"])
        );
        assert!(argv.windows(3).any(|w| w == ["--setenv", "USER", "ubuntu"]));
        assert!(
            argv.windows(3)
                .any(|w| w == ["--setenv", "LOGNAME", "ubuntu"])
        );
        assert!(argv.windows(2).any(|w| w == ["--dir", "/nix"]));
        assert!(argv.windows(2).any(|w| w == ["--dir", "/nix/store"]));
        assert!(
            argv.windows(3)
                .any(|w| w == ["--ro-bind", "/nix/store", "/nix/store"])
        );
        assert!(argv.windows(3).any(|w| w
            == [
                "--setenv",
                "PIPEWIRE_REMOTE",
                "/run/user/1000/cloister/pipewire/test"
            ]));
    }

    #[test]
    fn host_store_bind_args_mount_host_store_once() {
        assert_eq!(
            host_store_bind_args(),
            vec![
                "--ro-bind".to_string(),
                "/nix/store".to_string(),
                "/nix/store".to_string(),
            ]
        );
    }

    #[test]
    fn image_store_bind_args_mount_store_root_once() {
        assert_eq!(
            image_store_bind_args("/run/cloister/images/abc123"),
            vec![
                "--ro-bind".to_string(),
                "/run/cloister/images/abc123/nix/store".to_string(),
                "/nix/store".to_string(),
            ]
        );
    }

    #[test]
    fn mount_flags_indicate_read_only_detects_read_only_bit() {
        assert!(mount_flags_indicate_read_only(libc::ST_RDONLY));
        assert!(mount_flags_indicate_read_only(libc::ST_RDONLY | 0x20));
        assert!(!mount_flags_indicate_read_only(0));
    }

    #[test]
    fn ensure_image_store_mounted_accepts_ready_mount() {
        let dir = temp_test_dir("cloister-image-store-ready");
        let mount_path = dir.join("abc123");
        fs::create_dir_all(mount_path.join("nix/store/aaa-shell")).unwrap();
        fs::write(
            mount_path.join("meta.json"),
            r#"{"version":1,"mode":"image-store","storeId":"abc123"}"#,
        )
        .unwrap();

        let config = SandboxConfig {
            store_mode: StoreMode::ImageStore,
            store_id: Some("abc123".to_string()),
            store_image_path: Some("/var/lib/cloister/images/abc123.squashfs".to_string()),
            store_mount_path: Some(mount_path.to_string_lossy().into_owned()),
            ..config_with_flags(false, false, None, None, false)
        };

        let err = ensure_image_store_mounted(&config, "test").unwrap_err();
        assert!(err.contains("is not an active mountpoint"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ensure_image_store_mounted_rejects_missing_meta() {
        let dir = temp_test_dir("cloister-image-store-missing-meta");
        let mount_path = dir.join("abc123");
        fs::create_dir_all(mount_path.join("nix/store/aaa-shell")).unwrap();

        let config = SandboxConfig {
            store_mode: StoreMode::ImageStore,
            store_id: Some("abc123".to_string()),
            store_image_path: Some("/var/lib/cloister/images/abc123.squashfs".to_string()),
            store_mount_path: Some(mount_path.to_string_lossy().into_owned()),
            ..config_with_flags(false, false, None, None, false)
        };

        let err = ensure_image_store_mounted(&config, "test").unwrap_err();
        assert!(err.contains("is unreadable") || err.contains("is not an active mountpoint"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_image_store_meta_rejects_mismatched_store_id() {
        let dir = temp_test_dir("cloister-image-store-wrong-meta");
        let meta_path = dir.join("meta.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &meta_path,
            r#"{"version":1,"mode":"image-store","storeId":"wrong"}"#,
        )
        .unwrap();

        let meta = read_image_store_meta(&meta_path, "test").unwrap();
        let err = validate_image_store_meta(&meta, &meta_path, "abc123", "test").unwrap_err();
        assert!(err.contains("store ID 'wrong' but expected 'abc123'"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_image_store_meta_rejects_unexpected_mode() {
        let meta_path = Path::new("/tmp/meta.json");
        let meta = ImageStoreMeta {
            version: 1,
            mode: "host".to_string(),
            store_id: "abc123".to_string(),
        };

        let err = validate_image_store_meta(&meta, meta_path, "abc123", "test").unwrap_err();
        assert!(err.contains("unexpected mode 'host'"));
    }

    #[test]
    fn validate_image_store_meta_rejects_unsupported_version() {
        let meta_path = Path::new("/tmp/meta.json");
        let meta = ImageStoreMeta {
            version: 2,
            mode: "image-store".to_string(),
            store_id: "abc123".to_string(),
        };

        let err = validate_image_store_meta(&meta, meta_path, "abc123", "test").unwrap_err();
        assert!(err.contains("unsupported version 2"));
    }

    #[test]
    fn read_image_store_meta_rejects_invalid_json() {
        let dir = temp_test_dir("cloister-image-store-invalid-json");
        let meta_path = dir.join("meta.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&meta_path, "not-json").unwrap();

        let err = read_image_store_meta(&meta_path, "test").unwrap_err();
        assert!(err.contains("invalid JSON"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pulse_proxy_command_rejects_missing_image_store_mount_path() {
        let mut config = config_with_flags(false, false, None, None, false);
        config.store_mode = StoreMode::ImageStore;
        config.store_id = Some("abc123".to_string());
        config.store_image_path = Some("/var/lib/cloister/images/abc123.squashfs".to_string());
        config.store_mount_path = None;

        let err = build_pulse_only_proxy_command(
            &config,
            PulseProxyCommand {
                runtime_dir: "/run/user/1000/cloister/pulse/test-1",
                pipewire_remote: "/run/user/1000/cloister/pipewire/test",
                identity: "ubuntu",
                passwd_path: None,
                group_path: None,
                pipewire_pulse_bin: "/nix/store/xxx-pipewire/bin/pipewire-pulse",
                pipewire_pulse_conf: "/nix/store/xxx-pulse.conf",
            },
        )
        .unwrap_err();

        assert!(err.contains("missing store_mount_path"));
    }

    #[test]
    fn ensure_image_store_mounted_rejects_missing_store_subdir() {
        let dir = temp_test_dir("cloister-image-store-missing-store");
        let mount_path = dir.join("abc123");
        fs::create_dir_all(&mount_path).unwrap();

        let config = SandboxConfig {
            store_mode: StoreMode::ImageStore,
            store_id: Some("abc123".to_string()),
            store_image_path: Some("/var/lib/cloister/images/abc123.squashfs".to_string()),
            store_mount_path: Some(mount_path.to_string_lossy().into_owned()),
            ..config_with_flags(false, false, None, None, false)
        };

        let err = ensure_image_store_mounted(&config, "test").unwrap_err();
        assert!(err.contains("does not contain /nix/store"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_config_only() {
        let args = s(&["cloister-sandbox", "--config", "/nix/store/xxx.json"]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.config_path.as_deref(), Some("/nix/store/xxx.json"));
        assert!(!parsed.after_netns);
        assert!(parsed.sandbox_args.is_empty());
    }

    #[test]
    fn parse_config_with_command() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "echo",
            "hello",
        ]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.config_path.as_deref(), Some("/nix/store/xxx.json"));
        assert!(!parsed.after_netns);
        assert_eq!(parsed.sandbox_args, vec!["echo", "hello"]);
    }

    #[test]
    fn parse_after_netns() {
        let args = s(&[
            "cloister-sandbox",
            "--after-netns",
            "--config",
            "/nix/store/xxx.json",
        ]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.config_path.as_deref(), Some("/nix/store/xxx.json"));
        assert!(parsed.after_netns);
        assert!(parsed.sandbox_args.is_empty());
    }

    #[test]
    fn parse_shell_flag() {
        let args = s(&[
            "cloister-sandbox",
            "--shell",
            "--config",
            "/nix/store/xxx.json",
        ]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.config_path.as_deref(), Some("/nix/store/xxx.json"));
        assert_eq!(parsed.sandbox_args, vec!["--shell"]);
    }

    #[test]
    fn parse_with_c_flag() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "-c",
            "echo",
            "hello",
        ]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.sandbox_args, vec!["-c", "echo", "hello"]);
    }

    #[test]
    fn parse_version_without_config() {
        let args = s(&["cloister-sandbox", "--version"]);
        let parsed = parse_cli_args(&args).unwrap();
        assert!(parsed.config_path.is_none());
        assert!(!parsed.after_netns);
        assert_eq!(parsed.sandbox_args, vec!["--version"]);
    }

    #[test]
    fn parse_literal_separator_preserves_passthrough_args() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "--",
            "--",
            "--version",
        ]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.sandbox_args, vec!["--", "--version"]);
    }

    #[test]
    fn parse_separator_preserves_shell_literal_arg() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "--",
            "--shell",
        ]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.sandbox_args, vec!["--shell"]);
    }

    #[test]
    fn parse_shell_before_separator_keeps_shell_flag() {
        let args = s(&[
            "cloister-sandbox",
            "--shell",
            "--config",
            "/nix/store/xxx.json",
            "--",
            "document.pdf",
        ]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.config_path.as_deref(), Some("/nix/store/xxx.json"));
        assert_eq!(parsed.sandbox_args, vec!["--shell", "document.pdf"]);
    }

    #[test]
    fn parse_control_flags_after_sandbox_args() {
        let args = s(&[
            "cloister-sandbox",
            "--shell",
            "--version",
            "--config",
            "/nix/store/xxx.json",
        ]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.config_path.as_deref(), Some("/nix/store/xxx.json"));
        assert_eq!(parsed.sandbox_args, vec!["--shell", "--version"]);
    }

    #[test]
    fn parse_rejects_duplicate_config_before_sandbox_args() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/one.json",
            "--config",
            "/nix/store/two.json",
        ]);
        let err = parse_cli_args(&args).unwrap_err();
        assert_eq!(err, "cloister-sandbox: --config may only be passed once");
    }

    #[test]
    fn parse_rejects_duplicate_after_netns_before_sandbox_args() {
        let args = s(&[
            "cloister-sandbox",
            "--after-netns",
            "--after-netns",
            "--config",
            "/nix/store/xxx.json",
        ]);
        let err = parse_cli_args(&args).unwrap_err();
        assert_eq!(
            err,
            "cloister-sandbox: --after-netns may only be passed once"
        );
    }

    #[test]
    fn parse_config_after_command_args_is_preserved() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/one.json",
            "git",
            "--config",
            "user.name=foo",
            "status",
        ]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.config_path.as_deref(), Some("/nix/store/one.json"));
        assert_eq!(
            parsed.sandbox_args,
            vec!["git", "--config", "user.name=foo", "status"]
        );
    }

    #[test]
    fn parse_rejects_missing_config_value() {
        let args = s(&["cloister-sandbox", "--config"]);
        let err = parse_cli_args(&args).unwrap_err();
        assert_eq!(err, "cloister-sandbox: --config requires a path");
    }

    #[test]
    fn broker_launcher_parse_strips_private_selector_from_child_argv() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "--broker-launch-profile",
            "ephemeral",
            "--broker-launch-sandbox",
            "worker",
            "--",
            "git",
            "status",
        ]);
        let parsed = parse_cli_args(&args).unwrap();

        assert_eq!(parsed.sandbox_args, vec!["git", "status"]);
    }

    #[test]
    fn broker_launcher_parse_rejects_missing_profile_value() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "--broker-launch-profile",
        ]);
        let err = parse_cli_args(&args).unwrap_err();

        assert_eq!(
            err,
            "cloister-sandbox: --broker-launch-profile requires a profile"
        );
    }

    #[test]
    fn broker_launcher_parse_rejects_missing_sandbox_value() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "--broker-launch-sandbox",
        ]);
        let err = parse_cli_args(&args).unwrap_err();

        assert_eq!(
            err,
            "cloister-sandbox: --broker-launch-sandbox requires a sandbox"
        );
    }

    #[test]
    fn broker_launcher_parse_accepts_trusted_record_override() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "--broker-launch-profile",
            "ephemeral",
            "--broker-launch-sandbox",
            "worker",
            "--broker-trusted-record",
            "/tmp/session.json",
            "--",
            "git",
            "status",
        ]);
        let parsed = parse_cli_args(&args).unwrap();

        assert_eq!(
            parsed.broker_trusted_record_path.as_deref(),
            Some("/tmp/session.json")
        );
        assert_eq!(parsed.sandbox_args, vec!["git", "status"]);
    }

    #[test]
    fn broker_launcher_parse_rejects_missing_trusted_record_value() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "--broker-trusted-record",
        ]);
        let err = parse_cli_args(&args).unwrap_err();

        assert_eq!(
            err,
            "cloister-sandbox: --broker-trusted-record requires a path"
        );
    }

    #[test]
    fn broker_launcher_parse_rejects_incomplete_selector() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "--broker-launch-profile",
            "ephemeral",
            "--",
            "git",
            "status",
        ]);
        let err = parse_cli_args(&args).unwrap_err();

        assert_eq!(
            err,
            "cloister-sandbox: --broker-launch-profile and --broker-launch-sandbox must be passed together"
        );
    }

    #[test]
    fn broker_launcher_prepare_run_cmd_rejects_empty_command_argv() {
        let config = config_with_flags(false, false, None, None, false);
        let selector = BrokerLaunchSelector {
            profile: "overlay".to_string(),
            sandbox: "worker".to_string(),
        };
        let result = prepare_run_cmd(&config, &[], Some(&selector));

        assert_eq!(result, Err("broker launcher requires a command argv"),);
    }

    #[test]
    fn broker_launcher_prepare_run_cmd_rejects_c_shorthand() {
        let config = config_with_flags(false, false, None, None, false);
        let selector = BrokerLaunchSelector {
            profile: "overlay".to_string(),
            sandbox: "worker".to_string(),
        };
        let result = prepare_run_cmd(&config, &s(&["-c", "echo", "hello"]), Some(&selector));

        assert_eq!(
            result,
            Err("broker launcher does not support -c; pass the command argv directly"),
        );
    }

    #[test]
    fn prepare_run_cmd_rejects_bare_c_flag() {
        let config = config_with_flags(false, false, None, None, false);
        let result = prepare_run_cmd(&config, &s(&["-c"]), None);
        assert_eq!(result, Err("`-c` requires a command"));
    }

    #[test]
    fn prepare_run_cmd_shell_override_ignores_default_command() {
        let config: SandboxConfig = serde_json::from_value(serde_json::json!({
            "name": "test",
            "bwrap_path": "/nix/store/xxx-bubblewrap/bin/bwrap",
            "shell_bin": "/nix/store/xxx-zsh/bin/zsh",
            "shell_interactive_args": ["-i"],
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/user",
            "store_mode": "host",
            "store_roots": ["/nix/store/aaa-shell"],
            "per_dir": {},
            "copy_file_base": "/home/user/.local/state/cloister",
            "git_path": "/nix/store/xxx-git/bin/git",
            "default_command": ["firefox"]
        }))
        .unwrap();

        let (run_cmd, interactive) = prepare_run_cmd(&config, &s(&["--shell"]), None).unwrap();
        assert_eq!(run_cmd, vec!["/nix/store/xxx-zsh/bin/zsh", "-i"]);
        assert!(interactive);
    }

    #[test]
    fn xdg_runtime_dir_required_when_feature_enabled() {
        let config = config_with_flags(
            false,
            false,
            Some("cloister/pipewire/test".to_string()),
            None,
            false,
        );
        let result = validate_xdg_runtime_dir(&config, "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains(
            "XDG_RUNTIME_DIR must be set when using D-Bus, GUI, audio, or worker broker features."
        ));
    }

    #[test]
    fn xdg_runtime_dir_not_required_when_features_disabled() {
        let config = config_with_flags(false, false, None, None, false);
        let result = validate_xdg_runtime_dir(&config, "");
        assert!(result.is_ok());
    }

    #[test]
    fn xdg_runtime_dir_present_satisfies_requirement() {
        let config = config_with_flags(false, true, None, None, false);
        let result = validate_xdg_runtime_dir(&config, "/run/user/1000");
        assert!(result.is_ok());
    }

    #[test]
    fn xdg_runtime_dir_required_when_pulse_only_bridge_enabled() {
        let config = config_with_flags(
            false,
            false,
            None,
            Some("/nix/store/xxx-pulse.conf".to_string()),
            false,
        );
        let result = validate_xdg_runtime_dir(&config, "");
        assert!(result.is_err());
    }

    #[test]
    fn xdg_runtime_dir_required_when_worker_broker_enabled() {
        let config = config_with_worker_broker();

        let result = validate_xdg_runtime_dir(&config, "");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("XDG_RUNTIME_DIR must be set"));
    }

    #[test]
    fn ssh_filter_socket_path_uses_tmp_fallback_without_runtime_dir() {
        assert_eq!(
            ssh_filter_socket_path("", 4242),
            "/tmp/cloister-ssh-filter-4242/agent.sock"
        );
    }

    #[test]
    fn ssh_filter_socket_path_prefers_runtime_dir_when_present() {
        assert_eq!(
            ssh_filter_socket_path("/run/user/1000", 4242),
            "/run/user/1000/cloister-ssh-filter-4242"
        );
    }

    /// Reset signal-handler statics so tests don't interfere with each other.
    fn reset_signal_state() {
        clear_signal_targets();
        SIGINT_COUNT.store(0, Ordering::Release);
        LAST_SIGINT_SEC.store(0, Ordering::Release);
        INTERACTIVE_MODE.store(false, Ordering::Release);
        unsafe {
            libc::alarm(0);
        }
    }

    #[test]
    fn child_pid_initially_zero() {
        let _guard = signal_lock().lock().unwrap();
        assert_eq!(CHILD_PID.load(Ordering::Acquire), 0);
    }

    #[test]
    fn forward_signal_noop_when_no_child() {
        let _guard = signal_lock().lock().unwrap();
        reset_signal_state();
        // Should not panic or error when CHILD_PID is 0
        forward_signal(libc::SIGTERM);
        forward_signal(libc::SIGINT);
        forward_signal(libc::SIGALRM);
    }

    #[test]
    fn forward_signal_handles_nonexistent_pid() {
        let _guard = signal_lock().lock().unwrap();
        reset_signal_state();
        // Use a PID that almost certainly doesn't exist
        CHILD_PID.store(i32::MAX, Ordering::Release);
        forward_signal(libc::SIGTERM); // kill returns ESRCH, but we don't panic
        reset_signal_state();
    }

    #[test]
    fn clear_signal_targets_blocking_signals_resets_pids_and_alarm() {
        let _guard = signal_lock().lock().unwrap();
        reset_signal_state();
        CHILD_PID.store(123, Ordering::Release);
        GRACEFUL_PID.store(456, Ordering::Release);
        FORCE_PID.store(789, Ordering::Release);
        unsafe {
            libc::alarm(1);
        }

        clear_signal_targets_blocking_signals();

        assert_eq!(CHILD_PID.load(Ordering::Acquire), 0);
        assert_eq!(GRACEFUL_PID.load(Ordering::Acquire), 0);
        assert_eq!(FORCE_PID.load(Ordering::Acquire), 0);
        let remaining = unsafe { libc::alarm(0) };
        assert_eq!(remaining, 0);
        reset_signal_state();
    }

    #[test]
    fn sigint_escalation_rapid() {
        let _guard = signal_lock().lock().unwrap();
        reset_signal_state();
        CHILD_PID.store(i32::MAX, Ordering::Release);

        // First SIGINT: count should be 1
        forward_signal(libc::SIGINT);
        assert_eq!(SIGINT_COUNT.load(Ordering::Acquire), 1);

        // Second SIGINT (immediate — within escalation window): count should be 2
        forward_signal(libc::SIGINT);
        assert_eq!(SIGINT_COUNT.load(Ordering::Acquire), 2);

        // Third SIGINT: count should be 3
        forward_signal(libc::SIGINT);
        assert_eq!(SIGINT_COUNT.load(Ordering::Acquire), 3);

        reset_signal_state();
    }

    #[test]
    fn sigint_resets_after_window() {
        let _guard = signal_lock().lock().unwrap();
        reset_signal_state();
        CHILD_PID.store(i32::MAX, Ordering::Release);

        // First SIGINT
        forward_signal(libc::SIGINT);
        assert_eq!(SIGINT_COUNT.load(Ordering::Acquire), 1);

        // Simulate time passing beyond the escalation window
        let past = monotonic_seconds() - SIGINT_ESCALATION_WINDOW_SECS - 1;
        LAST_SIGINT_SEC.store(past, Ordering::Release);

        // Next SIGINT should reset to 1
        forward_signal(libc::SIGINT);
        assert_eq!(SIGINT_COUNT.load(Ordering::Acquire), 1);

        reset_signal_state();
    }

    #[test]
    fn parse_with_separator() {
        let args = s(&[
            "cloister-sandbox",
            "--config",
            "/nix/store/xxx.json",
            "--after-netns",
            "--",
            "-c",
            "echo",
        ]);
        let parsed = parse_cli_args(&args).unwrap();
        assert_eq!(parsed.config_path.as_deref(), Some("/nix/store/xxx.json"));
        assert!(parsed.after_netns);
        assert_eq!(parsed.sandbox_args, vec!["-c", "echo"]);
    }

    #[test]
    fn interactive_mode_skips_first_sigint() {
        let _guard = signal_lock().lock().unwrap();
        reset_signal_state();
        CHILD_PID.store(i32::MAX, Ordering::Release);
        INTERACTIVE_MODE.store(true, Ordering::Release);

        // First SIGINT in interactive mode: count increments but no kill is sent
        // (the shell gets SIGINT from the terminal directly).
        forward_signal(libc::SIGINT);
        assert_eq!(SIGINT_COUNT.load(Ordering::Acquire), 1);

        // Second rapid SIGINT: escalates to SIGTERM (same as non-interactive)
        forward_signal(libc::SIGINT);
        assert_eq!(SIGINT_COUNT.load(Ordering::Acquire), 2);

        // Third rapid SIGINT: escalates to SIGKILL
        forward_signal(libc::SIGINT);
        assert_eq!(SIGINT_COUNT.load(Ordering::Acquire), 3);

        reset_signal_state();
    }

    #[test]
    fn non_interactive_forwards_first_sigint() {
        let _guard = signal_lock().lock().unwrap();
        reset_signal_state();
        CHILD_PID.store(i32::MAX, Ordering::Release);
        INTERACTIVE_MODE.store(false, Ordering::Release);

        // First SIGINT in non-interactive mode: forwarded (kill returns ESRCH
        // for i32::MAX, but we don't panic — the important thing is the path
        // through the match arm is exercised).
        forward_signal(libc::SIGINT);
        assert_eq!(SIGINT_COUNT.load(Ordering::Acquire), 1);

        reset_signal_state();
    }

    #[test]
    fn signal_target_uses_process_group_for_noninteractive_children() {
        assert_eq!(force_signal_target(1234, false), -1234);
    }

    #[test]
    fn signal_target_uses_direct_pid_for_interactive_children() {
        assert_eq!(force_signal_target(1234, true), 1234);
    }

    #[test]
    fn graceful_signal_target_uses_direct_pid() {
        assert_eq!(graceful_signal_target(1234), 1234);
    }

    #[test]
    fn find_descendant_pid_returns_matching_grandchild() {
        let found = find_descendant_pid(
            100,
            |pid| match pid {
                100 => Ok(vec![200]),
                200 => Ok(vec![300]),
                300 => Ok(Vec::new()),
                other => Err(format!("unexpected pid {other}")),
            },
            |pid| Ok(pid == 300),
        )
        .unwrap();

        assert_eq!(found, 300);
    }

    #[test]
    fn parse_proc_nspid_reads_namespace_chain() {
        assert_eq!(
            parse_proc_nspid("Name:\tbwrap\nNSpid:\t1234 1\nState:\tS\n").unwrap(),
            vec![1234, 1]
        );
    }

    #[test]
    fn is_sandbox_pid1_nspid_requires_expected_depth_and_inner_pid_one() {
        assert!(is_sandbox_pid1_nspid(&[200, 300, 1], 3));
        assert!(!is_sandbox_pid1_nspid(&[200, 1], 3));
        assert!(!is_sandbox_pid1_nspid(&[200, 300, 2], 3));
    }

    #[test]
    fn sandbox_signal_pid_prefers_child_of_sandbox_pid1_namespace() {
        let signal_pids = sandbox_signal_pid_with_reader(
            100,
            |pid| match pid {
                100 => Ok(vec![200]),
                200 => Ok(vec![310, 300]),
                301 => Ok(Vec::new()),
                310 => Ok(vec![311]),
                311 => Ok(Vec::new()),
                300 => Ok(vec![301]),
                other => Err(format!("unexpected pid {other}")),
            },
            |pid| match pid {
                100 => Ok(vec![100]),
                200 => Ok(vec![200]),
                300 => Ok(vec![300, 1]),
                301 => Ok(vec![301, 2]),
                310 => Ok(vec![310, 99]),
                311 => Ok(vec![311, 5]),
                other => Err(format!("unexpected pid {other}")),
            },
        )
        .unwrap();
        assert_eq!(signal_pids, (301, 300));
    }

    #[test]
    fn sandbox_signal_pid_skips_wrapper_helpers_before_main_process() {
        let signal_pids = sandbox_signal_pid_with_reader(
            100,
            |pid| match pid {
                100 => Ok(vec![200]),
                200 => Ok(vec![300]),
                300 => Ok(vec![320, 310]),
                310 => Ok(Vec::new()),
                320 => Ok(vec![321]),
                321 => Ok(Vec::new()),
                other => Err(format!("unexpected pid {other}")),
            },
            |pid| match pid {
                100 => Ok(vec![100]),
                200 => Ok(vec![200]),
                300 => Ok(vec![300, 1]),
                310 => Ok(vec![310, 2]),
                320 => Ok(vec![320, 2]),
                321 => Ok(vec![321, 3]),
                other => Err(format!("unexpected pid {other}")),
            },
        )
        .unwrap();

        assert_eq!(signal_pids, (320, 300));
    }
}
