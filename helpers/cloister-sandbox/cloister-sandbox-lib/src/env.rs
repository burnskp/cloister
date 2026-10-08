//! Environment variable handling for sandboxes.

const INTERACTIVE_COMMAND_EXEC: &str =
    "if command -v direnv >/dev/null 2>&1; then exec direnv exec \"$PWD\" \"$@\"; fi; exec \"$@\"";

/// Parsed sandbox arguments plus flags for explicit-command and interactive-command modes.
pub struct ParsedSandboxArgs {
    pub args: Vec<String>,
    pub explicit_command: bool,
    pub interactive_command: bool,
    pub force_shell: bool,
}

/// Build the run command: forced shell > interactive command > explicit command > default command + args > passthrough args > interactive shell.
pub fn build_run_cmd(
    shell_bin: &str,
    shell_interactive_args: &[String],
    wrapped_command_shell_args: &[String],
    default_command: Option<&[String]>,
    parsed_args: &ParsedSandboxArgs,
) -> Vec<String> {
    if parsed_args.force_shell {
        let mut cmd = vec![shell_bin.to_string()];
        cmd.extend_from_slice(shell_interactive_args);
        cmd.extend_from_slice(&parsed_args.args);
        return cmd;
    }

    if parsed_args.interactive_command {
        let mut cmd = vec![shell_bin.to_string()];
        cmd.extend_from_slice(wrapped_command_shell_args);
        cmd.push("-c".to_string());
        cmd.push(INTERACTIVE_COMMAND_EXEC.to_string());
        cmd.push("--".to_string());
        cmd.extend_from_slice(&parsed_args.args);
        return cmd;
    }

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
/// This matches the shell-launching paths in [`build_run_cmd`]:
/// `--shell`, or the interactive shell fallback when there are no explicit
/// command args and no configured default command.
pub fn is_interactive(parsed_args: &ParsedSandboxArgs, default_command: Option<&[String]>) -> bool {
    parsed_args.force_shell
        || (parsed_args.args.is_empty()
            && default_command.is_none()
            && !parsed_args.explicit_command
            && !parsed_args.interactive_command)
}

/// Parse sandbox CLI arguments.
///
/// Modes:
/// - No args → interactive shell or default command
/// - `-c cmd [args...]` → explicit command mode (strip -c)
/// - `-i cmd [args...]` → run a command through the interactive shell startup path
/// - `--shell [args...]` → force the interactive shell and bypass `defaultCommand`
/// - `-- arg...` → pass args through literally without interpreting sandbox flags
/// - `cmd [args...]` → arguments for the default command when configured
pub fn parse_sandbox_args(args: &[String]) -> Result<ParsedSandboxArgs, &'static str> {
    if args.is_empty() {
        return Ok(ParsedSandboxArgs {
            args: Vec::new(),
            explicit_command: false,
            interactive_command: false,
            force_shell: false,
        });
    }

    if args[0] == "--" {
        Ok(ParsedSandboxArgs {
            args: args[1..].to_vec(),
            explicit_command: false,
            interactive_command: false,
            force_shell: false,
        })
    } else if args[0] == "--shell" {
        Ok(ParsedSandboxArgs {
            args: args[1..].to_vec(),
            explicit_command: false,
            interactive_command: false,
            force_shell: true,
        })
    } else if args[0] == "-c" {
        if args.len() == 1 {
            return Err("`-c` requires a command");
        }
        Ok(ParsedSandboxArgs {
            args: args[1..].to_vec(),
            explicit_command: true,
            interactive_command: false,
            force_shell: false,
        })
    } else if args[0] == "-i" {
        if args.len() == 1 {
            return Err("`-i` requires a command");
        }
        Ok(ParsedSandboxArgs {
            args: args[1..].to_vec(),
            explicit_command: false,
            interactive_command: true,
            force_shell: false,
        })
    } else {
        Ok(ParsedSandboxArgs {
            args: args.to_vec(),
            explicit_command: false,
            interactive_command: false,
            force_shell: false,
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
        assert!(!result.interactive_command);
        assert!(!result.force_shell);
    }

    #[test]
    fn parse_with_c_flag() {
        let args = vec!["-c".to_string(), "echo".to_string(), "hello".to_string()];
        let result = parse_sandbox_args(&args).unwrap();
        assert_eq!(result.args, vec!["echo", "hello"]);
        assert!(result.explicit_command);
        assert!(!result.interactive_command);
        assert!(!result.force_shell);
    }

    #[test]
    fn parse_with_i_flag() {
        let args = vec!["-i".to_string(), "echo".to_string(), "hello".to_string()];
        let result = parse_sandbox_args(&args).unwrap();
        assert_eq!(result.args, vec!["echo", "hello"]);
        assert!(!result.explicit_command);
        assert!(result.interactive_command);
        assert!(!result.force_shell);
    }

    #[test]
    fn parse_with_shell_flag() {
        let args = vec!["--shell".to_string()];
        let result = parse_sandbox_args(&args).unwrap();
        assert!(result.args.is_empty());
        assert!(!result.explicit_command);
        assert!(!result.interactive_command);
        assert!(result.force_shell);
    }

    #[test]
    fn parse_with_shell_flag_and_args() {
        let args = vec!["--shell".to_string(), "-l".to_string()];
        let result = parse_sandbox_args(&args).unwrap();
        assert_eq!(result.args, vec!["-l"]);
        assert!(!result.explicit_command);
        assert!(!result.interactive_command);
        assert!(result.force_shell);
    }

    #[test]
    fn parse_with_literal_separator() {
        let args = vec!["--".to_string(), "--version".to_string()];
        let result = parse_sandbox_args(&args).unwrap();
        assert_eq!(result.args, vec!["--version"]);
        assert!(!result.explicit_command);
        assert!(!result.force_shell);
    }

    #[test]
    fn parse_with_literal_separator_allows_c_flag() {
        let args = vec!["--".to_string(), "-c".to_string(), "echo".to_string()];
        let result = parse_sandbox_args(&args).unwrap();
        assert_eq!(result.args, vec!["-c", "echo"]);
        assert!(!result.explicit_command);
        assert!(!result.force_shell);
    }

    #[test]
    fn parse_with_literal_separator_allows_shell_flag() {
        let args = vec!["--".to_string(), "--shell".to_string()];
        let result = parse_sandbox_args(&args).unwrap();
        assert_eq!(result.args, vec!["--shell"]);
        assert!(!result.explicit_command);
        assert!(!result.force_shell);
    }

    #[test]
    fn parse_without_c_flag() {
        let args = vec!["echo".to_string(), "hello".to_string()];
        let result = parse_sandbox_args(&args).unwrap();
        assert_eq!(result.args, vec!["echo", "hello"]);
        assert!(!result.explicit_command);
        assert!(!result.force_shell);
    }

    #[test]
    fn parse_bare_c_flag_errors() {
        let args = vec!["-c".to_string()];
        let result = parse_sandbox_args(&args);
        assert!(matches!(result, Err("`-c` requires a command")));
    }

    #[test]
    fn parse_bare_i_flag_errors() {
        let args = vec!["-i".to_string()];
        let result = parse_sandbox_args(&args);
        assert!(matches!(result, Err("`-i` requires a command")));
    }

    #[test]
    fn build_interactive_cmd() {
        let cmd = build_run_cmd(
            "/bin/zsh",
            &["-i".to_string()],
            &["-ic".to_string()],
            None,
            &ParsedSandboxArgs {
                args: Vec::new(),
                explicit_command: false,
                interactive_command: false,
                force_shell: false,
            },
        );
        assert_eq!(cmd, vec!["/bin/zsh", "-i"]);
    }

    #[test]
    fn build_interactive_command_cmd() {
        let cmd = build_run_cmd(
            "/bin/zsh",
            &["-i".to_string()],
            &["-i".to_string()],
            None,
            &ParsedSandboxArgs {
                args: vec!["echo".to_string(), "hello".to_string()],
                explicit_command: false,
                interactive_command: true,
                force_shell: false,
            },
        );
        assert_eq!(
            cmd,
            vec![
                "/bin/zsh",
                "-i",
                "-c",
                INTERACTIVE_COMMAND_EXEC,
                "--",
                "echo",
                "hello",
            ]
        );
    }

    #[test]
    fn build_interactive_command_uses_wrapped_command_shell_args() {
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            &["-l".to_string(), "-i".to_string()],
            None,
            &ParsedSandboxArgs {
                args: vec!["echo".to_string(), "hello".to_string()],
                explicit_command: false,
                interactive_command: true,
                force_shell: false,
            },
        );
        assert_eq!(
            cmd,
            vec![
                "/bin/bash",
                "-l",
                "-i",
                "-c",
                INTERACTIVE_COMMAND_EXEC,
                "--",
                "echo",
                "hello",
            ]
        );
    }

    #[test]
    fn build_shell_override_cmd() {
        let default = vec!["evince".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            &["-lc".to_string()],
            Some(&default),
            &ParsedSandboxArgs {
                args: Vec::new(),
                explicit_command: false,
                interactive_command: false,
                force_shell: true,
            },
        );
        assert_eq!(cmd, vec!["/bin/bash", "-l"]);
    }

    #[test]
    fn build_default_command() {
        let default = vec!["evince".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            &["-lc".to_string()],
            Some(&default),
            &ParsedSandboxArgs {
                args: Vec::new(),
                explicit_command: false,
                interactive_command: false,
                force_shell: false,
            },
        );
        assert_eq!(cmd, vec!["evince"]);
    }

    #[test]
    fn build_explicit_args_append_to_default() {
        let default = vec!["evince".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            &["-lc".to_string()],
            Some(&default),
            &ParsedSandboxArgs {
                args: vec!["document.pdf".to_string()],
                explicit_command: false,
                interactive_command: false,
                force_shell: false,
            },
        );
        assert_eq!(cmd, vec!["evince", "document.pdf"]);
    }

    #[test]
    fn build_flag_args_prepend_default_command() {
        let default = vec!["helium".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            &["-lc".to_string()],
            Some(&default),
            &ParsedSandboxArgs {
                args: vec!["-ozone-platform=wayland".to_string()],
                explicit_command: false,
                interactive_command: false,
                force_shell: false,
            },
        );
        assert_eq!(cmd, vec!["helium", "-ozone-platform=wayland"]);
    }

    #[test]
    fn build_explicit_command_overrides_default() {
        let default = vec!["evince".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            &["-lc".to_string()],
            Some(&default),
            &ParsedSandboxArgs {
                args: vec!["mpv".to_string(), "video.mp4".to_string()],
                explicit_command: true,
                interactive_command: false,
                force_shell: false,
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
                interactive_command: false,
                force_shell: false,
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
                interactive_command: false,
                force_shell: false,
            },
            None
        ));
    }

    #[test]
    fn not_interactive_with_default_command() {
        let default = vec!["evince".to_string()];
        assert!(!is_interactive(
            &ParsedSandboxArgs {
                args: Vec::new(),
                explicit_command: false,
                interactive_command: false,
                force_shell: false,
            },
            Some(&default)
        ));
    }

    #[test]
    fn not_interactive_with_both() {
        let default = vec!["evince".to_string()];
        assert!(!is_interactive(
            &ParsedSandboxArgs {
                args: vec!["mpv".to_string()],
                explicit_command: false,
                interactive_command: false,
                force_shell: false,
            },
            Some(&default)
        ));
    }

    #[test]
    fn build_flag_args_without_default_passes_through() {
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            &["-lc".to_string()],
            None,
            &ParsedSandboxArgs {
                args: vec!["-ozone-platform=wayland".to_string()],
                explicit_command: false,
                interactive_command: false,
                force_shell: false,
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
                interactive_command: false,
                force_shell: false,
            },
            None
        ));
    }

    #[test]
    fn interactive_command_is_not_interactive() {
        assert!(!is_interactive(
            &ParsedSandboxArgs {
                args: vec!["echo".to_string()],
                explicit_command: false,
                interactive_command: true,
                force_shell: false,
            },
            None
        ));
    }

    #[test]
    fn shell_override_is_interactive() {
        let default = vec!["evince".to_string()];
        assert!(is_interactive(
            &ParsedSandboxArgs {
                args: Vec::new(),
                explicit_command: false,
                interactive_command: false,
                force_shell: true,
            },
            Some(&default)
        ));
    }
}
