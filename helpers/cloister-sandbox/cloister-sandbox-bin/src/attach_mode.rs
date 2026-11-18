//! `cloister-sandbox attach`: run a shell or command inside a running sandbox.
//!
//! Process layout:
//!
//! ```text
//! relay (host namespaces; owns the terminal)
//!   └─ joiner (joined namespaces, no capabilities, non-dumpable; outside the
//!      │        sandbox PID namespace, so sandbox processes cannot see it)
//!      └─ attached process (sandbox PID namespace, target's seccomp filter)
//! ```
//!
//! Host terminal descriptors stop at the relay. The sandbox only ever gets the
//! inner end of a fresh PTY, so a compromised process there cannot reach the
//! host terminal through the attached shell.

use std::ffi::CString;
use std::fs;
use std::io::{self, BufRead, Write};
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use cloister_sandbox_lib::attach::{
    ATTACH_USAGE, AttachArgs, InstanceRecord, Selection, attach_environment, load_live_records,
    parse_attach_args, pid_matches, registry_dir, select_instance,
};

/// Namespaces joined, compared one by one so that shared ones are skipped.
const NAMESPACES: [(&str, libc::c_int); 7] = [
    ("user", libc::CLONE_NEWUSER),
    ("mnt", libc::CLONE_NEWNS),
    ("pid", libc::CLONE_NEWPID),
    ("ipc", libc::CLONE_NEWIPC),
    ("uts", libc::CLONE_NEWUTS),
    ("cgroup", libc::CLONE_NEWCGROUP),
    ("net", libc::CLONE_NEWNET),
];

const SECCOMP_MODE_FILTER: libc::c_ulong = 2;
const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;

static TERMINATE: AtomicBool = AtomicBool::new(false);
static WINCH: AtomicBool = AtomicBool::new(false);

pub fn run(args: &[String]) -> i32 {
    match run_inner(args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("cloister-attach: {message}");
            1
        }
    }
}

fn run_inner(args: &[String]) -> Result<i32, String> {
    let parsed = parse_attach_args(args).map_err(|e| format!("{e}\n{ATTACH_USAGE}"))?;
    if parsed.help {
        println!("{ATTACH_USAGE}");
        return Ok(0);
    }
    ensure_stdio_open()?;

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|dir| !dir.is_empty())
        .ok_or("XDG_RUNTIME_DIR must be set")?;
    let records = load_live_records(&registry_dir(&runtime_dir), InstanceRecord::is_live)?;

    if parsed.list {
        print_list(&records);
        return Ok(0);
    }
    let record = choose_record(&records, &parsed)?;
    attach_to(record, &parsed)
}

fn age(record: &InstanceRecord) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs = now.saturating_sub(record.launched_at);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

fn describe(record: &InstanceRecord) -> String {
    format!(
        "{:<8} {:<16} {:<16} {:<6} {}",
        record.sandbox_pid1,
        record.sandbox,
        record.service.as_deref().unwrap_or("-"),
        age(record),
        record.host_project.as_deref().unwrap_or("-"),
    )
}

fn print_list(records: &[InstanceRecord]) {
    println!(
        "{:<8} {:<16} {:<16} {:<6} PROJECT",
        "PID", "SANDBOX", "SERVICE", "AGE"
    );
    for record in records {
        println!("{}", describe(record));
    }
}

fn choose_record<'a>(
    records: &'a [InstanceRecord],
    parsed: &AttachArgs,
) -> Result<&'a InstanceRecord, String> {
    if let Some(pid) = parsed.pid {
        return records
            .iter()
            .find(|r| r.sandbox_pid1 == pid || r.main_pid == pid)
            .ok_or_else(|| format!("no running sandbox has PID {pid}"));
    }
    let cwd = std::env::current_dir()
        .map_err(|e| format!("cannot read the current directory: {e}"))?
        .to_string_lossy()
        .into_owned();
    match select_instance(records, parsed.name.as_deref(), &cwd) {
        Selection::None => Err(match parsed.name.as_deref() {
            Some(name) => format!("no running sandbox or service named {name}"),
            None => "no running sandboxes".to_string(),
        }),
        Selection::One(record) => Ok(record),
        Selection::Ambiguous(candidates) => pick(&candidates),
    }
}

fn pick<'a>(candidates: &[&'a InstanceRecord]) -> Result<&'a InstanceRecord, String> {
    let listing = candidates
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{:>2}) {}", i + 1, describe(r)))
        .collect::<Vec<_>>()
        .join("\n");
    if !is_tty(0) || !is_tty(2) {
        return Err(format!(
            "several sandboxes match; choose one with --pid:\n{listing}"
        ));
    }
    eprintln!("{listing}");
    eprint!("Attach to which instance? ");
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("read selection: {e}"))?;
    line.trim()
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| candidates.get(i).copied())
        .ok_or_else(|| "no instance selected".to_string())
}

fn read_ns(pid: &str, name: &str) -> Result<std::path::PathBuf, String> {
    fs::read_link(format!("/proc/{pid}/ns/{name}"))
        .map_err(|e| format!("cannot inspect the {name} namespace of PID {pid}: {e}"))
}

/// Everything the joiner needs, gathered while still fully on the host: nothing
/// past `setns` may read host paths, because they then resolve in the sandbox.
struct Prepared {
    pidfd: RawFd,
    ns_flags: libc::c_int,
    env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    filter: Option<Vec<libc::sock_filter>>,
    argv: Vec<String>,
    start_dir: String,
}

fn attach_to(record: &InstanceRecord, parsed: &AttachArgs) -> Result<i32, String> {
    let pid = record.sandbox_pid1;
    let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) } as RawFd;
    if pidfd < 0 {
        return Err(format!("pidfd_open {pid}: {}", io::Error::last_os_error()));
    }
    // The pidfd now pins one process; check it is still the recorded one.
    if !pid_matches(pid, record.sandbox_pid1_start_time) {
        return Err(format!("sandbox PID {pid} has exited"));
    }

    let target = pid.to_string();
    let mut ns_flags = 0;
    for (name, flag) in NAMESPACES {
        if read_ns("self", name)? != read_ns(&target, name)? {
            ns_flags |= flag;
        } else if name == "user" {
            // Every sandbox has its own user namespace. Sharing ours means this
            // is not a sandbox, and running the command would run it on the host.
            return Err(format!(
                "PID {pid} shares this process's user namespace; refusing to attach"
            ));
        }
    }

    if ns_flags & libc::CLONE_NEWNET != 0 {
        if let Some(namespace) = record.network_namespace.as_deref() {
            if parsed.netns_entered {
                return Err(format!(
                    "still outside network namespace {namespace} after cloister-netns"
                ));
            }
            return Err(reexec_through_netns(record, namespace, parsed));
        }
    }

    let filter = match record.seccomp_filter_path.as_deref() {
        Some(path) => Some(load_seccomp_filter(path).map_err(|e| {
            format!("cannot load this instance's seccomp filter ({path}): {e}; restart the sandbox to attach")
        })?),
        None => None,
    };

    // Read after the start-time check above and verified again below, so a
    // reused PID cannot supply the environment.
    let environ = fs::read(format!("/proc/{}/environ", record.main_pid))
        .map_err(|e| format!("cannot read the sandbox environment: {e}"))?;
    if !pid_matches(record.main_pid, record.main_pid_start_time) {
        return Err("the sandbox main process has exited".to_string());
    }
    let term = std::env::var("TERM").ok();
    let colorterm = std::env::var("COLORTERM").ok();
    let env = attach_environment(&environ, term.as_deref(), colorterm.as_deref());

    let argv = if parsed.command.is_empty() {
        record.shell_argv.clone()
    } else {
        parsed.command.clone()
    };
    if argv.is_empty() {
        return Err("this instance has no shell configured".to_string());
    }

    relay_and_join(Prepared {
        pidfd,
        ns_flags,
        env,
        filter,
        argv,
        start_dir: record.start_dir.clone(),
    })
}

/// Named network namespaces belong to the host's user namespace, so entering one
/// takes the capability `cloister-netns` holds. It drops that capability and
/// re-executes this binary, the only target its allowlist accepts.
fn reexec_through_netns(record: &InstanceRecord, namespace: &str, parsed: &AttachArgs) -> String {
    let Some(helper) = record.netns_helper_path.as_deref() else {
        return format!("no cloister-netns helper recorded for network namespace {namespace}");
    };
    let self_exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => return format!("cannot determine own path: {e}"),
    };
    let mut cmd = Command::new(helper);
    cmd.args(["--netns", namespace, "--"])
        .arg(self_exe)
        .args(["attach", "--netns-entered", "--pid"])
        .arg(record.sandbox_pid1.to_string());
    if !parsed.command.is_empty() {
        cmd.arg("--").args(&parsed.command);
    }
    format!("exec {helper}: {}", cmd.exec())
}

fn load_seccomp_filter(path: &str) -> Result<Vec<libc::sock_filter>, String> {
    let bytes = fs::read(Path::new(path)).map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() % 8 != 0 {
        return Err(format!("invalid filter length {}", bytes.len()));
    }
    // A truncated length would load only part of the filter, so refuse it here.
    if u16::try_from(bytes.len() / 8).is_err() {
        return Err(format!("filter too long: {} bytes", bytes.len()));
    }
    let (instructions, _) = bytes.as_chunks::<8>();
    Ok(instructions
        .iter()
        .map(|c| libc::sock_filter {
            code: u16::from_ne_bytes([c[0], c[1]]),
            jt: c[2],
            jf: c[3],
            k: u32::from_ne_bytes([c[4], c[5], c[6], c[7]]),
        })
        .collect())
}

fn is_tty(fd: RawFd) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

/// A closed standard stream must not let a later descriptor land on 0, 1 or 2.
fn ensure_stdio_open() -> Result<(), String> {
    for fd in 0..=2 {
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
            let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDWR) };
            if null != fd {
                return Err(format!("cannot open /dev/null as descriptor {fd}"));
            }
        }
    }
    Ok(())
}

struct Pty {
    master: RawFd,
    slave: RawFd,
}

fn open_pty() -> Result<Pty, String> {
    let (mut master, mut slave) = (-1, -1);
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if rc != 0 {
        return Err(format!("openpty: {}", io::Error::last_os_error()));
    }
    unsafe {
        libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(master, libc::F_SETFL, libc::O_NONBLOCK);
    }
    Ok(Pty { master, slave })
}

fn copy_window_size(from: RawFd, to: RawFd) {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    unsafe {
        if libc::ioctl(from, libc::TIOCGWINSZ, &mut size) == 0 {
            libc::ioctl(to, libc::TIOCSWINSZ, &size);
        }
    }
}

/// A background job may not read the terminal or change its mode: the kernel
/// would stop it with SIGTTIN or SIGTTOU and the attach would hang.
fn detach_background_stdin() -> Result<(), String> {
    if is_tty(0) && unsafe { libc::tcgetpgrp(0) != libc::getpgrp() } {
        let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
        check(null as libc::c_long, "open /dev/null")?;
        check(unsafe { libc::dup2(null, 0) } as libc::c_long, "dup2")?;
        unsafe { libc::close(null) };
    }
    Ok(())
}

fn relay_and_join(prepared: Prepared) -> Result<i32, String> {
    detach_background_stdin()?;
    let tty_fds: Vec<RawFd> = (0..=2).filter(|&fd| is_tty(fd)).collect();
    let pty = if tty_fds.is_empty() {
        None
    } else {
        Some(open_pty()?)
    };
    // Output goes to the first terminal stream; stdin is the fallback for
    // `cloister-attach … < /dev/tty > file`, where only input is a terminal.
    let output_fd = [1, 2, 0].into_iter().find(|fd| tty_fds.contains(fd));
    if let (Some(pty), Some(fd)) = (&pty, output_fd) {
        copy_window_size(fd, pty.master);
    }
    let _ = io::stdout().flush();
    let pidfd = prepared.pidfd;

    let joiner = unsafe { libc::fork() };
    if joiner < 0 {
        return Err(format!("fork: {}", io::Error::last_os_error()));
    }
    if joiner == 0 {
        // The joiner must never return into host-side code.
        let code = match run_joiner(prepared, pty.as_ref(), &tty_fds) {
            Ok(code) => code,
            Err(message) => {
                eprintln!("cloister-attach: {message}");
                1
            }
        };
        unsafe { libc::_exit(code) };
    }

    let status = match (&pty, output_fd) {
        (Some(pty), Some(output_fd)) => {
            unsafe { libc::close(pty.slave) };
            relay(pty.master, output_fd, is_tty(0), joiner)
        }
        _ => wait_for(joiner),
    };
    if status == 128 + libc::SIGKILL && pidfd_signaled(pidfd) {
        eprintln!("cloister-attach: sandbox exited");
    }
    Ok(status)
}

/// True when the sandbox itself ended. Its PID namespace is torn down just before
/// its PID 1 reports exit, so a kill by that teardown needs a moment to show.
fn pidfd_signaled(pidfd: RawFd) -> bool {
    let mut poll_fd = libc::pollfd {
        fd: pidfd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut poll_fd, 1, 500) > 0 }
}

fn exit_code(status: libc::c_int) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        128 + libc::WTERMSIG(status)
    }
}

fn wait_for(pid: libc::pid_t) -> i32 {
    let mut status = 0;
    loop {
        let rc = unsafe { libc::waitpid(pid, &mut status, 0) };
        if rc == pid {
            return exit_code(status);
        }
        if rc < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return 1;
        }
    }
}

extern "C" fn on_terminate(_: libc::c_int) {
    TERMINATE.store(true, Ordering::Relaxed);
}

extern "C" fn on_winch(_: libc::c_int) {
    WINCH.store(true, Ordering::Relaxed);
}

fn install_handler(signal: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = handler as *const () as usize;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}

/// Copy bytes between the host terminal and the PTY until the joiner exits.
fn relay(master: RawFd, output_fd: RawFd, stdin_is_tty: bool, joiner: libc::pid_t) -> i32 {
    install_handler(libc::SIGINT, on_terminate);
    install_handler(libc::SIGTERM, on_terminate);
    install_handler(libc::SIGHUP, on_terminate);
    install_handler(libc::SIGWINCH, on_winch);
    // Backgrounded mid-session: restoring the terminal must not stop the relay.
    unsafe {
        libc::signal(libc::SIGTTOU, libc::SIG_IGN);
        libc::signal(libc::SIGTTIN, libc::SIG_IGN);
    }

    let saved_termios = stdin_is_tty.then(|| {
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        unsafe { libc::tcgetattr(0, &mut saved) };
        let mut raw = saved;
        unsafe {
            libc::cfmakeraw(&mut raw);
            libc::tcsetattr(0, libc::TCSANOW, &raw);
        }
        saved
    });

    let code = relay_loop(master, output_fd, stdin_is_tty, joiner);

    if let Some(saved) = saved_termios {
        unsafe { libc::tcsetattr(0, libc::TCSADRAIN, &saved) };
    }
    code
}

fn write_all_blocking(fd: RawFd, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if n > 0 {
            bytes = &bytes[n as usize..];
        } else if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return;
        }
    }
}

/// Drain whatever the PTY holds. Returns false once it has hung up.
fn pump_output(master: RawFd, output_fd: RawFd, buf: &mut [u8]) -> bool {
    loop {
        let n = unsafe { libc::read(master, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            write_all_blocking(output_fd, &buf[..n as usize]);
            continue;
        }
        let err = io::Error::last_os_error();
        return n < 0
            && matches!(
                err.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            );
    }
}

fn relay_loop(master: RawFd, output_fd: RawFd, stdin_is_tty: bool, joiner: libc::pid_t) -> i32 {
    let mut buf = [0u8; 8192];
    let mut pending: Vec<u8> = Vec::new();
    let mut stdin_open = stdin_is_tty;
    let mut master_open = true;

    loop {
        if TERMINATE.swap(false, Ordering::Relaxed) {
            unsafe { libc::kill(joiner, libc::SIGKILL) };
            return wait_for(joiner);
        }
        if WINCH.swap(false, Ordering::Relaxed) {
            copy_window_size(output_fd, master);
        }

        let mut status = 0;
        if unsafe { libc::waitpid(joiner, &mut status, libc::WNOHANG) } == joiner {
            if master_open {
                pump_output(master, output_fd, &mut buf);
            }
            return exit_code(status);
        }

        let mut fds = [
            libc::pollfd {
                fd: if stdin_open && pending.is_empty() {
                    0
                } else {
                    -1
                },
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if master_open { master } else { -1 },
                events: libc::POLLIN | if pending.is_empty() { 0 } else { libc::POLLOUT },
                revents: 0,
            },
        ];
        // The timeout bounds the window where a signal lands just before poll.
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, 200) } <= 0 {
            continue;
        }

        if fds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            master_open = pump_output(master, output_fd, &mut buf);
        }
        if fds[0].revents != 0 {
            let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                pending.extend_from_slice(&buf[..n as usize]);
            } else if n == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                stdin_open = false;
            }
        }
        if !pending.is_empty() && master_open {
            let n = unsafe { libc::write(master, pending.as_ptr().cast(), pending.len()) };
            if n > 0 {
                pending.drain(..n as usize);
            }
        }
    }
}

fn check(rc: libc::c_long, what: &str) -> Result<(), String> {
    if rc < 0 {
        return Err(format!("{what}: {}", io::Error::last_os_error()));
    }
    Ok(())
}

fn prctl(option: libc::c_int, arg: libc::c_ulong, what: &str) -> Result<(), String> {
    check(
        unsafe {
            libc::prctl(
                option,
                arg,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            )
        } as libc::c_long,
        what,
    )
}

/// Keep sandbox processes out of this process and tie its life to the parent's.
/// Installing new credentials resets both, so this runs again after `setns`.
fn harden_self(parent: libc::pid_t) -> Result<(), String> {
    prctl(libc::PR_SET_DUMPABLE, 0, "PR_SET_DUMPABLE")?;
    prctl(
        libc::PR_SET_PDEATHSIG,
        libc::SIGKILL as libc::c_ulong,
        "PR_SET_PDEATHSIG",
    )?;
    if unsafe { libc::getppid() } != parent {
        return Err("parent exited during setup".to_string());
    }
    Ok(())
}

fn close_host_descriptors(keep: RawFd) -> Result<(), String> {
    let close_range = |first: u32, last: u32| {
        check(
            unsafe { libc::syscall(libc::SYS_close_range, first, last, 0u32) },
            "close_range",
        )
    };
    if keep > 3 {
        close_range(3, keep as u32 - 1)?;
    }
    close_range(keep as u32 + 1, u32::MAX)
}

fn drop_capabilities() -> Result<(), String> {
    for cap in 0..64 {
        let rc = unsafe {
            libc::prctl(
                libc::PR_CAPBSET_DROP,
                cap as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            )
        };
        // EINVAL marks a capability this kernel does not have.
        if rc != 0 && io::Error::last_os_error().raw_os_error() != Some(libc::EINVAL) {
            return Err(format!(
                "PR_CAPBSET_DROP {cap}: {}",
                io::Error::last_os_error()
            ));
        }
    }
    check(
        unsafe {
            libc::prctl(
                libc::PR_CAP_AMBIENT,
                libc::PR_CAP_AMBIENT_CLEAR_ALL as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            )
        } as libc::c_long,
        "PR_CAP_AMBIENT_CLEAR_ALL",
    )?;

    #[repr(C)]
    struct Header {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Data {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let mut header = Header {
        version: LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };
    let data = [Data::default(); 2];
    check(
        unsafe { libc::syscall(libc::SYS_capset, &mut header as *mut Header, data.as_ptr()) },
        "capset",
    )
}

fn run_joiner(prepared: Prepared, pty: Option<&Pty>, tty_fds: &[RawFd]) -> Result<i32, String> {
    let relay_pid = unsafe { libc::getppid() };
    harden_self(relay_pid)?;
    // Leaves the host terminal's session, so /dev/tty cannot reach it either.
    check(unsafe { libc::setsid() } as libc::c_long, "setsid")?;

    if let Some(pty) = pty {
        for &fd in tty_fds {
            check(unsafe { libc::dup2(pty.slave, fd) } as libc::c_long, "dup2")?;
        }
    }
    close_host_descriptors(prepared.pidfd)?;

    check(
        unsafe { libc::setns(prepared.pidfd, prepared.ns_flags) } as libc::c_long,
        "setns",
    )?;
    harden_self(relay_pid)?;
    unsafe { libc::close(prepared.pidfd) };
    drop_capabilities()?;
    prctl(libc::PR_SET_NO_NEW_PRIVS, 1, "PR_SET_NO_NEW_PRIVS")?;

    // The PID namespace applies to children, so the command needs one more fork.
    // Seen from a joined PID namespace, the parent is outside it and reads as 0.
    let expected_parent = if prepared.ns_flags & libc::CLONE_NEWPID != 0 {
        0
    } else {
        unsafe { libc::getpid() }
    };
    let child = unsafe { libc::fork() };
    if child < 0 {
        return Err(format!("fork: {}", io::Error::last_os_error()));
    }
    if child > 0 {
        return Ok(wait_for(child));
    }

    let code = match exec_attached(prepared, tty_fds, expected_parent) {
        Ok(never) => match never {},
        Err(message) => {
            eprintln!("cloister-attach: {message}");
            127
        }
    };
    unsafe { libc::_exit(code) };
}

fn exec_attached(
    prepared: Prepared,
    tty_fds: &[RawFd],
    expected_parent: libc::pid_t,
) -> Result<std::convert::Infallible, String> {
    prctl(
        libc::PR_SET_PDEATHSIG,
        libc::SIGKILL as libc::c_ulong,
        "PR_SET_PDEATHSIG",
    )?;
    if unsafe { libc::getppid() } != expected_parent {
        return Err("joiner exited during setup".to_string());
    }
    check(unsafe { libc::setsid() } as libc::c_long, "setsid")?;
    if let Some(&fd) = tty_fds.first() {
        check(
            unsafe { libc::ioctl(fd, libc::TIOCSCTTY, 0) } as libc::c_long,
            "TIOCSCTTY",
        )?;
    }

    change_directory(&prepared.start_dir, &prepared.env);

    if let Some(filter) = &prepared.filter {
        let program = libc::sock_fprog {
            // load_seccomp_filter already rejected anything longer.
            len: u16::try_from(filter.len()).map_err(|e| format!("seccomp filter: {e}"))?,
            filter: filter.as_ptr().cast_mut(),
        };
        check(
            unsafe {
                libc::prctl(
                    libc::PR_SET_SECCOMP,
                    SECCOMP_MODE_FILTER,
                    &program as *const libc::sock_fprog,
                )
            } as libc::c_long,
            "PR_SET_SECCOMP",
        )?;
    }

    let mut command = Command::new(&prepared.argv[0]);
    command
        .args(&prepared.argv[1..])
        .env_clear()
        .envs(prepared.env.iter().map(|(k, v)| (k, v)));
    Err(format!("exec {}: {}", prepared.argv[0], command.exec()))
}

/// Start in the project, or at `$HOME` for sandboxes without one.
fn change_directory(start_dir: &str, env: &[(std::ffi::OsString, std::ffi::OsString)]) {
    let home = env
        .iter()
        .find(|(k, _)| k.as_os_str() == "HOME")
        .map(|(_, v)| v.as_bytes().to_vec());
    let candidates = [
        Some(start_dir.as_bytes().to_vec()),
        home,
        Some(b"/".to_vec()),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Ok(path) = CString::new(candidate) {
            if unsafe { libc::chdir(path.as_ptr()) } == 0 {
                return;
            }
        }
    }
}
