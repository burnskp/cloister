//! Sandbox configuration schema.
//!
//! Deserialized from a JSON file in the Nix store. Each sandbox gets its own
//! config file, and the binary is invoked via `--config /nix/store/...-config-<name>.json`.

use serde::Deserialize;
use std::collections::HashMap;

/// Top-level sandbox configuration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxConfig {
    /// Sandbox identity
    pub name: String,
    pub bwrap_path: String,
    pub shell_bin: String,
    pub shell_interactive_args: Vec<String>,
    pub shell_name: String,
    /// Command to run when no arguments are given. If null, launches interactive shell.
    #[serde(default)]
    pub default_command: Option<Vec<String>>,

    /// Feature flags
    #[serde(default)]
    pub network_enable: bool,
    #[serde(default)]
    pub network_namespace: Option<String>,
    #[serde(default)]
    pub wayland_enable: bool,
    #[serde(default)]
    pub wayland_security_context: bool,
    #[serde(default)]
    pub x11_enable: bool,
    #[serde(default)]
    pub gpu_enable: bool,
    #[serde(default)]
    pub gpu_shm: bool,
    #[serde(default)]
    pub ssh_enable: bool,
    #[serde(default)]
    pub pulseaudio_enable: bool,
    #[serde(default)]
    pub pipewire_socket_name: Option<String>,
    #[serde(default)]
    pub pipewire_pulse_wrapper_path: Option<String>,
    #[serde(default)]
    pub fido2_enable: bool,
    #[serde(default)]
    pub video_enable: bool,
    #[serde(default)]
    pub printing_enable: bool,
    #[serde(default)]
    pub dbus_enable: bool,
    #[serde(default)]
    pub seccomp_enable: bool,
    #[serde(default)]
    pub git_enable: bool,
    #[serde(default)]
    pub anonymize: bool,
    #[serde(default = "default_true")]
    pub shell_host_config: bool,
    #[serde(default = "default_true")]
    pub bind_working_directory: bool,

    /// SSH config
    #[serde(default)]
    pub ssh_allow_fingerprints: Vec<String>,
    #[serde(default = "default_ssh_timeout")]
    pub ssh_filter_timeout_seconds: u64,

    /// Paths
    pub home_directory: String,
    pub sandbox_home: String,
    #[serde(default)]
    pub seccomp_filter_path: Option<String>,
    pub per_dir_base: String,
    pub copy_file_base: String,
    #[serde(default)]
    pub netns_helper_path: Option<String>,
    pub git_path: String,
    #[serde(default)]
    pub init_path: Option<String>,

    /// Static bwrap args: pre-computed by Nix (dirs, tmpfs, symlinks, store-path binds, env)
    #[serde(default)]
    pub static_bwrap_args: Vec<String>,

    /// Runtime-resolved bind specifications
    #[serde(default)]
    pub dynamic_binds: Vec<DynamicBind>,

    /// Environment
    #[serde(default)]
    pub passthrough_env: Vec<String>,
    #[serde(default)]
    pub disallowed_paths: Vec<String>,
    #[serde(default)]
    pub dangerous_paths: Vec<String>,
    #[serde(default)]
    pub allow_dangerous_paths: Vec<String>,
    #[serde(default = "default_true")]
    pub dangerous_path_warnings: bool,
    #[serde(default)]
    pub dev_binds: Vec<String>,
    #[serde(default)]
    pub per_dir_paths: Vec<String>,
    /// All bind sources (static + dynamic) for runtime safety validation.
    #[serde(default)]
    pub bind_sources: Vec<String>,

    /// File operations
    #[serde(default)]
    pub dir_mkdirs: Vec<MkdirSpec>,
    #[serde(default)]
    pub file_mkdirs: Vec<FileMkdirSpec>,
    #[serde(default)]
    pub managed_file_host_mkdirs: Vec<String>,
    #[serde(default)]
    pub copy_files: Vec<CopyFileSpec>,

    /// Strict home policy
    #[serde(default = "default_true")]
    pub enforce_strict_home_policy: bool,

    /// D-Bus proxy socket name relative to XDG_RUNTIME_DIR (e.g. "cloister/dbus/<name>")
    #[serde(default)]
    pub dbus_proxy_socket_name: Option<String>,
}

/// A bind mount that needs runtime variable substitution.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicBind {
    pub src: String,
    pub dest: Option<String>,
    pub mode: BindMode,
    #[serde(default)]
    pub try_bind: bool,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BindMode {
    Ro,
    Rw,
}

/// Directory to create on the host before launching bwrap.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MkdirSpec {
    pub path: String,
}

/// File to create (touch) on the host before launching bwrap.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileMkdirSpec {
    pub path: String,
}

/// File to copy into sandbox state.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopyFileSpec {
    pub src: String,
    pub host_dest: String,
    pub mode: String,
    #[serde(default)]
    pub overwrite: bool,
}

fn default_ssh_timeout() -> u64 {
    60
}

fn default_true() -> bool {
    true
}

impl SandboxConfig {
    /// Load and validate a config from a JSON file path.
    /// The path must be in the Nix store for security.
    pub fn load(path: &str) -> Result<Self, String> {
        let canon = std::fs::canonicalize(path)
            .map_err(|e| format!("failed to resolve config path {path}: {e}"))?;
        if !canon.starts_with("/nix/store/") {
            return Err(format!(
                "config path must resolve under /nix/store/: {}",
                canon.display()
            ));
        }
        let meta = std::fs::metadata(&canon)
            .map_err(|e| format!("failed to stat config {}: {e}", canon.display()))?;
        if !meta.is_file() {
            return Err(format!("config is not a regular file: {}", canon.display()));
        }
        let data = std::fs::read_to_string(&canon)
            .map_err(|e| format!("failed to read config {}: {e}", canon.display()))?;
        let config: SandboxConfig = serde_json::from_str(&data)
            .map_err(|e| format!("failed to parse config {}: {e}", canon.display()))?;
        Ok(config)
    }

    /// Validate that required config fields are non-empty.
    pub fn validate(&self) -> Result<(), String> {
        if self.home_directory.is_empty() {
            return Err("home_directory must not be empty".into());
        }
        if self.sandbox_home.is_empty() {
            return Err("sandbox_home must not be empty".into());
        }
        Ok(())
    }

    /// Whether SSH filtering (not just passthrough) is enabled.
    pub fn ssh_filter_enabled(&self) -> bool {
        self.ssh_enable && !self.ssh_allow_fingerprints.is_empty()
    }

    /// Resolve runtime variables in a path string.
    /// Substitutes $HOME, $SANDBOX_HOME, $SANDBOX_DIR, $SANDBOX_DEST, $DIR_HASH,
    /// $XDG_RUNTIME_DIR.
    pub fn resolve_path(&self, template: &str, vars: &HashMap<String, String>) -> String {
        crate::vars::expand_vars(template, vars)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_config_json() -> String {
        serde_json::json!({
            "name": "test",
            "bwrap_path": "/nix/store/xxx-bubblewrap/bin/bwrap",
            "shell_bin": "/nix/store/xxx-zsh/bin/zsh",
            "shell_interactive_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/user",
            "per_dir_base": "/home/user/.local/state/cloister",
            "copy_file_base": "/home/user/.local/state/cloister",
            "git_path": "/nix/store/xxx-git/bin/git",
        })
        .to_string()
    }

    #[test]
    fn deserialize_minimal() {
        let json = minimal_config_json();
        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.name, "test");
        assert!(!config.network_enable);
        assert!(!config.wayland_enable);
        assert!(config.pipewire_pulse_wrapper_path.is_none());
        assert!(config.enforce_strict_home_policy);
        assert!(config.shell_host_config);
        assert_eq!(config.ssh_filter_timeout_seconds, 60);
    }

    #[test]
    fn deserialize_full() {
        let json = serde_json::json!({
            "name": "dev",
            "bwrap_path": "/nix/store/xxx/bin/bwrap",
            "shell_bin": "/nix/store/xxx/bin/zsh",
            "shell_interactive_args": ["-i"],
            "shell_name": "zsh",
            "shell_host_config": false,
            "network_enable": true,
            "wayland_enable": true,
            "wayland_security_context": true,
            "ssh_enable": true,
            "ssh_allow_fingerprints": ["SHA256:abc", "SHA256:def"],
            "ssh_filter_timeout_seconds": 30,
            "pipewire_pulse_wrapper_path": "/nix/store/xxx-wrapper",
            "home_directory": "/home/user",
            "sandbox_home": "/home/ubuntu",
            "anonymize": true,
            "per_dir_base": "/state/cloister",
            "copy_file_base": "/state/cloister",
            "git_path": "/nix/store/xxx/bin/git",
            "static_bwrap_args": ["--dir", "/var", "--tmpfs", "/tmp"],
            "passthrough_env": ["LANG", "TERM"],
            "disallowed_paths": ["/", "/root"],
            "per_dir_paths": [".cache", ".local/share"],
            "dbus_proxy_socket_name": "cloister/dbus/dev",
        })
        .to_string();

        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.name, "dev");
        assert!(config.network_enable);
        assert!(config.wayland_enable);
        assert!(config.wayland_security_context);
        assert!(config.ssh_filter_enabled());
        assert_eq!(
            config.pipewire_pulse_wrapper_path.as_deref(),
            Some("/nix/store/xxx-wrapper")
        );
        assert_eq!(config.ssh_allow_fingerprints.len(), 2);
        assert_eq!(config.ssh_filter_timeout_seconds, 30);
        assert!(!config.shell_host_config);
        assert!(config.anonymize);
        assert_eq!(config.static_bwrap_args.len(), 4);
    }

    #[test]
    fn ssh_filter_enabled_check() {
        let json = minimal_config_json();
        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        assert!(!config.ssh_filter_enabled());
    }

    #[test]
    fn resolve_path_substitution() {
        let json = minimal_config_json();
        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        let mut vars = HashMap::new();
        vars.insert("HOME".to_string(), "/home/user".to_string());
        vars.insert("SANDBOX_DIR".to_string(), "/projects/myapp".to_string());
        vars.insert("DIR_HASH".to_string(), "abc123".to_string());

        assert_eq!(
            config.resolve_path("$HOME/.config/git", &vars),
            "/home/user/.config/git"
        );
        assert_eq!(
            config.resolve_path("$SANDBOX_DIR", &vars),
            "/projects/myapp"
        );
    }

    #[test]
    fn load_rejects_non_nix_store_path() {
        let result = SandboxConfig::load("/tmp/config.json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("resolve config path"));
    }

    #[test]
    fn validate_rejects_empty_home_directory() {
        let json = serde_json::json!({
            "name": "test",
            "bwrap_path": "/nix/store/xxx/bin/bwrap",
            "shell_bin": "/nix/store/xxx/bin/zsh",
            "shell_interactive_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "",
            "sandbox_home": "/home/user",
            "per_dir_base": "/state",
            "copy_file_base": "/state",
            "git_path": "/nix/store/xxx/bin/git",
        })
        .to_string();
        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("home_directory"));
    }

    #[test]
    fn validate_rejects_empty_sandbox_home() {
        let json = serde_json::json!({
            "name": "test",
            "bwrap_path": "/nix/store/xxx/bin/bwrap",
            "shell_bin": "/nix/store/xxx/bin/zsh",
            "shell_interactive_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "",
            "per_dir_base": "/state",
            "copy_file_base": "/state",
            "git_path": "/nix/store/xxx/bin/git",
        })
        .to_string();
        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("sandbox_home"));
    }

    #[test]
    fn validate_accepts_valid_config() {
        let json = minimal_config_json();
        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn load_rejects_nix_store_prefix_traversal() {
        let temp_path =
            std::env::temp_dir().join(format!("cloister-config-test-{}.json", std::process::id()));
        std::fs::write(&temp_path, minimal_config_json()).unwrap();

        let traversal = format!(
            "/nix/store/../tmp/{}",
            temp_path.file_name().unwrap().to_string_lossy()
        );
        let result = SandboxConfig::load(&traversal);
        let _ = std::fs::remove_file(&temp_path);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("/nix/store/"));
    }
}
