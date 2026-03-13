use pipewire as pw;
use std::cell::Cell;
use std::cell::RefCell;
use std::fmt::Write;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

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

struct Args {
    verbose: bool,
    timeout: Duration,
}

fn usage(program: &str) -> String {
    format!("Usage: {program} [-v|--verbose] [--timeout-ms <milliseconds>]")
}

fn parse_args() -> Result<Args, String> {
    let mut verbose = false;
    let mut timeout = Duration::from_secs(3);
    let mut args = std::env::args();
    let program = args
        .next()
        .unwrap_or_else(|| "cloister-pipewire-validate".to_string());

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-v" | "--verbose" => verbose = true,
            "--timeout-ms" => {
                let value = args.next().ok_or_else(|| {
                    format!("Missing value for --timeout-ms\n{}", usage(&program))
                })?;
                let millis = value.parse::<u64>().map_err(|_| {
                    format!(
                        "Invalid timeout '{value}' for --timeout-ms\n{}",
                        usage(&program)
                    )
                })?;
                timeout = Duration::from_millis(millis);
            }
            "-h" | "--help" => {
                return Err(usage(&program));
            }
            _ => {
                return Err(format!("Unknown argument '{arg}'\n{}", usage(&program)));
            }
        }
    }

    Ok(Args { verbose, timeout })
}

fn collect_status(
    timeout: Duration,
    verbose: bool,
) -> Result<(FilterStatus, Vec<GlobalInfo>), String> {
    let timeout_hint = "This usually means the client can reach the PipeWire socket but lacks enough permissions to complete the initial registry sync.";

    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None)
        .map_err(|e| format!("Failed to create mainloop: {e}"))?;
    let context = pw::context::Context::new(&mainloop)
        .map_err(|e| format!("Failed to create context: {e}"))?;
    let core = context
        .connect(None)
        .map_err(|e| format!("Failed to connect to PipeWire: {e}"))?;
    let registry = core
        .get_registry()
        .map_err(|e| format!("Failed to get registry: {e}"))?;

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

    let pending = core
        .sync(0)
        .map_err(|e| format!("Failed to sync with PipeWire core: {e}"))?;
    let sync_complete = Rc::new(Cell::new(false));
    let sync_complete_clone = sync_complete.clone();
    let _sync_listener = core
        .add_listener_local()
        .done(move |_, seq| {
            if seq == pending {
                sync_complete_clone.set(true);
            }
        })
        .register();

    let start = Instant::now();
    while !sync_complete.get() {
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            return Err(format!(
                "Timed out waiting for PipeWire registry sync after {} ms.\n{}",
                timeout.as_millis(),
                timeout_hint
            ));
        }

        let remaining = timeout - elapsed;
        let step = remaining.min(Duration::from_millis(100));
        let result = mainloop.loop_().iterate(step);
        if result < 0 {
            return Err(format!(
                "PipeWire loop error while waiting for registry sync: {result}\n{}",
                timeout_hint
            ));
        }
    }

    drop(_sync_listener);
    drop(_listener);

    let status = Rc::try_unwrap(status)
        .map_err(|_| "Internal error: failed to unwrap PipeWire status state".to_string())?
        .into_inner();
    let globals = Rc::try_unwrap(globals)
        .map_err(|_| "Internal error: failed to unwrap PipeWire globals state".to_string())?
        .into_inner();

    Ok((status, globals))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return if message.starts_with("Usage:") {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
    };

    let (status, globals) = match collect_status(args.timeout, args.verbose) {
        Ok(result) => result,
        Err(message) => {
            eprintln!("Error: {message}");
            return ExitCode::FAILURE;
        }
    };

    if args.verbose {
        eprint!("{}", format_globals(&globals));
        eprintln!();
    }
    eprint!("{}", validate(&status));
    ExitCode::SUCCESS
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
    fn usage_mentions_timeout_flag() {
        let output = usage("cloister-pipewire-validate");
        assert!(output.contains("--timeout-ms"));
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
