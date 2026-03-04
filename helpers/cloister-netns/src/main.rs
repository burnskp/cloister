use std::env;
use std::fs::File;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::process;

use nix::sched::{CloneFlags, setns};

struct ExecArgs {
    netns: Option<String>,
    cmd_start: usize,
}

fn exit_invalid_args(msg: &str) -> ! {
    #[cfg(test)]
    panic!("{msg}");
    #[cfg(not(test))]
    {
        eprintln!("cloister-netns: {msg}");
        process::exit(2);
    }
}

/// Parse `--netns <name>` and locate the `--` separator.
/// Returns an `ExecArgs` whose `cmd_start` points at the first token after `--`.
fn parse_exec_args(args: &[String]) -> ExecArgs {
    let mut netns = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--netns" => {
                if args.get(i + 1).map(|s| s.as_str()) == Some("--") || i + 1 >= args.len() {
                    exit_invalid_args("option '--netns' requires an argument");
                }
                i += 1;
                netns = args.get(i).cloned();
            }
            "--" => {
                i += 1;
                break;
            }
            other => {
                eprintln!("cloister-netns: warning: unknown argument: {other}");
            }
        }
        i += 1;
    }
    ExecArgs {
        netns,
        cmd_start: i,
    }
}

/// Validate that a network namespace name is a safe single path component.
fn validate_netns_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains('\0') || name == "." || name == ".." {
        return Err(format!("invalid namespace name: {name:?}"));
    }

    #[cfg(test)]
    let allowlist_str = "vpn\nmy-ns\nns_123";
    // Security: option_env! is evaluated at compile time, not runtime. The
    // namespace allowlist is baked into the binary by the Nix build and cannot
    // be spoofed by setting environment variables at runtime.
    #[cfg(not(test))]
    let allowlist_str = option_env!("CLOISTER_NETNS_ALLOWLIST").unwrap_or("");
    let allowed: Vec<&str> = allowlist_str
        .lines()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if !allowed.contains(&name) {
        return Err(format!("namespace {name:?} is not in the allowed list"));
    }

    Ok(())
}

/// Drop all Linux capability sets after namespace switch.
fn drop_privileges() -> Result<(), String> {
    // linux/capability.h
    const LINUX_CAPABILITY_VERSION_3: u32 = 0x20080522;

    #[repr(C)]
    struct CapUserHeader {
        version: u32,
        pid: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CapUserData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }

    let mut header = CapUserHeader {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };
    let data = [
        CapUserData {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
        CapUserData {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
    ];

    unsafe {
        if nix::libc::syscall(
            nix::libc::SYS_capset,
            &mut header as *mut CapUserHeader,
            data.as_ptr(),
        ) != 0
        {
            return Err(format!(
                "failed to drop capability sets: {}",
                std::io::Error::last_os_error()
            ));
        }

        if nix::libc::prctl(
            nix::libc::PR_CAP_AMBIENT,
            nix::libc::PR_CAP_AMBIENT_CLEAR_ALL,
            0,
            0,
            0,
        ) != 0
        {
            return Err("failed to clear ambient capabilities".to_string());
        }

        if nix::libc::prctl(nix::libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err("failed to set no_new_privs".to_string());
        }

        // Verify capabilities were actually zeroed by reading them back
        let mut verify_header = CapUserHeader {
            version: LINUX_CAPABILITY_VERSION_3,
            pid: 0,
        };
        let mut verify_data = [
            CapUserData {
                effective: 0xFF,
                permitted: 0xFF,
                inheritable: 0xFF,
            },
            CapUserData {
                effective: 0xFF,
                permitted: 0xFF,
                inheritable: 0xFF,
            },
        ];
        if nix::libc::syscall(
            nix::libc::SYS_capget,
            &mut verify_header as *mut CapUserHeader,
            verify_data.as_mut_ptr(),
        ) != 0
        {
            return Err(format!(
                "failed to read back capability sets: {}",
                std::io::Error::last_os_error()
            ));
        }
        for (i, d) in verify_data.iter().enumerate() {
            if d.effective != 0 || d.permitted != 0 || d.inheritable != 0 {
                return Err(format!(
                    "capability set {} not zeroed after capset (eff={:#x}, perm={:#x}, inh={:#x})",
                    i, d.effective, d.permitted, d.inheritable
                ));
            }
        }

        // Verify no_new_privs took effect
        let nnp = nix::libc::prctl(nix::libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0);
        if nnp != 1 {
            return Err(format!("PR_GET_NO_NEW_PRIVS returned {nnp}, expected 1"));
        }
    }

    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let parsed = parse_exec_args(&args);

    if unsafe { nix::libc::geteuid() } == 0 {
        eprintln!(
            "cloister-netns: refusing to run as setuid root. Must use file capabilities (setcap cap_sys_admin+ep)."
        );
        process::exit(1);
    }

    if parsed.cmd_start >= args.len() || parsed.netns.is_none() {
        eprintln!("Usage: cloister-netns --netns <name> -- command [args...]");
        process::exit(2);
    }

    // Security: option_env! is evaluated at compile time, not runtime. The
    // exec enforcement flag and allowed exec paths are baked into the binary
    // by the Nix build, making them immutable once compiled. Attackers cannot
    // bypass the allowlist by setting environment variables.
    let enforce_exec = option_env!("CLOISTER_NETNS_ENFORCE_EXEC")
        .map(|v| v == "1")
        .unwrap_or(false);

    let allowed_exec_str = option_env!("CLOISTER_NETNS_ALLOWED_EXEC_PATHS").unwrap_or("");
    let allowed_exec: Vec<&str> = allowed_exec_str
        .lines()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if enforce_exec {
        if allowed_exec.is_empty() {
            eprintln!("cloister-netns: no allowed exec paths configured");
            process::exit(1);
        }

        let cmd = &args[parsed.cmd_start];
        if !allowed_exec.contains(&cmd.as_str()) {
            eprintln!("cloister-netns: refusing to exec non-allowed command: {cmd}");
            process::exit(1);
        }
    }

    // Switch network namespace — hard failure on error (silently using host
    // network would defeat the purpose of namespace isolation)
    let name = parsed.netns.unwrap();
    validate_netns_name(&name).unwrap_or_else(|e| {
        eprintln!("cloister-netns: {e}");
        process::exit(1);
    });
    let path = format!("/var/run/netns/{name}");
    let file = File::options()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&path)
        .unwrap_or_else(|e| {
            eprintln!("cloister-netns: open {path}: {e}");
            process::exit(1);
        });
    setns(&file, CloneFlags::CLONE_NEWNET).unwrap_or_else(|e| {
        eprintln!("cloister-netns: setns {path}: {e}");
        process::exit(1);
    });

    // Drop all privilege state before execing the sandbox.
    drop_privileges().unwrap_or_else(|e| {
        eprintln!("cloister-netns: {e}");
        process::exit(1);
    });

    // Replace this process with the allowed command
    let err = process::Command::new(&args[parsed.cmd_start])
        .args(&args[parsed.cmd_start + 1..])
        .exec();
    eprintln!("cloister-netns: exec: {err}");
    process::exit(127);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_full_args() {
        let args = s(&["prog", "--netns", "vpn", "--", "cmd", "arg"]);
        let p = parse_exec_args(&args);
        assert_eq!(p.netns.as_deref(), Some("vpn"));
        assert_eq!(p.cmd_start, 4);
    }

    #[test]
    fn parse_no_netns() {
        let args = s(&["prog", "--", "echo", "hello"]);
        let p = parse_exec_args(&args);
        assert_eq!(p.netns, None);
        assert_eq!(p.cmd_start, 2);
    }

    #[test]
    fn parse_only_separator() {
        let args = s(&["prog", "--"]);
        let p = parse_exec_args(&args);
        assert_eq!(p.cmd_start, 2);
    }

    #[test]
    #[should_panic]
    fn parse_netns_missing_value_panics() {
        // parse_exec_args exits on invalid input, so this will panic in tests.
        let args = s(&["prog", "--netns", "--", "cmd"]);
        let _ = parse_exec_args(&args);
    }

    #[test]
    fn parse_empty() {
        let args = s(&["prog"]);
        let p = parse_exec_args(&args);
        assert_eq!(p.netns, None);
        assert_eq!(p.cmd_start, 1);
    }

    #[test]
    fn parse_unknown_flags_skipped() {
        let args = s(&["prog", "--unknown", "--netns", "vpn", "--", "cmd"]);
        let p = parse_exec_args(&args);
        assert_eq!(p.netns.as_deref(), Some("vpn"));
        assert_eq!(p.cmd_start, 5);
    }

    #[test]
    fn validate_good_name() {
        assert!(validate_netns_name("vpn").is_ok());
        assert!(validate_netns_name("my-ns").is_ok());
        assert!(validate_netns_name("ns_123").is_ok());
    }

    #[test]
    fn validate_rejects_slash() {
        assert!(validate_netns_name("../etc/passwd").is_err());
        assert!(validate_netns_name("foo/bar").is_err());
    }

    #[test]
    fn validate_rejects_empty() {
        assert!(validate_netns_name("").is_err());
    }

    #[test]
    fn validate_rejects_dot() {
        assert!(validate_netns_name(".").is_err());
        assert!(validate_netns_name("..").is_err());
    }

    #[test]
    fn validate_rejects_null() {
        assert!(validate_netns_name("foo\0bar").is_err());
    }
}
