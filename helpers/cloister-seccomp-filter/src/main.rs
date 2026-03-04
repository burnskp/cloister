use std::env;
use std::fs;
use std::process;

use libseccomp::{
    ScmpAction, ScmpArch, ScmpArgCompare, ScmpCompareOp, ScmpFilterContext, ScmpSyscall,
};

/// Syscalls blocked by default. Derived from Flatpak's setup_seccomp() in flatpak-run.c.
/// Grouped by category for readability; the filter treats them identically.
const BLOCKED_SYSCALLS: &[&str] = &[
    // Kernel modules
    "init_module",
    "finit_module",
    "delete_module",
    // Reboot / kexec
    "reboot",
    "kexec_load",
    "kexec_file_load",
    // Swap
    "swapon",
    "swapoff",
    // Raw I/O (x86 only — silently skipped on aarch64)
    "ioperm",
    "iopl",
    // Mount / namespace escape
    "mount",
    "umount",
    "umount2",
    "pivot_root",
    "chroot",
    "fsopen",
    "fsconfig",
    "fsmount",
    "fspick",
    "open_tree",
    "move_mount",
    "mount_setattr",
    // Namespace creation
    "unshare",
    "setns",
    // Clock manipulation
    "adjtimex",
    "clock_adjtime",
    "clock_settime",
    "settimeofday",
    "stime",
    // Process introspection
    "ptrace",
    "process_vm_readv",
    "process_vm_writev",
    "process_madvise",
    "process_mrelease",
    "kcmp",
    // BPF / perf
    "bpf",
    "perf_event_open",
    "userfaultfd",
    // ABI switching
    "personality",
    // Keyring
    "add_key",
    "keyctl",
    "request_key",
    // NUMA
    "mbind",
    "move_pages",
    "migrate_pages",
    "set_mempolicy",
    "get_mempolicy",
    // File handle bypass
    "open_by_handle_at",
    "name_to_handle_at",
    // Misc dangerous
    "acct",
    "syslog",
    "uselib",
    "quotactl",
    "modify_ldt",
    "lookup_dcookie",
    "vhangup",
    "nfsservctl",
    "fanotify_init",
    "vmsplice",
    // io_uring (bypasses seccomp on submitted operations; large kernel attack surface)
    "io_uring_setup",
    "io_uring_enter",
    "io_uring_register",
    // Obsolete
    "_sysctl",
    "afs_syscall",
    "break",
    "create_module",
    "ftime",
    "get_kernel_syms",
    "getpmsg",
    "gtty",
    "idle",
    "lock",
    "mpx",
    "prof",
    "profil",
    "putpmsg",
    "query_module",
    "security",
    "sgetmask",
    "ssetmask",
    "stty",
    "sysfs",
    "tuxcall",
    "ulimit",
    "ustat",
    "vserver",
];

/// Syscalls exempted when --allow-chromium-sandbox is set.
/// Chromium/Electron's internal sandbox needs chroot for renderer isolation
/// and namespace creation (unshare/setns) for user/PID namespace setup.
/// Safe inside bwrap — the process is already in an unprivileged user namespace.
const CHROMIUM_SANDBOX_SYSCALLS: &[&str] = &["chroot", "unshare", "setns"];

fn build_filter(
    allow_chromium_sandbox: bool,
) -> Result<ScmpFilterContext, Box<dyn std::error::Error>> {
    // Default-allow: only explicitly listed syscalls are blocked.
    let mut filter = ScmpFilterContext::new(ScmpAction::Allow)?;
    filter.add_arch(ScmpArch::Native)?;

    let errno_action = ScmpAction::Errno(libc::ENOSYS);

    for name in BLOCKED_SYSCALLS {
        // When --allow-chromium-sandbox is set, skip blocking syscalls
        // needed by Chromium/Electron's internal sandbox.
        if allow_chromium_sandbox && CHROMIUM_SANDBOX_SYSCALLS.contains(name) {
            continue;
        }

        // from_name() returns Err for arch-specific syscalls that don't exist
        // on the current architecture (e.g. iopl/ioperm on aarch64). Skip them.
        match ScmpSyscall::from_name(name) {
            Ok(syscall) => {
                filter.add_rule(errno_action, syscall)?;
            }
            Err(_) => {
                // Silently skip — syscall doesn't exist on this architecture
            }
        }
    }

    // Block clone(2) when any CLONE_NEW* namespace flag is set in arg0.
    // Normal fork/pthread_create pass 0 or non-namespace flags, so they
    // are unaffected. Skipped when --allow-chromium-sandbox is set.
    if !allow_chromium_sandbox {
        const CLONE_NS_FLAGS: &[u64] = &[
            0x00000080, // CLONE_NEWTIME
            0x00020000, // CLONE_NEWNS
            0x02000000, // CLONE_NEWCGROUP
            0x04000000, // CLONE_NEWUTS
            0x08000000, // CLONE_NEWIPC
            0x10000000, // CLONE_NEWUSER
            0x20000000, // CLONE_NEWPID
            0x40000000, // CLONE_NEWNET
        ];

        if let Ok(clone_syscall) = ScmpSyscall::from_name("clone") {
            #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
            let flags_arg_index = 0;

            #[cfg(target_arch = "s390x")]
            let flags_arg_index = 1;

            #[cfg(not(any(
                target_arch = "x86_64",
                target_arch = "aarch64",
                target_arch = "s390x"
            )))]
            let flags_arg_index = {
                eprintln!(
                    "cloister-seccomp-filter: warning: unknown architecture, clone namespace enforcement may check the wrong argument"
                );
                0
            };

            for &flag in CLONE_NS_FLAGS {
                filter.add_rule_conditional(
                    errno_action,
                    clone_syscall,
                    &[ScmpArgCompare::new(
                        flags_arg_index,
                        ScmpCompareOp::MaskedEqual(flag),
                        flag,
                    )],
                )?;
            }
        }

        // Block clone3(2) unconditionally. clone3 passes flags inside a
        // userspace struct pointer — seccomp cannot dereference pointers,
        // so argument filtering is impossible. glibc/musl fall back to
        // clone(2) when clone3 returns ENOSYS.
        if let Ok(clone3_syscall) = ScmpSyscall::from_name("clone3") {
            filter.add_rule(errno_action, clone3_syscall)?;
        }
    }

    // Block faking input to the controlling tty via ioctl (TIOCSTI and TIOCLINUX).
    // These have been used to escape sandboxes by injecting commands into
    // the parent shell (e.g., CVE-2017-5226, CVE-2023-28100).
    if let Ok(ioctl_syscall) = ScmpSyscall::from_name("ioctl") {
        const TIOCSTI: u64 = 0x5412;
        const TIOCLINUX: u64 = 0x541C;

        for &cmd in &[TIOCSTI, TIOCLINUX] {
            filter.add_rule_conditional(
                errno_action,
                ioctl_syscall,
                &[ScmpArgCompare::new(
                    1, // 2nd argument (request)
                    ScmpCompareOp::MaskedEqual(0xFFFFFFFF),
                    cmd,
                )],
            )?;
        }
    }

    // Socket family allowlist.
    // We only allow a minimal set of address families (UNSPEC, LOCAL/UNIX, INET, INET6, NETLINK).
    // Unlike Flatpak, we do NOT provide options to allow CAN or BLUETOOTH; they are unconditionally blocked.
    // libseccomp doesn't have an "allow these values only" construct for a single argument,
    // so we must explicitly block the gaps between allowed values, and everything above the highest allowed value.
    if let Ok(socket_syscall) = ScmpSyscall::from_name("socket") {
        const AF_UNSPEC: u64 = 0;
        const AF_LOCAL: u64 = 1; // AF_UNIX
        const AF_INET: u64 = 2;
        const AF_INET6: u64 = 10;
        const AF_NETLINK: u64 = 16;

        let allowed_families = [AF_UNSPEC, AF_LOCAL, AF_INET, AF_INET6, AF_NETLINK];
        let mut last_allowed = -1i64;

        for &family in &allowed_families {
            let family = family as i64;
            for disallowed in (last_allowed + 1)..family {
                filter.add_rule_conditional(
                    ScmpAction::Errno(libc::EAFNOSUPPORT),
                    socket_syscall,
                    &[ScmpArgCompare::new(
                        0, // 1st argument (domain)
                        ScmpCompareOp::Equal,
                        disallowed as u64,
                    )],
                )?;
            }
            last_allowed = family;
        }

        // Block everything above the highest allowed family
        filter.add_rule_conditional(
            ScmpAction::Errno(libc::EAFNOSUPPORT),
            socket_syscall,
            &[ScmpArgCompare::new(
                0,
                ScmpCompareOp::GreaterEqual,
                (last_allowed + 1) as u64,
            )],
        )?;
    }

    // Block dangerous prctl operations that could manipulate the capability model.
    // Most prctl operations (PR_SET_NO_NEW_PRIVS, PR_SET_PDEATHSIG, PR_SET_NAME, etc.)
    // are needed for normal operation, so we only block specific arg0 values.
    if let Ok(prctl_syscall) = ScmpSyscall::from_name("prctl") {
        const PR_SET_SECUREBITS: u64 = 28;
        const PR_CAP_AMBIENT: u64 = 47;
        const PR_CAP_AMBIENT_RAISE: u64 = 2;

        // Block PR_SET_SECUREBITS — prevents manipulation of capability securebits
        filter.add_rule_conditional(
            errno_action,
            prctl_syscall,
            &[ScmpArgCompare::new(
                0,
                ScmpCompareOp::Equal,
                PR_SET_SECUREBITS,
            )],
        )?;

        // Block PR_CAP_AMBIENT with PR_CAP_AMBIENT_RAISE — prevents re-gaining ambient capabilities
        filter.add_rule_conditional(
            errno_action,
            prctl_syscall,
            &[
                ScmpArgCompare::new(0, ScmpCompareOp::Equal, PR_CAP_AMBIENT),
                ScmpArgCompare::new(1, ScmpCompareOp::Equal, PR_CAP_AMBIENT_RAISE),
            ],
        )?;

        // Block PR_SET_DUMPABLE with SUID_DUMP_USER — prevents re-enabling core dumps
        // after the kernel cleared dumpability on namespace entry. Disabling dumps
        // (arg1 == 0) is still allowed.
        const PR_SET_DUMPABLE: u64 = 4;
        const SUID_DUMP_USER: u64 = 1;

        filter.add_rule_conditional(
            errno_action,
            prctl_syscall,
            &[
                ScmpArgCompare::new(0, ScmpCompareOp::Equal, PR_SET_DUMPABLE),
                ScmpArgCompare::new(1, ScmpCompareOp::Equal, SUID_DUMP_USER),
            ],
        )?;
    }

    Ok(filter)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    let mut output_path: Option<&str> = None;
    let mut allow_chromium_sandbox = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--output" => {
                i += 1;
                output_path = Some(args.get(i).ok_or("--output requires a path argument")?);
            }
            "--allow-chromium-sandbox" => {
                allow_chromium_sandbox = true;
            }
            other => {
                return Err(format!("unknown argument: {other}").into());
            }
        }
        i += 1;
    }

    let output_path = output_path.ok_or("--output PATH is required")?;

    let filter = build_filter(allow_chromium_sandbox)?;
    filter.export_bpf(&fs::File::create(output_path)?)?;

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("cloister-seccomp-filter: {e}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter_builds() {
        let filter = build_filter(false);
        assert!(filter.is_ok(), "default filter should build without error");
    }

    #[test]
    fn chromium_sandbox_filter_builds() {
        let filter = build_filter(true);
        assert!(
            filter.is_ok(),
            "allow-chromium-sandbox filter should build without error"
        );
    }

    fn export_filter_bytes(allow_chromium_sandbox: bool) -> Vec<u8> {
        use std::io::Read;

        let filter = build_filter(allow_chromium_sandbox).unwrap();
        let dir = std::env::temp_dir();

        // Use a thread-safe unique ID to avoid parallel test failures
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);

        let path = dir.join(format!(
            "seccomp-test-chromium-{}-{}-{}.bpf",
            allow_chromium_sandbox,
            std::process::id(),
            id
        ));

        let mut file = fs::File::create(&path).unwrap();
        filter.export_bpf(&mut file).unwrap();
        drop(file);

        let mut buf = Vec::new();
        fs::File::open(&path)
            .unwrap()
            .read_to_end(&mut buf)
            .unwrap();
        let _ = fs::remove_file(&path);
        buf
    }

    #[test]
    fn filters_differ_when_chromium_sandbox_toggled() {
        let buf_default = export_filter_bytes(false);
        let buf_chromium = export_filter_bytes(true);

        assert_ne!(
            buf_default, buf_chromium,
            "default and allow-chromium-sandbox filters should produce different BPF"
        );
    }

    #[test]
    fn exported_bpf_is_nonempty() {
        let buf = export_filter_bytes(false);
        assert!(!buf.is_empty(), "exported BPF should be non-empty");
    }
}
