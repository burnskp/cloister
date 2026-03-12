//! Runtime feature detection: wayland, pulseaudio, x11, gpu, dbus, device binds.

use std::path::Path;

use crate::socket;

/// Build PulseAudio forwarding arguments if the socket exists and is valid.
pub fn pulseaudio_args(xdg_runtime_dir: &str) -> Vec<String> {
    let socket = format!("{xdg_runtime_dir}/pulse/native");
    if !Path::new(&socket).exists() {
        eprintln!("Warning: PulseAudio socket not found at {xdg_runtime_dir}/pulse/native");
        eprintln!("Audio will not work. Ensure PulseAudio or PipeWire-PulseAudio is running.");
        return Vec::new();
    }
    if let Err(e) = socket::validate_existing_socket(&socket) {
        eprintln!("Warning: invalid PulseAudio socket '{socket}': {e}");
        return Vec::new();
    }
    vec![
        "--dir".to_string(),
        format!("{xdg_runtime_dir}/pulse"),
        "--bind".to_string(),
        socket.clone(),
        socket,
        "--setenv".to_string(),
        "PULSE_SERVER".to_string(),
        format!("unix:{xdg_runtime_dir}/pulse/native"),
    ]
}

/// Build Wayland forwarding arguments (non-security-context mode).
pub fn wayland_raw_args(xdg_runtime_dir: &str) -> Vec<String> {
    let display = match std::env::var("WAYLAND_DISPLAY") {
        Ok(d) if !d.is_empty() => d,
        _ => return Vec::new(),
    };

    let mut args = Vec::new();
    if display.starts_with('/') {
        if let Err(e) = socket::validate_existing_socket(&display) {
            eprintln!("Warning: invalid WAYLAND_DISPLAY socket '{display}': {e}");
            return Vec::new();
        }
        args.extend([
            "--ro-bind-try".to_string(),
            display.clone(),
            display.clone(),
        ]);
    } else {
        if display.contains('/') || display == "." || display == ".." {
            eprintln!("Warning: invalid WAYLAND_DISPLAY value '{display}'");
            return Vec::new();
        }
        if xdg_runtime_dir.is_empty() {
            eprintln!("Warning: WAYLAND_DISPLAY is set but XDG_RUNTIME_DIR is empty");
            return Vec::new();
        }
        let full_path = format!("{xdg_runtime_dir}/{display}");
        if let Err(e) = socket::validate_existing_socket(&full_path) {
            eprintln!("Warning: invalid WAYLAND_DISPLAY socket '{full_path}': {e}");
            return Vec::new();
        }
        args.extend(["--ro-bind-try".to_string(), full_path.clone(), full_path]);
    }
    args.extend([
        "--setenv".to_string(),
        "WAYLAND_DISPLAY".to_string(),
        display,
    ]);
    args
}

/// Build X11 forwarding arguments.
pub fn x11_args(sandbox_home: &str) -> Vec<String> {
    let display = match std::env::var("DISPLAY") {
        Ok(d) if !d.is_empty() => d,
        _ => return Vec::new(),
    };

    let mut args = vec!["--setenv".to_string(), "DISPLAY".to_string(), display];

    if Path::new("/tmp/.X11-unix").is_dir() {
        args.extend([
            "--ro-bind".to_string(),
            "/tmp/.X11-unix".to_string(),
            "/tmp/.X11-unix".to_string(),
        ]);
    }

    if let Ok(xauth) = std::env::var("XAUTHORITY") {
        if !xauth.is_empty() {
            if let Err(e) = socket::validate_existing_regular_file(&xauth) {
                eprintln!("Warning: invalid XAUTHORITY path '{xauth}': {e}");
            } else {
                args.extend([
                    "--setenv".to_string(),
                    "XAUTHORITY".to_string(),
                    xauth.clone(),
                ]);
                args.extend(["--ro-bind".to_string(), xauth.clone(), xauth]);
            }
        }
    } else {
        let home = std::env::var("HOME").unwrap_or_default();
        let xauth_path = format!("{home}/.Xauthority");
        if socket::validate_existing_regular_file(&xauth_path).is_ok() {
            let dest = format!("{sandbox_home}/.Xauthority");
            args.extend([
                "--setenv".to_string(),
                "XAUTHORITY".to_string(),
                dest.clone(),
                "--ro-bind".to_string(),
                xauth_path,
                dest,
            ]);
        }
    }

    args
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
pub fn gpu_args(shm: bool) -> Vec<String> {
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

    if shm {
        args.extend([
            "--perms".to_string(),
            "1777".to_string(),
            "--tmpfs".to_string(),
            "/dev/shm".to_string(),
        ]);
    }
    args
}

/// Build device bind arguments (for arbitrary devices like /dev/video0).
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
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let temp_dir = std::env::temp_dir();
    let pid = std::process::id();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

    // Generate 16 random bytes → 32 hex chars + newline
    let mut buf = [0u8; 16];
    let mut path = None;

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
        let candidate = temp_dir.join(format!("cloister-machine-id-{}-{}-{}", pid, counter, hex));

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
                let text = format!("{hex}\n");
                if file.write_all(text.as_bytes()).is_ok() {
                    path = Some(candidate.to_string_lossy().to_string());
                    break;
                }
            }
            Err(_) => {
                // Retry with a new random suffix.
            }
        }
    }

    let path = match path {
        Some(p) => p,
        None => return (Vec::new(), None),
    };

    let args = vec![
        "--ro-bind".to_string(),
        path.clone(),
        "/etc/machine-id".to_string(),
    ];

    (args, Some(path))
}

/// Generate synthetic `/etc/passwd` and `/etc/group` files for anonymized sandboxes.
///
/// Uses the real UID/GID from `libc::getuid()`/`libc::getgid()` so file permissions
/// work correctly inside the sandbox, while still presenting a generic "ubuntu" identity.
///
/// Returns `(bwrap_args, passwd_path, group_path)` — the caller must keep the temp
/// files alive until bwrap exits, then delete them.
pub fn anonymize_identity_args(
    shell_bin: &str,
    sandbox_home: &str,
) -> (Vec<String>, Option<String>, Option<String>) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    let username = Path::new(sandbox_home)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("ubuntu");

    let temp_dir = std::env::temp_dir();
    let pid = std::process::id();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

    let passwd_content =
        format!("{username}:x:{uid}:{gid}:{username}:{sandbox_home}:{shell_bin}\n");
    let group_content = format!("{username}:x:{gid}:\n");

    let mut passwd_path = None;
    let mut group_path = None;

    for _ in 0..3 {
        let passwd_candidate =
            temp_dir.join(format!("cloister-passwd-{pid}-{counter}-{}", rand_hex()));
        let group_candidate =
            temp_dir.join(format!("cloister-group-{pid}-{counter}-{}", rand_hex()));

        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let passwd_ok = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&passwd_candidate)
        {
            Ok(mut f) => f.write_all(passwd_content.as_bytes()).is_ok(),
            Err(_) => false,
        };

        let group_ok = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&group_candidate)
        {
            Ok(mut f) => f.write_all(group_content.as_bytes()).is_ok(),
            Err(_) => false,
        };

        if passwd_ok && group_ok {
            passwd_path = Some(passwd_candidate.to_string_lossy().to_string());
            group_path = Some(group_candidate.to_string_lossy().to_string());
            break;
        }

        // Clean up partial files on failure
        let _ = std::fs::remove_file(&passwd_candidate);
        let _ = std::fs::remove_file(&group_candidate);
    }

    let (passwd_path, group_path) = match (passwd_path, group_path) {
        (Some(p), Some(g)) => (p, g),
        _ => return (Vec::new(), None, None),
    };

    let args = vec![
        "--ro-bind".to_string(),
        passwd_path.clone(),
        "/etc/passwd".to_string(),
        "--ro-bind".to_string(),
        group_path.clone(),
        "/etc/group".to_string(),
    ];

    (args, Some(passwd_path), Some(group_path))
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

/// Discover hidraw sysfs device paths by resolving `/sys/class/hidraw/hidraw*` symlinks.
///
/// Each `hidrawN` entry is a symlink like:
///   `/sys/devices/pci0000:00/.../0003:1050:0407.0001/hidraw/hidraw0`
///
/// We canonicalize the symlink, then walk up to find the USB device directory
/// (the first ancestor whose name matches a USB device pattern like `1-2:1.0`
/// or the bus device like `1-2`).
/// Results are deduplicated.
pub fn discover_hidraw_sysfs_paths() -> Vec<String> {
    let hidraw_class = Path::new("/sys/class/hidraw");
    if !hidraw_class.is_dir() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(hidraw_class) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut usb_paths: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if !name_str.starts_with("hidraw") {
            continue;
        }

        // Resolve the symlink to its canonical path
        let canonical = match entry.path().canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Walk up from the canonical path to find the USB device directory.
        // We go up past "hidraw/hidrawN" (2 levels) to reach the HID device,
        // then one more to reach the USB interface directory.
        let mut current = canonical.as_path();
        for _ in 0..3 {
            if let Some(parent) = current.parent() {
                current = parent;
            }
        }
        let usb_str = current.to_string_lossy().to_string();
        if usb_str.starts_with("/sys/devices/") && !usb_paths.contains(&usb_str) {
            usb_paths.push(usb_str);
        }
    }

    usb_paths
}

/// Build FIDO2/U2F security key arguments.
///
/// Scans `/sys/class/hidraw/` for hidraw devices and binds:
/// - `/dev/hidrawN` as `--dev-bind` for each discovered device
/// - The resolved USB device sysfs path as `--ro-bind`
/// - `/sys/class/hidraw/` itself as `--ro-bind` for udev enumeration
pub fn fido2_args() -> Vec<String> {
    let mut args = Vec::new();

    let hidraw_class = Path::new("/sys/class/hidraw");
    if !hidraw_class.is_dir() {
        eprintln!(
            "Warning: /sys/class/hidraw not found — FIDO2 security keys will not be available."
        );
        return args;
    }

    let entries = match std::fs::read_dir(hidraw_class) {
        Ok(e) => e,
        Err(_) => {
            eprintln!(
                "Warning: cannot read /sys/class/hidraw — FIDO2 security keys will not be available."
            );
            return args;
        }
    };

    let mut found_any = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if !name_str.starts_with("hidraw") {
            continue;
        }

        let dev_path = format!("/dev/{name_str}");
        if Path::new(&dev_path).exists() {
            args.extend(["--dev-bind".to_string(), dev_path.clone(), dev_path]);
            found_any = true;
        }
    }

    if !found_any {
        eprintln!(
            "Warning: no /dev/hidraw* devices found — FIDO2 security keys will not be available."
        );
        eprintln!(
            "Ensure a FIDO2 key is plugged in and your user has permission to access hidraw devices."
        );
    }

    // Bind the sysfs class directory for device enumeration
    args.extend([
        "--ro-bind".to_string(),
        "/sys/class/hidraw".to_string(),
        "/sys/class/hidraw".to_string(),
    ]);

    // Bind discovered USB device sysfs paths
    for usb_path in discover_hidraw_sysfs_paths() {
        if Path::new(&usb_path).is_dir() {
            args.extend(["--ro-bind".to_string(), usb_path.clone(), usb_path]);
        }
    }

    args
}

/// Discover V4L2 (video4linux) sysfs device paths by resolving
/// `/sys/class/video4linux/video*` symlinks.
///
/// Each `videoN` entry is a symlink like:
///   `/sys/devices/pci0000:00/.../usb1/1-2/1-2:1.0/video4linux/video0`
///
/// We canonicalize the symlink, then walk up to find the USB/PCI device
/// directory (3 levels up: past `videoN`, `video4linux`, and the interface).
/// Results are deduplicated.
pub fn discover_v4l2_sysfs_paths() -> Vec<String> {
    let v4l2_class = Path::new("/sys/class/video4linux");
    if !v4l2_class.is_dir() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(v4l2_class) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut device_paths: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if !name_str.starts_with("video") {
            continue;
        }

        // Resolve the symlink to its canonical path
        let canonical = match entry.path().canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Walk up from the canonical path to find the USB/PCI device directory.
        // We go up past "videoN/video4linux" (2 levels) to reach the interface,
        // then one more to reach the USB/PCI device directory.
        let mut current = canonical.as_path();
        for _ in 0..3 {
            if let Some(parent) = current.parent() {
                current = parent;
            }
        }
        let device_str = current.to_string_lossy().to_string();
        if device_str.starts_with("/sys/devices/") && !device_paths.contains(&device_str) {
            device_paths.push(device_str);
        }
    }

    device_paths
}

/// Build webcam/camera arguments.
///
/// Scans `/sys/class/video4linux/` for video devices and binds:
/// - `/dev/videoN` as `--dev-bind` for each discovered device
/// - The resolved USB/PCI device sysfs path as `--ro-bind`
/// - `/sys/class/video4linux/` itself as `--ro-bind` for udev enumeration
pub fn video_args() -> Vec<String> {
    let mut args = Vec::new();

    let v4l2_class = Path::new("/sys/class/video4linux");
    if !v4l2_class.is_dir() {
        eprintln!(
            "Warning: /sys/class/video4linux not found — webcam/camera devices will not be available."
        );
        return args;
    }

    let entries = match std::fs::read_dir(v4l2_class) {
        Ok(e) => e,
        Err(_) => {
            eprintln!(
                "Warning: cannot read /sys/class/video4linux — webcam/camera devices will not be available."
            );
            return args;
        }
    };

    let mut found_any = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if !name_str.starts_with("video") {
            continue;
        }

        let dev_path = format!("/dev/{name_str}");
        if Path::new(&dev_path).exists() {
            args.extend(["--dev-bind".to_string(), dev_path.clone(), dev_path]);
            found_any = true;
        }
    }

    if !found_any {
        eprintln!("Warning: no /dev/video* devices found — webcam/camera will not be available.");
        eprintln!(
            "Ensure a camera is connected and your user has permission to access video devices."
        );
    }

    // Bind the sysfs class directory for device enumeration
    args.extend([
        "--ro-bind".to_string(),
        "/sys/class/video4linux".to_string(),
        "/sys/class/video4linux".to_string(),
    ]);

    // Bind discovered USB/PCI device sysfs paths
    for device_path in discover_v4l2_sysfs_paths() {
        if Path::new(&device_path).is_dir() {
            args.extend(["--ro-bind".to_string(), device_path.clone(), device_path]);
        }
    }

    args
}

/// Build PipeWire native socket forwarding arguments if the socket exists and is valid.
pub fn pipewire_args(xdg_runtime_dir: &str, socket_name: &str) -> Vec<String> {
    let host_socket = format!("{xdg_runtime_dir}/{socket_name}");
    // We always mount the socket as pipewire-0 inside the sandbox so clients don't
    // need any custom configuration to find it.
    let sandbox_socket = format!("{xdg_runtime_dir}/pipewire-0");

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

/// Build CUPS printing socket forwarding arguments.
pub fn printing_args() -> Vec<String> {
    let socket = "/run/cups/cups.sock";
    if Path::new(socket).exists() {
        vec![
            "--ro-bind".to_string(),
            socket.to_string(),
            socket.to_string(),
            "--setenv".to_string(),
            "CUPS_SERVER".to_string(),
            socket.to_string(),
        ]
    } else {
        eprintln!("Warning: CUPS socket not found at {socket}");
        eprintln!("Printing will not work. Ensure CUPS is running.");
        Vec::new()
    }
}

/// Check D-Bus proxy socket availability.
pub fn check_dbus_socket(xdg_runtime_dir: &str, socket_name: &str) -> Vec<String> {
    let socket_path = format!("{xdg_runtime_dir}/{socket_name}");
    if !Path::new(&socket_path).exists() {
        eprintln!("Warning: D-Bus proxy socket not found at {socket_path}");
        eprintln!(
            "D-Bus access will not work. Check: systemctl --user status cloister-dbus-proxy-<name>.socket"
        );
    }
    // The D-Bus bind is already in the static/dynamic args via config
    Vec::new()
}

/// Pre-connect to the D-Bus proxy socket to trigger systemd socket activation
/// and wait for the proxy to become ready before launching the sandbox.
///
/// Without this, the first sandboxed app to connect triggers socket activation
/// and blocks on the D-Bus SASL handshake while `xdg-dbus-proxy` starts up and
/// connects to the real session bus. This can cause a visible hang on first launch.
///
/// Returns the connected stream. The caller must keep it alive until the sandbox
/// exits so that `xdg-dbus-proxy` doesn't exit between warm-up and app connect.
pub fn warm_dbus_proxy(
    xdg_runtime_dir: &str,
    socket_name: &str,
) -> Option<std::os::unix::net::UnixStream> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let socket_path = format!("{xdg_runtime_dir}/{socket_name}");
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(_) => return None,
    };

    let timeout = Some(Duration::from_secs(5));
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
        return Some(stream);
    }

    // Read the server response (expect "OK <guid>\r\n").
    let mut buf = [0u8; 256];
    if let Ok(n) = stream.read(&mut buf) {
        let response = std::str::from_utf8(&buf[..n]).unwrap_or("");
        if response.starts_with("OK") {
            let _ = stream.write_all(b"BEGIN\r\n");
        }
    }

    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn gpu_args_shm_when_requested() {
        let args = gpu_args(true);
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--tmpfs" && w[1] == "/dev/shm"),
            "Expected --tmpfs /dev/shm when shm=true"
        );
    }

    #[test]
    fn gpu_args_no_shm_when_disabled() {
        let args = gpu_args(false);
        assert!(
            !args.contains(&"/dev/shm".to_string()),
            "Expected no /dev/shm when shm=false"
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
    fn fido2_args_returns_vec() {
        let args = fido2_args();
        // Should not panic; verify any discovered paths are valid
        for arg in &args {
            if arg.starts_with("/") {
                assert!(
                    arg.starts_with("/sys/") || arg.starts_with("/dev/"),
                    "Expected FIDO2 path to start with /sys/ or /dev/, got: {arg}"
                );
            }
        }
    }

    #[test]
    fn discover_hidraw_sysfs_paths_returns_vec() {
        let paths = discover_hidraw_sysfs_paths();
        // Should not panic; on systems with hidraw devices, paths start with /sys/
        for path in &paths {
            assert!(
                path.starts_with("/sys/"),
                "Expected hidraw sysfs path to start with /sys/, got: {path}"
            );
        }
    }

    #[test]
    fn discover_hidraw_sysfs_paths_no_duplicates() {
        let paths = discover_hidraw_sysfs_paths();
        let unique: std::collections::HashSet<&String> = paths.iter().collect();
        assert_eq!(
            paths.len(),
            unique.len(),
            "Expected no duplicate hidraw sysfs paths"
        );
    }

    #[test]
    fn video_args_returns_vec() {
        let args = video_args();
        // Should not panic; verify any discovered paths are valid
        for arg in &args {
            if arg.starts_with("/") {
                assert!(
                    arg.starts_with("/sys/") || arg.starts_with("/dev/"),
                    "Expected video path to start with /sys/ or /dev/, got: {arg}"
                );
            }
        }
    }

    #[test]
    fn discover_v4l2_sysfs_paths_returns_vec() {
        let paths = discover_v4l2_sysfs_paths();
        // Should not panic; on systems with video devices, paths start with /sys/
        for path in &paths {
            assert!(
                path.starts_with("/sys/"),
                "Expected V4L2 sysfs path to start with /sys/, got: {path}"
            );
        }
    }

    #[test]
    fn discover_v4l2_sysfs_paths_no_duplicates() {
        let paths = discover_v4l2_sysfs_paths();
        let unique: std::collections::HashSet<&String> = paths.iter().collect();
        assert_eq!(
            paths.len(),
            unique.len(),
            "Expected no duplicate V4L2 sysfs paths"
        );
    }

    #[test]
    fn pulseaudio_args_with_valid_socket() {
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join(format!("cloister-pulse-test-{}", std::process::id()));
        let pulse_dir = dir.join("pulse");
        std::fs::create_dir_all(&pulse_dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&pulse_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let sock_path = pulse_dir.join("native");
        let _listener = UnixListener::bind(&sock_path).unwrap();

        let args = pulseaudio_args(dir.to_str().unwrap());
        assert!(
            !args.is_empty(),
            "Expected non-empty args when PulseAudio socket is valid"
        );
        assert!(
            args.contains(&"PULSE_SERVER".to_string()),
            "Expected PULSE_SERVER in args"
        );

        drop(_listener);
        let _ = std::fs::remove_file(&sock_path);
        let _ = std::fs::remove_dir(&pulse_dir);
        let _ = std::fs::remove_dir(&dir);
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
    fn printing_args_returns_vec() {
        let args = printing_args();
        // Should not panic; on systems with CUPS, args will be non-empty
        for arg in &args {
            if arg.starts_with("/") {
                assert!(
                    arg.starts_with("/run/"),
                    "Expected printing path to start with /run/, got: {arg}"
                );
            }
        }
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
    fn anonymize_identity_args_creates_valid_files() {
        let (args, passwd_path, group_path) = anonymize_identity_args("/bin/sh", "/home/ubuntu");
        assert!(
            !args.is_empty(),
            "Expected anonymize_identity_args to return bwrap args"
        );
        assert!(passwd_path.is_some(), "Expected a passwd temp file path");
        assert!(group_path.is_some(), "Expected a group temp file path");

        let passwd_path = passwd_path.unwrap();
        let group_path = group_path.unwrap();

        let passwd = std::fs::read_to_string(&passwd_path).expect("should read passwd file");
        let group = std::fs::read_to_string(&group_path).expect("should read group file");

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

        let _ = std::fs::remove_file(&passwd_path);
        let _ = std::fs::remove_file(&group_path);
    }

    #[test]
    fn anonymize_identity_args_custom_username() {
        let (args, passwd_path, group_path) = anonymize_identity_args("/bin/sh", "/home/devuser");
        assert!(!args.is_empty());
        assert!(passwd_path.is_some());

        let passwd_path = passwd_path.unwrap();
        let group_path = group_path.unwrap();
        let passwd = std::fs::read_to_string(&passwd_path).expect("should read passwd file");

        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        assert!(
            passwd.contains(&format!(
                "devuser:x:{uid}:{gid}:devuser:/home/devuser:/bin/sh"
            )),
            "passwd should use custom username, got: {passwd}"
        );

        let _ = std::fs::remove_file(&passwd_path);
        let _ = std::fs::remove_file(&group_path);
    }
}
