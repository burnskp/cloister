use std::process;

use wayrs_client::Connection;

/// Privileged Wayland protocols that should be blocked inside a security context.
/// Based on sway's `is_privileged()` plus common compositor extensions.
const PRIVILEGED: &[&str] = &[
    "zwlr_screencopy_manager_v1",
    "ext_image_copy_capture_manager_v1",
    "zwlr_export_dmabuf_manager_v1",
    "wp_security_context_manager_v1",
    "zwlr_gamma_control_manager_v1",
    "zwlr_layer_shell_v1",
    "ext_session_lock_manager_v1",
    "zwp_keyboard_shortcuts_inhibit_manager_v1",
    "zwp_virtual_keyboard_manager_v1",
    "zwlr_virtual_pointer_manager_v1",
    "ext_transient_seat_manager_v1",
    "zxdg_output_manager_v1",
    "zwlr_output_manager_v1",
    "zwlr_output_power_manager_v1",
    "zwp_input_method_manager_v2",
    "zwlr_foreign_toplevel_management_v1",
    "ext_foreign_toplevel_list_v1",
    "zwlr_data_control_manager_v1",
    "ext_data_control_manager_v1",
];

/// Core Wayland protocols expected in any functional session.
const CORE: &[&str] = &[
    "wl_compositor",
    "wl_shm",
    "wl_seat",
    "wl_output",
    "xdg_wm_base",
];

fn classify(interface: &str) -> &'static str {
    if PRIVILEGED.contains(&interface) {
        "PRIVILEGED"
    } else if CORE.contains(&interface) {
        "core"
    } else {
        ""
    }
}

struct GlobalInfo {
    interface: String,
    version: u32,
}

/// Validate advertised globals against known privileged/core lists.
/// Prints a human-readable report to stderr and returns true if all checks pass.
fn validate(globals: &[GlobalInfo]) -> bool {
    let mut exposed: Vec<&str> = Vec::new();
    let mut missing_core: Vec<&str> = Vec::new();

    let width = globals
        .iter()
        .map(|g| g.interface.len())
        .chain(PRIVILEGED.iter().map(|s| s.len()))
        .chain(CORE.iter().map(|s| s.len()))
        .max()
        .unwrap_or(40);

    // Section 1: list all advertised globals
    eprintln!(
        "\u{2500}\u{2500} Advertised globals ({} total) \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        globals.len()
    );
    for g in globals {
        let tag = classify(&g.interface);
        if tag.is_empty() {
            eprintln!("  {:<width$} v{}", g.interface, g.version);
        } else {
            eprintln!("  {:<width$} v{:<4} {}", g.interface, g.version, tag);
        }
    }

    // Section 2: check privileged protocols
    eprintln!();
    eprintln!(
        "\u{2500}\u{2500} Privileged protocols \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
    );
    for &proto in PRIVILEGED {
        if globals.iter().any(|g| g.interface == proto) {
            eprintln!("  \u{2717} {:<width$} EXPOSED", proto);
            exposed.push(proto);
        } else {
            eprintln!("  \u{2713} {:<width$} blocked", proto);
        }
    }

    // Section 3: check core protocols
    eprintln!();
    eprintln!(
        "\u{2500}\u{2500} Core protocols \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
    );
    for &proto in CORE {
        match globals.iter().find(|g| g.interface == proto) {
            Some(g) => eprintln!("  \u{2713} {:<width$} present (v{})", proto, g.version),
            None => {
                eprintln!("  \u{2717} {:<width$} MISSING", proto);
                missing_core.push(proto);
            }
        }
    }

    // Summary
    eprintln!();
    let pass = exposed.is_empty() && missing_core.is_empty();
    if pass {
        eprintln!("RESULT: PASS \u{2014} all privileged protocols blocked");
    } else {
        let mut parts = Vec::new();
        if !exposed.is_empty() {
            parts.push(format!(
                "{} privileged protocol{} exposed ({})",
                exposed.len(),
                if exposed.len() == 1 { "" } else { "s" },
                exposed.join(", ")
            ));
        }
        if !missing_core.is_empty() {
            parts.push(format!(
                "{} core protocol{} missing ({})",
                missing_core.len(),
                if missing_core.len() == 1 { "" } else { "s" },
                missing_core.join(", ")
            ));
        }
        eprintln!("RESULT: FAIL \u{2014} {}", parts.join("; "));
    }

    pass
}

fn main() {
    let mut conn = match Connection::<()>::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cloister-wayland-validate: connect: {e}");
            process::exit(1);
        }
    };

    if let Err(e) = conn.blocking_roundtrip() {
        eprintln!("cloister-wayland-validate: roundtrip: {e}");
        process::exit(1);
    }

    let globals: Vec<GlobalInfo> = conn
        .globals()
        .iter()
        .map(|g| GlobalInfo {
            interface: g.interface.to_string_lossy().into_owned(),
            version: g.version,
        })
        .collect();

    let pass = validate(&globals);
    process::exit(if pass { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(interface: &str, version: u32) -> GlobalInfo {
        GlobalInfo {
            interface: interface.to_string(),
            version,
        }
    }

    #[test]
    fn classify_privileged() {
        assert_eq!(classify("zwlr_screencopy_manager_v1"), "PRIVILEGED");
        assert_eq!(classify("zwlr_layer_shell_v1"), "PRIVILEGED");
        assert_eq!(classify("zwlr_data_control_manager_v1"), "PRIVILEGED");
    }

    #[test]
    fn classify_core() {
        assert_eq!(classify("wl_compositor"), "core");
        assert_eq!(classify("xdg_wm_base"), "core");
        assert_eq!(classify("wl_shm"), "core");
    }

    #[test]
    fn classify_unknown() {
        assert_eq!(classify("wp_viewporter"), "");
        assert_eq!(classify("wl_subcompositor"), "");
    }

    #[test]
    fn pass_all_core_no_privileged() {
        let globals = vec![
            g("wl_compositor", 6),
            g("wl_shm", 2),
            g("wl_seat", 9),
            g("wl_output", 4),
            g("xdg_wm_base", 6),
            g("wp_viewporter", 1),
        ];
        assert!(validate(&globals));
    }

    #[test]
    fn fail_privileged_exposed() {
        let globals = vec![
            g("wl_compositor", 6),
            g("wl_shm", 2),
            g("wl_seat", 9),
            g("wl_output", 4),
            g("xdg_wm_base", 6),
            g("zwlr_layer_shell_v1", 4),
        ];
        assert!(!validate(&globals));
    }

    #[test]
    fn fail_core_missing() {
        let globals = vec![g("wl_compositor", 6), g("wl_shm", 2)];
        assert!(!validate(&globals));
    }

    #[test]
    fn fail_empty() {
        assert!(!validate(&[]));
    }

    #[test]
    fn fail_both_exposed_and_missing() {
        let globals = vec![g("wl_compositor", 6), g("zwlr_screencopy_manager_v1", 3)];
        assert!(!validate(&globals));
    }

    #[test]
    fn privileged_list_complete() {
        assert_eq!(PRIVILEGED.len(), 19);
    }

    #[test]
    fn core_list_complete() {
        assert_eq!(CORE.len(), 5);
    }
}
