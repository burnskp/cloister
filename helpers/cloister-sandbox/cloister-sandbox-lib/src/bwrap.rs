//! Bubblewrap argument construction.
//!
//! Assembles the full bwrap command-line from static config args,
//! runtime-resolved dynamic binds, and conditional feature args.
//!
//! Arguments are passed to bwrap via `--args FD` (NUL-separated on a pipe)
//! to keep `ps` output clean and avoid ARG_MAX limits.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use std::process::Command;

use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl};
use nix::unistd::pipe2;

use crate::config::{BindMode, SandboxConfig};
use crate::vars;

/// Build the complete bwrap Command from config and runtime state.
///
/// Returns the Command and an `OwnedFd` for the pipe read end that carries
/// NUL-separated arguments via `--args FD`. The caller must keep the
/// `OwnedFd` alive until `cmd.status()` returns.
pub fn build_bwrap_command(
    config: &SandboxConfig,
    runtime_vars: &HashMap<String, String>,
    extra_args: Vec<String>,
    run_cmd: &[String],
    start_dir: &str,
    interactive: bool,
) -> io::Result<(Command, OwnedFd)> {
    // --- Collect all bwrap namespace/mount options ---
    let mut args: Vec<String> = Vec::new();

    // Core isolation flags
    args.push("--die-with-parent".into());
    if !interactive {
        args.push("--new-session".into());
    }
    args.push("--unshare-all".into());

    // Network
    if config.network_enable {
        args.push("--share-net".into());
    }

    // Anonymization: hostname + uid/gid override
    if config.anonymize {
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        args.extend([
            "--hostname".to_string(),
            "ubuntu".to_string(),
            "--uid".to_string(),
            uid.to_string(),
            "--gid".to_string(),
            gid.to_string(),
        ]);
    }

    // Clear environment
    args.push("--clearenv".into());

    // Seccomp (added by caller via extra_args since it requires FD management)

    // /proc and /dev
    args.extend(["--proc", "/proc", "--dev", "/dev"].map(String::from));

    // XDG_RUNTIME_DIR
    if let Some(runtime_dir) = runtime_vars.get("XDG_RUNTIME_DIR") {
        if !runtime_dir.is_empty() {
            args.push("--dir".into());
            args.push(runtime_dir.clone());
        }
    }

    // If /etc/netns/<ns>/hosts is missing, preserve host /etc/hosts as a
    // fallback. The later static --ro-bind-try for /etc/netns/<ns>/hosts will
    // override this when present.
    args.extend(netns_hosts_fallback_args(config));

    // Static bwrap args (pre-computed by Nix: dirs, tmpfs, symlinks, store-path binds, env)
    args.extend(config.static_bwrap_args.iter().cloned());

    // Dynamic binds: resolve runtime variables and add
    for bind in &config.dynamic_binds {
        let src = vars::expand_vars(&bind.src, runtime_vars);
        let dest = bind
            .dest
            .as_ref()
            .map(|d| vars::expand_vars(d, runtime_vars))
            .unwrap_or_else(|| src.clone());

        let flag = bind_flag(bind.mode, bind.try_bind);
        args.push(flag.into());
        args.push(src);
        args.push(dest);
    }

    // Extra args (passthrough env, zsh, ssh, pulse, wayland, gpu, dev, seccomp)
    args.extend(extra_args);

    // --- Write args NUL-separated to a pipe ---
    let (read_fd, write_fd) =
        pipe2(OFlag::O_CLOEXEC).map_err(|e| io::Error::other(format!("pipe2: {e}")))?;

    // Clear FD_CLOEXEC on read end so bwrap inherits it
    let raw_read = read_fd.as_raw_fd();
    let mut flags = fcntl(raw_read, FcntlArg::F_GETFD)
        .map(FdFlag::from_bits_truncate)
        .map_err(|e| io::Error::other(format!("fcntl getfd: {e}")))?;
    flags.remove(FdFlag::FD_CLOEXEC);
    fcntl(raw_read, FcntlArg::F_SETFD(flags))
        .map_err(|e| io::Error::other(format!("fcntl setfd: {e}")))?;

    // Serialize arguments as NUL-terminated strings
    let mut buf = Vec::new();
    for arg in &args {
        buf.extend(arg.as_bytes());
        buf.push(0);
    }

    // Write all at once, then close write end to signal EOF to bwrap
    let mut write_file = File::from(write_fd);
    write_file.write_all(&buf)?;
    drop(write_file);

    // --- Build the real command line (visible in ps) ---
    let mut cmd = Command::new(&config.bwrap_path);
    cmd.args(["--args", &raw_read.to_string()]);

    // --chdir stays on the real command line for debuggability
    cmd.args(["--chdir", start_dir]);

    // Terminate bwrap option processing so app flags aren't misinterpreted
    cmd.arg("--");

    // The command to run inside the sandbox
    cmd.args(run_cmd);

    Ok((cmd, read_fd))
}

fn netns_hosts_fallback_args(config: &SandboxConfig) -> Vec<String> {
    let Some(ns) = config.network_namespace.as_deref() else {
        return Vec::new();
    };
    let hosts_path = format!("/etc/netns/{ns}/hosts");
    if Path::new(&hosts_path).is_file() {
        return Vec::new();
    }
    vec![
        "--ro-bind".to_string(),
        "/etc/hosts".to_string(),
        "/etc/hosts".to_string(),
    ]
}

/// Resolve $VAR references in a string using the runtime variable map.
/// Map bind mode + try flag to the correct bwrap argument.
fn bind_flag(mode: BindMode, try_bind: bool) -> &'static str {
    match (mode, try_bind) {
        (BindMode::Ro, false) => "--ro-bind",
        (BindMode::Ro, true) => "--ro-bind-try",
        (BindMode::Rw, false) => "--bind",
        (BindMode::Rw, true) => "--bind-try",
    }
}

/// Build the passthrough environment arguments.
/// For each variable name in the list, if it's set in the host environment,
/// add `--setenv <name> <value>` to the args.
pub fn passthrough_env_args(var_names: &[String]) -> Vec<String> {
    let mut args = Vec::new();
    for name in var_names {
        if let Ok(value) = std::env::var(name) {
            if !value.is_empty() {
                args.push("--setenv".to_string());
                args.push(name.clone());
                args.push(value);
            }
        }
    }
    args
}

/// Build ZDOTDIR forwarding args for zsh sandboxes.
pub fn zdotdir_args(home: &str, sandbox_home: &str) -> Vec<String> {
    let mut args = Vec::new();
    if let Ok(zdotdir) = std::env::var("ZDOTDIR") {
        if !zdotdir.is_empty() {
            let dest = match std::path::Path::new(&zdotdir).strip_prefix(home) {
                Ok(suffix) => std::path::Path::new(sandbox_home)
                    .join(suffix)
                    .to_string_lossy()
                    .to_string(),
                Err(_) => zdotdir.clone(),
            };
            args.push("--setenv".to_string());
            args.push("ZDOTDIR".to_string());
            args.push(dest.clone());
            if std::path::Path::new(&zdotdir).is_dir() {
                args.push("--ro-bind".to_string());
                args.push(zdotdir);
                args.push(dest);
            }
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn resolve_vars_basic() {
        let mut vars = HashMap::new();
        vars.insert("HOME".to_string(), "/home/user".to_string());
        vars.insert("DIR_HASH".to_string(), "abc123".to_string());

        assert_eq!(
            crate::vars::expand_vars("$HOME/.config", &vars),
            "/home/user/.config"
        );
        assert_eq!(
            crate::vars::expand_vars("/state/$DIR_HASH/data", &vars),
            "/state/abc123/data"
        );
    }

    #[test]
    fn resolve_vars_no_match() {
        let vars = HashMap::new();
        assert_eq!(
            crate::vars::expand_vars("/static/path", &vars),
            "/static/path"
        );
    }

    #[test]
    fn bind_flag_mapping() {
        assert_eq!(bind_flag(BindMode::Ro, false), "--ro-bind");
        assert_eq!(bind_flag(BindMode::Ro, true), "--ro-bind-try");
        assert_eq!(bind_flag(BindMode::Rw, false), "--bind");
        assert_eq!(bind_flag(BindMode::Rw, true), "--bind-try");
    }

    #[test]
    fn passthrough_env_skips_unset() {
        // CLOISTER_TEST_UNSET_VAR is presumably not set
        let args = passthrough_env_args(&["CLOISTER_TEST_UNSET_VAR_12345".to_string()]);
        assert!(args.is_empty());
    }

    #[test]
    fn zdotdir_remaps_only_under_home() {
        let _guard = env_lock().lock().unwrap();
        let original = std::env::var_os("ZDOTDIR");

        std::env::set_var("ZDOTDIR", "/home/user/.config/zsh");
        let args = zdotdir_args("/home/user", "/home/ubuntu");
        assert!(args.contains(&"ZDOTDIR".to_string()));
        assert!(args.contains(&"/home/ubuntu/.config/zsh".to_string()));

        if let Some(val) = original {
            std::env::set_var("ZDOTDIR", val);
        } else {
            std::env::remove_var("ZDOTDIR");
        }
    }

    #[test]
    fn zdotdir_does_not_remap_prefix_only() {
        let _guard = env_lock().lock().unwrap();
        let original = std::env::var_os("ZDOTDIR");

        std::env::set_var("ZDOTDIR", "/home/user2/.config/zsh");
        let args = zdotdir_args("/home/user", "/home/ubuntu");
        assert!(args.contains(&"ZDOTDIR".to_string()));
        assert!(args.contains(&"/home/user2/.config/zsh".to_string()));
        assert!(!args.contains(&"/home/ubuntu2/.config/zsh".to_string()));

        if let Some(val) = original {
            std::env::set_var("ZDOTDIR", val);
        } else {
            std::env::remove_var("ZDOTDIR");
        }
    }

    fn minimal_config() -> SandboxConfig {
        let json = serde_json::json!({
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
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn netns_hosts_fallback_added_when_netns_hosts_missing() {
        let mut config = minimal_config();
        config.network_namespace = Some("cloister-missing-netns-hosts-test".to_string());
        assert_eq!(
            netns_hosts_fallback_args(&config),
            vec!["--ro-bind", "/etc/hosts", "/etc/hosts"]
        );
    }

    #[test]
    fn netns_hosts_fallback_precedes_static_netns_hosts_bind_try() {
        let mut config = minimal_config();
        config.network_namespace = Some("cloister-missing-netns-hosts-test".to_string());
        config.static_bwrap_args = vec![
            "--ro-bind-try".to_string(),
            "/etc/netns/cloister-missing-netns-hosts-test/hosts".to_string(),
            "/etc/hosts".to_string(),
        ];

        let vars = HashMap::new();
        let run_cmd = vec!["echo".to_string(), "hello".to_string()];
        let (_cmd, args_fd) =
            build_bwrap_command(&config, &vars, vec![], &run_cmd, "/home/user", false)
                .expect("build_bwrap_command failed");
        let pipe_args = read_pipe_args(args_fd);

        let fallback_pos = pipe_args
            .windows(3)
            .position(|w| w == ["--ro-bind", "/etc/hosts", "/etc/hosts"])
            .expect("fallback /etc/hosts bind not found");
        let try_pos = pipe_args
            .windows(3)
            .position(|w| {
                w == [
                    "--ro-bind-try",
                    "/etc/netns/cloister-missing-netns-hosts-test/hosts",
                    "/etc/hosts",
                ]
            })
            .expect("netns hosts --ro-bind-try not found");

        assert!(fallback_pos < try_pos);
    }

    #[test]
    fn bwrap_command_uses_args_fd() {
        let config = minimal_config();
        let vars = HashMap::new();
        let run_cmd = vec!["helium".to_string(), "-ozone-platform=wayland".to_string()];
        let (cmd, args_fd) =
            build_bwrap_command(&config, &vars, vec![], &run_cmd, "/home/user", false)
                .expect("build_bwrap_command failed");
        let cmd_args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();

        // The real command line should contain --args <fd>
        let args_pos = cmd_args
            .iter()
            .position(|&a| a == "--args")
            .expect("--args not found");
        let fd_str = cmd_args[args_pos + 1];
        assert_eq!(fd_str, args_fd.as_raw_fd().to_string());

        // -- and run command should still be on the real command line
        let dash_pos = cmd_args
            .iter()
            .position(|&a| a == "--")
            .expect("-- not found");
        assert_eq!(
            &cmd_args[dash_pos + 1..],
            &["helium", "-ozone-platform=wayland"]
        );

        // Read the pipe and verify NUL-separated bwrap options
        use std::io::Read;
        let mut pipe_file = File::from(args_fd);
        let mut content = Vec::new();
        pipe_file.read_to_end(&mut content).unwrap();
        let pipe_args: Vec<&str> = content
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| std::str::from_utf8(s).unwrap())
            .collect();

        // Core isolation flags should be in the pipe
        assert!(pipe_args.contains(&"--die-with-parent"));
        assert!(pipe_args.contains(&"--unshare-all"));
        assert!(pipe_args.contains(&"--clearenv"));
        assert!(pipe_args.contains(&"--proc"));

        // --chdir should NOT be in the pipe (it stays on the real command line)
        assert!(!pipe_args.contains(&"--chdir"));
    }

    /// Helper to read NUL-separated args from the pipe returned by `build_bwrap_command`.
    fn read_pipe_args(fd: OwnedFd) -> Vec<String> {
        use std::io::Read;
        let mut pipe_file = File::from(fd);
        let mut content = Vec::new();
        pipe_file.read_to_end(&mut content).unwrap();
        content
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| std::str::from_utf8(s).unwrap().to_string())
            .collect()
    }

    #[test]
    fn new_session_present_when_non_interactive() {
        let config = minimal_config();
        let vars = HashMap::new();
        let run_cmd = vec!["echo".to_string(), "hello".to_string()];
        let (_cmd, args_fd) =
            build_bwrap_command(&config, &vars, vec![], &run_cmd, "/home/user", false)
                .expect("build_bwrap_command failed");
        let pipe_args = read_pipe_args(args_fd);
        assert!(pipe_args.contains(&"--new-session".to_string()));
    }

    #[test]
    fn new_session_absent_when_interactive() {
        let config = minimal_config();
        let vars = HashMap::new();
        let run_cmd = vec!["/nix/store/xxx-zsh/bin/zsh".to_string(), "-i".to_string()];
        let (_cmd, args_fd) =
            build_bwrap_command(&config, &vars, vec![], &run_cmd, "/home/user", true)
                .expect("build_bwrap_command failed");
        let pipe_args = read_pipe_args(args_fd);
        assert!(!pipe_args.contains(&"--new-session".to_string()));
    }
}
