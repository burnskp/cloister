//! Environment variable handling for sandboxes.

/// Build the run command: explicit args > default command > interactive shell.
pub fn build_run_cmd(
    shell_bin: &str,
    shell_interactive_args: &[String],
    default_command: Option<&[String]>,
    command_args: &[String],
) -> Vec<String> {
    if !command_args.is_empty() {
        // If args start with a flag and we have a default command,
        // prepend the default command so the binary name is always present
        if command_args[0].starts_with('-') {
            if let Some(default_cmd) = default_command {
                let mut cmd = default_cmd.to_vec();
                cmd.extend_from_slice(command_args);
                return cmd;
            }
        }
        // Explicit command args take priority
        command_args.to_vec()
    } else if let Some(default_cmd) = default_command {
        // Default command when no args given
        default_cmd.to_vec()
    } else {
        // Interactive shell fallback
        let mut cmd = vec![shell_bin.to_string()];
        cmd.extend_from_slice(shell_interactive_args);
        cmd
    }
}

/// Parse sandbox CLI arguments.
///
/// Modes:
/// - No args → interactive shell
/// - `-c cmd [args...]` → command mode (strip -c)
/// - `cmd [args...]` → command mode (pass through)
pub fn parse_sandbox_args(args: &[String]) -> Vec<String> {
    if args.is_empty() {
        return Vec::new();
    }
    if args[0] == "-c" {
        args[1..].to_vec()
    } else {
        args.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_args() {
        let result = parse_sandbox_args(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn parse_with_c_flag() {
        let args = vec!["-c".to_string(), "echo".to_string(), "hello".to_string()];
        let result = parse_sandbox_args(&args);
        assert_eq!(result, vec!["echo", "hello"]);
    }

    #[test]
    fn parse_without_c_flag() {
        let args = vec!["echo".to_string(), "hello".to_string()];
        let result = parse_sandbox_args(&args);
        assert_eq!(result, vec!["echo", "hello"]);
    }

    #[test]
    fn build_interactive_cmd() {
        let cmd = build_run_cmd("/bin/zsh", &["-i".to_string()], None, &[]);
        assert_eq!(cmd, vec!["/bin/zsh", "-i"]);
    }

    #[test]
    fn build_command_cmd() {
        let cmd = build_run_cmd(
            "/bin/zsh",
            &["-i".to_string()],
            None,
            &["echo".to_string(), "hello".to_string()],
        );
        assert_eq!(cmd, vec!["echo", "hello"]);
    }

    #[test]
    fn build_default_command() {
        let default = vec!["geeqie".to_string()];
        let cmd = build_run_cmd("/bin/bash", &["-l".to_string()], Some(&default), &[]);
        assert_eq!(cmd, vec!["geeqie"]);
    }

    #[test]
    fn build_explicit_overrides_default() {
        let default = vec!["geeqie".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            Some(&default),
            &["mpv".to_string(), "video.mp4".to_string()],
        );
        assert_eq!(cmd, vec!["mpv", "video.mp4"]);
    }

    #[test]
    fn build_flag_args_prepend_default_command() {
        let default = vec!["helium".to_string()];
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            Some(&default),
            &["-ozone-platform=wayland".to_string()],
        );
        assert_eq!(cmd, vec!["helium", "-ozone-platform=wayland"]);
    }

    #[test]
    fn build_flag_args_without_default_passes_through() {
        let cmd = build_run_cmd(
            "/bin/bash",
            &["-l".to_string()],
            None,
            &["-ozone-platform=wayland".to_string()],
        );
        assert_eq!(cmd, vec!["-ozone-platform=wayland"]);
    }
}
