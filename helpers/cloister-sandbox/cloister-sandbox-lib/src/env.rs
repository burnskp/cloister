//! Environment variable handling for sandboxes.

/// Parsed sandbox arguments and whether they explicitly override `defaultCommand`.
pub struct ParsedSandboxArgs {
    pub args: Vec<String>,
    pub explicit_command: bool,
}

/// Build the run command: default command + args > explicit command > interactive shell.
pub fn build_run_cmd(
    shell_bin: &str,
    shell_interactive_args: &[String],
    default_command: Option<&[String]>,
    parsed_args: &ParsedSandboxArgs,
) -> Vec<String> {
    if parsed_args.explicit_command {
        return parsed_args.args.clone();
    }

    if let Some(default_cmd) = default_command {
        let mut cmd = default_cmd.to_vec();
        cmd.extend_from_slice(&parsed_args.args);
        return cmd;
    }

    if !parsed_args.args.is_empty() {
        return parsed_args.args.clone();
    }

    // Interactive shell fallback
    let mut cmd = vec![shell_bin.to_string()];
    cmd.extend_from_slice(shell_interactive_args);
    cmd
}

/// Returns `true` when the sandbox will launch an interactive shell.
///
/// This matches the "interactive shell fallback" path in [`build_run_cmd`]:
/// no explicit command args and no configured default command.
pub fn is_interactive(parsed_args: &ParsedSandboxArgs, default_command: Option<&[String]>) -> bool {
    parsed_args.args.is_empty() && default_command.is_none() && !parsed_args.explicit_command
}

/// Parse sandbox CLI arguments.
///
/// Modes:
/// - No args → interactive shell or default command
/// - `-c cmd [args...]` → explicit command mode (strip -c)
/// - `cmd [args...]` → arguments for the default command when configured
pub fn parse_sandbox_args(args: &[String]) -> Result<ParsedSandboxArgs, &'static str> {
    if args.is_empty() {
        return Ok(ParsedSandboxArgs {
            args: Vec::new(),
            explicit_command: false,
        });
    }

    if args[0] == "-c" {
        if args.len() == 1 {
            return Err("`-c` requires a command");
        }
        Ok(ParsedSandboxArgs {
            args: args[1..].to_vec(),
            explicit_command: true,
        })
    } else {
        Ok(ParsedSandboxArgs {
            args: args.to_vec(),
            explicit_command: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_args() {
        let result = parse_sandbox_args(&[]).unwrap();
        assert!(result.args.is_empty());
        assert!(!result.explicit_command);
    }

    #[test]
    fn parse_with_c_flag() {
        let args = vec!["-c".to_string(), "echo".to_string(), "hello".to_string()];
        let result = parse_sandbox_args(&args).unwrap();
        assert_eq!(result.args, vec!["echo", "hello"]);
        assert!(result.explicit_command);
    }

    #[test]
    fn parse_without_c_flag() {
        let args = vec!["echo".to_string(), "hello".to_string()];
        let result = parse_sandbox_args(&args).unwrap();
        assert_eq!(result.args, vec!["echo", "hello"]);
        assert!(!result.explicit_command);
    }

    #[test]
    fn parse_bare_c_flag_errors() {
        let args = vec!["-c".to_string()];
        let result = parse_sandbox_args(&args);
        assert!(matches!(result, Err("`-c` requires a command")));
    }

    #[test]
    fn build_interactive_cmd() {
        let cmd = build_run_cmd(
            "/bin/zsh",
            &["-i".to_string()],
            None,
            &ParsedSandboxArgs {
                args: Vec::new(),
                explicit_command: false,
            },
        );
        assert_eq!(cmd, vec!["/bin/zsh", "-i"]);
    }

    #[test]
    fn build_command_cmd() {
        let cmd = build_run_cmd(
            "/bin/zsh",
            &["-i".to_string()],
            None,
            &ParsedSandboxArgs {
                args: vec!["echo".to_string(), "hello".to_string()],
                explicit_command: false,
            },
        );
        assert_eq!(cmd, vec!["echo", "hello"]);
    }

    #[test]
    fn build_default_command() {
        let default = vec!["geeqie".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            Some(&default),
            &ParsedSandboxArgs {
                args: Vec::new(),
                explicit_command: false,
            },
        );
        assert_eq!(cmd, vec!["geeqie"]);
    }

    #[test]
    fn build_explicit_args_append_to_default() {
        let default = vec!["geeqie".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            Some(&default),
            &ParsedSandboxArgs {
                args: vec!["photo.jpg".to_string()],
                explicit_command: false,
            },
        );
        assert_eq!(cmd, vec!["geeqie", "photo.jpg"]);
    }

    #[test]
    fn build_flag_args_prepend_default_command() {
        let default = vec!["helium".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            Some(&default),
            &ParsedSandboxArgs {
                args: vec!["-ozone-platform=wayland".to_string()],
                explicit_command: false,
            },
        );
        assert_eq!(cmd, vec!["helium", "-ozone-platform=wayland"]);
    }

    #[test]
    fn build_explicit_command_overrides_default() {
        let default = vec!["geeqie".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            Some(&default),
            &ParsedSandboxArgs {
                args: vec!["mpv".to_string(), "video.mp4".to_string()],
                explicit_command: true,
            },
        );
        assert_eq!(cmd, vec!["mpv", "video.mp4"]);
    }

    #[test]
    fn interactive_no_args_no_default() {
        assert!(is_interactive(
            &ParsedSandboxArgs {
                args: Vec::new(),
                explicit_command: false,
            },
            None
        ));
    }

    #[test]
    fn not_interactive_with_command_args() {
        assert!(!is_interactive(
            &ParsedSandboxArgs {
                args: vec!["echo".to_string()],
                explicit_command: false,
            },
            None
        ));
    }

    #[test]
    fn not_interactive_with_default_command() {
        let default = vec!["geeqie".to_string()];
        assert!(!is_interactive(
            &ParsedSandboxArgs {
                args: Vec::new(),
                explicit_command: false,
            },
            Some(&default)
        ));
    }

    #[test]
    fn not_interactive_with_both() {
        let default = vec!["geeqie".to_string()];
        assert!(!is_interactive(
            &ParsedSandboxArgs {
                args: vec!["mpv".to_string()],
                explicit_command: false,
            },
            Some(&default)
        ));
    }

    #[test]
    fn build_flag_args_without_default_passes_through() {
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            None,
            &ParsedSandboxArgs {
                args: vec!["-ozone-platform=wayland".to_string()],
                explicit_command: false,
            },
        );
        assert_eq!(cmd, vec!["-ozone-platform=wayland"]);
    }

    #[test]
    fn explicit_command_is_not_interactive() {
        assert!(!is_interactive(
            &ParsedSandboxArgs {
                args: vec!["echo".to_string()],
                explicit_command: true,
            },
            None
        ));
    }
}
