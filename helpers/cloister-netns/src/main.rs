use std::env;
use std::ffi::CString;
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

fn validate_invocation(parsed: &ExecArgs, args: &[String]) -> Result<(), &'static str> {
    if parsed.cmd_start >= args.len() || parsed.netns.is_none() {
        Err("Usage: cloister-netns --netns <name> -- command [args...]")
    } else {
        Ok(())
    }
}

fn validate_exec_target(
    cmd: &str,
    enforce_exec: bool,
    allowed_exec: &[&str],
) -> Result<(), String> {
    if !enforce_exec {
        return Ok(());
    }

    if allowed_exec.is_empty() {
        return Err("no allowed exec paths configured".to_string());
    }

    if allowed_exec.contains(&cmd) {
        Ok(())
    } else {
        Err(format!("refusing to exec non-allowed command: {cmd}"))
    }
}

fn user_in_group(required_gid: u32, group_ids: &[u32]) -> bool {
    group_ids.contains(&required_gid)
}

fn validate_required_group_with<F>(
    required_group: Option<&str>,
    resolve_gid: F,
    group_ids: &[u32],
) -> Result<(), String>
where
    F: FnOnce(&str) -> Result<u32, String>,
{
    let Some(required_group) = required_group.filter(|g| !g.is_empty()) else {
        return Ok(());
    };

    let required_gid = resolve_gid(required_group)?;
    if user_in_group(required_gid, group_ids) {
        Ok(())
    } else {
        Err(format!(
            "current user is not in required group {required_group:?}"
        ))
    }
}

fn current_group_ids() -> Result<Vec<u32>, String> {
    let effective_gid = unsafe { nix::libc::getegid() } as u32;
    let count = unsafe { nix::libc::getgroups(0, std::ptr::null_mut()) };
    if count < 0 {
        return Err(format!(
            "failed to read supplementary groups: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut groups = vec![0; count as usize];
    if count > 0 {
        let rc = unsafe { nix::libc::getgroups(count, groups.as_mut_ptr()) };
        if rc < 0 {
            return Err(format!(
                "failed to read supplementary groups: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    let mut group_ids: Vec<u32> = groups;
    if !group_ids.contains(&effective_gid) {
        group_ids.push(effective_gid);
    }
    Ok(group_ids)
}

fn resolve_group_gid(name: &str) -> Result<u32, String> {
    let c_name = CString::new(name).map_err(|_| format!("invalid group name: {name:?}"))?;
    let group = unsafe { nix::libc::getgrnam(c_name.as_ptr()) };
    if group.is_null() {
        return Err(format!("required group {name:?} does not exist"));
    }
    Ok(unsafe { (*group).gr_gid } as u32)
}

fn validate_required_group(required_group: Option<&str>) -> Result<(), String> {
    let group_ids = current_group_ids()?;
    validate_required_group_with(required_group, resolve_group_gid, &group_ids)
}

fn should_set_no_new_privs(cmd: &str, args: &[String]) -> bool {
    let is_cloister_sandbox = std::path::Path::new(cmd)
        .file_name()
        .and_then(|name| name.to_str())
        == Some("cloister-sandbox");

    !(is_cloister_sandbox && args.iter().any(|arg| arg == "--after-netns"))
}

/// Drop all Linux capability sets after namespace switch.
fn drop_privileges(set_no_new_privs: bool) -> Result<(), String> {
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

        if set_no_new_privs && nix::libc::prctl(nix::libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
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

        if set_no_new_privs {
            // Verify no_new_privs took effect
            let nnp = nix::libc::prctl(nix::libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0);
            if nnp != 1 {
                return Err(format!("PR_GET_NO_NEW_PRIVS returned {nnp}, expected 1"));
            }
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

    if let Err(msg) = validate_invocation(&parsed, &args) {
        eprintln!("{msg}");
        process::exit(2);
    }

    let name = parsed
        .netns
        .as_deref()
        .expect("validated invocation has netns");

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

    validate_netns_name(name).unwrap_or_else(|e| {
        eprintln!("cloister-netns: {e}");
        process::exit(1);
    });

    if let Err(msg) = validate_required_group(option_env!("CLOISTER_NETNS_REQUIRED_GROUP")) {
        eprintln!("cloister-netns: {msg}");
        process::exit(1);
    }

    if let Err(msg) = validate_exec_target(&args[parsed.cmd_start], enforce_exec, &allowed_exec) {
        eprintln!("cloister-netns: {msg}");
        process::exit(1);
    }

    // Switch network namespace — hard failure on error (silently using host
    // network would defeat the purpose of namespace isolation)
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

    // Drop all privilege state before execing the sandbox. Keep no_new_privs
    // disabled only for cloister-sandbox's trusted --after-netns re-exec so the
    // host-side broker can still launch additional netns sandboxes.
    let set_no_new_privs =
        should_set_no_new_privs(&args[parsed.cmd_start], &args[parsed.cmd_start + 1..]);
    drop_privileges(set_no_new_privs).unwrap_or_else(|e| {
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

    #[test]
    fn should_set_no_new_privs_for_generic_command() {
        assert!(should_set_no_new_privs(
            "/nix/store/allowed/bin/ip",
            &s(&["route"])
        ));
    }

    #[test]
    fn should_skip_no_new_privs_for_cloister_sandbox_after_netns_reexec() {
        assert!(!should_set_no_new_privs(
            "/nix/store/allowed/bin/cloister-sandbox",
            &s(&["--after-netns", "--config", "/nix/store/cfg.json"])
        ));
    }

    #[test]
    fn validate_invocation_requires_netns() {
        let args = s(&["prog", "--", "echo"]);
        let parsed = parse_exec_args(&args);
        assert_eq!(
            validate_invocation(&parsed, &args),
            Err("Usage: cloister-netns --netns <name> -- command [args...]")
        );
    }

    #[test]
    fn validate_invocation_requires_command() {
        let args = s(&["prog", "--netns", "vpn", "--"]);
        let parsed = parse_exec_args(&args);
        assert_eq!(
            validate_invocation(&parsed, &args),
            Err("Usage: cloister-netns --netns <name> -- command [args...]")
        );
    }

    #[test]
    fn validate_exec_target_allows_anything_when_not_enforced() {
        assert!(validate_exec_target("/bin/sh", false, &[]).is_ok());
    }

    #[test]
    fn validate_exec_target_rejects_empty_allowlist_when_enforced() {
        assert_eq!(
            validate_exec_target("/bin/sh", true, &[]),
            Err("no allowed exec paths configured".to_string())
        );
    }

    #[test]
    fn validate_exec_target_rejects_non_allowlisted_command() {
        assert_eq!(
            validate_exec_target(
                "/bin/sh",
                true,
                &["/nix/store/allowed/bin/cloister-sandbox"]
            ),
            Err("refusing to exec non-allowed command: /bin/sh".to_string())
        );
    }

    #[test]
    fn validate_exec_target_accepts_allowlisted_command() {
        assert!(
            validate_exec_target(
                "/nix/store/allowed/bin/cloister-sandbox",
                true,
                &["/nix/store/allowed/bin/cloister-sandbox"]
            )
            .is_ok()
        );
    }

    #[test]
    fn user_in_group_matches_present_gid() {
        assert!(user_in_group(100, &[10, 100, 200]));
        assert!(!user_in_group(101, &[10, 100, 200]));
    }

    #[test]
    fn validate_required_group_allows_empty_configuration() {
        assert!(validate_required_group_with(None, |_| Ok(42), &[42]).is_ok());
        assert!(validate_required_group_with(Some(""), |_| Ok(42), &[42]).is_ok());
    }

    #[test]
    fn validate_required_group_rejects_missing_membership() {
        assert_eq!(
            validate_required_group_with(Some("cloister-netns"), |_| Ok(42), &[7, 8]),
            Err("current user is not in required group \"cloister-netns\"".to_string())
        );
    }

    #[test]
    fn validate_required_group_accepts_membership() {
        assert!(
            validate_required_group_with(Some("cloister-netns"), |_| Ok(42), &[7, 42, 8]).is_ok()
        );
    }

    #[test]
    fn validate_required_group_surfaces_lookup_errors() {
        assert_eq!(
            validate_required_group_with(
                Some("cloister-netns"),
                |_| Err("lookup failed".to_string()),
                &[42]
            ),
            Err("lookup failed".to_string())
        );
    }
}
