use pipewire as pw;
use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;

#[derive(Default)]
struct FilterStatus {
    audio_out: bool,
    audio_in: bool,
    video_in: bool,
    has_write: bool,
    has_metadata_perm: bool,
}

impl FilterStatus {
    fn control(&self) -> bool {
        self.has_write
    }

    fn routing(&self) -> bool {
        self.has_metadata_perm
    }
}

struct GlobalInfo {
    id: u32,
    type_name: String,
    permissions: String,
    media_class: Option<String>,
    node_name: Option<String>,
}

fn format_permissions(perm_debug: &str) -> String {
    let mut s = String::with_capacity(4);
    s.push(if perm_debug.contains('R') { 'r' } else { '-' });
    s.push(if perm_debug.contains('W') { 'w' } else { '-' });
    s.push(if perm_debug.contains('X') { 'x' } else { '-' });
    s.push(if perm_debug.contains('M') { 'm' } else { '-' });
    s
}

fn format_globals(globals: &[GlobalInfo]) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "\u{2500}\u{2500} PipeWire globals ({} visible) \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
        globals.len()
    )
    .unwrap();
    for g in globals {
        write!(
            out,
            "  id={:<4} {:<12} {}",
            g.id, g.type_name, g.permissions
        )
        .unwrap();
        if let Some(mc) = &g.media_class {
            write!(out, "  media.class={mc}").unwrap();
        }
        if let Some(nn) = &g.node_name {
            write!(out, "  node.name={nn}").unwrap();
        }
        writeln!(out).unwrap();
    }
    out
}

fn validate(status: &FilterStatus) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "\u{2500}\u{2500} PipeWire filter status \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
    )
    .unwrap();
    writeln!(out, "  audioOut:  {}", status.audio_out).unwrap();
    writeln!(out, "  audioIn:   {}", status.audio_in).unwrap();
    writeln!(out, "  videoIn:   {}", status.video_in).unwrap();
    writeln!(out, "  control:   {}", status.control()).unwrap();
    writeln!(out, "  routing:   {}", status.routing()).unwrap();
    out
}

fn main() {
    let verbose = std::env::args().any(|a| a == "-v" || a == "--verbose");

    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None).expect("Failed to create mainloop");
    let context = pw::context::Context::new(&mainloop).expect("Failed to create context");
    let core = context
        .connect(None)
        .expect("Failed to connect to PipeWire");
    let registry = core.get_registry().expect("Failed to get registry");

    let status = Rc::new(RefCell::new(FilterStatus::default()));
    let globals: Rc<RefCell<Vec<GlobalInfo>>> = Rc::new(RefCell::new(Vec::new()));

    let status_clone = status.clone();
    let globals_clone = globals.clone();
    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            let type_str = global.type_.to_string();
            let perm_debug = format!("{:?}", global.permissions);

            let media_class = global.props.as_ref().and_then(|p| p.get("media.class"));
            let node_name = global.props.as_ref().and_then(|p| p.get("node.name"));

            {
                let mut s = status_clone.borrow_mut();

                if let Some(mc) = media_class {
                    match mc {
                        "Audio/Sink" => s.audio_out = true,
                        "Audio/Source" => s.audio_in = true,
                        "Video/Source" => s.video_in = true,
                        _ => {}
                    }
                    if perm_debug.contains('W') {
                        s.has_write = true;
                    }
                }

                if type_str.contains("Metadata") && perm_debug.contains('M') {
                    s.has_metadata_perm = true;
                }
            }

            if verbose {
                globals_clone.borrow_mut().push(GlobalInfo {
                    id: global.id,
                    type_name: type_str,
                    permissions: format_permissions(&perm_debug),
                    media_class: media_class.map(String::from),
                    node_name: node_name.map(String::from),
                });
            }
        })
        .register();

    let pending = core.sync(0).expect("Failed to sync");
    let mainloop_clone = mainloop.clone();
    let _sync_listener = core
        .add_listener_local()
        .done(move |_, seq| {
            if seq == pending {
                mainloop_clone.quit();
            }
        })
        .register();
    mainloop.run();

    if verbose {
        eprint!("{}", format_globals(&globals.borrow()));
        eprintln!();
    }
    eprint!("{}", validate(&status.borrow()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_all_false() {
        let s = FilterStatus::default();
        assert!(!s.audio_out);
        assert!(!s.audio_in);
        assert!(!s.video_in);
        assert!(!s.control());
        assert!(!s.routing());
    }

    #[test]
    fn control_requires_write() {
        let mut s = FilterStatus::default();
        assert!(!s.control());
        s.has_write = true;
        assert!(s.control());
    }

    #[test]
    fn routing_from_metadata_permission() {
        let mut s = FilterStatus::default();
        assert!(!s.routing());
        s.has_metadata_perm = true;
        assert!(s.routing());
    }

    #[test]
    fn routing_not_enabled_by_metadata_visibility_alone() {
        let s = FilterStatus::default();
        assert!(!s.routing());
    }

    #[test]
    fn audio_out_detection() {
        let mut s = FilterStatus::default();
        s.audio_out = true;
        let output = validate(&s);
        assert!(output.contains("audioOut:  true"));
        assert!(output.contains("audioIn:   false"));
    }

    #[test]
    fn audio_in_detection() {
        let mut s = FilterStatus::default();
        s.audio_in = true;
        let output = validate(&s);
        assert!(output.contains("audioIn:   true"));
        assert!(output.contains("audioOut:  false"));
    }

    #[test]
    fn video_in_detection() {
        let mut s = FilterStatus::default();
        s.video_in = true;
        let output = validate(&s);
        assert!(output.contains("videoIn:   true"));
    }

    #[test]
    fn validate_all_enabled() {
        let s = FilterStatus {
            audio_out: true,
            audio_in: true,
            video_in: true,
            has_write: true,
            has_metadata_perm: true,
        };
        let output = validate(&s);
        assert!(output.contains("audioOut:  true"));
        assert!(output.contains("audioIn:   true"));
        assert!(output.contains("videoIn:   true"));
        assert!(output.contains("control:   true"));
        assert!(output.contains("routing:   true"));
    }

    #[test]
    fn validate_output_has_header() {
        let s = FilterStatus::default();
        let output = validate(&s);
        assert!(output.contains("PipeWire filter status"));
    }

    #[test]
    fn format_globals_shows_count() {
        let globals = vec![GlobalInfo {
            id: 32,
            type_name: "Node".to_string(),
            permissions: "rwx-".to_string(),
            media_class: Some("Audio/Sink".to_string()),
            node_name: Some("alsa_output".to_string()),
        }];
        let output = format_globals(&globals);
        assert!(output.contains("1 visible"));
        assert!(output.contains("id=32"));
        assert!(output.contains("media.class=Audio/Sink"));
    }

    #[test]
    fn format_permissions_all() {
        assert_eq!(format_permissions("Permission(R | W | X | M)"), "rwxm");
    }

    #[test]
    fn format_permissions_read_only() {
        assert_eq!(format_permissions("Permission(R)"), "r---");
    }

    #[test]
    fn format_permissions_none() {
        assert_eq!(format_permissions("Permission()"), "----");
    }
}
