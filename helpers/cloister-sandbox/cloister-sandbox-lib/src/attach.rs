//! Registry of running sandbox instances, used by `cloister-attach`.
//!
//! The launcher writes one record per running sandbox under
//! `$XDG_RUNTIME_DIR/cloister/instances`. Records pin the policy the instance
//! started with, so an attach after a rebuild still uses that policy. The
//! directory is host-only: a sandbox that could write here could choose the
//! policy of a later attach.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceRecord {
    /// Host PID of bwrap inside the sandbox PID namespace. Namespaces are
    /// joined through this process because it is never the (possibly
    /// non-dumpable) application.
    pub sandbox_pid1: u32,
    pub sandbox_pid1_start_time: u64,
    /// Host PID of PID 2 in the sandbox, after its exec. PID 1 still carries the
    /// launcher's unsanitized environment, so the environment comes from here.
    pub main_pid: u32,
    pub main_pid_start_time: u64,
    pub sandbox: String,
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub host_project: Option<String>,
    pub start_dir: String,
    pub launched_at: u64,
    pub shell_argv: Vec<String>,
    /// `None` only when the sandbox runs without seccomp.
    #[serde(default)]
    pub seccomp_filter_path: Option<String>,
    #[serde(default)]
    pub network_namespace: Option<String>,
    #[serde(default)]
    pub netns_helper_path: Option<String>,
}

impl InstanceRecord {
    pub fn is_live(&self) -> bool {
        pid_matches(self.sandbox_pid1, self.sandbox_pid1_start_time)
            && pid_matches(self.main_pid, self.main_pid_start_time)
    }
}

pub fn registry_dir(host_runtime_dir: &str) -> PathBuf {
    Path::new(host_runtime_dir)
        .join("cloister")
        .join("instances")
}

/// Atomically write a record, creating the private registry directory if needed.
pub fn write_record(dir: &Path, record: &InstanceRecord) -> io::Result<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let stem = format!("{}-{}", record.sandbox_pid1, record.sandbox_pid1_start_time);
    let tmp = dir.join(format!(".{stem}.tmp"));
    let path = dir.join(format!("{stem}.json"));
    let body = serde_json::to_vec(record).map_err(io::Error::other)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&body)?;
    drop(file);
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Read every record `is_live` accepts, deleting stale and unreadable ones.
///
/// A launcher killed with SIGKILL cannot remove its own record, so stale
/// entries are expected. Refuses a registry someone else could have written.
pub fn load_live_records<F>(dir: &Path, is_live: F) -> Result<Vec<InstanceRecord>, String>
where
    F: Fn(&InstanceRecord) -> bool,
{
    let meta = match fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    if !meta.is_dir() || meta.mode() & 0o077 != 0 || meta.uid() != current_euid() {
        return Err(format!(
            "refusing registry {}: it must be a directory private to this user",
            dir.display()
        ));
    }

    let mut records = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let parsed = fs::read(&path)
            .ok()
            .and_then(|body| serde_json::from_slice::<InstanceRecord>(&body).ok());
        match parsed {
            Some(record) if is_live(&record) => records.push(record),
            _ => {
                let _ = fs::remove_file(&path);
            }
        }
    }
    records.sort_by_key(|r| (r.launched_at, r.sandbox_pid1));
    Ok(records)
}

/// Parse field 22 (starttime) of `/proc/<pid>/stat`. `comm` is attacker-chosen
/// and may contain spaces and parentheses, so fields are counted from its end.
pub fn parse_start_time(stat: &str) -> Option<u64> {
    let (_, rest) = stat.rsplit_once(')')?;
    // `rest` starts at field 3, so field 22 is at index 19.
    rest.split_ascii_whitespace().nth(19)?.parse().ok()
}

pub fn read_start_time(pid: u32) -> Option<u64> {
    parse_start_time(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// True when `pid` is still the process that had `start_time`, not a reused PID.
pub fn pid_matches(pid: u32, start_time: u64) -> bool {
    read_start_time(pid) == Some(start_time)
}

/// Result of matching running instances against a request.
#[derive(Debug, PartialEq, Eq)]
pub enum Selection<'a> {
    None,
    One(&'a InstanceRecord),
    Ambiguous(Vec<&'a InstanceRecord>),
}

fn project_contains(project: &str, cwd: &str) -> bool {
    let project = project.trim_end_matches('/');
    cwd == project
        || cwd
            .strip_prefix(project)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Pick an instance by sandbox or service name, narrowing by the project that
/// contains `cwd`. With no name, every instance is a candidate.
pub fn select_instance<'a>(
    records: &'a [InstanceRecord],
    name: Option<&str>,
    cwd: &str,
) -> Selection<'a> {
    let candidates: Vec<&InstanceRecord> = records
        .iter()
        .filter(|r| name.is_none_or(|n| r.sandbox == n || r.service.as_deref() == Some(n)))
        .collect();
    let in_cwd: Vec<&InstanceRecord> = candidates
        .iter()
        .copied()
        .filter(|r| {
            r.host_project
                .as_deref()
                .is_some_and(|p| project_contains(p, cwd))
        })
        .collect();

    match (candidates.as_slice(), in_cwd.as_slice()) {
        ([], _) => Selection::None,
        ([only], _) | (_, [only]) => Selection::One(only),
        _ => Selection::Ambiguous(candidates),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AttachArgs {
    pub name: Option<String>,
    pub pid: Option<u32>,
    pub list: bool,
    pub help: bool,
    /// Internal: `cloister-netns` has already moved this process into the
    /// target's named network namespace.
    pub netns_entered: bool,
    pub command: Vec<String>,
}

pub const ATTACH_USAGE: &str = "\
Usage: cloister-attach [<sandbox|service>] [-- command [args...]]
       cloister-attach --pid <pid> [-- command [args...]]
       cloister-attach --list";

/// Parse the arguments that follow `cloister-sandbox attach`.
pub fn parse_attach_args(args: &[String]) -> Result<AttachArgs, String> {
    let mut parsed = AttachArgs::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--" => {
                parsed.command = iter.by_ref().cloned().collect();
                if parsed.command.is_empty() {
                    return Err("-- requires a command".to_string());
                }
            }
            "--list" => parsed.list = true,
            "-h" | "--help" => parsed.help = true,
            "--netns-entered" => parsed.netns_entered = true,
            "--pid" => {
                let value = iter.next().ok_or("--pid requires a value")?;
                let pid = value
                    .parse()
                    .map_err(|_| format!("invalid --pid value: {value}"))?;
                parsed.pid = Some(pid);
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                if parsed.name.replace(other.to_string()).is_some() {
                    return Err(format!("unexpected argument: {other}"));
                }
            }
        }
    }
    if parsed.name.is_some() && parsed.pid.is_some() {
        return Err("give either a name or --pid, not both".to_string());
    }
    Ok(parsed)
}

/// Build the attached process's environment from the sandbox's own
/// (`/proc/<main_pid>/environ`). The bytes are sandbox-controlled: they may only
/// ever be handed to a process that is already inside the sandbox.
pub fn attach_environment(
    sandbox_environ: &[u8],
    term: Option<&str>,
    colorterm: Option<&str>,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = Vec::new();
    let mut set = |key: &OsStr, value: &OsStr| match env.iter_mut().find(|(k, _)| k == key) {
        Some(entry) => entry.1 = value.to_os_string(),
        None => env.push((key.to_os_string(), value.to_os_string())),
    };
    for entry in sandbox_environ.split(|&b| b == 0) {
        if let Some(eq) = entry.iter().position(|&b| b == b'=') {
            if eq > 0 {
                set(
                    OsStr::from_bytes(&entry[..eq]),
                    OsStr::from_bytes(&entry[eq + 1..]),
                );
            }
        }
    }
    if let Some(term) = term {
        set(OsStr::new("TERM"), OsStr::new(term));
    }
    if let Some(colorterm) = colorterm {
        set(OsStr::new("COLORTERM"), OsStr::new(colorterm));
    }
    set(OsStr::new("CLOISTER_ATTACHED"), OsStr::new("1"));
    env
}

/// Number of values a bwrap option consumes, so option values are not mistaken
/// for options. Options not listed here take none.
fn bwrap_option_arity(option: &str) -> usize {
    match option {
        "--bind" | "--bind-try" | "--ro-bind" | "--ro-bind-try" | "--dev-bind"
        | "--dev-bind-try" | "--setenv" | "--symlink" | "--chmod" | "--file" | "--bind-data"
        | "--ro-bind-data" | "--overlay" => 2,
        "--unsetenv" | "--dir" | "--tmpfs" | "--proc" | "--dev" | "--mqueue" | "--remount-ro"
        | "--hostname" | "--uid" | "--gid" | "--chdir" | "--perms" | "--size" | "--seccomp"
        | "--add-seccomp-fd" | "--overlay-src" | "--tmp-overlay" | "--ro-overlay" | "--args"
        | "--exec-label" | "--file-label" | "--lock-file" | "--sync-fd" | "--info-fd"
        | "--json-status-fd" | "--block-fd" | "--userns-block-fd" | "--userns" | "--userns2"
        | "--pidns" | "--cap-add" | "--cap-drop" => 1,
        _ => 0,
    }
}

fn is_host_source_option(option: &str) -> bool {
    matches!(
        option,
        "--bind"
            | "--bind-try"
            | "--ro-bind"
            | "--ro-bind-try"
            | "--dev-bind"
            | "--dev-bind-try"
            | "--overlay-src"
    )
}

/// True when bwrap arguments bind the registry, or anything containing it or
/// inside it, into the sandbox.
pub fn bwrap_args_expose_registry(args: &[String], registry: &Path) -> bool {
    let mut i = 0;
    while let Some(option) = args.get(i) {
        if is_host_source_option(option) {
            if let Some(source) = args.get(i + 1) {
                let source = Path::new(source);
                if registry.starts_with(source) || source.starts_with(registry) {
                    return true;
                }
            }
        }
        i += 1 + bwrap_option_arity(option);
    }
    false
}

fn current_euid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn record(pid: u32, start_time: u64) -> InstanceRecord {
        InstanceRecord {
            sandbox_pid1: pid,
            sandbox_pid1_start_time: start_time,
            main_pid: pid + 1,
            main_pid_start_time: start_time + 1,
            sandbox: "dev".to_string(),
            service: None,
            host_project: Some("/home/u/project".to_string()),
            start_dir: "/home/u/project".to_string(),
            launched_at: 1_700_000_000,
            shell_argv: vec!["/nix/store/x/bin/zsh".to_string(), "-i".to_string()],
            seccomp_filter_path: Some("/nix/store/x/filter.bpf".to_string()),
            network_namespace: None,
            netns_helper_path: None,
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cloister-attach-test-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn registered_instance_is_listed_and_private() {
        let dir = temp_dir("roundtrip");
        let path = write_record(&dir, &record(10, 100)).unwrap();

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(
            load_live_records(&dir, |_| true).unwrap(),
            vec![record(10, 100)]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_and_corrupt_records_are_pruned_on_read() {
        let dir = temp_dir("stale");
        let live = write_record(&dir, &record(10, 100)).unwrap();
        let stale = write_record(&dir, &record(20, 200)).unwrap();
        let corrupt = dir.join("garbage.json");
        std::fs::write(&corrupt, "{not json").unwrap();

        let found = load_live_records(&dir, |r| r.sandbox_pid1 == 10).unwrap();

        assert_eq!(found, vec![record(10, 100)]);
        assert!(live.exists());
        assert!(!stale.exists());
        assert!(!corrupt.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn group_accessible_registry_is_refused() {
        let dir = temp_dir("perms");
        write_record(&dir, &record(10, 100)).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o770)).unwrap();

        assert!(load_live_records(&dir, |_| true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_registry_means_no_instances() {
        let dir = temp_dir("missing");
        assert_eq!(load_live_records(&dir, |_| true).unwrap(), vec![]);
    }

    #[test]
    fn start_time_is_parsed_past_a_hostile_comm() {
        // Field 22 of /proc/<pid>/stat. comm may contain spaces and parens.
        let mut stat = String::from("42 (a) b) (c) S");
        for field in 4..=21 {
            stat.push_str(&format!(" {field}"));
        }
        stat.push_str(" 987654 23 24");
        assert_eq!(parse_start_time(&stat), Some(987654));
        assert_eq!(parse_start_time("42 (truncated) S 1 2"), None);
    }

    #[test]
    fn current_process_start_time_is_stable_and_a_mismatch_is_not_live() {
        let pid = std::process::id();
        let start = read_start_time(pid).unwrap();
        assert_eq!(read_start_time(pid), Some(start));
        assert!(pid_matches(pid, start));
        // Same PID, different start time: the PID was reused.
        assert!(!pid_matches(pid, start + 1));
    }

    fn at(pid: u32, sandbox: &str, service: Option<&str>, project: Option<&str>) -> InstanceRecord {
        InstanceRecord {
            sandbox: sandbox.to_string(),
            service: service.map(str::to_string),
            host_project: project.map(str::to_string),
            ..record(pid, 100)
        }
    }

    fn pids(selection: &Selection) -> Vec<u32> {
        match selection {
            Selection::None => vec![],
            Selection::One(r) => vec![r.sandbox_pid1],
            Selection::Ambiguous(rs) => rs.iter().map(|r| r.sandbox_pid1).collect(),
        }
    }

    #[test]
    fn name_matches_sandbox_or_service() {
        let all = vec![
            at(1, "agents", Some("paseo"), Some("/srv/paseo")),
            at(2, "browser", None, None),
        ];
        assert!(
            matches!(select_instance(&all, Some("paseo"), "/x"), Selection::One(r) if r.sandbox_pid1 == 1)
        );
        assert!(
            matches!(select_instance(&all, Some("agents"), "/x"), Selection::One(r) if r.sandbox_pid1 == 1)
        );
        assert!(
            matches!(select_instance(&all, Some("browser"), "/x"), Selection::One(r) if r.sandbox_pid1 == 2)
        );
        assert!(matches!(
            select_instance(&all, Some("nope"), "/x"),
            Selection::None
        ));
    }

    #[test]
    fn several_matches_prefer_the_one_containing_the_current_directory() {
        let all = vec![
            at(1, "dev", None, Some("/home/u/a")),
            at(2, "dev", None, Some("/home/u/b")),
        ];
        assert_eq!(
            pids(&select_instance(&all, Some("dev"), "/home/u/b/src")),
            vec![2]
        );
        assert_eq!(
            pids(&select_instance(&all, Some("dev"), "/home/u/c")),
            vec![1, 2]
        );
        assert!(matches!(
            select_instance(&all, Some("dev"), "/home/u/c"),
            Selection::Ambiguous(_)
        ));
    }

    #[test]
    fn project_match_respects_path_boundaries() {
        let all = vec![
            at(1, "dev", None, Some("/home/u/a")),
            at(2, "dev", None, Some("/home/u/ab")),
        ];
        assert_eq!(
            pids(&select_instance(&all, Some("dev"), "/home/u/ab")),
            vec![2]
        );
    }

    #[test]
    fn no_name_uses_the_current_directory_then_every_instance() {
        let all = vec![
            at(1, "dev", None, Some("/home/u/a")),
            at(2, "browser", None, None),
        ];
        assert_eq!(pids(&select_instance(&all, None, "/home/u/a/src")), vec![1]);
        assert_eq!(pids(&select_instance(&all, None, "/elsewhere")), vec![1, 2]);
        assert!(matches!(
            select_instance(&all[1..], None, "/elsewhere"),
            Selection::One(_)
        ));
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn attach_args_cover_the_documented_forms() {
        let parsed = parse_attach_args(&args(&[])).unwrap();
        assert_eq!(parsed, AttachArgs::default());

        let parsed = parse_attach_args(&args(&["dev", "--", "cat", "--flag"])).unwrap();
        assert_eq!(parsed.name.as_deref(), Some("dev"));
        assert_eq!(parsed.command, args(&["cat", "--flag"]));

        let parsed = parse_attach_args(&args(&["--pid", "42", "--", "id"])).unwrap();
        assert_eq!(parsed.pid, Some(42));
        assert_eq!(parsed.command, args(&["id"]));

        assert!(parse_attach_args(&args(&["--list"])).unwrap().list);
        assert!(
            !parse_attach_args(&args(&["--netns-entered", "--pid", "4"]))
                .unwrap()
                .list
        );
        assert!(
            parse_attach_args(&args(&["--netns-entered", "--pid", "4"]))
                .unwrap()
                .netns_entered
        );
    }

    #[test]
    fn attach_args_reject_misuse() {
        assert!(parse_attach_args(&args(&["--bogus"])).is_err());
        assert!(parse_attach_args(&args(&["a", "b"])).is_err());
        assert!(parse_attach_args(&args(&["--pid"])).is_err());
        assert!(parse_attach_args(&args(&["--pid", "x"])).is_err());
        assert!(parse_attach_args(&args(&["dev", "--pid", "4"])).is_err());
        assert!(parse_attach_args(&args(&["--"])).is_err());
    }

    #[test]
    fn environment_comes_from_the_sandbox_with_terminal_overrides() {
        let env = attach_environment(
            b"PATH=/sandbox/bin\0TERM=dumb\0SECRET=in-sandbox\0ignored-no-equals\0",
            Some("xterm-kitty"),
            None,
        );
        let get = |k: &str| {
            env.iter()
                .find(|(key, _)| key.as_os_str() == k)
                .map(|(_, v)| v.to_str().unwrap().to_string())
        };
        assert_eq!(get("PATH").as_deref(), Some("/sandbox/bin"));
        assert_eq!(get("TERM").as_deref(), Some("xterm-kitty"));
        assert_eq!(get("COLORTERM"), None);
        assert_eq!(get("SECRET").as_deref(), Some("in-sandbox"));
        assert_eq!(get("CLOISTER_ATTACHED").as_deref(), Some("1"));
        assert_eq!(
            env.iter().filter(|(k, _)| k.as_os_str() == "TERM").count(),
            1
        );
        assert_eq!(env.len(), 4);
    }

    #[test]
    fn environment_keeps_the_sandbox_term_when_the_caller_has_none() {
        let env = attach_environment(b"TERM=dumb\0", None, Some("truecolor"));
        assert!(
            env.iter()
                .any(|(k, v)| k.as_os_str() == "TERM" && v.as_os_str() == "dumb")
        );
        assert!(
            env.iter()
                .any(|(k, v)| k.as_os_str() == "COLORTERM" && v.as_os_str() == "truecolor")
        );
    }

    #[test]
    fn binds_covering_the_registry_are_detected() {
        let dir = Path::new("/run/user/1000/cloister/instances");
        let exposes = |a: &[&str]| bwrap_args_expose_registry(&args(a), dir);

        assert!(exposes(&["--bind", "/run/user/1000", "/run/user/1000"]));
        assert!(exposes(&["--ro-bind", "/run", "/x"]));
        assert!(exposes(&[
            "--ro-bind-try",
            "/run/user/1000/cloister/instances/1.json",
            "/x"
        ]));
        assert!(exposes(&["--dev-bind", "/", "/"]));
        // A socket elsewhere under the runtime dir is the normal case.
        assert!(!exposes(&[
            "--bind",
            "/run/user/1000/cloister/dbus/x",
            "/x"
        ]));
        assert!(!exposes(&["--bind", "/run/user/1000/wayland-1", "/x"]));
        // Destinations and values of other options are not sources.
        assert!(!exposes(&["--tmpfs", "/run"]));
        assert!(!exposes(&["--setenv", "--bind", "/run", "--dir", "/x"]));
    }
}
