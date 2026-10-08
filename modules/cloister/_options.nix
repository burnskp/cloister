{
  lib,
  config,
  pkgs,
  ...
}@args:
let
  shells = import ./_mkShells.nix { inherit pkgs lib; };
  patterns = import ./_patterns.nix;

  inherit (config.cloister) defaultShell;
  inherit (config.xdg) configHome;

  bindEntryType = lib.types.oneOf [
    lib.types.str
    (lib.types.submodule {
      options = {
        src = lib.mkOption {
          type = lib.types.oneOf [
            lib.types.path
            lib.types.str
          ];
          description = "Source path on host. Relative paths are resolved under the sandbox home.";
        };
        dest = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Destination inside sandbox. Defaults to src when null.";
        };
        optional = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "If true, missing source paths are ignored.";
        };
      };
    })
  ];

  managedEntryType = lib.types.oneOf [
    lib.types.str
    (lib.types.submodule {
      options = {
        src = lib.mkOption {
          type = lib.types.oneOf [
            lib.types.path
            lib.types.str
          ];
          description = "Source file or tree to bind read-only into the sandbox.";
        };
        dest = lib.mkOption {
          type = lib.types.str;
          description = "Home-relative destination path for the read-only bind.";
        };
      };
    })
  ];

  copyEntryType = lib.types.submodule {
    options = {
      src = lib.mkOption {
        type = lib.types.str;
        description = "Absolute source path to copy from. Missing or non-regular sources fail startup.";
      };
      dest = lib.mkOption {
        type = lib.types.str;
        description = "Destination path inside the sandbox home.";
      };
      mode = lib.mkOption {
        type = lib.types.str;
        default = "0644";
        description = "File permissions mode (for example, '0644').";
      };
      overwrite = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "If true, overwrites the writable sandbox-state copy on each launch.";
      };
    };
  };

  symlinkEntryType = lib.types.submodule {
    options = {
      target = lib.mkOption {
        type = lib.types.str;
        description = "Symlink target. May be relative to the link's parent or absolute inside the sandbox.";
      };
      link = lib.mkOption {
        type = lib.types.str;
        description = "Symlink path inside the sandbox. Relative paths are resolved under the sandbox home.";
      };
    };
  };

  dbusPolicySubmodule = {
    options = {
      talk = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = ''
          Well-known bus names to allow TALK access for (method calls and signals).
          Supports a ".*" suffix to match sub-names.
        '';
      };

      own = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = ''
          Well-known bus names to allow OWN access for (RequestName/ReleaseName).
          Supports a ".*" suffix to match sub-names.
        '';
      };

      see = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = ''
          Well-known bus names to allow SEE access for (visibility in ListNames, NameOwnerChanged).
          Supports a ".*" suffix to match sub-names.
        '';
      };

      call = lib.mkOption {
        type = lib.types.attrsOf (lib.types.listOf lib.types.str);
        default = { };
        description = ''
          Per-name rules for allowed method calls. Keys are well-known bus names,
          values are lists of RULE strings in the form [METHOD][@PATH].
        '';
      };

      broadcast = lib.mkOption {
        type = lib.types.attrsOf (lib.types.listOf lib.types.str);
        default = { };
        description = ''
          Per-name rules for allowed broadcast signals. Keys are well-known bus names,
          values are lists of RULE strings in the form [METHOD][@PATH].
        '';
      };
    };
  };

  # The per-sandbox submodule: options + defaults + registry rendering.
  # This is the ONLY place that writes to config.cloister.sandboxes.<name>.
  # External modules (_sandbox.nix, _registry.nix, _wrappers.nix) only READ.
  sandboxModule =
    { name, config, ... }:
    let
      # --- Registry rendering (computed from submodule's own config) ---
      regCfg = config.registry;
      shellLib = shells.${config.shell.name};
      basePackages = [
        pkgs.bash
        pkgs.coreutils
        pkgs.curl
        pkgs.findutils
        pkgs.gawk
        pkgs.git
        pkgs.gnugrep
        pkgs.gnused
        pkgs.gnutar
        pkgs.gzip
        pkgs.less
        pkgs.nix
        pkgs.openssh
        pkgs.which
        shellLib.package
      ];
      aliasNames = lib.attrNames regCfg.aliases;
      functionNames = lib.attrNames regCfg.functions;
      generatedLauncherNames = map (profileName: "clb-${profileName}") (
        lib.attrNames config.workerBroker.profiles
      );
      invalidGeneratedLauncherNames = builtins.filter (
        launcherName: builtins.match patterns.safeCommand launcherName == null
      ) generatedLauncherNames;
      generatedLaunchers = lib.mapAttrs' (
        profileName: profile:
        lib.nameValuePair "clb-${profileName}" {
          profile = profileName;
          inherit (profile) sandbox;
        }
      ) config.workerBroker.profiles;

      pipewireNativeEnabled = config.audio.pipewire.enable && !config.audio.pipewire.pulseOnly;
      fileChooserPortalEnabled = config.dbus.portal.fileChooser;
      notificationBusEnabled = config.dbus.notifications;
      openUriPortalEnabled = config.dbus.portal.openUri;

      xdg-open-portal = pkgs.writeShellScriptBin "xdg-open" ''
        exec ${pkgs.glib.bin}/bin/gdbus call --session \
          --dest org.freedesktop.portal.Desktop \
          --object-path /org/freedesktop/portal/desktop \
          --method org.freedesktop.portal.OpenURI.OpenURI \
          "" "$1" "@a{sv} {}" > /dev/null
      '';

      customShellSource = "$HOME/.config/cl-shell/${name}/custom";

      renderAliases = lib.concatMapStringsSep "\n" (
        n: shellLib.renderAlias n regCfg.aliases.${n}
      ) aliasNames;

      renderFunctions = lib.concatMapStringsSep "\n\n" (
        n: shellLib.renderFunction n regCfg.functions.${n}
      ) functionNames;

      inside = lib.concatStringsSep "\n\n" (
        lib.filter (snippet: snippet != "") [
          renderAliases
          renderFunctions
        ]
      );

      # Outside rendering uses cl-<name> and parent shell info via closures
      initPath = "${configHome}/${shellLib.configDir}/cloister-${name}.${shellLib.initExt}";

      wrappableAliases = lib.filterAttrs (n: _: !builtins.elem n regCfg.noWrap) regCfg.aliases;

      wrappableCommands = lib.filter (cmd: !builtins.elem cmd regCfg.noWrap) regCfg.commands;
      wrappableInteractiveCommands = lib.filter (
        cmd: !builtins.elem cmd regCfg.noWrap
      ) regCfg.interactiveCommands;

      wrappableFunctions = lib.filter (n: !builtins.elem n regCfg.noWrap) functionNames;

      defaultCommandName =
        if config.defaultCommand != null && config.defaultCommand != [ ] then
          lib.head config.defaultCommand
        else
          null;

      defaultCommandArgs =
        if config.defaultCommand != null && config.defaultCommand != [ ] then
          lib.escapeShellArgs config.defaultCommand
        else
          "";

      renderOutsideFor =
        hostShellLib:
        let
          inherit (shellLib) command;
          inherit initPath;
          renderOutsideRunner = hostShellLib.renderOutsideRunner name;
          renderOutsideAliases = lib.concatMapStringsSep "\n" (
            n: hostShellLib.renderAlias n "__cloister_run_${name} -c ${wrappableAliases.${n}}"
          ) (lib.attrNames wrappableAliases);

          renderOutsideCommands = lib.concatMapStringsSep "\n" (
            cmd:
            hostShellLib.renderAlias cmd (
              if defaultCommandName == cmd then
                "__cloister_run_${name} -c ${defaultCommandArgs}"
              else
                "__cloister_run_${name} -c ${cmd}"
            )
          ) wrappableCommands;

          renderOutsideInteractiveCommands = lib.concatMapStringsSep "\n" (
            cmd:
            let
              wrappedCommand =
                if defaultCommandName == cmd then defaultCommandArgs else lib.escapeShellArgs [ cmd ];
            in
            hostShellLib.renderOutsideCommand {
              name = cmd;
              sandbox = name;
              inherit wrappedCommand;
            }
          ) wrappableInteractiveCommands;

          renderOutsideFunctions = lib.concatMapStringsSep "\n\n" (
            n:
            hostShellLib.renderOutsideFunction {
              name = n;
              sandbox = name;
              inherit initPath;
              inherit command;
            }
          ) wrappableFunctions;
        in
        lib.concatStringsSep "\n\n" (
          lib.filter (snippet: snippet != "") [
            renderOutsideRunner
            renderOutsideAliases
            renderOutsideCommands
            renderOutsideInteractiveCommands
            renderOutsideFunctions
          ]
        );

      outside = lib.mapAttrs (_: renderOutsideFor) shells;

      shellInit =
        let
          customZsh = ''
            if [[ -f "${customShellSource}/zshenv" ]]; then
              source "${customShellSource}/zshenv"
            fi
            if [[ -f "${customShellSource}/zshrc" ]]; then
              source "${customShellSource}/zshrc"
            fi
          '';
          customBash = ''
            if [[ -f "${customShellSource}/bashenv" ]]; then
              source "${customShellSource}/bashenv"
            fi
            if [[ -f "${customShellSource}/bashrc" ]]; then
              source "${customShellSource}/bashrc"
            fi
            if [[ -f "${customShellSource}/profile" ]]; then
              source "${customShellSource}/profile"
            fi
          '';
        in
        if config.shell.name == "zsh" then
          lib.optionalString (
            config.shell.customRcPath.zshenv != null || config.shell.customRcPath.zshrc != null
          ) customZsh
        else if config.shell.name == "bash" then
          lib.optionalString (
            config.shell.customRcPath.bashenv != null
            || config.shell.customRcPath.bashrc != null
            || config.shell.customRcPath.profile != null
          ) customBash
        else
          throw "cloister: unsupported shell '${config.shell.name}'";
    in
    {
      options = {
        homeManager = {
          enable = lib.mkEnableOption "sandbox-local Home Manager profile";

          modules = lib.mkOption {
            type = lib.types.listOf lib.types.deferredModule;
            default = [ ];
            description = ''
              Home Manager modules to evaluate for this sandbox only. Cloister
              consumes only home.packages, xdg.configFile, and home.file from
              the resulting configuration and never runs activation scripts.
            '';
          };

          extraSpecialArgs = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
            description = "Extra specialArgs passed to the nested Home Manager evaluation.";
          };

          config = lib.mkOption {
            type = lib.types.nullOr lib.types.attrs;
            default = null;
            description = ''
              Pre-evaluated Home Manager config to consume for this sandbox.
              This is useful when the sandbox profile is defined as a separate
              homeConfigurations entry. When set, modules are ignored.
            '';
          };

          includeFiles = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Bind xdg.configFile and home.file entries from the sandbox Home Manager profile.";
          };

          includePackages = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Append home.packages from the sandbox Home Manager profile to the sandbox PATH.";
          };
        };

        _basePackages = lib.mkOption {
          type = lib.types.listOf lib.types.package;
          internal = true;
          description = "Internal base package set required for the sandbox shell and core tools.";
        };

        extraPackages = lib.mkOption {
          type = lib.types.listOf lib.types.package;
          default = [ ];
          description = "Additional packages appended to the sandbox PATH.";
        };

        preset = lib.mkOption {
          type = lib.types.nullOr (
            lib.types.enum [
              "hardened"
              "developer"
              "gui"
              "chromium"
            ]
          );
          default = null;
          description = ''
            Opinionated preset that applies a bundle of sandbox defaults.
            Explicit sandbox settings still override the preset.
          '';
        };

        shell = lib.mkOption {
          type =
            lib.types.coercedTo
              (lib.types.enum [
                "zsh"
                "bash"
              ])
              (value: { name = value; })
              (
                lib.types.submodule {
                  options = {
                    name = lib.mkOption {
                      type = lib.types.enum [
                        "zsh"
                        "bash"
                      ];
                      default = defaultShell;
                      description = "Interactive shell inside this sandbox and for wrapper integration outside. Defaults to cloister.defaultShell.";
                    };

                    hostConfig = lib.mkOption {
                      type = lib.types.bool;
                      default = false;
                      description = "Bind host shell configuration into the sandbox.";
                    };

                    customRcPath = lib.mkOption {
                      type = lib.types.submodule {
                        options = {
                          zshenv = lib.mkOption {
                            type = lib.types.nullOr lib.types.path;
                            default = null;
                            description = "Custom zshenv file to source inside the sandbox.";
                          };
                          zshrc = lib.mkOption {
                            type = lib.types.nullOr lib.types.path;
                            default = null;
                            description = "Custom zshrc file to source inside the sandbox.";
                          };
                          bashenv = lib.mkOption {
                            type = lib.types.nullOr lib.types.path;
                            default = null;
                            description = "Custom bashenv file to source inside the sandbox.";
                          };
                          bashrc = lib.mkOption {
                            type = lib.types.nullOr lib.types.path;
                            default = null;
                            description = "Custom bashrc file to source inside the sandbox.";
                          };
                          profile = lib.mkOption {
                            type = lib.types.nullOr lib.types.path;
                            default = null;
                            description = "Custom profile file to source inside the sandbox.";
                          };
                        };
                      };
                      default = { };
                      description = "Custom shell rc files bound into the sandbox (sourced after host config, before registry).";
                    };
                  };
                }
              );
          default = {
            name = defaultShell;
          };
          description = "Shell configuration for this sandbox.";
        };

        defaultCommand = lib.mkOption {
          type = lib.types.nullOr (lib.types.listOf lib.types.str);
          default = null;
          description = ''
            Command to run when the sandbox is invoked without arguments.
            If null, an interactive shell is launched.
            For app-specific sandboxes, set this to the application command,
            e.g. `[ "evince" ]` so that `cl-evince` launches evince directly.
          '';
        };

        workerBroker = {
          profiles = lib.mkOption {
            type = lib.types.attrsOf (
              lib.types.submodule {
                options = {
                  sandbox = lib.mkOption {
                    type = lib.types.str;
                    description = "Sandbox name used for this worker broker profile.";
                  };

                  workspace.mode = lib.mkOption {
                    type = lib.types.enum [
                      "project-rw"
                      "project-overlay"
                    ];
                    description = "Workspace exposure mode for this worker broker profile.";
                  };

                  delegatedPerDirMounts = lib.mkOption {
                    type = lib.types.attrsOf (
                      lib.types.submodule {
                        options = {
                          mode = lib.mkOption {
                            type = lib.types.enum [
                              "ro"
                              "rw"
                            ];
                            description = "Access mode for this delegated per-directory mount.";
                          };

                          path = lib.mkOption {
                            type = lib.types.str;
                            description = "Host base path exposed for this worker broker delegated mount.";
                          };

                          subPath = lib.mkOption {
                            type = lib.types.nullOr lib.types.str;
                            default = null;
                            description = "Optional subpath under the delegated mount base path.";
                          };
                        };
                      }
                    );
                    default = { };
                    description = "Per-directory delegated mounts exposed to this worker broker profile, keyed by sandbox-relative destination.";
                  };
                };
              }
            );
            default = { };
            description = "Worker broker profiles keyed by profile name. Non-empty profiles enable worker broker support.";
          };

          generatedLaunchers = lib.mkOption {
            type = lib.types.attrsOf (
              lib.types.submodule {
                options = {
                  profile = lib.mkOption {
                    type = lib.types.str;
                    readOnly = true;
                    description = "Worker broker profile referenced by the generated launcher.";
                  };

                  sandbox = lib.mkOption {
                    type = lib.types.str;
                    readOnly = true;
                    description = "Spawnable sandbox referenced by the generated launcher.";
                  };
                };
              }
            );
            readOnly = true;
            description = "Computed worker broker launcher metadata keyed by generated launcher name.";
          };

          invalidGeneratedLauncherNames = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            readOnly = true;
            description = "Generated worker broker launcher names that do not satisfy safe command naming rules.";
          };
        };

        sandbox = {
          bindWorkingDirectory = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Bind-mount the working directory (git root or CWD) read-write into the sandbox. Disable for app-specific sandboxes that don't need host directory access.";
          };

          nixStore.mode = lib.mkOption {
            type = lib.types.enum [
              "host"
              "image-store"
            ];
            default = "host";
            description = ''
              How Cloister exposes `/nix/store` inside the sandbox.

              `host` bind-mounts the host `/nix/store` directly.
              `image-store` mounts a prebuilt immutable store image prepared by the
              NixOS `cloister-image-store` module and bind-mounts that image's
              `/nix/store` into the sandbox.
            '';
          };

          readOnly = lib.mkOption {
            type = lib.types.listOf bindEntryType;
            default = [ ];
            description = "Read-only host path binds. Strings are home-relative paths; attrs may set src, dest, and optional.";
          };

          readWrite = lib.mkOption {
            type = lib.types.listOf bindEntryType;
            default = [ ];
            description = "Read-write host path binds. Strings are home-relative paths; attrs may set src, dest, and optional.";
          };

          managed = lib.mkOption {
            type = lib.types.listOf managedEntryType;
            default = [ ];
            description = ''
              Home Manager managed file keys/prefixes or explicit read-only binds.
              String entries resolve xdg.configFile and home.file entries; attr
              entries bind a fixed src to a home-relative dest.
            '';
          };

          state = {
            dirs = lib.mkOption {
              type = lib.types.attrsOf (lib.types.listOf lib.types.str);
              default = { };
              description = "Volume-backed writable directories, keyed by host base directory.";
            };

            files = lib.mkOption {
              type = lib.types.attrsOf (lib.types.listOf lib.types.str);
              default = { };
              description = "Volume-backed writable files, keyed by host base directory.";
            };

            projectDirs = lib.mkOption {
              type = lib.types.attrsOf (lib.types.listOf lib.types.str);
              default = { };
              description = "Per-project writable directories keyed by host base directory and isolated by the working-directory hash.";
            };
          };

          enforceStrictHomePolicy = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Prevent sandboxing of the home parent directory, any user home directory under it, and any dot-directory directly inside a user home.";
          };

          disallowedPaths = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [
              "/"
              "/root"
            ];
            description = "Absolute paths (or prefixes) that are not allowed to be used as SANDBOX_DIR. Strict home directory policy is enforced separately.";
          };

          copyBase = lib.mkOption {
            type = lib.types.str;
            default = "${args.config.xdg.stateHome}/cloister";
            defaultText = "\${config.xdg.stateHome}/cloister";
            description = "Host base directory where sandbox.copies writable state is stored.";
          };

          copies = lib.mkOption {
            type = lib.types.listOf copyEntryType;
            default = [ ];
            description = "Files to copy into writable sandbox state before binding into the sandbox.";
          };

          symlinks = lib.mkOption {
            type = lib.types.listOf symlinkEntryType;
            default = [ ];
            description = "Symlinks to create inside the sandbox. Relative link paths are resolved under the sandbox home.";
          };

          devices = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = "Device paths to pass through with --dev-bind. Missing devices are warned about at runtime.";
          };

          env = lib.mkOption {
            type = lib.types.attrsOf lib.types.str;
            default = { };
            description = ''
              Environment variables set inside the sandbox (--setenv).
              PATH is always computed from packages and cannot be overridden here.
            '';
          };

          passthroughEnv = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            description = ''
              Host environment variables to pass through when they are set.
              The default is set in the submodule config block (locale variables).
              Use `lib.mkAfter` to append more while preserving those defaults.
            '';
          };

          seccomp = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = ''
                Apply a seccomp-bpf filter that blocks dangerous syscalls (kernel module loading,
                mount/namespace escape, ptrace, bpf, etc.) with ENOSYS. The denylist is derived
                from Flatpak and complements bwrap's namespace isolation.
              '';
            };

            allowChromiumSandbox = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Allow Chromium/Electron's internal sandbox syscalls: chroot, namespace creation
                (unshare, clone with CLONE_NEW* flags, clone3), and setns. Required for apps
                built on Chromium's multi-process architecture. Safe inside bwrap because the
                process is already in an unprivileged user namespace.
              '';
            };
          };

          anonymize = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Present a generic identity inside the sandbox. When enabled:
                - Username and hostname become the configured username (default "ubuntu")
                - Home directory becomes /home/<username>
                - Synthetic /etc/passwd and /etc/group replace the host files
                - task-unrelated top-level /proc entries are hidden via procfs subset=pid
                - Selected task-related /proc files are replaced with synthetic regular files
              '';
            };
            username = lib.mkOption {
              type = lib.types.strMatching "^[a-z_][a-z0-9_-]*$";
              default = "ubuntu";
              description = ''
                Username presented inside the anonymized sandbox.
                Also determines the sandbox home directory (/home/<username>).
                Must start with a lowercase letter or underscore and then contain
                only lowercase letters, digits, underscores, or dashes.
              '';
            };
          };

        };

        gui = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = ''
              Enable GUI integration for this sandbox. This forwards Wayland via
              wp-security-context-v1, enables GPU rendering support with private
              shared memory, and applies declared fonts, packages, and theme settings.
            '';
          };

          fonts = lib.mkOption {
            type = lib.types.listOf lib.types.package;
            default = [ ];
            description = ''
              Font packages available inside the sandbox. A fontconfig configuration
              is generated automatically from this list and set via FONTCONFIG_FILE.

              When GUI is enabled this defaults to Noto fonts plus Noto emoji.
              Set to [ ] with lib.mkForce to disable the generated fontconfig.
            '';
          };

          packages = lib.mkOption {
            type = lib.types.listOf lib.types.package;
            default = [ ];
            description = ''
              GUI asset and plugin packages available inside the sandbox. Each
              package's /share directory is added to XDG_DATA_DIRS and Qt plugin
              directories are added to QT_PLUGIN_PATH.
            '';
          };

          theme = {
            gtk = lib.mkOption {
              type = lib.types.str;
              default = "Adwaita";
              description = "GTK theme name. Sets GTK_THEME and gtk-theme-name inside the sandbox.";
            };

            icon = lib.mkOption {
              type = lib.types.str;
              default = "Adwaita";
              description = "Icon theme name written to GTK settings inside the sandbox.";
            };

            qt = {
              platform = lib.mkOption {
                type = lib.types.str;
                default = "gtk3";
                description = "Qt platform theme plugin name. The default 'gtk3' reads GTK_THEME.";
              };

              style = lib.mkOption {
                type = lib.types.nullOr lib.types.str;
                default = null;
                description = "Qt widget style override (e.g. 'adwaita', 'adwaita-dark', 'breeze', 'fusion'). When null, no QT_STYLE_OVERRIDE is set.";
              };
            };
          };

          desktopEntry = lib.mkOption {
            type = lib.types.nullOr (
              lib.types.submodule {
                options = {
                  name = lib.mkOption {
                    type = lib.types.str;
                    default = "";
                    description = "Display name in the desktop entry. Falls back to cl-<name> when empty.";
                  };
                  execArgs = lib.mkOption {
                    type = lib.types.str;
                    default = "";
                    description = ''Additional arguments appended after the sandbox binary path in the Exec line (e.g. "%U" for URL handling). Requires defaultCommand so the wrapper acts as an app launcher instead of opening an interactive shell.'';
                  };
                  icon = lib.mkOption {
                    type = lib.types.str;
                    default = "";
                    description = "Icon name or path for the desktop entry.";
                  };
                  categories = lib.mkOption {
                    type = lib.types.listOf lib.types.str;
                    default = [ ];
                    description = ''XDG categories for the desktop entry (e.g. ["Network" "WebBrowser"]).'';
                  };
                  mimeTypes = lib.mkOption {
                    type = lib.types.listOf lib.types.str;
                    default = [ ];
                    description = "MIME types the application can handle.";
                  };
                  terminal = lib.mkOption {
                    type = lib.types.bool;
                    default = false;
                    description = "Whether the application should run in a terminal.";
                  };
                  genericName = lib.mkOption {
                    type = lib.types.str;
                    default = "";
                    description = ''Generic name for the desktop entry (e.g. "Web Browser").'';
                  };
                  comment = lib.mkOption {
                    type = lib.types.str;
                    default = "";
                    description = "Tooltip/comment for the desktop entry.";
                  };
                  startupNotify = lib.mkOption {
                    type = lib.types.bool;
                    default = false;
                    description = "Whether the application supports startup notification.";
                  };
                };
              }
            );
            default = null;
            description = ''
              XDG .desktop entry metadata for this sandbox. Set to null to disable
              launcher generation; set to an attribute set to generate a launcher.
            '';
          };
        };

        ssh = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Forward SSH_AUTH_SOCK into the sandbox for SSH agent access.";
          };

          allowFingerprints = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = ''
              SSH key fingerprints (SHA256:...) allowed inside the sandbox.
              When non-empty, a filtering proxy hides all other keys from
              the agent. When empty (default), the agent is passed through
              unfiltered. Get fingerprints with: ssh-add -l -E sha256
            '';
          };

          filterTimeoutSeconds = lib.mkOption {
            type = lib.types.ints.unsigned;
            default = 60;
            description = ''
              Read/write timeout in seconds for the SSH agent filtering proxy.
              Applies only when allowFingerprints is non-empty. Set to 0 to disable
              timeouts for interactive agents that may require user confirmation.
            '';
          };
        };

        git = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Bind git configuration files (.gitconfig and .config/git/config) read-only into the sandbox. Disable to prevent git credential helper configuration from being visible inside the sandbox.";
          };
        };

        network = {
          dns = {
            nameservers = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              example = [
                "1.1.1.1"
                "2606:4700:4700::1111"
              ];
              description = ''
                DNS servers written to the sandbox's /etc/resolv.conf, overriding
                host and namespace DNS. Mutually exclusive with resolvConf.
                Entries must be nonempty and contain no whitespace.
                This does not enable networking.
              '';
            };

            resolvConf = lib.mkOption {
              type = lib.types.nullOr lib.types.path;
              default = null;
              example = "/run/NetworkManager/resolv.conf";
              description = ''
                Absolute path to a resolver file bound read-only at /etc/resolv.conf,
                overriding host and namespace DNS. Mutually exclusive with nameservers.
                The file must exist at launch; runtime files should use quoted paths.
                Relaunch the sandbox to pick up file replacements.
                This does not enable networking.
              '';
            };
          };

          enable = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Share the host network namespace with the sandbox. When false, the sandbox does not share host networking and seccomp also denies new AF_NETLINK sockets.";
          };

          namespace = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "Linux network namespace to join before launching the sandbox (e.g. a VPN namespace created with `ip netns add`). Requires the cloister-netns NixOS module for capability setup.";
          };
        };

        dbus = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Share filtered D-Bus proxy inside sandbox. Policies are configured per sandbox. See docs/dbus.md for setup.";
          };

          log = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Enable xdg-dbus-proxy logging. Prints all filtering decisions to stderr from the per-launch proxy wrapper.";
          };

          notifications = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = ''
              Allow desktop notifications on the session bus by exposing
              org.freedesktop.Notifications. Requires dbus.enable = true.
            '';
          };

          portal = lib.mkOption {
            type = lib.types.submodule {
              options = {
                fileChooser = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                  description = ''
                    Enable xdg-desktop-portal file chooser integration. Adds
                    the synthetic .flatpak-info marker, binds the per-app
                    document portal view at /run/flatpak/doc and
                    $XDG_RUNTIME_DIR/doc when it already exists, sets
                    GTK_USE_PORTAL=1, and grants the D-Bus rules needed for
                    FileChooser requests. If the per-app document subtree is
                    absent, the sandbox still starts and those binds are
                    skipped for that launch.
                    Requires dbus.enable = true.
                  '';
                };

                openUri = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                  description = ''
                    Allow the sandbox to use the OpenURI portal for opening
                    links, files, and directories via host handlers.
                    Requires dbus.enable = true.
                  '';
                };

                screencast = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                  description = ''
                    Allow the sandbox to request screen/window capture through
                    the ScreenCast portal. Requires dbus.enable = true.
                  '';
                };

                camera = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                  description = ''
                    Allow the sandbox to request camera access through the
                    Camera portal. Requires dbus.enable = true.
                  '';
                };
              };
            };
            default = { };
            description = "Desktop integration toggles for portals.";
          };

          rawPolicies = lib.mkOption {
            type = lib.types.submodule dbusPolicySubmodule;
            default = { };
            description = "Raw xdg-dbus-proxy policy rules appended to Cloister's generated portal policies.";
          };

          _portalPolicies = lib.mkOption {
            type = lib.types.submodule dbusPolicySubmodule;
            default = { };
            internal = true;
            description = "Internal D-Bus policy rules generated from dbus.portal toggles.";
          };
        };

        audio = {
          pipewire = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Enable PipeWire-backed audio. Native PipeWire exposure is always
                through Cloister's per-sandbox filtered socket; unfiltered host
                PipeWire forwarding is not supported.

                Set audio.pipewire.pulseOnly = true for applications that need a
                PulseAudio-compatible socket instead of native PipeWire.
              '';
            };

            pulseOnly = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Expose only a PulseAudio-compatible socket inside the sandbox while
                keeping PipeWire filtering on the host side. This is intended for
                audio-only sandboxes that need playback and optional microphone
                access without exposing the PipeWire native protocol or host
                PipeWire core identity inside the sandbox.

                When enabled, Cloister starts a per-sandbox pipewire-pulse proxy in
                the user session and binds only its PulseAudio socket into the
                sandbox. The existing audio.pipewire.filters.* toggles continue to
                control playback, microphone, and volume-control access.

                This mode does not support native PipeWire clients, camera access,
                or portal-based PipeWire features.
              '';
            };

            filters = {
              audioOut = lib.mkOption {
                type = lib.types.bool;
                default = true;
                description = ''
                  Allow playback to audio sinks (media.class: "Audio/Sink") when PipeWire audio is enabled.
                '';
              };
              audioIn = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = ''
                  Allow recording from audio sources (media.class: "Audio/Source") when PipeWire audio is enabled.
                '';
              };
              videoIn = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = ''
                  Allow recording from cameras/video sources (media.class: "Video/Source") when PipeWire audio is enabled.
                '';
              };
              control = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = ''
                  Grant 'w' (write) permissions to allow changing volume and mute state
                  of visible nodes when PipeWire audio is enabled.
                '';
              };
              routing = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = ''
                  Grant 'm' (metadata) permissions to allow changing default system
                  routing, moving streams, and managing metadata when PipeWire audio is enabled.
                '';
              };
            };

            dbus = {
              enable = lib.mkOption {
                type = lib.types.bool;
                default = true;
                description = ''
                  Allow sandboxed PipeWire clients to use D-Bus support when a
                  filtered session bus is available via dbus.enable. When false,
                  sandboxed PipeWire and pipewire-pulse force support.dbus = false.
                '';
              };
            };

          };
        };

        registry = {
          aliases = lib.mkOption {
            type = lib.types.attrsOf lib.types.str;
            default = { };
            description = ''
              Alias definitions available inside the sandbox and wrappable outside.
              Outside-wrapped aliases are argv-style only; use registry.functions for
              shell syntax, quoting, pipes, redirects, or variable expansion.
              Alias names must match ${patterns.safeAlias}.
            '';
          };

          functions = lib.mkOption {
            type = lib.types.attrsOf lib.types.lines;
            default = { };
            description = ''
              Function bodies available inside the sandbox and wrappable outside.
              Function names must match ${patterns.safeFunction}.
            '';
          };

          commands = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = ''
              Command names to wrap outside the sandbox.
              Command names must match ${patterns.safeCommand}.
            '';
          };

          interactiveCommands = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = ''
              Command names to wrap outside the sandbox via `cl-<name> -i ...`,
              so they run through the sandbox shell startup path before exec.
              Command names must match ${patterns.safeCommand}.
            '';
          };

          noWrap = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = "Names that should not be wrapped outside the sandbox.";
          };

          rendered = lib.mkOption {
            type = lib.types.submodule {
              options = {
                inside = lib.mkOption {
                  type = lib.types.lines;
                  readOnly = true;
                  description = "Shell snippet sourced inside the sandbox to register commands and aliases.";
                };
                outside = lib.mkOption {
                  type = lib.types.attrsOf lib.types.lines;
                  readOnly = true;
                  description = "Attribute set of shell wrapper scripts installed on the host, keyed by command name.";
                };
              };
            };
            readOnly = true;
            description = "Rendered shell snippets for inside and outside the sandbox (computed).";
          };
        };

        init = {
          text = lib.mkOption {
            type = lib.types.lines;
            default = "";
            description = "Shell snippet sourced inside the sandbox.";
          };
          rendered = lib.mkOption {
            type = lib.types.lines;
            readOnly = true;
            description = "Computed shell snippet sourced inside the sandbox (custom rc + init.text).";
          };
        };
      };

      # --- Submodule config: defaults + computed registry ---
      config = lib.mkMerge [
        (lib.mkIf (config.preset == "hardened") {
          gui.enable = lib.mkDefault false;

          ssh.enable = lib.mkDefault false;
          git.enable = lib.mkDefault false;
          network.enable = lib.mkDefault false;
          dbus.enable = lib.mkDefault false;

          audio = {
            pipewire = {
              enable = lib.mkDefault false;
              pulseOnly = lib.mkDefault false;
            };
          };
        })
        (lib.mkIf (config.preset == "developer") {
          shell.hostConfig = lib.mkDefault true;

          gui.enable = lib.mkDefault false;

          network.enable = lib.mkDefault true;
          git.enable = lib.mkDefault true;
          ssh.enable = lib.mkDefault true;
          dbus.enable = lib.mkDefault true;
          dbus.notifications = lib.mkDefault true;

          audio = {
            pipewire = {
              enable = lib.mkDefault false;
              pulseOnly = lib.mkDefault false;
            };
          };
        })
        (lib.mkIf (config.preset == "gui") {

          network.enable = lib.mkDefault true;
          git.enable = lib.mkDefault false;
          ssh.enable = lib.mkDefault false;
          dbus.enable = lib.mkDefault true;
          dbus.notifications = lib.mkDefault true;

          gui.enable = lib.mkDefault true;

          audio = {
            pipewire = {
              enable = lib.mkDefault false;
              pulseOnly = lib.mkDefault false;
            };
          };
        })
        (lib.mkIf (config.preset == "chromium") {

          network.enable = lib.mkDefault true;
          git.enable = lib.mkDefault false;
          ssh.enable = lib.mkDefault false;
          dbus.enable = lib.mkDefault true;
          dbus.notifications = lib.mkDefault true;

          gui.enable = lib.mkDefault true;

          sandbox.seccomp.allowChromiumSandbox = lib.mkDefault true;

          audio = {
            pipewire = {
              enable = lib.mkDefault true;
              pulseOnly = lib.mkDefault true;
            };
          };
        })
        {
          _basePackages = basePackages;

          extraPackages = lib.mkMerge [
            (lib.mkIf pipewireNativeEnabled [ pkgs.pipewire ])
            (lib.mkIf config.audio.pipewire.pulseOnly [ pkgs.pipewire ])
            (lib.mkIf openUriPortalEnabled [
              pkgs.glib.bin
              xdg-open-portal
            ])
          ];

          sandbox = {
            env = lib.mkMerge [
              (lib.mapAttrs (_: lib.mkDefault) (
                {
                  HOME =
                    if config.sandbox.anonymize.enable then
                      "/home/${config.sandbox.anonymize.username}"
                    else
                      args.config.home.homeDirectory;
                  USER =
                    if config.sandbox.anonymize.enable then
                      config.sandbox.anonymize.username
                    else
                      args.config.home.username;
                  SHELL = shellLib.shellEnv;
                  TERM = "xterm-256color";
                  CLOISTER = name;
                  LOCALE_ARCHIVE = "${pkgs.glibcLocales}/lib/locale/locale-archive";
                }
                // lib.optionalAttrs config.sandbox.anonymize.enable {
                  LANG = "C.UTF-8";
                  LC_ALL = "C.UTF-8";
                  TZ = "UTC";
                }
              ))
            ];

            passthroughEnv = lib.mkDefault (
              if config.sandbox.anonymize.enable then
                [ ]
              else
                [
                  "LANG"
                  "LC_ALL"
                  "LC_CTYPE"
                  "LC_MESSAGES"
                  "LC_NUMERIC"
                  "LC_TIME"
                  "LC_COLLATE"
                  "LC_MONETARY"
                ]
            );
          };

          gui = {
            packages = lib.mkDefault (
              lib.optionals config.gui.enable [
                pkgs.hicolor-icon-theme
                pkgs.adwaita-icon-theme
                pkgs.gtk3
                pkgs.gtk4
                pkgs.gsettings-desktop-schemas
              ]
            );

            fonts = lib.mkDefault (
              lib.optionals config.gui.enable [
                pkgs.noto-fonts
                pkgs.noto-fonts-color-emoji
              ]
            );
          };

          # Computed registry rendering
          registry.rendered = { inherit inside outside; };

          workerBroker.generatedLaunchers = generatedLaunchers;
          workerBroker.invalidGeneratedLauncherNames = invalidGeneratedLauncherNames;

          init.rendered = lib.mkDefault (
            lib.concatStringsSep "\n" (
              lib.filter (s: s != "") [
                shellInit
                config.init.text
              ]
            )
          );

          dbus._portalPolicies = {
            talk = lib.mkMerge [
              (lib.mkIf notificationBusEnabled [ "org.freedesktop.Notifications" ])
            ];
            call = lib.mkMerge [
              (lib.mkIf fileChooserPortalEnabled {
                "org.freedesktop.portal.Desktop" = [
                  "org.freedesktop.DBus.Properties.Get@/org/freedesktop/portal/desktop"
                  "org.freedesktop.portal.FileChooser.*@/org/freedesktop/portal/desktop"
                  "org.freedesktop.portal.Request.*@/org/freedesktop/portal/desktop/request/*"
                ];
              })
              (lib.mkIf config.dbus.portal.openUri {
                "org.freedesktop.portal.Desktop" = [
                  "org.freedesktop.DBus.Properties.Get@/org/freedesktop/portal/desktop"
                  "org.freedesktop.portal.OpenURI.*@/org/freedesktop/portal/desktop"
                  "org.freedesktop.portal.Request.*@/org/freedesktop/portal/desktop/request/*"
                ];
              })
              (lib.mkIf config.dbus.portal.screencast {
                "org.freedesktop.portal.Desktop" = [
                  "org.freedesktop.portal.ScreenCast.*@/org/freedesktop/portal/desktop"
                  "org.freedesktop.portal.Request.*@/org/freedesktop/portal/desktop/request/*"
                  "org.freedesktop.portal.Session.*@/org/freedesktop/portal/desktop/session/*"
                ];
              })
              (lib.mkIf config.dbus.portal.camera {
                "org.freedesktop.portal.Desktop" = [
                  "org.freedesktop.portal.Camera.*@/org/freedesktop/portal/desktop"
                  "org.freedesktop.portal.Request.*@/org/freedesktop/portal/desktop/request/*"
                ];
              })
            ];
            broadcast = lib.mkMerge [
              (lib.mkIf fileChooserPortalEnabled {
                "org.freedesktop.portal.Desktop" = [
                  "org.freedesktop.portal.Request.Response@/org/freedesktop/portal/desktop/request/*"
                ];
              })
              (lib.mkIf config.dbus.portal.openUri {
                "org.freedesktop.portal.Desktop" = [
                  "org.freedesktop.portal.Request.Response@/org/freedesktop/portal/desktop/request/*"
                ];
              })
              (lib.mkIf config.dbus.portal.screencast {
                "org.freedesktop.portal.Desktop" = [
                  "org.freedesktop.portal.Request.Response@/org/freedesktop/portal/desktop/request/*"
                  "org.freedesktop.portal.Session.Closed@/org/freedesktop/portal/desktop/session/*"
                ];
              })
              (lib.mkIf config.dbus.portal.camera {
                "org.freedesktop.portal.Desktop" = [
                  "org.freedesktop.portal.Request.Response@/org/freedesktop/portal/desktop/request/*"
                ];
              })
            ];
          };
        }
      ];
    };
in
{
  options.cloister = {
    enable = lib.mkEnableOption "bubblewrap namespace sandbox";

    _internal.imageStoreInfos = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [ ];
      internal = true;
      description = "Internal image-store metadata consumed by the NixOS image-store module.";
    };

    _internal.sandboxConfigs = lib.mkOption {
      type = lib.types.attrs;
      default = { };
      internal = true;
      description = "Internal rendered sandbox configs consumed by tests.";
    };

    _internal.sandboxInternals = lib.mkOption {
      type = lib.types.attrs;
      default = { };
      internal = true;
      description = "Internal rendered sandbox helper data consumed by tests.";
    };

    homeManager.builder = lib.mkOption {
      type = lib.types.nullOr lib.types.raw;
      default = null;
      description = ''
        Function used to evaluate sandbox-local Home Manager module lists.
        Set this to inputs.home-manager.lib.homeManagerConfiguration when using
        cloister.sandboxes.<name>.homeManager.modules.
      '';
    };

    defaultShell = lib.mkOption {
      type = lib.types.enum [
        "zsh"
        "bash"
      ];
      default = "zsh";
      description = "Default interactive shell for sandboxes.";
    };

    sandboxes = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule sandboxModule);
      default = { };
      description = "Per-sandbox configurations. Each attribute name becomes a sandbox (cl-<name>) and must match ^[A-Za-z0-9_-]+$.";
    };
  };
}
