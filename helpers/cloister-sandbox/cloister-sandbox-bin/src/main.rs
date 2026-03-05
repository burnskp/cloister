use std::io;
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::process::{self, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};

use cloister_sandbox_lib::bwrap;
use cloister_sandbox_lib::config::SandboxConfig;
use cloister_sandbox_lib::env;
use cloister_sandbox_lib::features;
use cloister_sandbox_lib::runtime;
use cloister_sandbox_lib::seccomp;
use cloister_sandbox_lib::socket;
use cloister_sandbox_lib::ssh_filter;
use cloister_sandbox_lib::validate;
use cloister_sandbox_lib::wayland;

/// PID of the active child process. 0 means no child is running.
static CHILD_PID: AtomicI32 = AtomicI32::new(0);

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
    let pid = CHILD_PID.load(Ordering::Acquire);
    if pid <= 0 {
        return;
    }

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
                    if !INTERACTIVE_MODE.load(Ordering::Acquire) {
                        unsafe {
                            libc::kill(pid, libc::SIGINT);
                        }
                    }
                    // In interactive mode the shell gets SIGINT from the terminal directly.
                }
                2 => {
                    signal_write_stderr(
                        b"\ncloister-sandbox: requesting sandbox shutdown (Ctrl-C again to force)...\n",
                    );
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                }
                _ => {
                    signal_write_stderr(b"\ncloister-sandbox: force-killing sandbox.\n");
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
            }
        }
        libc::SIGTERM => unsafe {
            libc::kill(pid, libc::SIGTERM);
            libc::alarm(SIGTERM_GRACE_SECS);
        },
        libc::SIGALRM => {
            signal_write_stderr(
                b"cloister-sandbox: graceful shutdown timed out, force-killing sandbox.\n",
            );
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        _ => unsafe {
            libc::kill(pid, sig);
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
fn spawn_and_wait(cmd: &mut process::Command, interactive: bool) -> io::Result<ExitStatus> {
    // Reset escalation state for new child
    SIGINT_COUNT.store(0, Ordering::Release);
    LAST_SIGINT_SEC.store(0, Ordering::Release);

    // Block forwarded signals before spawn so none are lost in the race window
    let mut block_set: libc::sigset_t = unsafe { std::mem::zeroed() };
    let mut old_set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut block_set);
        libc::sigaddset(&mut block_set, libc::SIGTERM);
        libc::sigaddset(&mut block_set, libc::SIGINT);
        libc::sigaddset(&mut block_set, libc::SIGHUP);
        libc::sigaddset(&mut block_set, libc::SIGALRM);
        libc::sigprocmask(libc::SIG_BLOCK, &block_set, &mut old_set);
    }

    if interactive {
        // Safety: only calls async-signal-safe functions (sigaction, sigemptyset,
        // sigprocmask). Runs between fork and exec in the child process.
        unsafe {
            cmd.pre_exec(|| {
                // Ignore SIGINT in bwrap's outer process so Ctrl-C doesn't kill it.
                libc::signal(libc::SIGINT, libc::SIG_IGN);

                // Reset signal mask so the shell starts with no blocked signals.
                let mut empty_set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut empty_set);
                libc::sigprocmask(libc::SIG_SETMASK, &empty_set, std::ptr::null_mut());

                Ok(())
            });
        }
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

    // Restore old signal mask — any pending signals are delivered now
    unsafe {
        libc::sigprocmask(libc::SIG_SETMASK, &old_set, std::ptr::null_mut());
    }

    let status = child.wait();
    CHILD_PID.store(0, Ordering::Release);
    // Cancel any pending alarm (e.g. from SIGTERM grace period)
    unsafe {
        libc::alarm(0);
    }
    status
}

fn main() {
    let exit_code = run();
    process::exit(exit_code);
}

fn err_prefix(name: &str) -> String {
    format!("cloister-sandbox[{name}]")
}

fn run() -> i32 {
    // --- 1. Parse CLI args ---
    let args: Vec<String> = std::env::args().collect();
    let (config_path, after_netns, sandbox_args) = parse_cli_args(&args);

    let config_path = config_path.unwrap_or_else(|| {
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

    let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    if let Err(e) = validate_xdg_runtime_dir(&config, &xdg_runtime_dir) {
        eprintln!("{prefix}: {e}");
        process::exit(1);
    }

    // --- 2b. Install signal handlers so SIGTERM/SIGINT/SIGHUP forward to children ---
    install_signal_handlers();

    // --- 3. Netns re-exec ---
    if let Some(ref netns_helper) = config.netns_helper_path {
        if let (Some(ns), false) = (&config.network_namespace, after_netns) {
            // Re-exec through netns helper: netns_helper --netns <name> -- <self> --after-netns --config <path> [original args]
            let self_exe = std::env::current_exe().unwrap_or_else(|e| {
                eprintln!("{prefix}: cannot determine self path: {e}");
                process::exit(1);
            });

            let mut cmd = process::Command::new(netns_helper);
            cmd.args(["--netns", ns, "--"]);
            cmd.arg(&self_exe);
            cmd.args(["--after-netns", "--config", &config_path]);
            cmd.args(&sandbox_args);

            let status = spawn_and_wait(&mut cmd, false).unwrap_or_else(|e| {
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
        let dir_hash = if !config.per_dir_paths.is_empty() {
            if let Err(e) = runtime::validate_per_dir_base(&config.per_dir_base) {
                eprintln!("{prefix}: {e}");
                process::exit(1);
            }
            let hash = runtime::compute_dir_hash(&sandbox_dir);

            // Create per-dir directories
            let per_dir_mkdirs: Vec<String> = config
                .per_dir_paths
                .iter()
                .map(|p| format!("{}/{}/{}", config.per_dir_base, hash, p))
                .collect();
            if let Err(e) = runtime::ensure_dirs(&per_dir_mkdirs) {
                eprintln!("{prefix}: {e}");
                process::exit(1);
            }

            // Update manifest
            let manifest_path = format!("{}/manifest.json", config.per_dir_base);
            if let Err(e) = runtime::update_manifest(&manifest_path, &hash, &sandbox_dir) {
                eprintln!("{prefix}: {e}");
                process::exit(1);
            }

            hash
        } else {
            String::new()
        };

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

    // Copy files
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
        &xdg_runtime_dir,
    );

    if let Some(ref socket_name) = config.dbus_proxy_socket_name {
        runtime_vars.insert(
            "DBUS_PROXY_SOCKET".to_string(),
            format!("{xdg_runtime_dir}/{socket_name}"),
        );
    }

    if config.dangerous_path_warnings {
        if let Err(e) = validate::validate_dangerous_binds(
            &config.bind_sources,
            &runtime_vars,
            &configured_home_resolved,
            &config.dangerous_paths,
            &config.allow_dangerous_paths,
        ) {
            eprintln!("{prefix}: {e}");
            process::exit(1);
        }
    }

    let mut extra_args = Vec::new();

    // Passthrough env
    extra_args.extend(bwrap::passthrough_env_args(&config.passthrough_env));

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
                            format!("{xdg_runtime_dir}/cloister-ssh-filter-{}", process::id());
                        match ssh_filter::start_listener(
                            &filter_socket,
                            &auth_sock,
                            config.ssh_allow_fingerprints.clone(),
                            config.ssh_filter_timeout_seconds,
                        ) {
                            Ok(handle) => {
                                extra_args.extend([
                                    "--bind".to_string(),
                                    filter_socket.clone(),
                                    filter_socket.clone(),
                                    "--setenv".to_string(),
                                    "SSH_AUTH_SOCK".to_string(),
                                    filter_socket,
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
                    extra_args.extend([
                        "--bind".to_string(),
                        auth_sock.clone(),
                        auth_sock.clone(),
                        "--setenv".to_string(),
                        "SSH_AUTH_SOCK".to_string(),
                        auth_sock,
                    ]);
                }
            }
        }
        ssh_filter_handle = None;
    } else {
        ssh_filter_handle = None;
    }

    // PulseAudio
    if config.pulseaudio_enable {
        extra_args.extend(features::pulseaudio_args(&xdg_runtime_dir));
    }

    // PipeWire
    if config.pipewire_enable {
        extra_args.extend(features::pipewire_args(&xdg_runtime_dir));
    }

    // Wayland
    let _wayland_keep_alive;
    let wayland_socket_path;
    if config.wayland_enable {
        if std::env::var("WAYLAND_DISPLAY")
            .map(|d| !d.is_empty())
            .unwrap_or(false)
        {
            if config.wayland_security_context {
                let socket = format!("{xdg_runtime_dir}/cloister-wayland-{}", process::id());
                if !wayland::probe() {
                    eprintln!("{prefix}: compositor does not support wp-security-context-v1.");
                    eprintln!(
                        "Either use a supported compositor (sway 1.9+, Hyprland, niri, labwc 0.8.2+)"
                    );
                    eprintln!(
                        "or set gui.wayland.securityContext.enable = false for raw socket passthrough."
                    );
                    process::exit(1);
                }
                let app_id = format!("cloister-{}", config.name);
                match wayland::setup_context(&socket, "cloister", &app_id) {
                    Ok(fd) => {
                        extra_args.extend([
                            "--ro-bind".to_string(),
                            socket.clone(),
                            format!("{xdg_runtime_dir}/wayland-ds"),
                            "--setenv".to_string(),
                            "WAYLAND_DISPLAY".to_string(),
                            "wayland-ds".to_string(),
                        ]);
                        _wayland_keep_alive = Some(fd);
                        wayland_socket_path = Some(socket);
                    }
                    Err(e) => {
                        eprintln!("{prefix}: wayland setup: {e}");
                        process::exit(1);
                    }
                }
            } else {
                extra_args.extend(features::wayland_raw_args(&xdg_runtime_dir));
                _wayland_keep_alive = None;
                wayland_socket_path = None;
            }
        } else {
            _wayland_keep_alive = None;
            wayland_socket_path = None;
        }
    } else {
        _wayland_keep_alive = None;
        wayland_socket_path = None;
    }

    // X11
    if config.x11_enable {
        extra_args.extend(features::x11_args(&config.sandbox_home));
    }

    // GPU
    if config.gpu_enable {
        extra_args.extend(features::gpu_args(config.gpu_shm));
    }

    // FIDO2
    if config.fido2_enable {
        extra_args.extend(features::fido2_args());
    }

    // Video/Camera
    if config.video_enable {
        extra_args.extend(features::video_args());
    }

    // Printing
    if config.printing_enable {
        extra_args.extend(features::printing_args());
    }

    // Device binds
    extra_args.extend(features::dev_bind_args(&config.dev_binds));

    // Anonymized identity (synthetic /etc/passwd + /etc/group with real UID/GID)
    let (anon_passwd_path, anon_group_path);
    if config.anonymize {
        let (anon_args, passwd, group) =
            features::anonymize_identity_args(&config.shell_bin, &config.sandbox_home);
        extra_args.extend(anon_args);
        anon_passwd_path = passwd;
        anon_group_path = group;
    } else {
        anon_passwd_path = None;
        anon_group_path = None;
    }

    // Machine ID (random per invocation — avoids host fingerprinting)
    let (machine_id_bwrap_args, machine_id_path) = features::machine_id_args();
    extra_args.extend(machine_id_bwrap_args);

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
                    process::exit(1);
                }
            }
        } else {
            _seccomp_file = None;
        }
    } else {
        _seccomp_file = None;
    }

    // D-Bus
    if config.dbus_enable {
        if let Some(ref socket_name) = config.dbus_proxy_socket_name {
            features::check_dbus_socket(&xdg_runtime_dir, socket_name);
        }
        if !xdg_runtime_dir.is_empty() {
            extra_args.extend([
                "--setenv".to_string(),
                "DBUS_SESSION_BUS_ADDRESS".to_string(),
                format!("unix:path={xdg_runtime_dir}/bus"),
            ]);
        }
    }

    // --- 10. Parse command args and build run_cmd ---
    let command_args = env::parse_sandbox_args(&sandbox_args);
    let run_cmd = env::build_run_cmd(
        &config.shell_bin,
        &config.shell_interactive_args,
        config.default_command.as_deref(),
        &command_args,
    );

    // --- 11. Spawn bwrap ---
    let is_interactive = env::is_interactive(&command_args, config.default_command.as_deref());
    INTERACTIVE_MODE.store(is_interactive, Ordering::Release);

    let (mut cmd, _args_fd) = match bwrap::build_bwrap_command(
        &config,
        &runtime_vars,
        extra_args,
        &run_cmd,
        &effective_start_dir,
        is_interactive,
    ) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{prefix}: args pipe setup: {e}");
            cleanup(
                ssh_filter_handle,
                wayland_socket_path,
                machine_id_path,
                anon_passwd_path,
                anon_group_path,
            );
            process::exit(1);
        }
    };

    let status = match spawn_and_wait(&mut cmd, is_interactive) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{prefix}: exec bwrap: {e}");
            cleanup(
                ssh_filter_handle,
                wayland_socket_path,
                machine_id_path,
                anon_passwd_path,
                anon_group_path,
            );
            return 127;
        }
    };

    // --- 12. Cleanup ---
    cleanup(
        ssh_filter_handle,
        wayland_socket_path,
        machine_id_path,
        anon_passwd_path,
        anon_group_path,
    );

    // --- 13. Exit with bwrap's exit code ---
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

fn cleanup(
    ssh_handle: Option<ssh_filter::SshFilterHandle>,
    wayland_socket: Option<String>,
    machine_id_path: Option<String>,
    anon_passwd_path: Option<String>,
    anon_group_path: Option<String>,
) {
    // SSH filter cleanup (SshFilterHandle::drop handles this)
    drop(ssh_handle);

    // Wayland socket cleanup
    if let Some(path) = wayland_socket {
        let _ = cloister_sandbox_lib::socket::remove_stale_socket(&path);
    }

    // Machine-id temp file cleanup
    if let Some(path) = machine_id_path {
        let _ = std::fs::remove_file(&path);
    }

    // Anonymization temp file cleanup
    if let Some(path) = anon_passwd_path {
        let _ = std::fs::remove_file(&path);
    }
    if let Some(path) = anon_group_path {
        let _ = std::fs::remove_file(&path);
    }
}

fn requires_xdg_runtime_dir(config: &SandboxConfig) -> bool {
    config.dbus_enable
        || config.wayland_enable
        || config.pulseaudio_enable
        || config.pipewire_enable
        || config.ssh_enable
}

fn validate_xdg_runtime_dir(config: &SandboxConfig, xdg_runtime_dir: &str) -> Result<(), String> {
    if requires_xdg_runtime_dir(config) && xdg_runtime_dir.is_empty() {
        return Err(
            "XDG_RUNTIME_DIR must be set when using D-Bus, Wayland, audio, or SSH features."
                .to_string(),
        );
    }
    Ok(())
}

/// Parse --config, --after-netns, and remaining sandbox arguments.
fn parse_cli_args(args: &[String]) -> (Option<String>, bool, Vec<String>) {
    let mut config_path = None;
    let mut after_netns = false;
    let mut sandbox_args = Vec::new();
    let mut i = 1;
    let mut past_separator = false;

    while i < args.len() {
        if past_separator {
            sandbox_args.push(args[i].clone());
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "--config" => {
                i += 1;
                if i < args.len() {
                    config_path = Some(args[i].clone());
                }
            }
            "--after-netns" => {
                after_netns = true;
            }
            "--" => {
                past_separator = true;
            }
            _ => {
                // Everything else is a sandbox arg (no -- separator needed for simple usage)
                sandbox_args.extend_from_slice(&args[i..]);
                break;
            }
        }
        i += 1;
    }

    (config_path, after_netns, sandbox_args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// Serialize tests that touch shared signal statics (CHILD_PID, SIGINT_COUNT, etc.).
    fn signal_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn config_with_flags(
        dbus_enable: bool,
        wayland_enable: bool,
        pulseaudio_enable: bool,
        pipewire_enable: bool,
        ssh_enable: bool,
    ) -> SandboxConfig {
        serde_json::from_value(serde_json::json!({
            "name": "test",
            "bwrap_path": "/nix/store/xxx-bubblewrap/bin/bwrap",
            "shell_bin": "/nix/store/xxx-zsh/bin/zsh",
            "shell_interactive_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/user",
            "per_dir_base": "/home/user/.local/state/cloister",
            "copy_file_base": "/home/user/.local/state/cloister",
            "git_path": "/nix/store/xxx-git/bin/git",
            "dbus_enable": dbus_enable,
            "wayland_enable": wayland_enable,
            "pulseaudio_enable": pulseaudio_enable,
            "pipewire_enable": pipewire_enable,
            "ssh_enable": ssh_enable
        }))
        .expect("valid config")
    }

    #[test]
    fn parse_config_only() {
        let args = s(&["cloister-sandbox", "--config", "/nix/store/xxx.json"]);
        let (config, after_netns, sandbox_args) = parse_cli_args(&args);
        assert_eq!(config.as_deref(), Some("/nix/store/xxx.json"));
        assert!(!after_netns);
        assert!(sandbox_args.is_empty());
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
        let (config, after_netns, sandbox_args) = parse_cli_args(&args);
        assert_eq!(config.as_deref(), Some("/nix/store/xxx.json"));
        assert!(!after_netns);
        assert_eq!(sandbox_args, vec!["echo", "hello"]);
    }

    #[test]
    fn parse_after_netns() {
        let args = s(&[
            "cloister-sandbox",
            "--after-netns",
            "--config",
            "/nix/store/xxx.json",
        ]);
        let (config, after_netns, sandbox_args) = parse_cli_args(&args);
        assert_eq!(config.as_deref(), Some("/nix/store/xxx.json"));
        assert!(after_netns);
        assert!(sandbox_args.is_empty());
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
        let (_, _, sandbox_args) = parse_cli_args(&args);
        assert_eq!(sandbox_args, vec!["-c", "echo", "hello"]);
    }

    #[test]
    fn xdg_runtime_dir_required_when_feature_enabled() {
        let config = config_with_flags(true, false, false, false, false);
        let result = validate_xdg_runtime_dir(&config, "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains(
            "XDG_RUNTIME_DIR must be set when using D-Bus, Wayland, audio, or SSH features."
        ));
    }

    #[test]
    fn xdg_runtime_dir_not_required_when_features_disabled() {
        let config = config_with_flags(false, false, false, false, false);
        let result = validate_xdg_runtime_dir(&config, "");
        assert!(result.is_ok());
    }

    #[test]
    fn xdg_runtime_dir_present_satisfies_requirement() {
        let config = config_with_flags(false, true, false, false, false);
        let result = validate_xdg_runtime_dir(&config, "/run/user/1000");
        assert!(result.is_ok());
    }

    /// Reset signal-handler statics so tests don't interfere with each other.
    fn reset_signal_state() {
        CHILD_PID.store(0, Ordering::Release);
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
        let (config, after_netns, sandbox_args) = parse_cli_args(&args);
        assert_eq!(config.as_deref(), Some("/nix/store/xxx.json"));
        assert!(after_netns);
        assert_eq!(sandbox_args, vec!["-c", "echo"]);
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
}
