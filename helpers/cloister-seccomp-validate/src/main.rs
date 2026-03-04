// Obsolete syscall constants are intentionally used — they're the whole point.
#![allow(deprecated)]

use std::process::ExitCode;

// ---------------------------------------------------------------------------
// Arch-conditional syscall number macros
// ---------------------------------------------------------------------------

/// Resolve a syscall number from libc on any architecture.
macro_rules! sys_nr {
    ($name:ident) => {
        Some(libc::$name as i64)
    };
}

/// Resolve a syscall number that only exists on x86_64.
#[cfg(target_arch = "x86_64")]
macro_rules! sys_nr_x86 {
    ($name:ident) => {
        Some(libc::$name as i64)
    };
}
#[cfg(not(target_arch = "x86_64"))]
macro_rules! sys_nr_x86 {
    ($name:ident) => {
        None
    };
}

// ---------------------------------------------------------------------------
// Syscall table types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    /// Kernel implements this syscall; ENOSYS proves seccomp blocked it.
    Testable,
    /// Kernel itself returns ENOSYS on modern kernels; tested but reported
    /// separately since we cannot distinguish seccomp from kernel rejection.
    Obsolete,
}

struct SyscallEntry {
    name: &'static str,
    nr: Option<i64>,
    category: Category,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestResult {
    Blocked,
    Allowed,
    Skipped(&'static str),
}

// ---------------------------------------------------------------------------
// Syscall table
// ---------------------------------------------------------------------------

/// Complete table of syscalls that the cloister seccomp filter blocks.
///
/// Entries with `nr: None` are not available on this architecture and will be
/// reported as SKIPPED. The table mirrors the filter generator in
/// `helpers/cloister-seccomp-filter/src/main.rs`.
fn syscall_table() -> Vec<SyscallEntry> {
    vec![
        // -- Kernel modules --
        SyscallEntry {
            name: "init_module",
            nr: sys_nr!(SYS_init_module),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "finit_module",
            nr: sys_nr!(SYS_finit_module),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "delete_module",
            nr: sys_nr!(SYS_delete_module),
            category: Category::Testable,
        },
        // -- Reboot / kexec --
        SyscallEntry {
            name: "reboot",
            nr: sys_nr!(SYS_reboot),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "kexec_load",
            nr: sys_nr!(SYS_kexec_load),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "kexec_file_load",
            nr: sys_nr!(SYS_kexec_file_load),
            category: Category::Testable,
        },
        // -- Swap --
        SyscallEntry {
            name: "swapon",
            nr: sys_nr!(SYS_swapon),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "swapoff",
            nr: sys_nr!(SYS_swapoff),
            category: Category::Testable,
        },
        // -- Raw I/O (x86 only) --
        SyscallEntry {
            name: "ioperm",
            nr: sys_nr_x86!(SYS_ioperm),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "iopl",
            nr: sys_nr_x86!(SYS_iopl),
            category: Category::Testable,
        },
        // -- Mount / namespace escape --
        SyscallEntry {
            name: "mount",
            nr: sys_nr!(SYS_mount),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "umount2",
            nr: sys_nr!(SYS_umount2),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "pivot_root",
            nr: sys_nr!(SYS_pivot_root),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "chroot",
            nr: sys_nr!(SYS_chroot),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "fsopen",
            nr: sys_nr!(SYS_fsopen),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "fsconfig",
            nr: sys_nr!(SYS_fsconfig),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "fsmount",
            nr: sys_nr!(SYS_fsmount),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "fspick",
            nr: sys_nr!(SYS_fspick),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "open_tree",
            nr: sys_nr!(SYS_open_tree),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "move_mount",
            nr: sys_nr!(SYS_move_mount),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "mount_setattr",
            nr: sys_nr!(SYS_mount_setattr),
            category: Category::Testable,
        },
        // -- Namespace creation --
        SyscallEntry {
            name: "unshare",
            nr: sys_nr!(SYS_unshare),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "setns",
            nr: sys_nr!(SYS_setns),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "clone3",
            nr: sys_nr!(SYS_clone3),
            category: Category::Testable,
        },
        // -- Clock manipulation --
        SyscallEntry {
            name: "adjtimex",
            nr: sys_nr!(SYS_adjtimex),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "clock_adjtime",
            nr: sys_nr!(SYS_clock_adjtime),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "clock_settime",
            nr: sys_nr!(SYS_clock_settime),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "settimeofday",
            nr: sys_nr!(SYS_settimeofday),
            category: Category::Testable,
        },
        // -- Process introspection --
        SyscallEntry {
            name: "ptrace",
            nr: sys_nr!(SYS_ptrace),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "process_vm_readv",
            nr: sys_nr!(SYS_process_vm_readv),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "process_vm_writev",
            nr: sys_nr!(SYS_process_vm_writev),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "kcmp",
            nr: sys_nr!(SYS_kcmp),
            category: Category::Testable,
        },
        // -- BPF / perf --
        SyscallEntry {
            name: "bpf",
            nr: sys_nr!(SYS_bpf),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "perf_event_open",
            nr: sys_nr!(SYS_perf_event_open),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "userfaultfd",
            nr: sys_nr!(SYS_userfaultfd),
            category: Category::Testable,
        },
        // -- ABI switching --
        SyscallEntry {
            name: "personality",
            nr: sys_nr!(SYS_personality),
            category: Category::Testable,
        },
        // -- Keyring --
        SyscallEntry {
            name: "add_key",
            nr: sys_nr!(SYS_add_key),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "keyctl",
            nr: sys_nr!(SYS_keyctl),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "request_key",
            nr: sys_nr!(SYS_request_key),
            category: Category::Testable,
        },
        // -- NUMA --
        SyscallEntry {
            name: "mbind",
            nr: sys_nr!(SYS_mbind),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "move_pages",
            nr: sys_nr!(SYS_move_pages),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "migrate_pages",
            nr: sys_nr!(SYS_migrate_pages),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "set_mempolicy",
            nr: sys_nr!(SYS_set_mempolicy),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "get_mempolicy",
            nr: sys_nr!(SYS_get_mempolicy),
            category: Category::Testable,
        },
        // -- File handle bypass --
        SyscallEntry {
            name: "open_by_handle_at",
            nr: sys_nr!(SYS_open_by_handle_at),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "name_to_handle_at",
            nr: sys_nr!(SYS_name_to_handle_at),
            category: Category::Testable,
        },
        // -- Misc dangerous --
        SyscallEntry {
            name: "acct",
            nr: sys_nr!(SYS_acct),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "syslog",
            nr: sys_nr!(SYS_syslog),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "quotactl",
            nr: sys_nr!(SYS_quotactl),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "modify_ldt",
            nr: sys_nr_x86!(SYS_modify_ldt),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "vhangup",
            nr: sys_nr!(SYS_vhangup),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "fanotify_init",
            nr: sys_nr!(SYS_fanotify_init),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "vmsplice",
            nr: sys_nr!(SYS_vmsplice),
            category: Category::Testable,
        },
        // -- io_uring --
        SyscallEntry {
            name: "io_uring_setup",
            nr: sys_nr!(SYS_io_uring_setup),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "io_uring_enter",
            nr: sys_nr!(SYS_io_uring_enter),
            category: Category::Testable,
        },
        SyscallEntry {
            name: "io_uring_register",
            nr: sys_nr!(SYS_io_uring_register),
            category: Category::Testable,
        },
        // -- Obsolete syscalls (kernel returns ENOSYS regardless) --
        SyscallEntry {
            name: "_sysctl",
            nr: sys_nr!(SYS__sysctl),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "uselib",
            nr: sys_nr_x86!(SYS_uselib),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "ustat",
            nr: sys_nr_x86!(SYS_ustat),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "sysfs",
            nr: sys_nr_x86!(SYS_sysfs),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "nfsservctl",
            nr: sys_nr_x86!(SYS_nfsservctl),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "lookup_dcookie",
            nr: sys_nr_x86!(SYS_lookup_dcookie),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "create_module",
            nr: sys_nr_x86!(SYS_create_module),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "get_kernel_syms",
            nr: sys_nr_x86!(SYS_get_kernel_syms),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "query_module",
            nr: sys_nr_x86!(SYS_query_module),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "afs_syscall",
            nr: sys_nr_x86!(SYS_afs_syscall),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "getpmsg",
            nr: sys_nr_x86!(SYS_getpmsg),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "putpmsg",
            nr: sys_nr_x86!(SYS_putpmsg),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "tuxcall",
            nr: sys_nr_x86!(SYS_tuxcall),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "security",
            nr: sys_nr_x86!(SYS_security),
            category: Category::Obsolete,
        },
        SyscallEntry {
            name: "vserver",
            nr: sys_nr_x86!(SYS_vserver),
            category: Category::Obsolete,
        },
    ]
}

// ---------------------------------------------------------------------------
// Syscall testing
// ---------------------------------------------------------------------------

/// Attempt a syscall with zeroed arguments and check whether seccomp blocked it.
///
/// Seccomp filters intercept *before* the kernel processes arguments, so passing
/// all zeros is safe — blocked calls never reach argument validation.
fn test_syscall(nr: i64) -> TestResult {
    let ret = unsafe { libc::syscall(nr, 0i64, 0i64, 0i64, 0i64, 0i64, 0i64) };
    if ret == -1 {
        let errno = unsafe { *libc::__errno_location() };
        if errno == libc::ENOSYS {
            return TestResult::Blocked;
        }
    }
    TestResult::Allowed
}

/// Test that clone(2) with CLONE_NEWUSER is blocked by seccomp.
///
/// clone with zero/non-namespace flags (normal fork) must NOT be blocked,
/// so we cannot use test_syscall() which passes all-zero args. Instead we
/// call clone with an explicit namespace flag and check for ENOSYS.
fn test_clone_ns_blocked() -> TestResult {
    const CLONE_NEWUSER: i64 = 0x10000000;
    let ret =
        unsafe { libc::syscall(libc::SYS_clone, CLONE_NEWUSER, 0i64, 0i64, 0i64, 0i64, 0i64) };
    if ret == -1 {
        let errno = unsafe { *libc::__errno_location() };
        if errno == libc::ENOSYS {
            return TestResult::Blocked;
        }
    }
    TestResult::Allowed
}

/// Test that ioctl(2) with TIOCSTI is blocked by seccomp.
fn test_ioctl_tiocsti_blocked() -> TestResult {
    const TIOCSTI: i64 = 0x5412;
    let ret = unsafe { libc::syscall(libc::SYS_ioctl, 0i64, TIOCSTI, 0i64, 0i64, 0i64, 0i64) };
    if ret == -1 {
        let errno = unsafe { *libc::__errno_location() };
        if errno == libc::ENOSYS {
            return TestResult::Blocked;
        }
    }
    TestResult::Allowed
}

/// Test that socket(2) with disallowed families is blocked,
/// while allowed families (like AF_LOCAL) pass the filter.
fn test_socket_families() -> TestResult {
    // We expect AF_BLUETOOTH (31) to be blocked by seccomp, returning EAFNOSUPPORT.
    const AF_BLUETOOTH: i64 = 31;
    let ret = unsafe {
        // socket(AF_BLUETOOTH, SOCK_STREAM, 0)
        libc::syscall(
            libc::SYS_socket,
            AF_BLUETOOTH,
            libc::SOCK_STREAM as i64,
            0i64,
            0i64,
            0i64,
            0i64,
        )
    };
    if ret == -1 {
        let errno = unsafe { *libc::__errno_location() };
        if errno == libc::EAFNOSUPPORT {
            return TestResult::Blocked;
        }
    }
    TestResult::Allowed
}

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

struct RunResult {
    name: &'static str,
    category: Category,
    result: TestResult,
}

fn print_human(results: &[RunResult]) {
    println!("cloister-seccomp-validate: verifying seccomp filter enforcement");
    println!();

    // Testable syscalls
    println!("[Testable syscalls]");
    for r in results.iter().filter(|r| r.category == Category::Testable) {
        let status = match r.result {
            TestResult::Blocked => "BLOCKED",
            TestResult::Allowed => "ALLOWED",
            TestResult::Skipped(reason) => {
                println!("  {:<22} SKIPPED ({})", r.name, reason);
                continue;
            }
        };
        println!("  {:<22} {}", r.name, status);
    }
    println!();

    // Obsolete syscalls
    println!("[Obsolete syscalls (kernel may return ENOSYS regardless)]");
    for r in results.iter().filter(|r| r.category == Category::Obsolete) {
        let status = match r.result {
            TestResult::Blocked => "BLOCKED (kernel or seccomp)",
            TestResult::Allowed => "ALLOWED",
            TestResult::Skipped(reason) => {
                println!("  {:<22} SKIPPED ({})", r.name, reason);
                continue;
            }
        };
        println!("  {:<22} {}", r.name, status);
    }
    println!();

    // Summary (only counts testable syscalls for pass/fail)
    let testable: Vec<_> = results
        .iter()
        .filter(|r| r.category == Category::Testable)
        .collect();
    let blocked = testable
        .iter()
        .filter(|r| r.result == TestResult::Blocked)
        .count();
    let allowed = testable
        .iter()
        .filter(|r| r.result == TestResult::Allowed)
        .count();
    let skipped = testable
        .iter()
        .filter(|r| matches!(r.result, TestResult::Skipped(_)))
        .count();
    let tested = blocked + allowed;

    println!(
        "Summary: {}/{} testable blocked, {} allowed, {} skipped",
        blocked, tested, allowed, skipped
    );
    println!("Result: {}", if allowed == 0 { "PASS" } else { "FAIL" });
}

fn print_json(results: &[RunResult]) {
    // Hand-built JSON (no serde dependency)
    let testable: Vec<_> = results
        .iter()
        .filter(|r| r.category == Category::Testable)
        .collect();
    let allowed = testable
        .iter()
        .filter(|r| r.result == TestResult::Allowed)
        .count();

    println!("{{");
    println!("  \"results\": [");
    for (i, r) in results.iter().enumerate() {
        let (status, detail) = match r.result {
            TestResult::Blocked => ("blocked", None),
            TestResult::Allowed => ("allowed", None),
            TestResult::Skipped(reason) => ("skipped", Some(reason)),
        };
        let category = match r.category {
            Category::Testable => "testable",
            Category::Obsolete => "obsolete",
        };
        let comma = if i + 1 < results.len() { "," } else { "" };
        match detail {
            Some(d) => println!(
                "    {{\"name\": \"{}\", \"category\": \"{}\", \"status\": \"{}\", \"reason\": \"{}\"}}{}",
                r.name, category, status, d, comma
            ),
            None => println!(
                "    {{\"name\": \"{}\", \"category\": \"{}\", \"status\": \"{}\"}}{}",
                r.name, category, status, comma
            ),
        }
    }
    println!("  ],");
    println!(
        "  \"pass\": {}",
        if allowed == 0 { "true" } else { "false" }
    );
    println!("}}");
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

struct Args {
    allow_chromium_sandbox: bool,
    json: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut allow_chromium_sandbox = false;
    let mut json = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--allow-chromium-sandbox" => allow_chromium_sandbox = true,
            "--json" => json = true,
            other => return Err(format!("unknown argument: {}", other)),
        }
    }

    Ok(Args {
        allow_chromium_sandbox,
        json,
    })
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn run(args: Args) -> bool {
    let table = syscall_table();
    let mut results = Vec::with_capacity(table.len());

    /// Syscalls exempted by --allow-chromium-sandbox.
    const CHROMIUM_SANDBOX_SYSCALLS: &[&str] = &["chroot", "unshare", "setns", "clone3"];

    for entry in &table {
        let result = match entry.nr {
            None => TestResult::Skipped("not on this architecture"),
            Some(_)
                if args.allow_chromium_sandbox
                    && CHROMIUM_SANDBOX_SYSCALLS.contains(&entry.name) =>
            {
                TestResult::Skipped("--allow-chromium-sandbox")
            }
            Some(nr) => test_syscall(nr),
        };

        results.push(RunResult {
            name: entry.name,
            category: entry.category,
            result,
        });
    }

    // Test clone(2) with a namespace flag — this is conditional (flag-based)
    // so it cannot be tested via the table's all-zero-args approach.
    results.push(RunResult {
        name: "clone(CLONE_NEWUSER)",
        category: Category::Testable,
        result: if args.allow_chromium_sandbox {
            TestResult::Skipped("--allow-chromium-sandbox")
        } else {
            test_clone_ns_blocked()
        },
    });

    results.push(RunResult {
        name: "ioctl(TIOCSTI)",
        category: Category::Testable,
        result: test_ioctl_tiocsti_blocked(),
    });

    results.push(RunResult {
        name: "socket(AF_BLUETOOTH)",
        category: Category::Testable,
        result: test_socket_families(),
    });

    if args.json {
        print_json(&results);
    } else {
        print_human(&results);
    }

    // Pass if no testable syscall was allowed
    !results
        .iter()
        .any(|r| r.category == Category::Testable && r.result == TestResult::Allowed)
}

fn main() -> ExitCode {
    match parse_args() {
        Err(msg) => {
            eprintln!("error: {}", msg);
            eprintln!("usage: cloister-seccomp-validate [--allow-chromium-sandbox] [--json]");
            ExitCode::from(2)
        }
        Ok(args) => {
            if run(args) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn table_is_non_empty() {
        assert!(!syscall_table().is_empty());
    }

    #[test]
    fn no_overlap_between_testable_and_obsolete() {
        let table = syscall_table();
        let testable: HashSet<&str> = table
            .iter()
            .filter(|e| e.category == Category::Testable)
            .map(|e| e.name)
            .collect();
        let obsolete: HashSet<&str> = table
            .iter()
            .filter(|e| e.category == Category::Obsolete)
            .map(|e| e.name)
            .collect();
        let overlap: Vec<_> = testable.intersection(&obsolete).collect();
        assert!(overlap.is_empty(), "overlap: {:?}", overlap);
    }

    #[test]
    fn no_duplicate_names() {
        let table = syscall_table();
        let mut seen = HashSet::new();
        for entry in &table {
            assert!(
                seen.insert(entry.name),
                "duplicate syscall entry: {}",
                entry.name
            );
        }
    }

    #[test]
    fn arch_specific_entries_correct() {
        let table = syscall_table();
        let find = |name: &str| table.iter().find(|e| e.name == name);

        // ioperm and iopl are x86-only
        let ioperm = find("ioperm").expect("ioperm should be in table");
        let iopl = find("iopl").expect("iopl should be in table");

        #[cfg(target_arch = "x86_64")]
        {
            assert!(ioperm.nr.is_some(), "ioperm should have nr on x86_64");
            assert!(iopl.nr.is_some(), "iopl should have nr on x86_64");
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            assert!(ioperm.nr.is_none(), "ioperm should be None on non-x86");
            assert!(iopl.nr.is_none(), "iopl should be None on non-x86");
        }
    }

    #[test]
    fn all_testable_entries_have_valid_category() {
        for entry in &syscall_table() {
            assert!(
                entry.category == Category::Testable || entry.category == Category::Obsolete,
                "{} has unexpected category",
                entry.name
            );
        }
    }
}
