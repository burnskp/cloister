//! Sandbox configuration schema.
//!
//! Deserialized from a JSON file in the Nix store. Each sandbox gets its own
//! config file, and the binary is invoked via `--config /nix/store/...-config-<name>.json`.

use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum StoreMode {
    #[default]
    Host,
    ImageStore,
}

/// Top-level sandbox configuration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxConfig {
    /// Sandbox identity
    pub name: String,
    pub bwrap_path: String,
    pub shell_bin: String,
    pub shell_interactive_args: Vec<String>,
    pub wrapped_command_shell_args: Vec<String>,
    pub shell_name: String,
    /// Command to run when no arguments are given. If null, launches interactive shell.
    #[serde(default)]
    pub default_command: Option<Vec<String>>,

    /// Feature flags
    #[serde(default)]
    pub network_enable: bool,
    #[serde(default)]
    pub network_namespace: Option<String>,
    /// Explicit DNS configuration suppresses the namespace resolver fallback.
    #[serde(default)]
    pub network_dns_override: bool,
    #[serde(default)]
    pub gui_enable: bool,
    #[serde(default)]
    pub ssh_enable: bool,
    #[serde(default)]
    pub pipewire_backend_socket_name: Option<String>,
    #[serde(default)]
    pub pipewire_socket_name: Option<String>,
    #[serde(default)]
    pub pipewire_pulse_binary_path: Option<String>,
    #[serde(default)]
    pub pipewire_pulse_config_path: Option<String>,
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
    #[serde(default)]
    pub store_mode: StoreMode,
    #[serde(default)]
    pub store_roots: Vec<String>,
    #[serde(default)]
    pub store_id: Option<String>,
    #[serde(default)]
    pub store_image_path: Option<String>,
    #[serde(default)]
    pub store_mount_path: Option<String>,

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
    #[serde(default)]
    pub per_dir: BTreeMap<String, Vec<String>>,
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
    pub dev_binds: Vec<String>,

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

    /// D-Bus proxy wrapper path in the Nix store.
    #[serde(default)]
    pub dbus_proxy_path: Option<String>,

    /// Synthetic Flatpak app id used for portal integration.
    #[serde(default)]
    pub flatpak_app_id: Option<String>,

    /// Worker broker config passed through from Nix.
    #[serde(default)]
    pub worker_broker: WorkerBrokerConfig,

    /// Build revision string embedded into generated launchers and configs.
    #[serde(default)]
    pub build_revision: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerBrokerConfig {
    #[serde(default)]
    pub profiles: BTreeMap<String, SpawnableProfile>,
    #[serde(default)]
    pub generated_launchers: BTreeMap<String, GeneratedLauncher>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedLauncher {
    pub profile: String,
    pub sandbox: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnableProfile {
    pub sandbox: String,
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub delegated_per_dir_mounts: BTreeMap<String, DelegatedPerDirMount>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub mode: WorkspaceMode,
}

#[derive(Debug, serde::Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceMode {
    ProjectRw,
    ProjectOverlay,
}

#[derive(Debug, serde::Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DelegatedAccessMode {
    Ro,
    Rw,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DelegatedPerDirMount {
    pub mode: DelegatedAccessMode,
    pub path: String,
    #[serde(default)]
    pub sub_path: Option<String>,
}

/// A bind mount that needs runtime variable substitution.
#[derive(Debug, Deserialize, Clone)]
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
        if self.anonymize && self.anonymized_identity().is_none() {
            return Err(
                "sandbox_home must end with a non-empty final path component when anonymize is enabled"
                    .into(),
            );
        }
        let portal_desktop_enabled = self.flatpak_app_id.is_some();
        if self.dbus_enable
            && self
                .dbus_proxy_socket_name
                .as_deref()
                .unwrap_or("")
                .is_empty()
        {
            return Err("dbus_proxy_socket_name must not be empty when dbus is enabled".into());
        }
        if self.dbus_enable && self.dbus_proxy_path.as_deref().unwrap_or("").is_empty() {
            return Err("dbus_proxy_path must not be empty when dbus is enabled".into());
        }
        if portal_desktop_enabled && !self.dbus_enable {
            return Err("portal integration requires dbus_enable".into());
        }
        if portal_desktop_enabled && self.flatpak_app_id.as_deref().unwrap_or("").is_empty() {
            return Err(
                "flatpak_app_id must not be empty when portal integration is enabled".into(),
            );
        }
        if !self.worker_broker.profiles.is_empty() && !self.bind_working_directory {
            return Err("worker_broker.profiles requires bind_working_directory = true".into());
        }
        if self.store_mode == StoreMode::ImageStore {
            if self.store_id.as_deref().unwrap_or("").is_empty() {
                return Err("store_id must not be empty when store_mode is image-store".into());
            }
            if self.store_image_path.as_deref().unwrap_or("").is_empty() {
                return Err(
                    "store_image_path must not be empty when store_mode is image-store".into(),
                );
            }
            if self.store_mount_path.as_deref().unwrap_or("").is_empty() {
                return Err(
                    "store_mount_path must not be empty when store_mode is image-store".into(),
                );
            }
            if self.store_roots.is_empty() {
                return Err("store_roots must not be empty when store_mode is image-store".into());
            }
        }
        Ok(())
    }

    /// Return the configured anonymized identity when anonymization is enabled.
    pub fn anonymized_identity(&self) -> Option<&str> {
        if !self.anonymize {
            return None;
        }

        let path = Path::new(&self.sandbox_home);
        let parent = path.parent()?;
        if parent != Path::new("/home") {
            return None;
        }

        path.file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty() && *name != "." && *name != "..")
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
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/user",
            "per_dir": {},
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
        assert_eq!(config.shell_interactive_args, vec!["-i"]);
        assert_eq!(config.wrapped_command_shell_args, vec!["-i"]);
        assert!(!config.network_enable);
        assert!(!config.network_dns_override);
        assert!(!config.gui_enable);
        assert!(config.pipewire_backend_socket_name.is_none());
        assert!(config.pipewire_pulse_binary_path.is_none());
        assert!(config.pipewire_pulse_config_path.is_none());
        assert!(config.enforce_strict_home_policy);
        assert!(config.shell_host_config);
        assert_eq!(config.ssh_filter_timeout_seconds, 60);
        assert_eq!(config.store_mode, StoreMode::Host);
        assert!(config.store_roots.is_empty());
        assert!(config.dbus_proxy_path.is_none());
        assert!(config.flatpak_app_id.is_none());
        assert!(config.build_revision.is_none());
    }

    #[test]
    fn deserialize_full() {
        let json = serde_json::json!({
            "name": "dev",
            "bwrap_path": "/nix/store/xxx/bin/bwrap",
            "shell_bin": "/nix/store/xxx/bin/zsh",
            "shell_interactive_args": ["-i"],
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "shell_host_config": false,
            "network_enable": true,
            "gui_enable": true,
            "ssh_enable": true,
            "ssh_allow_fingerprints": ["SHA256:abc", "SHA256:def"],
            "ssh_filter_timeout_seconds": 30,
            "pipewire_backend_socket_name": "cloister/pipewire/dev",
            "pipewire_pulse_binary_path": "/nix/store/xxx-pipewire/bin/pipewire-pulse",
            "pipewire_pulse_config_path": "/nix/store/xxx-pulse.conf",
            "home_directory": "/home/user",
            "sandbox_home": "/home/ubuntu",
            "anonymize": true,
            "store_mode": "image-store",
            "store_roots": [
                "/nix/store/xxx-hello"
            ],
            "store_id": "abc123",
            "store_image_path": "/var/lib/cloister/images/abc123.squashfs",
            "store_mount_path": "/run/cloister/images/abc123",
            "per_dir": {
                "/state/cloister": [".cache", ".local/share"]
            },
            "copy_file_base": "/state/cloister",
            "git_path": "/nix/store/xxx/bin/git",
            "static_bwrap_args": ["--dir", "/var", "--tmpfs", "/tmp"],
            "passthrough_env": ["LANG", "TERM"],
            "disallowed_paths": ["/", "/root"],
            "dbus_proxy_socket_name": "cloister/dbus/dev",
            "dbus_proxy_path": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-cloister-dbus-proxy-dev",
            "flatpak_app_id": "dev.cloister.dev",
            "build_revision": "source-1234567890-abcdef",
        })
        .to_string();

        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.name, "dev");
        assert!(config.network_enable);
        assert!(config.gui_enable);
        assert!(config.ssh_filter_enabled());
        assert_eq!(
            config.pipewire_backend_socket_name.as_deref(),
            Some("cloister/pipewire/dev")
        );
        assert_eq!(
            config.pipewire_pulse_binary_path.as_deref(),
            Some("/nix/store/xxx-pipewire/bin/pipewire-pulse")
        );
        assert_eq!(
            config.pipewire_pulse_config_path.as_deref(),
            Some("/nix/store/xxx-pulse.conf")
        );
        assert_eq!(config.ssh_allow_fingerprints.len(), 2);
        assert_eq!(config.ssh_filter_timeout_seconds, 30);
        assert!(!config.shell_host_config);
        assert!(config.anonymize);
        assert_eq!(
            config.dbus_proxy_path.as_deref(),
            Some("/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-cloister-dbus-proxy-dev")
        );
        assert_eq!(config.flatpak_app_id.as_deref(), Some("dev.cloister.dev"));
        assert_eq!(
            config.build_revision.as_deref(),
            Some("source-1234567890-abcdef")
        );
        assert_eq!(config.store_mode, StoreMode::ImageStore);
        assert_eq!(config.store_roots.len(), 1);
        assert_eq!(config.static_bwrap_args.len(), 4);
        assert_eq!(
            config.per_dir.get("/state/cloister"),
            Some(&vec![".cache".to_string(), ".local/share".to_string()])
        );
    }

    #[test]
    fn parses_worker_broker_config() {
        let json = serde_json::json!({
            "name": "dev",
            "bwrap_path": "/nix/store/xxx/bin/bwrap",
            "shell_bin": "/nix/store/xxx/bin/zsh",
            "shell_interactive_args": ["-i"],
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/user",
            "per_dir": {},
            "copy_file_base": "/state/cloister",
            "git_path": "/nix/store/xxx/bin/git",
            "worker_broker": {
                "profiles": {
                    "ephemeral": {
                        "sandbox": "worker",
                        "workspace": {
                            "mode": "project-overlay"
                        },
                        "delegated_per_dir_mounts": {
                            ".cache/pre-commit": {
                                "mode": "ro",
                                "path": "/local/ephemeral/dev",
                                "sub_path": ".cache/pre-commit"
                            }
                        }
                    }
                },
                "generated_launchers": {
                    "clb-ephemeral": {
                        "profile": "ephemeral",
                        "sandbox": "worker"
                    }
                }
            }
        })
        .to_string();

        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        let worker_broker = &config.worker_broker;
        let profile = worker_broker
            .profiles
            .get("ephemeral")
            .expect("ephemeral profile");
        let generated_launcher = worker_broker
            .generated_launchers
            .get("clb-ephemeral")
            .expect("generated launcher");
        assert_eq!(config.name, "dev");
        assert!(!worker_broker.profiles.is_empty());
        assert_eq!(profile.sandbox, "worker");
        assert_eq!(profile.workspace.mode, WorkspaceMode::ProjectOverlay);
        assert_eq!(generated_launcher.profile, "ephemeral");
        assert_eq!(generated_launcher.sandbox, "worker");
        let delegated_mount = profile
            .delegated_per_dir_mounts
            .get(".cache/pre-commit")
            .expect("delegated mount");
        assert_eq!(delegated_mount.mode, DelegatedAccessMode::Ro);
        assert_eq!(delegated_mount.path, "/local/ephemeral/dev");
        assert_eq!(
            delegated_mount.sub_path.as_deref(),
            Some(".cache/pre-commit")
        );
    }

    #[test]
    fn validate_rejects_worker_broker_without_bind_working_directory() {
        let json = serde_json::json!({
            "name": "dev",
            "bwrap_path": "/nix/store/xxx/bin/bwrap",
            "shell_bin": "/nix/store/xxx/bin/zsh",
            "shell_interactive_args": ["-i"],
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/user",
            "bind_working_directory": false,
            "per_dir": {},
            "copy_file_base": "/state/cloister",
            "git_path": "/nix/store/xxx/bin/git",
            "worker_broker": {
                "profiles": {
                    "ephemeral": {
                        "sandbox": "worker",
                        "workspace": {
                            "mode": "project-overlay"
                        }
                    }
                }
            }
        })
        .to_string();

        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.contains("worker_broker.profiles requires bind_working_directory"));
    }

    #[test]
    fn validate_rejects_missing_image_store_fields() {
        let json = serde_json::json!({
            "name": "test",
            "bwrap_path": "/nix/store/xxx/bin/bwrap",
            "shell_bin": "/nix/store/xxx/bin/zsh",
            "shell_interactive_args": ["-i"],
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/user",
            "store_mode": "image-store",
            "per_dir": {},
            "copy_file_base": "/state",
            "git_path": "/nix/store/xxx/bin/git"
        })
        .to_string();
        let config: SandboxConfig = serde_json::from_str(&json).unwrap();
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("store_id"));
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
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "",
            "sandbox_home": "/home/user",
            "per_dir": {},
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
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "",
            "per_dir": {},
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
    fn validate_rejects_portal_without_dbus() {
        let json = serde_json::json!({
            "name": "test",
            "bwrap_path": "/nix/store/xxx/bin/bwrap",
            "shell_bin": "/nix/store/xxx/bin/zsh",
            "shell_interactive_args": ["-i"],
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/user",
            "per_dir": {},
            "copy_file_base": "/state",
            "git_path": "/nix/store/xxx/bin/git",
            "flatpak_app_id": "dev.cloister.test",
        })
        .to_string();
        let config: SandboxConfig = serde_json::from_str(&json).unwrap();

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("portal integration requires dbus_enable")
        );
    }

    #[test]
    fn anonymized_identity_uses_sandbox_home_leaf() {
        let json = serde_json::json!({
            "name": "test",
            "bwrap_path": "/nix/store/xxx/bin/bwrap",
            "shell_bin": "/nix/store/xxx/bin/zsh",
            "shell_interactive_args": ["-i"],
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/devuser",
            "anonymize": true,
            "per_dir": {},
            "copy_file_base": "/state",
            "git_path": "/nix/store/xxx/bin/git",
        })
        .to_string();
        let config: SandboxConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.anonymized_identity(), Some("devuser"));
    }

    #[test]
    fn validate_rejects_anonymize_with_invalid_sandbox_home_leaf() {
        let json = serde_json::json!({
            "name": "test",
            "bwrap_path": "/nix/store/xxx/bin/bwrap",
            "shell_bin": "/nix/store/xxx/bin/zsh",
            "shell_interactive_args": ["-i"],
            "wrapped_command_shell_args": ["-i"],
            "shell_name": "zsh",
            "home_directory": "/home/user",
            "sandbox_home": "/home/",
            "anonymize": true,
            "per_dir": {},
            "copy_file_base": "/state",
            "git_path": "/nix/store/xxx/bin/git",
        })
        .to_string();
        let config: SandboxConfig = serde_json::from_str(&json).unwrap();

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("sandbox_home must end with a non-empty final path component")
        );
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
