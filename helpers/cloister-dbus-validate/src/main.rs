use std::collections::BTreeSet;
use std::process::ExitCode;

use zbus::{Connection, Proxy};

const DEFAULT_DENIED: &[&str] = &[
    "org.freedesktop.secrets",
    "org.gnome.keyring",
    "org.kde.kwalletd5",
    "org.kde.kwalletd6",
    "org.freedesktop.login1",
    "org.freedesktop.systemd1",
    "org.freedesktop.NetworkManager",
    "org.freedesktop.ModemManager1",
    "org.freedesktop.PackageKit",
    "org.freedesktop.Flatpak",
    "org.freedesktop.UPower",
    "org.freedesktop.RealtimeKit1",
    "org.freedesktop.hostname1",
    "org.freedesktop.timedate1",
    "org.freedesktop.GeoClue2",
    "org.freedesktop.IBus",
    "org.freedesktop.Tracker3",
    "org.freedesktop.Tracker3.Miner.Files",
    "org.freedesktop.Tracker3.Miner.Extract",
    "org.gnome.Mutter.DisplayConfig",
    "org.gnome.Mutter.RemoteDesktop",
    "org.gnome.Mutter.ScreenCast",
    "org.gnome.SettingsDaemon",
];

#[derive(Debug)]
enum Mode {
    List,
    Validate,
}

#[derive(Debug)]
struct Args {
    mode: Mode,
    deny: Vec<String>,
    show_all: bool,
    show_activatable: bool,
    json: bool,
    verbose: bool,
}

fn parse_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect()
}

fn parse_args() -> Result<Args, String> {
    let mut mode = Mode::Validate;
    let mut deny = Vec::new();
    let mut show_all = true;
    let mut show_activatable = false;
    let mut json = false;
    let mut verbose = false;

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => mode = Mode::List,
            "--validate" => mode = Mode::Validate,
            "--deny" => {
                let value = args.next().ok_or("--deny requires a value")?;
                deny.extend(parse_csv(&value));
            }
            "--show-all" => show_all = true,
            "--quiet" => show_all = false,
            "--activatable" => show_activatable = true,
            "--json" => json = true,
            "-v" | "--verbose" => verbose = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    if deny.is_empty() {
        deny = DEFAULT_DENIED.iter().map(|s| s.to_string()).collect();
    }

    Ok(Args {
        mode,
        deny,
        show_all,
        show_activatable,
        json,
        verbose,
    })
}

async fn list_names(conn: &Connection) -> Result<Vec<String>, String> {
    let proxy = Proxy::new(
        conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await
    .map_err(|e| format!("dbus proxy: {e}"))?;

    let names: Vec<String> = proxy
        .call("ListNames", &())
        .await
        .map_err(|e| format!("ListNames: {e}"))?;
    Ok(names)
}

async fn list_activatable(conn: &Connection) -> Result<Vec<String>, String> {
    let proxy = Proxy::new(
        conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await
    .map_err(|e| format!("dbus proxy: {e}"))?;

    let names: Vec<String> = proxy
        .call("ListActivatableNames", &())
        .await
        .map_err(|e| format!("ListActivatableNames: {e}"))?;
    Ok(names)
}

fn name_matches(name: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix(".*") {
        name == prefix || name.starts_with(&format!("{}.", prefix))
    } else {
        name == pattern
    }
}

fn find_matches(names: &BTreeSet<String>, patterns: &[String]) -> Vec<String> {
    let mut matches = Vec::new();
    for pattern in patterns {
        if names.iter().any(|n| name_matches(n, pattern)) {
            matches.push(pattern.clone());
        }
    }
    matches
}

fn print_list(names: &BTreeSet<String>) {
    println!("Discovered names ({}):", names.len());
    for name in names {
        println!("  {name}");
    }
}

fn print_validation(
    names: &BTreeSet<String>,
    deny: &[String],
    deny_exposed: &[String],
    verbose: bool,
    quiet: bool,
) {
    let found_set: BTreeSet<&str> = deny_exposed.iter().map(|s| s.as_str()).collect();

    println!(
        "Checked {} denied names, found {}",
        deny.len(),
        deny_exposed.len()
    );

    if verbose {
        for pattern in deny {
            if found_set.contains(pattern.as_str()) {
                println!("  {pattern}: found");
            } else {
                println!("  {pattern}: not found");
            }
        }
    } else if !deny_exposed.is_empty() {
        for name in deny_exposed {
            println!("  {name}");
        }
    }

    println!();
    if deny_exposed.is_empty() {
        println!("RESULT: PASS");
    } else {
        println!("RESULT: FAIL");
    }

    if !quiet {
        println!();
        print_list(names);
    }
}

fn print_json(
    names: &BTreeSet<String>,
    deny_checked: usize,
    deny_exposed: &[String],
    activatable: Option<&BTreeSet<String>>,
    pass: bool,
) {
    println!("{{");
    println!("  \"pass\": {},", if pass { "true" } else { "false" });
    println!("  \"denied_checked\": {deny_checked},");
    println!("  \"exposed_denied\": [");
    for (i, name) in deny_exposed.iter().enumerate() {
        let comma = if i + 1 < deny_exposed.len() { "," } else { "" };
        println!("    \"{}\"{}", name, comma);
    }
    println!("  ],");
    println!("  \"names\": [");
    for (i, name) in names.iter().enumerate() {
        let comma = if i + 1 < names.len() { "," } else { "" };
        println!("    \"{}\"{}", name, comma);
    }
    if let Some(activatable) = activatable {
        println!("  ],");
        println!("  \"activatable\": [");
        for (i, name) in activatable.iter().enumerate() {
            let comma = if i + 1 < activatable.len() { "," } else { "" };
            println!("    \"{}\"{}", name, comma);
        }
        println!("  ]");
    } else {
        println!("  ]");
    }
    println!("}}");
}

fn usage() {
    eprintln!(
        "usage: cloister-dbus-validate [--list|--validate] [--deny CSV] [--show-all|--quiet] [-v|--verbose] [--activatable] [--json]"
    );
    eprintln!("  --list         list names and exit 0");
    eprintln!("  --validate     verify deny list (default)");
    eprintln!("  --deny CSV     comma-separated denylist (supports .*)");
    eprintln!("  --show-all     always print discovered names (default)");
    eprintln!("  --quiet        only print failures + result");
    eprintln!("  -v, --verbose  show status of every denied name checked");
    eprintln!("  --activatable  include activatable names list");
    eprintln!("  --json         emit JSON output");
}

async fn run(args: Args) -> Result<bool, String> {
    let conn = Connection::session()
        .await
        .map_err(|e| format!("connect session bus: {e}"))?;

    let names = list_names(&conn).await?;
    let names_set: BTreeSet<String> = names.into_iter().collect();
    let activatable_set: Option<BTreeSet<String>> = if args.show_activatable {
        let activatable_list = list_activatable(&conn).await?;
        Some(activatable_list.into_iter().collect())
    } else {
        None
    };

    let mut all_names = names_set.clone();
    if let Some(activatable) = activatable_set.as_ref() {
        all_names.extend(activatable.iter().cloned());
    }

    if let Mode::List = args.mode {
        if args.json {
            let activatable = activatable_set.as_ref();
            print_json(&names_set, 0, &[], activatable, true);
        } else {
            print_list(&names_set);
            if let Some(activatable) = activatable_set.as_ref() {
                println!();
                println!("Activatable names ({}):", activatable.len());
                for name in activatable {
                    println!("  {name}");
                }
            }
        }
        return Ok(true);
    }

    let deny_exposed = find_matches(&all_names, &args.deny);
    let pass = deny_exposed.is_empty();

    if args.json {
        let activatable = activatable_set.as_ref();
        print_json(
            &names_set,
            args.deny.len(),
            &deny_exposed,
            activatable,
            pass,
        );
    } else {
        print_validation(
            &names_set,
            &args.deny,
            &deny_exposed,
            args.verbose,
            !args.show_all,
        );
        if let Some(activatable) = activatable_set.as_ref() {
            println!();
            println!("Activatable names ({}):", activatable.len());
            for name in activatable {
                println!("  {name}");
            }
        }
    }

    Ok(pass)
}

#[tokio::main]
async fn main() -> ExitCode {
    match parse_args() {
        Err(msg) => {
            eprintln!("error: {msg}");
            usage();
            ExitCode::from(2)
        }
        Ok(args) => match run(args).await {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(1),
            Err(msg) => {
                eprintln!("error: {msg}");
                ExitCode::from(1)
            }
        },
    }
}
