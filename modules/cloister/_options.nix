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

  bindSubmodule = {
    options = {
      src = lib.mkOption {
        type = lib.types.str;
        description = "Source path on host (use config.home.homeDirectory instead of $HOME)";
      };
      dest = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Destination inside sandbox. Defaults to src when null.";
      };
      try = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "If true, uses --ro-bind-try/--bind-try (won't fail if source is missing).";
      };
    };
  };

  symlinkSubmodule = {
    options = {
      target = lib.mkOption {
        type = lib.types.str;
        description = "Symlink target path.";
      };
      link = lib.mkOption {
        type = lib.types.str;
        description = "Symlink location inside sandbox.";
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
      allCommands = regCfg.commands ++ regCfg.extraCommands;
      aliasNames = lib.attrNames regCfg.aliases;
      functionNames = lib.attrNames regCfg.functions;

      validatorPackages =
        let
          cloister-wayland-validate = pkgs.callPackage ../../helpers/cloister-wayland-validate { };
          cloister-dbus-validate = pkgs.callPackage ../../helpers/cloister-dbus-validate { };
          cloister-seccomp-validate = pkgs.callPackage ../../helpers/cloister-seccomp-validate { };
          cloister-pipewire-validate = pkgs.callPackage ../../helpers/cloister-pipewire-validate { };
        in
        [
          cloister-wayland-validate
          cloister-dbus-validate
          cloister-seccomp-validate
          cloister-pipewire-validate
        ];

      validatorCommands = [
        "cloister-wayland-validate"
        "cloister-dbus-validate"
        "cloister-seccomp-validate"
        "cloister-pipewire-validate"
      ];

      sandboxHome =
        if config.sandbox.anonymize.enable then
          "/home/${config.sandbox.anonymize.username}"
        else
          args.config.home.homeDirectory;
      customShellDest = "${sandboxHome}/.config/cl-shell/${name}/custom";
      customShellSource = "$HOME/.config/cl-shell/${name}/custom";

      customShellBinds =
        let
          mkCustomBind = destName: srcPath: {
            src = toString srcPath;
            dest = "${customShellDest}/${destName}";
            try = false;
          };
          rcFields = [
            "zshenv"
            "zshrc"
            "bashenv"
            "bashrc"
            "profile"
          ];
        in
        lib.concatMap (
          field:
          lib.optional (config.shell.customRcPath.${field} != null) (
            mkCustomBind field config.shell.customRcPath.${field}
          )
        ) rcFields;

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

      wrappableCommands = lib.filter (cmd: !builtins.elem cmd regCfg.noWrap) allCommands;

      wrappableFunctions = lib.filter (n: !builtins.elem n regCfg.noWrap) functionNames;

      renderOutsideFor =
        hostShellLib:
        let
          inherit (shellLib) command;
          inherit initPath;
          renderOutsideAliases = lib.concatMapStringsSep "\n" (
            n: hostShellLib.renderAlias n "cl-${name} ${wrappableAliases.${n}}"
          ) (lib.attrNames wrappableAliases);

          renderOutsideCommands = lib.concatMapStringsSep "\n" (
            cmd: hostShellLib.renderAlias cmd "cl-${name} ${cmd}"
          ) wrappableCommands;

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
            renderOutsideAliases
            renderOutsideCommands
            renderOutsideFunctions
          ]
        );

      outside = lib.mapAttrs (_: renderOutsideFor) shells;

      shellInit =
        let
          hostZsh = ''
            if [[ -f "$HOME/.zshenv" ]]; then
              source "$HOME/.zshenv"
            fi
            if [[ -f "$HOME/.zshrc" ]]; then
              source "$HOME/.zshrc"
            fi
          '';
          customZsh = ''
            if [[ -f "${customShellSource}/zshenv" ]]; then
              source "${customShellSource}/zshenv"
            fi
            if [[ -f "${customShellSource}/zshrc" ]]; then
              source "${customShellSource}/zshrc"
            fi
          '';
          hostBash = ''
            if [[ -f "$HOME/.bashrc" ]]; then
              source "$HOME/.bashrc"
            fi
            if [[ -f "$HOME/.bash_profile" ]]; then
              source "$HOME/.bash_profile"
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
          (lib.optionalString config.shell.hostConfig hostZsh)
          + lib.optionalString (
            config.shell.customRcPath.zshenv != null || config.shell.customRcPath.zshrc != null
          ) customZsh
        else if config.shell.name == "bash" then
          (lib.optionalString config.shell.hostConfig hostBash)
          + lib.optionalString (
            config.shell.customRcPath.bashenv != null
            || config.shell.customRcPath.bashrc != null
            || config.shell.customRcPath.profile != null
          ) customBash
        else
          throw "cloister: unsupported shell '${config.shell.name}'";
    in
    {
      options = {
        packages = lib.mkOption {
          type = lib.types.listOf lib.types.package;
          default = [ ];
          description = "Packages available inside the sandbox. Their bin dirs form the base PATH.";
        };

        extraPackages = lib.mkOption {
          type = lib.types.listOf lib.types.package;
          default = [ ];
          description = "Additional packages appended to the sandbox PATH.";
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
                      default = true;
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
            e.g. `[ "geeqie" ]` so that `cl-geeqie` launches geeqie directly.
          '';
        };

        validators.enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Install cloister validator helpers inside the sandbox and wrap them outside.";
        };

        sandbox = {
          bindWorkingDirectory = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Bind-mount the working directory (git root or CWD) read-write into the sandbox. Disable for app-specific sandboxes that don't need host directory access.";
          };

          dirs = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = "Directories to create inside the sandbox (--dir).";
          };

          extraDirs = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = "Additional directories appended to sandbox dirs.";
          };

          tmpfs = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = "Tmpfs mounts inside the sandbox (--tmpfs).";
          };

          symlinks = lib.mkOption {
            type = lib.types.listOf (lib.types.submodule symlinkSubmodule);
            default = [ ];
            description = "Symlinks to create inside the sandbox (--symlink).";
          };

          extraSymlinks = lib.mkOption {
            type = lib.types.listOf (lib.types.submodule symlinkSubmodule);
            default = [ ];
            description = "Additional symlinks appended to sandbox symlinks.";
          };

          binds = {
            ro = lib.mkOption {
              type = lib.types.listOf (lib.types.submodule bindSubmodule);
              default = [ ];
              description = "Read-only bind mounts.";
            };

            rw = lib.mkOption {
              type = lib.types.listOf (lib.types.submodule bindSubmodule);
              default = [ ];
              description = "Read-write bind mounts.";
            };
          };

          extraBinds = {
            required = {
              ro = lib.mkOption {
                type = lib.types.listOf lib.types.str;
                default = [ ];
                description = "Home-relative paths for required read-only binds (--ro-bind). Must exist.";
              };

              rw = lib.mkOption {
                type = lib.types.listOf lib.types.str;
                default = [ ];
                description = "Home-relative paths for required read-write binds (--bind). Must exist.";
              };
            };

            optional = {
              ro = lib.mkOption {
                type = lib.types.listOf lib.types.str;
                default = [ ];
                description = "Home-relative paths for optional read-only binds (--ro-bind-try). May not exist.";
              };

              rw = lib.mkOption {
                type = lib.types.listOf lib.types.str;
                default = [ ];
                description = "Home-relative paths for optional read-write binds (--bind-try). May not exist.";
              };
            };

            dir = lib.mkOption {
              type = lib.types.attrsOf (lib.types.listOf lib.types.str);
              default = { };
              description = ''
                Volume-backed binds. Keys are base directories (e.g. "/local", "/persist"),
                values are home-relative paths. Source = {key}/cloister/{name}/{path},
                dest = $HOME/{path}. Always rw, mkdir'd before bwrap.
              '';
            };

            file = lib.mkOption {
              type = lib.types.attrsOf (lib.types.listOf lib.types.str);
              default = { };
              description = ''
                Volume-backed file binds. Like dir, but creates files instead of directories.
                Keys are base directories (e.g. "/local", "/persist"),
                values are home-relative paths. Source = {key}/cloister/{name}/{path},
                dest = $HOME/{path}. Always rw, touch'd before bwrap.
              '';
            };

            perDir = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              description = ''
                Per-directory binds. Home-relative paths isolated by a hash of the
                sandbox directory. Source = {perDirBase}/$DIR_HASH/{path},
                dest = $HOME/{path}. Always rw, mkdir'd before bwrap.
              '';
            };

            managedFile = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              description = ''
                Home-manager managed file keys or directory prefixes whose Nix store
                sources are ro-bound into the sandbox.

                Keys are resolved against xdg.configFile first (bound at
                $HOME/.config/<key>), then home.file as configHome/<key> (also bound
                at $HOME/.config/<key>), then home.file directly (bound at
                $HOME/<key>). Exact keys bind a single file; prefixes bind all
                entries under that prefix.
              '';
            };
          };

          perDirBase = lib.mkOption {
            type = lib.types.str;
            default = "${args.config.xdg.stateHome}/cloister";
            defaultText = "\${config.xdg.stateHome}/cloister";
            description = "Base directory for per-directory state. DIR_HASH subdirs are created here.";
          };

          dangerousPathWarnings = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Check extraBinds paths and managedFile-resolved paths against known credential-storing locations and fail if any match.";
          };

          allowDangerousPaths = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = "Home-relative paths (or managedFile path prefixes) acknowledged as intentionally bound despite being known credential locations.";
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

          copyFileBase = lib.mkOption {
            type = lib.types.str;
            default = "${args.config.xdg.stateHome}/cloister";
            defaultText = "\${config.xdg.stateHome}/cloister";
            description = "Base directory on the host where copyFiles are stored. Defaults to per-sandbox state dir.";
          };

          copyFiles = lib.mkOption {
            type = lib.types.listOf (
              lib.types.submodule {
                options = {
                  src = lib.mkOption {
                    type = lib.types.str;
                    description = "Source path to copy from (use config.home.homeDirectory instead of $HOME)";
                  };
                  dest = lib.mkOption {
                    type = lib.types.str;
                    description = "Destination path inside sandbox (must start with \${config.home.homeDirectory}/)";
                  };
                  mode = lib.mkOption {
                    type = lib.types.str;
                    default = "0644";
                    description = "File permissions mode (e.g. '0644').";
                  };
                  overwrite = lib.mkOption {
                    type = lib.types.bool;
                    default = false;
                    description = "If true, copies the file even if it already exists in the sandbox state.";
                  };
                };
              }
            );
            default = [ ];
            description = "Files to copy into the sandbox state writable. Useful for config files you want to edit inside the sandbox without affecting the host.";
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
              The default is set in the submodule config block (locale variables
              and, for GUI/audio sandboxes, XDG_RUNTIME_DIR). You can append
              more via list merging.
            '';
          };

          devBinds = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = "Arbitrary device paths to pass through with --dev-bind (e.g. /dev/video0). Missing devices are warned about at runtime.";
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
                - Fingerprintable /proc entries are masked with /dev/null
              '';
            };
            username = lib.mkOption {
              type = lib.types.str;
              default = "ubuntu";
              description = ''
                Username presented inside the anonymized sandbox.
                Also determines the sandbox home directory (/home/<username>).
              '';
            };
          };

        };

        gui = {
          wayland = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Forward Wayland display socket into the sandbox. Requires wp-security-context-v1 by default.";
            };
            securityContext = {
              enable = lib.mkOption {
                type = lib.types.bool;
                default = true;
                description = ''
                  Require the wp-security-context-v1 protocol for Wayland forwarding.
                  When true (default), the sandbox refuses to start if the compositor
                  does not support the protocol — preventing exposure of privileged
                  extensions (screencopy, foreign-toplevel, virtual-keyboard).
                  When false, falls back to raw Wayland socket passthrough.
                '';
              };
            };
          };

          x11 = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Forward the DISPLAY environment variable into the sandbox for X11/XWayland applications.

                WARNING: X11 provides no client isolation. Any X11 client can keylog,
                take screenshots, and inject input into other clients on the same display.
                Prefer Wayland with securityContext for GUI applications when possible.
              '';
            };
          };

          scaleFactor = lib.mkOption {
            type = lib.types.nullOr lib.types.float;
            default = null;
            description = ''
              Display scale factor for HiDPI rendering. When set, configures
              GDK_SCALE, GDK_DPI_SCALE, and QT_SCALE_FACTOR environment
              variables inside the sandbox.

              Set this to the host's display scale (e.g. 2.0 for a 2× HiDPI
              display) so that GUI applications render at the correct size.
            '';
          };

          fonts = {
            packages = lib.mkOption {
              type = lib.types.listOf lib.types.package;
              default = [ ];
              description = ''
                Font packages available inside the sandbox. A fontconfig configuration
                is generated automatically from this list and set via FONTCONFIG_FILE,
                replacing the host /etc/fonts dependency.

                When GUI is enabled this defaults to [ pkgs.dejavu_fonts ].
                Set to [ ] to disable the generated fontconfig entirely.
              '';
            };
          };

          gpu = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Bind /dev/dri for GPU-accelerated rendering inside the sandbox.";
            };
            shm = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Mount a private tmpfs at /dev/shm when GPU is enabled. Provides POSIX shared memory for GPU drivers without exposing host shared memory.";
            };
          };

          desktopEntry = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Generate an XDG .desktop file for this sandbox so it appears in app launchers.";
            };
            name = lib.mkOption {
              type = lib.types.str;
              default = "";
              description = "Display name in the desktop entry. Falls back to cl-<name> when empty.";
            };
            execArgs = lib.mkOption {
              type = lib.types.str;
              default = "";
              description = ''Additional arguments appended after the sandbox binary path in the Exec line (e.g. "%U" for URL handling). The command itself is provided by defaultCommand.'';
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
            mimeType = lib.mkOption {
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

          dataPackages = lib.mkOption {
            type = lib.types.listOf lib.types.package;
            default = [ ];
            description = "Packages whose /share directories are added to XDG_DATA_DIRS. Used for icon theme, MIME type, and widget theme discovery when GUI is enabled.";
          };

          gtk = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Enable GTK theme integration. Sets GTK_THEME and adds gtk3/gtk4 theme assets to XDG_DATA_DIRS.";
            };

            theme = lib.mkOption {
              type = lib.types.str;
              default = "Adwaita";
              description = "GTK theme name. Sets the GTK_THEME environment variable inside the sandbox.";
            };

            packages = lib.mkOption {
              type = lib.types.listOf lib.types.package;
              default = [ ];
              description = "Additional packages providing GTK themes. Their /share directories are merged into XDG_DATA_DIRS.";
            };
          };

          qt = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Enable Qt theme integration. Sets QT_QPA_PLATFORMTHEME so Qt apps follow the GTK theme.";
            };

            platformTheme = lib.mkOption {
              type = lib.types.str;
              default = "gtk3";
              description = "Qt platform theme plugin name. The default 'gtk3' reads GTK_THEME and is built into qtbase (no extra packages needed).";
            };

            style = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "Qt widget style override (e.g. 'adwaita', 'adwaita-dark', 'breeze', 'fusion'). When null, the platform theme controls the style.";
            };

            packages = lib.mkOption {
              type = lib.types.listOf lib.types.package;
              default = [ ];
              description = "Packages providing Qt style plugins. Their plugin directories are added to QT_PLUGIN_PATH.";
            };
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
            description = "Enable xdg-dbus-proxy logging. Prints all filtering decisions to stderr (visible via journalctl --user -u cloister-dbus-proxy-<name>).";
          };

          portal = lib.mkOption {
            type = lib.types.coercedTo lib.types.bool (value: { enable = value; }) (
              lib.types.submodule {
                options = {
                  enable = lib.mkOption {
                    type = lib.types.bool;
                    default = false;
                    description = ''
                      Enable xdg-desktop-portal integration. Creates a synthetic
                      .flatpak-info so portals detect the sandbox, sets
                      GTK_USE_PORTAL=1, and auto-merges required D-Bus portal
                      policies. Requires dbus.enable = true.
                    '';
                  };

                  documentFUSE.enable = lib.mkOption {
                    type = lib.types.bool;
                    default = true;
                    description = ''
                      Bind the host xdg-document-portal FUSE mount
                      ($XDG_RUNTIME_DIR/doc) into /run/flatpak/doc inside the
                      sandbox. Disable this to keep portal D-Bus integration while
                      preventing document-portal file access paths from being
                      mounted.
                    '';
                  };
                };
              }
            );
            default = { };
            description = "xdg-desktop-portal integration settings.";
          };

          policies = {
            talk = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ "org.freedesktop.Notifications" ];
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

        audio = {
          pulseaudio = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Forward PulseAudio socket into the sandbox for audio playback and recording.
                Works with both PulseAudio and PipeWire's PulseAudio compatibility layer.

                WARNING: This grants full audio access including microphone recording.
                PulseAudio does not support per-client restriction of recording vs playback.
                Do not enable this for sandboxes that should not have microphone access.
              '';
            };
          };

          pipewire = {
            enable = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = ''
                Forward PipeWire native socket into the sandbox. Required for
                portal-based screen sharing (ScreenCast) and camera access,
                and for applications that use PipeWire directly.
                Can be enabled alongside audio.pulseaudio for full compatibility.
              '';
            };

            filters = {
              enable = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = ''
                  Enable strict PipeWire filtering via WirePlumber access rules.
                  When true, a dedicated PipeWire socket is created and only the
                  specified media classes and capabilities are exposed to the sandbox.
                  Defaults to audioOut=true and everything else disabled.
                '';
              };
              audioOut = lib.mkOption {
                type = lib.types.bool;
                default = true;
                description = ''
                  Allow playback to audio sinks (media.class: "Audio/Sink").
                  Only takes effect if filters.enable is true.
                '';
              };
              audioIn = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = ''
                  Allow recording from audio sources (media.class: "Audio/Source").
                  Only takes effect if filters.enable is true.
                '';
              };
              videoIn = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = ''
                  Allow recording from cameras/video sources (media.class: "Video/Source").
                  Only takes effect if filters.enable is true.
                '';
              };
              control = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = ''
                  Grant 'w' (write) permissions to allow changing volume and mute state
                  of visible nodes.
                  Only takes effect if filters.enable is true.
                '';
              };
              routing = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = ''
                  Grant 'm' (metadata) permissions to allow changing default system
                  routing, moving streams, and managing metadata.
                  Only takes effect if filters.enable is true.
                '';
              };
            };
          };
        };

        fido2 = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Bind /dev/hidraw* devices and related sysfs paths for FIDO2/U2F security key access (e.g. YubiKey WebAuthn).";
          };
        };

        video = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Bind /dev/video* devices and related sysfs paths for webcam/camera access (e.g. video calls in sandboxed browsers).";
          };
        };

        printing = {
          enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Forward the CUPS printing socket into the sandbox for printer access.";
          };
        };

        registry = {
          aliases = lib.mkOption {
            type = lib.types.attrsOf lib.types.str;
            default = { };
            description = ''
              Alias definitions available inside the sandbox and wrappable outside.
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

          extraCommands = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            description = ''
              Additional command names appended to wrapped commands.
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
      config = {
        # Minimal packages — just enough for a functional sandbox shell
        packages = lib.mkDefault [
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

        extraPackages = lib.mkIf config.validators.enable (lib.mkDefault validatorPackages);

        registry.extraCommands = lib.mkIf config.validators.enable (lib.mkDefault validatorCommands);

        sandbox = {
          dirs = lib.mkDefault [
            "/var"
            "/run"
            "/run/current-system/sw/bin"
            "/usr/bin"
            "/bin"
            "/etc/ssl"
            "/etc/ssl/certs"
          ];

          tmpfs = lib.mkDefault [ "/tmp" ];

          symlinks = lib.mkDefault (
            [
              {
                target = "${pkgs.coreutils}/bin/env";
                link = "/usr/bin/env";
              }
              {
                target = "${pkgs.bash}/bin/bash";
                link = "/bin/sh";
              }
              {
                target = "${pkgs.bash}/bin/bash";
                link = "/bin/bash";
              }
              {
                target = "/bin/bash";
                link = "/run/current-system/sw/bin/bash";
              }
              {
                target = "pts/ptmx";
                link = "/dev/ptmx";
              }
              {
                target = "/etc/ssl/certs/ca-bundle.crt";
                link = "/etc/ssl/certs/ca-certificates.crt";
              }
            ]
            ++ shellLib.symlinks
          );

          binds.ro = lib.mkDefault (
            lib.filter
              (b: !(config.sandbox.anonymize.enable && (b.src == "/etc/passwd" || b.src == "/etc/group")))
              [
                { src = "/nix"; }
                { src = "/etc/passwd"; }
                { src = "/etc/group"; }
                {
                  src = "/etc/shells";
                  try = true;
                }
                (
                  if config.network.namespace != null then
                    {
                      src = "/etc/netns/${config.network.namespace}/hosts";
                      dest = "/etc/hosts";
                      try = true;
                    }
                  else
                    { src = "/etc/hosts"; }
                )
                (
                  if config.network.namespace != null then
                    {
                      src = "/etc/netns/${config.network.namespace}/resolv.conf";
                      dest = "/etc/resolv.conf";
                      try = true;
                    }
                  else
                    {
                      src = "/etc/resolv.conf";
                    }
                )
                {
                  src = "/etc/ssh/ssh_known_hosts";
                  try = true;
                }
                {
                  src = "/etc/nix/nix.conf";
                  try = true;
                }
                {
                  src = "/etc/localtime";
                  try = true;
                }
                {
                  src = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
                  dest = "/etc/ssl/certs/ca-bundle.crt";
                }
              ]
            ++ customShellBinds
          );

          binds.rw = lib.mkDefault [ ];

          env = lib.mapAttrs (_: lib.mkDefault) {
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
          };

          passthroughEnv = lib.mkDefault (
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
            ++ lib.optionals (
              config.gui.wayland.enable
              || config.gui.x11.enable
              || config.audio.pulseaudio.enable
              || config.audio.pipewire.enable
            ) [ "XDG_RUNTIME_DIR" ]
          );
        };

        gui = {
          # Auto-enable GPU when any GUI display protocol is active
          gpu.enable = lib.mkDefault (config.gui.wayland.enable || config.gui.x11.enable);

          # Auto-enable GTK theme integration when any GUI display protocol is active
          gtk.enable = lib.mkDefault (config.gui.wayland.enable || config.gui.x11.enable);

          # Default data packages for GUI sandboxes: icon theme fallback + conditionally GTK theme assets
          dataPackages = lib.mkDefault (
            [ pkgs.hicolor-icon-theme ]
            ++ lib.optionals config.gui.gtk.enable [
              pkgs.gtk3
              pkgs.gtk4
              pkgs.gsettings-desktop-schemas
            ]
          );

          # Default font packages for GUI sandboxes
          fonts.packages = lib.mkDefault (
            lib.optionals (config.gui.wayland.enable || config.gui.x11.enable) [
              pkgs.dejavu_fonts
            ]
          );
        };

        # Computed registry rendering
        registry.rendered = { inherit inside outside; };

        init.rendered = lib.mkDefault (
          lib.concatStringsSep "\n" (
            lib.filter (s: s != "") [
              shellInit
              config.init.text
            ]
          )
        );

        sandbox.extraDirs = lib.mkMerge [
          (lib.mkIf (customShellBinds != [ ]) (lib.mkBefore [ "${sandboxHome}/.config/cl-shell/${name}" ]))
          (lib.mkIf config.dbus.portal.enable (
            [
              "/run/flatpak"
            ]
            ++ lib.optionals config.dbus.portal.documentFUSE.enable [ "/run/flatpak/doc" ]
          ))
        ];

        dbus.policies = lib.mkIf config.dbus.portal.enable {
          call = {
            "org.freedesktop.portal.*" = lib.mkDefault [ "*" ];
          };
          broadcast = {
            "org.freedesktop.portal.*" = lib.mkDefault [ "*@/org/freedesktop/portal/*" ];
          };
        };
      };
    };
in
{
  options.cloister = {
    enable = lib.mkEnableOption "bubblewrap namespace sandbox";

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
