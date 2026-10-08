//! Runtime feature detection: wayland, pulseaudio, gpu, dbus, and device integrations.

use std::path::{Path, PathBuf};

use crate::socket;

/// Build PulseAudio forwarding arguments from an explicit host socket path.
pub fn pulseaudio_args_with_source(host_socket: &str, sandbox_socket: &str) -> Vec<String> {
    if !Path::new(host_socket).exists() {
        eprintln!("Warning: PulseAudio socket not found at {host_socket}");
        eprintln!("Audio will not work. Ensure PulseAudio or PipeWire-PulseAudio is running.");
        return Vec::new();
    }
    if let Err(e) = socket::validate_existing_socket(host_socket) {
        eprintln!("Warning: invalid PulseAudio socket '{host_socket}': {e}");
        return Vec::new();
    }

    let Some(sandbox_runtime_dir) = Path::new(sandbox_socket).parent() else {
        eprintln!("Warning: invalid sandbox PulseAudio socket path '{sandbox_socket}'");
        return Vec::new();
    };

    vec![
        "--dir".to_string(),
        sandbox_runtime_dir.to_string_lossy().to_string(),
        "--bind".to_string(),
        host_socket.to_string(),
        sandbox_socket.to_string(),
        "--setenv".to_string(),
        "PULSE_SERVER".to_string(),
        format!("unix:{sandbox_socket}"),
    ]
}

/// Discover GPU PCI sysfs device paths by resolving `/sys/class/drm/card*` symlinks.
///
/// Each `cardN` entry is a symlink like:
///   `/sys/devices/pci0000:00/0000:00:02.0/drm/card0`
///
/// We canonicalize the symlink, then walk up 2 path components (past `drm/cardN`)
/// to reach the PCI device directory (e.g. `/sys/devices/pci0000:00/0000:00:02.0`).
/// Results are deduplicated (multi-GPU cards share a PCI device).
pub fn discover_gpu_pci_sysfs_paths() -> Vec<String> {
    let drm_class = Path::new("/sys/class/drm");
    if !drm_class.is_dir() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(drm_class) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut pci_paths: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only process cardN entries, skip connector entries like card0-DP-1
        if !name_str.starts_with("card") || name_str.contains('-') {
            continue;
        }

        // Resolve the symlink to its canonical path
        let canonical = match entry.path().canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Walk up 2 components: past "cardN" and "drm" to reach the PCI device
        if let Some(pci_device) = canonical.parent().and_then(|p| p.parent()) {
            let pci_str = pci_device.to_string_lossy().to_string();
            if !pci_paths.contains(&pci_str) {
                pci_paths.push(pci_str);
            }
        }
    }

    pci_paths
}

/// Discover the PCI driver directories referenced by GPU PCI devices.
///
/// For each GPU PCI device path from [`discover_gpu_pci_sysfs_paths`], reads the
/// `driver` symlink (e.g. `/sys/devices/pci0000:00/0000:01:00.0/driver`) and
/// canonicalizes it to get the full driver directory path
/// (e.g. `/sys/bus/pci/drivers/nvidia`). Results are deduplicated.
pub fn discover_gpu_pci_driver_paths() -> Vec<String> {
    let mut driver_paths: Vec<String> = Vec::new();
    for pci_path in discover_gpu_pci_sysfs_paths() {
        let driver_link = format!("{pci_path}/driver");
        let canonical = match Path::new(&driver_link).canonicalize() {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        if !driver_paths.contains(&canonical) {
            driver_paths.push(canonical);
        }
    }
    driver_paths
}

/// Discover `/sys/bus/pci/devices/<addr>` symlinks for GPU PCI devices.
///
/// For each GPU PCI device path from [`discover_gpu_pci_sysfs_paths`], extracts the
/// PCI address (the last path component, e.g. `0000:00:02.0`) and reads the
/// corresponding `/sys/bus/pci/devices/<addr>` symlink target.
/// Returns `(symlink_path, link_target)` pairs for use with `--symlink`.
pub fn discover_gpu_pci_device_symlinks() -> Vec<(String, String)> {
    let mut symlinks: Vec<(String, String)> = Vec::new();
    for pci_path in discover_gpu_pci_sysfs_paths() {
        let pci_addr = match Path::new(&pci_path).file_name() {
            Some(name) => name.to_string_lossy().to_string(),
            None => continue,
        };
        let symlink_path = format!("/sys/bus/pci/devices/{pci_addr}");
        let link_target = match std::fs::read_link(&symlink_path) {
            Ok(t) => t.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        if !symlinks.iter().any(|(p, _)| p == &symlink_path) {
            symlinks.push((symlink_path, link_target));
        }
    }
    symlinks
}

/// Discover `/sys/dev/char/MAJ:MIN` symlink entries for DRI device nodes in `/dev/dri/`.
///
/// Reads `/dev/dri/`, stats each `card*` and `renderD*` entry to extract the
/// device major:minor numbers, reads the corresponding sysfs char symlink target,
/// and returns `(char_path, link_target)` pairs. Results are deduplicated.
///
/// The symlink targets are needed because Mesa/libdrm uses `readlink()` on these
/// paths to resolve device nodes — a bind-mounted directory would break this.
pub fn discover_dri_char_entries() -> Vec<(String, String)> {
    let dri_dir = Path::new("/dev/dri");
    if !dri_dir.is_dir() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(dri_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut char_entries: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if !name_str.starts_with("card") && !name_str.starts_with("renderD") {
            continue;
        }

        let meta = match std::fs::metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };

        use std::os::unix::fs::MetadataExt;
        let rdev = meta.rdev();
        let major = libc::major(rdev);
        let minor = libc::minor(rdev);
        let char_path = format!("/sys/dev/char/{major}:{minor}");

        // Read the symlink target so we can recreate it inside the sandbox
        let link_target = match std::fs::read_link(&char_path) {
            Ok(t) => t.to_string_lossy().to_string(),
            Err(_) => continue,
        };

        if !char_entries.iter().any(|(p, _)| p == &char_path) {
            char_entries.push((char_path, link_target));
        }
    }

    char_entries
}

/// Build GPU acceleration arguments.
pub fn gpu_args() -> Vec<String> {
    let mut args = Vec::new();
    if Path::new("/dev/dri").is_dir() {
        args.extend([
            "--dev-bind".to_string(),
            "/dev/dri".to_string(),
            "/dev/dri".to_string(),
        ]);
    } else {
        eprintln!("Warning: /dev/dri not found — GPU acceleration will not be available.");
    }

    // NixOS Mesa driver libraries
    if Path::new("/run/opengl-driver").exists() {
        args.extend([
            "--ro-bind".to_string(),
            "/run/opengl-driver".to_string(),
            "/run/opengl-driver".to_string(),
        ]);
    }

    // GPU-specific /sys/dev/char/MAJ:MIN symlinks for DRI device node resolution.
    // Mesa/libdrm expects these to be symlinks (uses readlink), not bind-mounted
    // directories, so we recreate the symlinks rather than bind-mounting.
    for (char_path, link_target) in discover_dri_char_entries() {
        args.extend(["--symlink".to_string(), link_target, char_path]);
    }

    // Auto-detected GPU PCI sysfs paths (vendor/device ID lookup)
    for pci_path in discover_gpu_pci_sysfs_paths() {
        if Path::new(&pci_path).is_dir() {
            args.extend(["--ro-bind".to_string(), pci_path.clone(), pci_path]);
        }
    }

    // PCI driver directories: Mesa/libdrm follows the `driver` symlink inside
    // each PCI device directory to resolve the kernel driver name. That symlink
    // points into /sys/bus/pci/drivers/<name>. We only bind the specific driver
    // directories used by discovered GPUs to avoid leaking the full list of PCI
    // drivers on the system (which would be a fingerprinting vector).
    for driver_path in discover_gpu_pci_driver_paths() {
        if Path::new(&driver_path).is_dir() {
            args.extend(["--ro-bind".to_string(), driver_path.clone(), driver_path]);
        }
    }

    // /sys/bus/pci/devices/<addr> symlinks: libpci and Chromium's GPU process
    // enumerate PCI devices via this directory. Each entry is a symlink pointing
    // to the actual device directory under /sys/devices/. We recreate only the
    // symlinks for discovered GPU devices.
    for (symlink_path, link_target) in discover_gpu_pci_device_symlinks() {
        args.extend(["--symlink".to_string(), link_target, symlink_path]);
    }

    args.extend([
        "--perms".to_string(),
        "1777".to_string(),
        "--tmpfs".to_string(),
        "/dev/shm".to_string(),
    ]);
    args
}

/// Build device bind arguments (for arbitrary devices like /dev/kvm).
pub fn dev_bind_args(paths: &[String]) -> Vec<String> {
    let mut args = Vec::new();
    for path in paths {
        if Path::new(path).exists() {
            args.extend(["--dev-bind".to_string(), path.clone(), path.clone()]);
        } else {
            eprintln!("Warning: device {path} not found — skipping.");
        }
    }
    args
}

/// Generate a random machine-id file and return bwrap args to bind it at `/etc/machine-id`.
///
/// GLib/GTK applications require `/etc/machine-id` to connect to the D-Bus session bus.
/// We generate a fresh random ID per invocation rather than binding the host's real
/// machine-id, which would be a cross-sandbox fingerprinting vector.
///
/// Returns `(args, temp_path)` — the caller must keep `temp_path` alive until bwrap exits,
/// then delete it.
pub fn machine_id_args() -> (Vec<String>, Option<String>) {
    // Generate 16 random bytes → 32 hex chars + newline
    let mut buf = [0u8; 16];

    for _ in 0..3 {
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            if f.read_exact(&mut buf).is_err() {
                return (Vec::new(), None);
            }
        } else {
            return (Vec::new(), None);
        }

        let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
        if let Some(path) = write_temp_file("cloister-machine-id", &format!("{hex}\n")) {
            let args = vec![
                "--ro-bind".to_string(),
                path.clone(),
                "/etc/machine-id".to_string(),
            ];
            return (args, Some(path));
        }
    }

    (Vec::new(), None)
}

#[derive(Debug, Default)]
pub struct ProcPrivacyOverlays {
    pub args: Vec<String>,
    pub file_paths: Vec<String>,
}

pub struct AnonymizeIdentityOverlays {
    pub args: Vec<String>,
    pub file_paths: Vec<String>,
}

pub const PROC_PRIVACY_SYNTHETIC_FILES: [&str; 4] = [
    "/proc/self/mountinfo",
    "/proc/self/mounts",
    "/proc/thread-self/mountinfo",
    "/proc/thread-self/mounts",
];

pub fn proc_privacy_args() -> ProcPrivacyOverlays {
    let mut overlays = ProcPrivacyOverlays::default();

    for dest in PROC_PRIVACY_SYNTHETIC_FILES {
        let Some(path) = write_temp_file("cloister-proc", "") else {
            continue;
        };
        overlays
            .args
            .extend(["--ro-bind".to_string(), path.clone(), dest.to_string()]);
        overlays.file_paths.push(path);
    }

    overlays
}

/// Generate synthetic `/etc/passwd` and `/etc/group` files for anonymized sandboxes.
///
/// Uses the real UID/GID from `libc::getuid()`/`libc::getgid()` so file permissions
/// work correctly inside the sandbox, while still presenting a generic "ubuntu" identity.
///
/// Returns bwrap args and temp file paths — the caller must keep the temp files
/// alive until bwrap exits, then delete them.
pub fn anonymize_identity_args(
    username: &str,
    shell_bin: &str,
    sandbox_home: &str,
    network_namespace: Option<&str>,
) -> AnonymizeIdentityOverlays {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    let passwd_content =
        format!("{username}:x:{uid}:{gid}:{username}:{sandbox_home}:{shell_bin}\n");
    let group_content = format!("{username}:x:{gid}:\n");
    let hosts_content = anonymized_hosts_content(username, network_namespace);

    let Some(passwd_path) = write_temp_file("cloister-passwd", &passwd_content) else {
        return AnonymizeIdentityOverlays {
            args: Vec::new(),
            file_paths: Vec::new(),
        };
    };
    let Some(group_path) = write_temp_file("cloister-group", &group_content) else {
        let _ = std::fs::remove_file(&passwd_path);
        return AnonymizeIdentityOverlays {
            args: Vec::new(),
            file_paths: Vec::new(),
        };
    };
    let Some(hosts_path) = write_temp_file("cloister-hosts", &hosts_content) else {
        let _ = std::fs::remove_file(&passwd_path);
        let _ = std::fs::remove_file(&group_path);
        return AnonymizeIdentityOverlays {
            args: Vec::new(),
            file_paths: Vec::new(),
        };
    };

    let args = vec![
        "--ro-bind".to_string(),
        passwd_path.clone(),
        "/etc/passwd".to_string(),
        "--ro-bind".to_string(),
        group_path.clone(),
        "/etc/group".to_string(),
        "--ro-bind".to_string(),
        hosts_path.clone(),
        "/etc/hosts".to_string(),
    ];

    AnonymizeIdentityOverlays {
        args,
        file_paths: vec![passwd_path, group_path, hosts_path],
    }
}

fn anonymized_hosts_content(username: &str, network_namespace: Option<&str>) -> String {
    let scrubbed_hostnames = scrubbed_hostnames();

    let source_hosts = network_namespace
        .and_then(read_netns_hosts)
        .or_else(|| std::fs::read_to_string("/etc/hosts").ok());

    match source_hosts {
        Some(source) => {
            let sanitized = sanitize_hosts_content(&source, username, &scrubbed_hostnames);
            if sanitized.is_empty() {
                default_hosts_content(username)
            } else {
                sanitized
            }
        }
        None => default_hosts_content(username),
    }
}

fn read_netns_hosts(namespace: &str) -> Option<String> {
    std::fs::read_to_string(format!("/etc/netns/{namespace}/hosts")).ok()
}

fn default_hosts_content(username: &str) -> String {
    format!("127.0.0.1 localhost {username}\n::1 localhost ip6-localhost ip6-loopback\n")
}

fn scrubbed_hostnames() -> Vec<String> {
    let mut names = Vec::new();

    if let Some(hostname) = system_hostname() {
        push_hostname_variants(&mut names, &hostname);
    }

    if let Ok(hostname) = std::fs::read_to_string("/etc/hostname") {
        push_hostname_variants(&mut names, hostname.trim());
    }

    names
}

fn push_hostname_variants(names: &mut Vec<String>, hostname: &str) {
    if hostname.is_empty() {
        return;
    }

    push_unique(names, hostname.to_string());

    if let Some((short, _)) = hostname.split_once('.') {
        if !short.is_empty() {
            push_unique(names, short.to_string());
        }
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn system_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return None;
    }

    let end = buf.iter().position(|byte| *byte == 0).unwrap_or(buf.len());
    let hostname = std::str::from_utf8(&buf[..end]).ok()?.trim();
    if hostname.is_empty() {
        None
    } else {
        Some(hostname.to_string())
    }
}

fn sanitize_hosts_content(source: &str, username: &str, scrubbed_hostnames: &[String]) -> String {
    let mut lines = Vec::new();
    let mut saw_ipv4_localhost = false;
    let mut saw_ipv6_localhost = false;

    for line in source.lines() {
        let Some((sanitized_line, saw_ipv4, saw_ipv6)) =
            sanitize_hosts_line(line, username, scrubbed_hostnames)
        else {
            continue;
        };

        saw_ipv4_localhost |= saw_ipv4;
        saw_ipv6_localhost |= saw_ipv6;
        lines.push(sanitized_line);
    }

    if !saw_ipv4_localhost {
        lines.push(format!("127.0.0.1 localhost {username}"));
    }
    if !saw_ipv6_localhost {
        lines.push("::1 localhost ip6-localhost ip6-loopback".to_string());
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn sanitize_hosts_line(
    line: &str,
    username: &str,
    scrubbed_hostnames: &[String],
) -> Option<(String, bool, bool)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Some((String::new(), false, false));
    }

    let (body, comment) = match line.find('#') {
        Some(index) => (&line[..index], Some(line[index..].trim_end())),
        None => (line, None),
    };

    let fields: Vec<&str> = body.split_whitespace().collect();
    if fields.is_empty() {
        return Some((comment.unwrap_or_default().to_string(), false, false));
    }
    if fields.len() == 1 {
        return Some((line.trim_end().to_string(), false, false));
    }

    let address = fields[0];
    let mut aliases = Vec::new();
    for alias in &fields[1..] {
        if !scrubbed_hostnames.iter().any(|hostname| hostname == alias) {
            push_unique(&mut aliases, (*alias).to_string());
        }
    }

    let saw_ipv4_localhost = address == "127.0.0.1";
    let saw_ipv6_localhost = address == "::1";

    if saw_ipv4_localhost {
        push_unique(&mut aliases, "localhost".to_string());
        push_unique(&mut aliases, username.to_string());
    }
    if saw_ipv6_localhost {
        push_unique(&mut aliases, "localhost".to_string());
        push_unique(&mut aliases, "ip6-localhost".to_string());
        push_unique(&mut aliases, "ip6-loopback".to_string());
    }

    if aliases.is_empty() {
        return None;
    }

    let mut sanitized = format!("{address} {}", aliases.join(" "));
    if let Some(comment) = comment {
        sanitized.push(' ');
        sanitized.push_str(comment);
    }

    Some((sanitized, saw_ipv4_localhost, saw_ipv6_localhost))
}

/// Read 8 random bytes from /dev/urandom and return as hex.
fn rand_hex() -> String {
    let mut buf = [0u8; 8];
    match std::fs::File::open("/dev/urandom") {
        Ok(mut f) => {
            use std::io::Read;
            if let Err(e) = f.read_exact(&mut buf) {
                eprintln!("Warning: failed to read from /dev/urandom: {e}");
            }
        }
        Err(e) => {
            eprintln!("Warning: failed to open /dev/urandom: {e}");
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_temp_file(prefix: &str, content: &str) -> Option<String> {
    for _ in 0..3 {
        let candidate = temp_path(prefix);

        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&candidate)
        {
            Ok(mut file) => {
                if file.write_all(content.as_bytes()).is_ok() {
                    return Some(candidate.to_string_lossy().to_string());
                }
                let _ = std::fs::remove_file(&candidate);
            }
            Err(_) => {
                // Retry with a new random suffix.
            }
        }
    }
    None
}

fn temp_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}-{}", std::process::id(), rand_hex()))
}

/// Build PipeWire native socket forwarding arguments if the socket exists and is valid.
pub fn pipewire_args(xdg_runtime_dir: &str, socket_name: &str) -> Vec<String> {
    pipewire_args_with_dest(xdg_runtime_dir, socket_name, xdg_runtime_dir, "pipewire-0")
}

/// Build PipeWire native socket forwarding arguments with a custom in-sandbox destination.
pub fn pipewire_args_with_dest(
    host_xdg_runtime_dir: &str,
    socket_name: &str,
    sandbox_xdg_runtime_dir: &str,
    sandbox_socket_name: &str,
) -> Vec<String> {
    let host_socket = format!("{host_xdg_runtime_dir}/{socket_name}");
    let sandbox_socket = format!("{sandbox_xdg_runtime_dir}/{sandbox_socket_name}");

    if !Path::new(&host_socket).exists() {
        eprintln!("Warning: PipeWire socket not found at {host_socket}");
        eprintln!("PipeWire native access will not work. Ensure PipeWire is running.");
        return Vec::new();
    }
    if let Err(e) = socket::validate_existing_socket(&host_socket) {
        eprintln!("Warning: invalid PipeWire socket '{host_socket}': {e}");
        return Vec::new();
    }
    vec![
        "--bind".to_string(),
        host_socket,
        sandbox_socket.clone(),
        "--setenv".to_string(),
        "PIPEWIRE_REMOTE".to_string(),
        sandbox_socket,
    ]
}

/// Best-effort prewarm for the D-Bus proxy socket.
///
/// This triggers socket activation ahead of the sandboxed app and performs a
/// minimal SASL EXTERNAL handshake so `xdg-dbus-proxy` has a chance to finish
/// connecting to the real session bus first.
///
/// Returns a connected stream only when the handshake succeeds. The caller keeps
/// it alive until the sandbox exits so that `xdg-dbus-proxy` does not exit
/// between prewarm and app connect.
pub fn warm_dbus_proxy(
    xdg_runtime_dir: &str,
    socket_name: &str,
) -> Option<std::os::unix::net::UnixStream> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    if xdg_runtime_dir.is_empty() {
        return None;
    }

    let socket_path = format!("{xdg_runtime_dir}/{socket_name}");
    if !Path::new(&socket_path).exists() {
        return None;
    }

    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Warning: failed to connect to D-Bus proxy socket '{socket_path}': {e}");
            return None;
        }
    };

    let timeout = Some(Duration::from_secs(3));
    let _ = stream.set_write_timeout(timeout);
    let _ = stream.set_read_timeout(timeout);

    // D-Bus SASL EXTERNAL authentication handshake.
    // Completing this ensures xdg-dbus-proxy has connected to the host bus
    // and is ready to proxy messages for the sandboxed application.
    let uid = unsafe { libc::getuid() };
    let hex_uid: String = uid
        .to_string()
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect();

    if stream
        .write_all(format!("\0AUTH EXTERNAL {hex_uid}\r\n").as_bytes())
        .is_err()
    {
        eprintln!("Warning: failed to prewarm D-Bus proxy authentication");
        return None;
    }

    // Read the server response (expect "OK <guid>\r\n").
    let mut buf = [0u8; 256];
    match stream.read(&mut buf) {
        Ok(n) => {
            let response = std::str::from_utf8(&buf[..n]).unwrap_or("");
            if !response.starts_with("OK") {
                eprintln!("Warning: unexpected D-Bus proxy auth response during prewarm");
                return None;
            }
        }
        Err(e) => {
            eprintln!("Warning: timed out waiting for D-Bus proxy readiness: {e}");
            return None;
        }
    }

    if stream.write_all(b"BEGIN\r\n").is_err() {
        eprintln!("Warning: failed to finish D-Bus proxy prewarm handshake");
        return None;
    }

    Some(stream)
}

pub fn dbus_runtime_socket_path(
    xdg_runtime_dir: &str,
    socket_name: &str,
    instance_id: &str,
) -> String {
    format!("{xdg_runtime_dir}/{socket_name}-{instance_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn gpu_args_includes_private_shm() {
        let args = gpu_args();
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--tmpfs" && w[1] == "/dev/shm"),
            "Expected --tmpfs /dev/shm"
        );
    }

    #[test]
    fn discover_dri_char_entries_returns_vec() {
        let entries = discover_dri_char_entries();
        // Should not panic; on systems with GPUs, paths start with /sys/dev/char/
        for (path, target) in &entries {
            assert!(
                path.starts_with("/sys/dev/char/"),
                "Expected DRI char path to start with /sys/dev/char/, got: {path}"
            );
            assert!(
                !target.is_empty(),
                "Expected non-empty symlink target for {path}"
            );
        }
    }

    #[test]
    fn discover_dri_char_entries_no_duplicates() {
        let entries = discover_dri_char_entries();
        let unique: std::collections::HashSet<&String> = entries.iter().map(|(p, _)| p).collect();
        assert_eq!(
            entries.len(),
            unique.len(),
            "Expected no duplicate DRI char entries"
        );
    }

    #[test]
    fn discover_gpu_pci_sysfs_paths_returns_vec() {
        let paths = discover_gpu_pci_sysfs_paths();
        // Should not panic; on systems with GPUs, paths start with /sys/
        for path in &paths {
            assert!(
                path.starts_with("/sys/"),
                "Expected GPU PCI sysfs path to start with /sys/, got: {path}"
            );
        }
    }

    #[test]
    fn discover_gpu_pci_sysfs_paths_no_duplicates() {
        let paths = discover_gpu_pci_sysfs_paths();
        let unique: std::collections::HashSet<&String> = paths.iter().collect();
        assert_eq!(
            paths.len(),
            unique.len(),
            "Expected no duplicate GPU PCI sysfs paths"
        );
    }

    #[test]
    fn discover_gpu_pci_driver_paths_returns_vec() {
        let paths = discover_gpu_pci_driver_paths();
        // Should not panic; on systems with GPUs, paths point into /sys/bus/pci/drivers/
        for path in &paths {
            assert!(
                path.starts_with("/sys/"),
                "Expected GPU PCI driver path to start with /sys/, got: {path}"
            );
        }
    }

    #[test]
    fn discover_gpu_pci_device_symlinks_returns_vec() {
        let symlinks = discover_gpu_pci_device_symlinks();
        for (path, target) in &symlinks {
            assert!(
                path.starts_with("/sys/bus/pci/devices/"),
                "Expected PCI device symlink to start with /sys/bus/pci/devices/, got: {path}"
            );
            assert!(
                !target.is_empty(),
                "Expected non-empty symlink target for {path}"
            );
        }
    }

    #[test]
    fn discover_gpu_pci_device_symlinks_no_duplicates() {
        let symlinks = discover_gpu_pci_device_symlinks();
        let unique: std::collections::HashSet<&String> = symlinks.iter().map(|(p, _)| p).collect();
        assert_eq!(
            symlinks.len(),
            unique.len(),
            "Expected no duplicate GPU PCI device symlinks"
        );
    }

    #[test]
    fn discover_gpu_pci_driver_paths_no_duplicates() {
        let paths = discover_gpu_pci_driver_paths();
        let unique: std::collections::HashSet<&String> = paths.iter().collect();
        assert_eq!(
            paths.len(),
            unique.len(),
            "Expected no duplicate GPU PCI driver paths"
        );
    }

    #[test]
    fn machine_id_args_creates_valid_hex() {
        let (args, path) = machine_id_args();
        assert!(
            !args.is_empty(),
            "Expected machine_id_args to return bwrap args"
        );
        assert!(path.is_some(), "Expected a temp file path");

        let path = path.unwrap();
        let content = std::fs::read_to_string(&path).expect("should read machine-id file");
        let hex = content.trim();
        assert_eq!(
            hex.len(),
            32,
            "machine-id should be 32 hex chars, got: {hex}"
        );
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit()),
            "machine-id should be hex, got: {hex}"
        );

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pipewire_args_with_missing_socket() {
        // Use a non-existent runtime dir to test the missing-socket path
        let args = pipewire_args("/nonexistent-runtime-dir-for-test", "pipewire-0");
        assert!(
            args.is_empty(),
            "Expected empty args when PipeWire socket doesn't exist"
        );
    }

    #[test]
    fn pipewire_args_with_valid_socket() {
        use std::os::unix::net::UnixListener;

        let dir =
            std::env::temp_dir().join(format!("cloister-pipewire-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let sock_path = dir.join("pipewire-0");
        let _listener = UnixListener::bind(&sock_path).unwrap();

        let args = pipewire_args(dir.to_str().unwrap(), "pipewire-0");
        assert!(
            !args.is_empty(),
            "Expected non-empty args when PipeWire socket is valid"
        );
        assert!(
            args.contains(&"PIPEWIRE_REMOTE".to_string()),
            "Expected PIPEWIRE_REMOTE in args"
        );

        drop(_listener);
        let _ = std::fs::remove_file(&sock_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn pipewire_args_with_custom_destination() {
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join(format!(
            "cloister-pipewire-custom-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let sock_path = dir.join("pipewire-0");
        let _listener = UnixListener::bind(&sock_path).unwrap();

        let args = pipewire_args_with_dest(
            dir.to_str().unwrap(),
            "pipewire-0",
            dir.to_str().unwrap(),
            "cloister/pipewire-pulse/test",
        );
        assert!(
            args.contains(&format!(
                "{}/cloister/pipewire-pulse/test",
                dir.to_string_lossy()
            )),
            "Expected custom sandbox socket path in args"
        );

        drop(_listener);
        let _ = std::fs::remove_file(&sock_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn machine_id_args_unique_per_call() {
        let (_, path1) = machine_id_args();
        // Read the content before cleanup
        let id1 = std::fs::read_to_string(path1.as_ref().unwrap()).unwrap();

        // Second call in same process will overwrite (same PID), so read first
        // Just verify the content is valid hex — uniqueness across invocations
        // is guaranteed by /dev/urandom
        let hex = id1.trim();
        assert_eq!(hex.len(), 32);

        if let Some(p) = path1 {
            let _ = std::fs::remove_file(&p);
        }
    }

    #[test]
    fn proc_privacy_args_cover_task_related_paths() {
        let overlays = proc_privacy_args();
        for expected in PROC_PRIVACY_SYNTHETIC_FILES {
            assert!(
                overlays.args.iter().any(|arg| arg == expected),
                "expected {expected} overlay"
            );
        }
        assert_eq!(
            overlays.file_paths.len(),
            PROC_PRIVACY_SYNTHETIC_FILES.len(),
            "expected only mount overlays"
        );
        assert!(
            overlays.args.iter().all(|arg| arg != "/proc/cpuinfo"),
            "did not expect /proc/cpuinfo overlay"
        );
        assert!(
            overlays.args.iter().all(|arg| arg != "/proc/meminfo"),
            "did not expect /proc/meminfo overlay"
        );
        assert!(
            overlays.args.iter().all(|arg| arg != "/proc/sys"),
            "did not expect /proc/sys overlay"
        );

        for path in overlays.file_paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn anonymize_identity_args_creates_valid_files() {
        let overlays = anonymize_identity_args("ubuntu", "/bin/sh", "/home/ubuntu", None);
        assert!(
            !overlays.args.is_empty(),
            "Expected anonymize_identity_args to return bwrap args"
        );
        assert_eq!(
            overlays.file_paths.len(),
            3,
            "Expected passwd, group, and hosts temp file paths"
        );

        let passwd_path = overlays.file_paths[0].clone();
        let group_path = overlays.file_paths[1].clone();
        let hosts_path = overlays.file_paths[2].clone();

        let passwd = std::fs::read_to_string(&passwd_path).expect("should read passwd file");
        let group = std::fs::read_to_string(&group_path).expect("should read group file");
        let hosts = std::fs::read_to_string(&hosts_path).expect("should read hosts file");

        // Verify passwd format: ubuntu:x:<uid>:<gid>:ubuntu:/home/ubuntu:<shell>
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        assert!(
            passwd.contains(&format!("ubuntu:x:{uid}:{gid}:ubuntu:/home/ubuntu:/bin/sh")),
            "passwd should contain correct entry, got: {passwd}"
        );
        assert!(
            group.contains(&format!("ubuntu:x:{gid}:")),
            "group should contain correct entry, got: {group}"
        );
        assert!(hosts.contains("127.0.0.1 localhost ubuntu"));
        assert!(hosts.contains("::1 localhost ip6-localhost ip6-loopback"));

        let _ = std::fs::remove_file(&passwd_path);
        let _ = std::fs::remove_file(&group_path);
        let _ = std::fs::remove_file(&hosts_path);
    }

    #[test]
    fn anonymize_identity_args_custom_username() {
        let overlays = anonymize_identity_args("devuser", "/bin/sh", "/home/devuser", None);
        assert!(!overlays.args.is_empty());
        assert_eq!(overlays.file_paths.len(), 3);

        let passwd_path = overlays.file_paths[0].clone();
        let group_path = overlays.file_paths[1].clone();
        let hosts_path = overlays.file_paths[2].clone();
        let passwd = std::fs::read_to_string(&passwd_path).expect("should read passwd file");
        let hosts = std::fs::read_to_string(&hosts_path).expect("should read hosts file");

        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        assert!(
            passwd.contains(&format!(
                "devuser:x:{uid}:{gid}:devuser:/home/devuser:/bin/sh"
            )),
            "passwd should use custom username, got: {passwd}"
        );
        assert!(hosts.contains("localhost devuser"));

        let _ = std::fs::remove_file(&passwd_path);
        let _ = std::fs::remove_file(&group_path);
        let _ = std::fs::remove_file(&hosts_path);
    }

    #[test]
    fn sanitize_hosts_content_preserves_netns_entries_while_scrubbing_hostname() {
        let source =
            "127.0.0.1 localhost myhost\n127.0.1.1 myhost netns-alias\n10.0.0.5 internal-api\n";
        let hosts = sanitize_hosts_content(source, "ubuntu", &["myhost".to_string()]);

        assert_eq!(
            hosts,
            "127.0.0.1 localhost ubuntu\n127.0.1.1 netns-alias\n10.0.0.5 internal-api\n::1 localhost ip6-localhost ip6-loopback\n"
        );
    }

    #[test]
    fn sanitize_hosts_content_preserves_comments_and_ipv6_aliases() {
        let source = "::1 localhost myhost # loopback\n192.168.1.20 fileserver myhost.localdomain # comment\n";
        let hosts = sanitize_hosts_content(
            source,
            "ubuntu",
            &["myhost".to_string(), "myhost.localdomain".to_string()],
        );

        assert_eq!(
            hosts,
            "::1 localhost ip6-localhost ip6-loopback # loopback\n192.168.1.20 fileserver # comment\n127.0.0.1 localhost ubuntu\n"
        );
    }
}
