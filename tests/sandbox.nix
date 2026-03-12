{ testLib }:
let
  inherit (testLib)
    pkgs
    evalConfig
    mkCheck
    mkConfigCheck
    mkAssertionCheck
    lib
    ;

  # Base module that enables cloister with a "test" sandbox
  baseModule = {
    cloister = {
      enable = true;
      sandboxes.test = { };
    };
  };

  pipewireFiltersConfig = evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.audio.pipewire = {
            enable = true;
            filters.enable = true;
          };
        };
      }
    ];
  };

  pipewireRoutingConfig = evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.audio.pipewire = {
            enable = true;
            filters = {
              enable = true;
              routing = true;
            };
          };
        };
      }
    ];
  };

  getConfigFileText =
    config: prefix:
    let
      names = builtins.attrNames config.xdg.configFile;
      matches = builtins.filter (name: lib.hasPrefix prefix name) names;
    in
    (builtins.getAttr (builtins.head matches) config.xdg.configFile).text;

  pipewireConfText = getConfigFileText pipewireFiltersConfig "pipewire/pipewire.conf.d/99-cloister.conf";
  pipewireWireplumberConfText = getConfigFileText pipewireFiltersConfig "wireplumber/wireplumber.conf.d/99-cloister-";

  pipewireRoutingConfText = getConfigFileText pipewireRoutingConfig "wireplumber/wireplumber.conf.d/99-cloister-";
in
{
  # ── Assertion tests (bad config should fire) ──────────────────────────

  duplicate-bind-dest = mkAssertionCheck "sandbox-duplicate-bind-dest" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.binds.ro = lib.mkForce [
          {
            src = "/foo";
            dest = "/same";
          }
          {
            src = "/bar";
            dest = "/same";
          }
        ];
      };
    }
  ] "duplicate bind mount destinations";

  dir-tmpfs-overlap = mkAssertionCheck "sandbox-dir-tmpfs-overlap" [
    {
      cloister = {
        enable = true;
        sandboxes.test = {
          sandbox.dirs = lib.mkForce [ "/overlap" ];
          sandbox.tmpfs = lib.mkForce [ "/overlap" ];
        };
      };
    }
  ] "both sandbox dirs and tmpfs";

  duplicate-symlink = mkAssertionCheck "sandbox-duplicate-symlink" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.symlinks = lib.mkForce [
          {
            target = "/a";
            link = "/same-link";
          }
          {
            target = "/b";
            link = "/same-link";
          }
        ];
      };
    }
  ] "duplicate symlink destinations";

  duplicate-managed-file = mkAssertionCheck "sandbox-duplicate-managed-file" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.extraBinds.managedFile = [
          "foo.toml"
          "foo.toml"
        ];
      };
      # Provide the managed file so resolution doesn't throw
      xdg.configFile."foo.toml".text = "content";
    }
  ] "duplicate managedFile entries";

  env-override-path = mkAssertionCheck "sandbox-env-override-path" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.env.PATH = lib.mkForce "/bad";
      };
    }
  ] "cannot be overridden";

  # ── Positive assertion test (valid config passes) ─────────────────────

  valid-config-no-assertions = mkCheck "sandbox-valid-config" (
    let
      config = evalConfig { modules = [ baseModule ]; };
      failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
    in
    failedAssertions == [ ]
  );

  # ── Bind resolution tests (verify JSON config content) ────────────────

  required-ro = mkConfigCheck "sandbox-required-ro" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.extraBinds.required.ro = [ ".config/foo" ];
        };
      }
    ];
  }) "test" "$HOME/.config/foo" true;

  optional-ro = mkConfigCheck "sandbox-optional-ro" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.extraBinds.optional.ro = [ ".config/bar" ];
        };
      }
    ];
  }) "test" "$HOME/.config/bar" true;

  required-rw = mkConfigCheck "sandbox-required-rw" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.extraBinds.required.rw = [ ".local/share/x" ];
        };
      }
    ];
  }) "test" "$HOME/.local/share/x" true;

  optional-rw = mkConfigCheck "sandbox-optional-rw" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.extraBinds.optional.rw = [ ".cargo/reg" ];
        };
      }
    ];
  }) "test" "$HOME/.cargo/reg" true;

  dir-binds = mkConfigCheck "sandbox-dir-binds" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.extraBinds.dir."/persist" = [ ".notes" ];
        };
      }
    ];
  }) "test" "/persist/cloister/test/.notes" true;

  perdir-binds = mkConfigCheck "sandbox-perdir-binds" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.extraBinds.perDir = [ ".cache" ];
        };
      }
    ];
  }) "test" ".cache" true;

  managed-file-xdg-exact = mkConfigCheck "sandbox-managed-file-xdg-exact" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.extraBinds.managedFile = [ "starship.toml" ];
        };
        xdg.configFile."starship.toml".text = "format = '$all'";
      }
    ];
  }) "test" "/home/testuser/.config/starship.toml" true;

  dbus-enable = mkConfigCheck "sandbox-dbus-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.dbus.enable = true;
        };
      }
    ];
  }) "test" "dbus-proxy-test" true;

  dbus-disable = mkConfigCheck "sandbox-dbus-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.dbus.enable = false;
        };
      }
    ];
  }) "test" ''"dbus_enable":false'' true;

  # ── managedFile resolution branch coverage ──────────────────────────

  managed-file-xdg-prefix = mkConfigCheck "sandbox-managed-file-xdg-prefix" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.extraBinds.managedFile = [ "nvim" ];
        };
        xdg.configFile."nvim/init.lua".text = "-- nvim config";
      }
    ];
  }) "test" "/home/testuser/.config/nvim/init.lua" true;

  managed-file-home-config-exact = mkConfigCheck "sandbox-managed-file-home-config-exact" (evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test.sandbox.extraBinds.managedFile = [ "foo.conf" ];
          };
          home.file."/home/testuser/.config/foo.conf".text = "content";
        }
      ];
    }
  ) "test" "/home/testuser/.config/foo.conf" true;

  managed-file-home-config-prefix = mkConfigCheck "sandbox-managed-file-home-config-prefix" (
    evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test.sandbox.extraBinds.managedFile = [ "bar" ];
          };
          home.file."/home/testuser/.config/bar/settings.json".text = "{}";
        }
      ];
    }
  ) "test" "/home/testuser/.config/bar/settings.json" true;

  managed-file-home-direct-exact = mkConfigCheck "sandbox-managed-file-home-direct-exact" (evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test.sandbox.extraBinds.managedFile = [ ".tmux.conf" ];
          };
          home.file.".tmux.conf".text = "set -g mouse on";
        }
      ];
    }
  ) "test" "/home/testuser/.tmux.conf" true;

  managed-file-home-direct-prefix = mkConfigCheck "sandbox-managed-file-home-direct-prefix" (
    evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test.sandbox.extraBinds.managedFile = [ ".mytool" ];
          };
          home.file.".mytool/config".text = "{}";
        }
      ];
    }
  ) "test" "/home/testuser/.mytool/config" true;

  managed-file-not-found = mkCheck "sandbox-managed-file-not-found" (
    let
      result = builtins.tryEval (
        let
          config = evalConfig {
            modules = [
              {
                cloister = {
                  enable = true;
                  sandboxes.test.sandbox.extraBinds.managedFile = [ "nonexistent.toml" ];
                };
              }
            ];
          };
        in
        builtins.seq config.home.packages config
      );
    in
    !result.success
  );

  # ── JSON config content tests ──────────────────────────────────────

  copy-files = mkConfigCheck "sandbox-copy-files" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.copyFiles = [
            {
              src = "/home/testuser/.config/task/home-manager-taskrc";
              dest = "$HOME/.config/task/taskrc";
            }
          ];
        };
      }
    ];
  }) "test" "/home/testuser/.config/task/home-manager-taskrc" true;

  dir-mkdir = mkConfigCheck "sandbox-dir-mkdir" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.extraBinds.dir."/persist" = [ ".notes" ];
        };
      }
    ];
  }) "test" "/persist/cloister/test/.notes" true;

  # ── managedFile dir-overlap tests ──────────────────────────────────

  managed-file-dir-overlap-mkdir = mkConfigCheck "sandbox-managed-file-dir-overlap-mkdir" (evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test = {
              sandbox.extraBinds.dir."/local" = [ ".claude" ];
              sandbox.extraBinds.managedFile = [
                ".claude/plugins/nix-lsp"
                ".claude/settings.json"
              ];
            };
          };
          home.file.".claude/plugins/nix-lsp/.claude-plugin/plugin.json".text = "{}";
          home.file.".claude/settings.json".text = "{}";
        }
      ];
    }
  ) "test" "/local/cloister/test/.claude/plugins/nix-lsp/.claude-plugin" true;

  managed-file-dir-overlap-mkdir-shallow =
    mkConfigCheck "sandbox-managed-file-dir-overlap-mkdir-shallow"
      (evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test = {
                sandbox.extraBinds.dir."/local" = [ ".claude" ];
                sandbox.extraBinds.managedFile = [
                  ".claude/plugins/nix-lsp"
                  ".claude/settings.json"
                ];
              };
            };
            home.file.".claude/plugins/nix-lsp/.claude-plugin/plugin.json".text = "{}";
            home.file.".claude/settings.json".text = "{}";
          }
        ];
      })
      "test"
      "/local/cloister/test/.claude"
      true;

  managed-file-dir-overlap-no-bwrap-dir =
    mkConfigCheck "sandbox-managed-file-dir-overlap-no-bwrap-dir"
      (evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test = {
                sandbox.extraBinds.dir."/local" = [ ".claude" ];
                sandbox.extraBinds.managedFile = [
                  ".claude/plugins/nix-lsp"
                  ".claude/settings.json"
                ];
              };
            };
            home.file.".claude/plugins/nix-lsp/.claude-plugin/plugin.json".text = "{}";
            home.file.".claude/settings.json".text = "{}";
          }
        ];
      })
      "test"
      ''"--dir","/home/testuser/.claude/plugins/nix-lsp/.claude-plugin"''
      false;

  managed-file-no-overlap-keeps-bwrap-dir =
    mkConfigCheck "sandbox-managed-file-no-overlap-keeps-bwrap-dir"
      (evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test = {
                sandbox.extraBinds.dir."/local" = [ ".claude" ];
                sandbox.extraBinds.managedFile = [ "starship.toml" ];
              };
            };
            xdg.configFile."starship.toml".text = "format = '$all'";
          }
        ];
      })
      "test"
      ''"--dir","/home/testuser/.config"''
      true;

  # ── D-Bus env override assertion ────────────────────────────────────

  dbus-env-override = mkAssertionCheck "sandbox-dbus-env-override" [
    {
      cloister = {
        enable = true;
        sandboxes.test = {
          dbus.enable = true;
          sandbox.env.DBUS_SESSION_BUS_ADDRESS = lib.mkForce "unix:path=/bad";
        };
      };
    }
  ] "managed by dbus";

  # ── D-Bus policy flags in systemd unit ───────────────────────────────

  dbus-policy-flags = mkCheck "sandbox-dbus-policy-flags" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.dbus = {
                enable = true;
                policies = {
                  talk = [
                    "org.freedesktop.Notifications"
                    "org.freedesktop.portal.Desktop"
                  ];
                  own = [ "org.example.App" ];
                  see = [ "org.example.Visible" ];
                  call."org.freedesktop.portal.Desktop" = [ "*@/org/freedesktop/portal/*" ];
                  broadcast."org.freedesktop.portal.Desktop" = [ "*@/org/freedesktop/portal/*" ];
                };
              };
            };
          }
        ];
      };
      service = config.systemd.user.services."cloister-dbus-proxy-test".Service;
      execStart = service.ExecStart;
    in
    lib.all (flag: lib.hasInfix flag execStart) [
      "--talk=org.freedesktop.Notifications"
      "--talk=org.freedesktop.portal.Desktop"
      "--own=org.example.App"
      "--see=org.example.Visible"
      "--call=org.freedesktop.portal.Desktop=*@/org/freedesktop/portal/*"
      "--broadcast=org.freedesktop.portal.Desktop=*@/org/freedesktop/portal/*"
    ]
  );

  # ── D-Bus log enable/disable tests ─────────────────────────────────

  dbus-log-enable = mkCheck "sandbox-dbus-log-enable" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.dbus = {
                enable = true;
                log = true;
              };
            };
          }
        ];
      };
      service = config.systemd.user.services."cloister-dbus-proxy-test".Service;
      execStart = service.ExecStart;
    in
    lib.hasInfix "--log" execStart
  );

  dbus-log-disable = mkCheck "sandbox-dbus-log-disable" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.dbus = {
                enable = true;
                log = false;
              };
            };
          }
        ];
      };
      service = config.systemd.user.services."cloister-dbus-proxy-test".Service;
      execStart = service.ExecStart;
    in
    !(lib.hasInfix "--log" execStart)
  );

  # ── Portal tests ───────────────────────────────────────────────

  portal-flatpak-info = mkConfigCheck "sandbox-portal-flatpak-info" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.dbus = {
            enable = true;
            portal = true;
          };
        };
      }
    ];
  }) "test" "/.flatpak-info" true;

  portal-fuse-mount = mkConfigCheck "sandbox-portal-fuse-mount" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.dbus = {
            enable = true;
            portal = true;
          };
        };
      }
    ];
  }) "test" "/run/flatpak/doc" true;

  portal-gtk-use-portal = mkConfigCheck "sandbox-portal-gtk-use-portal" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.dbus = {
            enable = true;
            portal = true;
          };
        };
      }
    ];
  }) "test" "GTK_USE_PORTAL" true;

  portal-auto-policy = mkCheck "sandbox-portal-auto-policy" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.dbus = {
                enable = true;
                portal = true;
              };
            };
          }
        ];
      };
      service = config.systemd.user.services."cloister-dbus-proxy-test".Service;
      execStart = service.ExecStart;
    in
    lib.all (flag: lib.hasInfix flag execStart) [
      "--call=org.freedesktop.portal.*=*"
      "--broadcast=org.freedesktop.portal.*=*@/org/freedesktop/portal/*"
    ]
  );

  portal-user-policy-merge = mkCheck "sandbox-portal-user-policy-merge" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.dbus = {
                enable = true;
                portal = true;
                policies.call."org.freedesktop.portal.*" = [ "org.freedesktop.portal.FileChooser.*" ];
              };
            };
          }
        ];
      };
      service = config.systemd.user.services."cloister-dbus-proxy-test".Service;
      execStart = service.ExecStart;
    in
    lib.hasInfix "--call=org.freedesktop.portal.*=org.freedesktop.portal.FileChooser.*" execStart
    && !(lib.hasInfix "--call=org.freedesktop.portal.*=* " execStart)
  );

  portal-requires-dbus = mkAssertionCheck "sandbox-portal-requires-dbus" [
    {
      cloister = {
        enable = true;
        sandboxes.test.dbus = {
          enable = false;
          portal = true;
        };
      };
    }
  ] "dbus.portal requires dbus.enable";

  portal-disabled-no-flatpak-info = mkConfigCheck "sandbox-portal-disabled-no-flatpak-info" (
    evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test.dbus = {
              enable = true;
              portal = false;
            };
          };
        }
      ];
    }
  ) "test" "flatpak-info" false;

  portal-disabled-no-gtk-use-portal = mkConfigCheck "sandbox-portal-disabled-no-gtk-use-portal" (
    evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test.dbus = {
              enable = true;
              portal = false;
            };
          };
        }
      ];
    }
  ) "test" "GTK_USE_PORTAL" false;

  portal-disabled-no-fuse-mount = mkConfigCheck "sandbox-portal-disabled-no-fuse-mount" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.dbus = {
            enable = true;
            portal = false;
          };
        };
      }
    ];
  }) "test" "/run/flatpak/doc" false;

  # ── GTK env var tests ──────────────────────────────────────────

  gtk-env-gio-extra-modules = mkConfigCheck "sandbox-gtk-env-gio-extra-modules" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" "GIO_EXTRA_MODULES" true;

  gtk-env-gsettings-schema-dir = mkConfigCheck "sandbox-gtk-env-gsettings-schema-dir" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" "GSETTINGS_SCHEMA_DIR" true;

  gtk-env-gdk-pixbuf-module-file = mkConfigCheck "sandbox-gtk-env-gdk-pixbuf-module-file" (evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test.gui.wayland = {
              enable = true;
              securityContext.enable = false;
            };
          };
        }
      ];
    }
  ) "test" "GDK_PIXBUF_MODULE_FILE" true;

  # ── Seccomp enable/disable tests ───────────────────────────────

  seccomp-enable = mkConfigCheck "sandbox-seccomp-enable" (evalConfig {
    modules = [ baseModule ];
  }) "test" ''"seccomp_enable":true'' true;

  seccomp-disable = mkConfigCheck "sandbox-seccomp-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.seccomp.enable = false;
        };
      }
    ];
  }) "test" ''"seccomp_enable":false'' true;

  seccomp-allow-chromium-sandbox-different-path =
    mkCheck "sandbox-seccomp-allow-chromium-sandbox-different-path"
      (
        let
          configDefault = evalConfig { modules = [ baseModule ]; };
          configChromium = evalConfig {
            modules = [
              {
                cloister = {
                  enable = true;
                  sandboxes.test.sandbox.seccomp.allowChromiumSandbox = true;
                };
              }
            ];
          };
          scriptDefault = lib.findFirst (
            p: (p.pname or p.name or "") == "cl-test"
          ) null configDefault.home.packages;
          scriptChromium = lib.findFirst (
            p: (p.pname or p.name or "") == "cl-test"
          ) null configChromium.home.packages;
          # The two packages should reference different store paths for the BPF filter
        in
        scriptDefault != scriptChromium
      );

  # ── Wayland enable/disable tests ────────────────────────────────

  wayland-enable = mkConfigCheck "sandbox-wayland-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" ''"wayland_enable":true'' true;

  wayland-disable = mkConfigCheck "sandbox-wayland-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland.enable = false;
        };
      }
    ];
  }) "test" ''"wayland_enable":false'' true;

  # ── X11 enable/disable tests ──────────────────────────────────

  x11-enable = mkConfigCheck "sandbox-x11-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.x11.enable = true;
        };
      }
    ];
  }) "test" ''"x11_enable":true'' true;

  x11-disable = mkConfigCheck "sandbox-x11-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.x11.enable = false;
        };
      }
    ];
  }) "test" ''"x11_enable":false'' true;

  # ── Wayland security-context tests ────────────────────────────

  wayland-ctx-probe = mkConfigCheck "sandbox-wayland-ctx-probe" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = true;
          };
        };
      }
    ];
  }) "test" ''"wayland_security_context":true'' true;

  wayland-ctx-disabled = mkConfigCheck "sandbox-wayland-ctx-disabled" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" ''"wayland_security_context":false'' true;

  wayland-ctx-raw-fallback = mkConfigCheck "sandbox-wayland-ctx-raw-fallback" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" ''"wayland_security_context":false'' true;

  xdg-runtime-dir-absent-without-gui = mkConfigCheck "sandbox-xdg-runtime-dir-absent-without-gui" (
    evalConfig
    { modules = [ baseModule ]; }
  ) "test" "XDG_RUNTIME_DIR" false;

  xdg-runtime-dir-present-with-wayland =
    mkConfigCheck "sandbox-xdg-runtime-dir-present-with-wayland"
      (evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.gui.wayland = {
                enable = true;
                securityContext.enable = false;
              };
            };
          }
        ];
      })
      "test"
      "XDG_RUNTIME_DIR"
      true;

  wayland-no-ctx-when-off = mkConfigCheck "sandbox-wayland-no-ctx-when-off" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland.enable = false;
        };
      }
    ];
  }) "test" ''"wayland_enable":false'' true;

  # ── Network namespace enable/disable tests ───────────────────────

  netns-enable = mkConfigCheck "sandbox-netns-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.network.namespace = "vpn";
        };
      }
    ];
  }) "test" ''"network_namespace":"vpn"'' true;

  netns-disable = mkConfigCheck "sandbox-netns-disable" (evalConfig {
    modules = [ baseModule ];
  }) "test" ''"network_namespace":null'' true;

  netns-wrapper-path = mkConfigCheck "sandbox-netns-wrapper-path" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.network.namespace = "vpn";
        };
      }
    ];
  }) "test" "/run/wrappers/bin/cloister-netns" true;

  netns-share-net-preserved = mkConfigCheck "sandbox-netns-share-net-preserved" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.network.namespace = "vpn";
        };
      }
    ];
  }) "test" ''"network_enable":true'' true;

  # ── SSH enable/disable tests ─────────────────────────────────────

  ssh-enable = mkConfigCheck "sandbox-ssh-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.ssh.enable = true;
        };
      }
    ];
  }) "test" ''"ssh_enable":true'' true;

  ssh-disable = mkConfigCheck "sandbox-ssh-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.ssh.enable = false;
        };
      }
    ];
  }) "test" ''"ssh_enable":false'' true;

  # ── SSH filter tests ────────────────────────────────────────────

  ssh-filter-enable = mkConfigCheck "sandbox-ssh-filter-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.ssh = {
            enable = true;
            allowFingerprints = [ "SHA256:test123" ];
          };
        };
      }
    ];
  }) "test" "SHA256:test123" true;

  ssh-filter-disabled-when-empty = mkConfigCheck "sandbox-ssh-filter-disabled-when-empty" (evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test.ssh = {
              enable = true;
              allowFingerprints = [ ];
            };
          };
        }
      ];
    }
  ) "test" ''"ssh_allow_fingerprints":[]'' true;

  ssh-filter-passthrough-when-no-fingerprints =
    mkConfigCheck "sandbox-ssh-filter-passthrough-when-no-fingerprints"
      (evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.ssh = {
                enable = true;
                allowFingerprints = [ ];
              };
            };
          }
        ];
      })
      "test"
      ''"ssh_enable":true''
      true;

  ssh-filter-allow-args = mkConfigCheck "sandbox-ssh-filter-allow-args" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.ssh = {
            enable = true;
            allowFingerprints = [ "SHA256:test123" ];
          };
        };
      }
    ];
  }) "test" "SHA256:test123" true;

  # ── PulseAudio enable/disable tests ──────────────────────────────

  pulseaudio-enable = mkConfigCheck "sandbox-pulseaudio-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.audio.pulseaudio.enable = true;
        };
      }
    ];
  }) "test" ''"pulseaudio_enable":true'' true;

  pulseaudio-disable = mkConfigCheck "sandbox-pulseaudio-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.audio.pulseaudio.enable = false;
        };
      }
    ];
  }) "test" ''"pulseaudio_enable":false'' true;

  # ── FIDO2 enable/disable tests ──────────────────────────────────

  fido2-enable = mkConfigCheck "sandbox-fido2-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.fido2.enable = true;
        };
      }
    ];
  }) "test" ''"fido2_enable":true'' true;

  fido2-disable = mkConfigCheck "sandbox-fido2-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.fido2.enable = false;
        };
      }
    ];
  }) "test" ''"fido2_enable":false'' true;

  # ── Video enable/disable tests ─────────────────────────────────

  video-enable = mkConfigCheck "sandbox-video-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.video.enable = true;
        };
      }
    ];
  }) "test" ''"video_enable":true'' true;

  video-disable = mkConfigCheck "sandbox-video-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.video.enable = false;
        };
      }
    ];
  }) "test" ''"video_enable":false'' true;

  # ── PipeWire enable/disable tests ──────────────────────────────

  pipewire-enable = mkConfigCheck "sandbox-pipewire-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.audio.pipewire.enable = true;
        };
      }
    ];
  }) "test" ''"pipewire_socket_name":"pipewire-0"'' true;

  pipewire-disable = mkConfigCheck "sandbox-pipewire-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.audio.pipewire.enable = false;
        };
      }
    ];
  }) "test" ''"pipewire_socket_name":null'' true;

  pipewire-filters =
    mkConfigCheck "sandbox-pipewire-filters" pipewireFiltersConfig "test"
      ''"pipewire_socket_name":"pipewire-cloister-''
      true;

  pipewire-filters-configures-protocol-native-socket =
    mkCheck "sandbox-pipewire-filters-configures-protocol-native-socket"
      (
        lib.hasInfix "module.protocol-native.args = {" pipewireConfText
        && lib.hasInfix ''{ name = "pipewire-cloister-'' pipewireConfText
        && lib.hasInfix "module.access.args = {" pipewireConfText
        && lib.hasInfix ''pipewire-0-manager = "unrestricted"'' pipewireConfText
      );

  pipewire-filters-keep-baseline-rx = mkCheck "sandbox-pipewire-filters-keep-baseline-rx" (
    lib.hasInfix ''default_permissions = "rx"'' pipewireWireplumberConfText
  );

  pipewire-routing-uses-metadata-object-ids =
    let
      confFile = pkgs.writeText "routing-conf" pipewireRoutingConfText;
    in
    pkgs.runCommand "check-sandbox-pipewire-routing-uses-metadata-object-ids" { } ''
      # Extract the Lua script store path from the WirePlumber conf
      lua_path=$(${pkgs.gnugrep}/bin/grep -oP '/nix/store/[^,]+\.lua' ${confFile})
      ${pkgs.gnugrep}/bin/grep -qF 'type = "metadata"' "$lua_path"
      ${pkgs.gnugrep}/bin/grep -qF 'local metadata_id = metadata["bound-id"]' "$lua_path"
      ${pkgs.gnugrep}/bin/grep -qF 'client:update_permissions { [metadata_id] = "rxm" }' "$lua_path"
      ! ${pkgs.gnugrep}/bin/grep -qF '["Metadata"]' "$lua_path"
      touch $out
    '';

  # ── Printing enable/disable tests ──────────────────────────────

  printing-enable = mkConfigCheck "sandbox-printing-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.printing.enable = true;
        };
      }
    ];
  }) "test" ''"printing_enable":true'' true;

  printing-disable = mkConfigCheck "sandbox-printing-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.printing.enable = false;
        };
      }
    ];
  }) "test" ''"printing_enable":false'' true;

  # ── File binds test ──────────────────────────────────────────────

  file-binds = mkConfigCheck "sandbox-file-binds" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.extraBinds.file."/persist" = [ ".config/settings.json" ];
        };
      }
    ];
  }) "test" "/persist/cloister/test/.config/settings.json" true;

  # ── Home directory policy tests ──────────────────────────────────

  home-dir-policy = mkConfigCheck "sandbox-home-dir-policy" (evalConfig {
    modules = [ baseModule ];
  }) "test" ''"enforce_strict_home_policy":true'' true;

  # ── Unsafe path assertion test ───────────────────────────────────

  unsafe-path-chars = mkAssertionCheck "sandbox-unsafe-path-chars" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.dirs = lib.mkForce [
          "/safe"
          "/has$var"
        ];
      };
    }
  ] "variable expansions";

  unsafe-path-dollar-paren = mkAssertionCheck "sandbox-unsafe-path-dollar-paren" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.dirs = lib.mkForce [
          "/safe"
          "/has$(cmd)injection"
        ];
      };
    }
  ] "variable expansions";

  # ── Dangerous path assertion tests ─────────────────────────────────

  dangerous-path-exact = mkAssertionCheck "sandbox-dangerous-path-exact" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.extraBinds.required.ro = [ ".ssh" ];
      };
    }
  ] "expose credentials";

  dangerous-path-ancestor = mkAssertionCheck "sandbox-dangerous-path-ancestor" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.extraBinds.required.rw = [ ".config" ];
      };
    }
  ] "expose credentials";

  dangerous-path-normalized = mkAssertionCheck "sandbox-dangerous-path-normalized" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.extraBinds.required.ro = [ "./.ssh" ];
      };
    }
  ] "expose credentials";

  dangerous-path-normalized-allowed = mkCheck "sandbox-dangerous-path-normalized-allowed" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.sandbox.extraBinds.required.ro = [ "./.ssh" ];
              sandboxes.test.sandbox.allowDangerousPaths = [ ".ssh" ];
            };
          }
        ];
      };
      failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
      dangerousFailures = builtins.filter (
        x: lib.hasInfix "expose credentials" x.message
      ) failedAssertions;
    in
    dangerousFailures == [ ]
  );

  dangerous-path-allowed = mkCheck "sandbox-dangerous-path-allowed" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.sandbox.extraBinds.required.ro = [ ".ssh" ];
              sandboxes.test.sandbox.allowDangerousPaths = [ ".ssh" ];
            };
          }
        ];
      };
      failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
      dangerousFailures = builtins.filter (
        x: lib.hasInfix "expose credentials" x.message
      ) failedAssertions;
    in
    dangerousFailures == [ ]
  );

  dangerous-path-warnings-disabled = mkCheck "sandbox-dangerous-path-warnings-disabled" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.sandbox.extraBinds.required.ro = [ ".ssh" ];
              sandboxes.test.sandbox.dangerousPathWarnings = false;
            };
          }
        ];
      };
      failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
      dangerousFailures = builtins.filter (
        x: lib.hasInfix "expose credentials" x.message
      ) failedAssertions;
    in
    dangerousFailures == [ ]
  );

  dangerous-path-dir-bind = mkAssertionCheck "sandbox-dangerous-path-dir-bind" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.extraBinds.dir."/persist" = [ ".gnupg" ];
      };
    }
  ] "expose credentials";

  dangerous-path-child = mkAssertionCheck "sandbox-dangerous-path-child" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.extraBinds.required.ro = [ ".ssh/id_rsa" ];
      };
    }
  ] "expose credentials";

  dangerous-path-child-deep = mkAssertionCheck "sandbox-dangerous-path-child-deep" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.extraBinds.required.rw = [ ".aws/credentials" ];
      };
    }
  ] "expose credentials";

  dangerous-path-raw-bind-ro = mkAssertionCheck "sandbox-dangerous-path-raw-bind-ro" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.binds.ro = [
          {
            src = "$HOME/.ssh";
            dest = "/tmp/ssh";
          }
        ];
      };
    }
  ] "expose credentials";

  dangerous-path-raw-bind-absolute-home =
    mkAssertionCheck "sandbox-dangerous-path-raw-bind-absolute-home"
      [
        {
          cloister = {
            enable = true;
            sandboxes.test.sandbox.binds.rw = [
              {
                src = "/home/testuser/.aws";
                dest = "/tmp/aws";
              }
            ];
          };
        }
      ]
      "expose credentials";

  dangerous-path-managed-file = mkAssertionCheck "sandbox-dangerous-path-managed-file" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.extraBinds.managedFile = [ ".ssh" ];
      };
      home.file.".ssh/config".text = "Host *";
    }
  ] "expose credentials";

  dangerous-path-managed-file-allowed = mkCheck "sandbox-dangerous-path-managed-file-allowed" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.sandbox.extraBinds.managedFile = [ ".ssh" ];
              sandboxes.test.sandbox.allowDangerousPaths = [ ".ssh" ];
            };
            home.file.".ssh/config".text = "Host *";
          }
        ];
      };
      failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
      dangerousFailures = builtins.filter (
        x: lib.hasInfix "expose credentials" x.message
      ) failedAssertions;
    in
    dangerousFailures == [ ]
  );

  dangerous-path-raw-bind-allowed = mkCheck "sandbox-dangerous-path-raw-bind-allowed" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.sandbox.binds.ro = [
                {
                  src = "/home/testuser/.ssh";
                  dest = "/tmp/ssh";
                }
              ];
              sandboxes.test.sandbox.allowDangerousPaths = [ ".ssh" ];
            };
          }
        ];
      };
      failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
      dangerousFailures = builtins.filter (
        x: lib.hasInfix "expose credentials" x.message
      ) failedAssertions;
    in
    dangerousFailures == [ ]
  );

  safe-path-no-warning = mkCheck "sandbox-safe-path-no-warning" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.sandbox.extraBinds.required.rw = [ ".local/share/atuin" ];
            };
          }
        ];
      };
      failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
      dangerousFailures = builtins.filter (
        x: lib.hasInfix "expose credentials" x.message
      ) failedAssertions;
    in
    dangerousFailures == [ ]
  );

  # ── Feature bind-source exclusion tests ─────────────────────────────

  # git.enable excludes its own bind sources ($HOME/.config/git,
  # $HOME/.gitconfig) from bind_sources so the runtime dangerous-path
  # check won't trip on the .config/git/credentials overlap.  But user-
  # supplied extraBinds targeting the same tree must still be caught.
  git-enable-still-catches-credentials =
    mkAssertionCheck "sandbox-git-enable-still-catches-credentials"
      [
        {
          cloister = {
            enable = true;
            sandboxes.test.git.enable = true;
            sandboxes.test.sandbox.extraBinds.required.ro = [ ".config/git/credentials" ];
          };
        }
      ]
      "expose credentials";

  # ssh.enable does not bind anything under ~/.ssh, so .ssh in
  # extraBinds must still be caught as dangerous.
  ssh-enable-still-catches-ssh = mkAssertionCheck "sandbox-ssh-enable-still-catches-ssh" [
    {
      cloister = {
        enable = true;
        sandboxes.test.ssh.enable = true;
        sandboxes.test.sandbox.extraBinds.required.ro = [ ".ssh" ];
      };
    }
  ] "expose credentials";

  # ── Network enable/disable tests ────────────────────────────────────

  network-enable = mkConfigCheck "sandbox-network-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.network.enable = true;
        };
      }
    ];
  }) "test" ''"network_enable":true'' true;

  network-disable = mkConfigCheck "sandbox-network-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.network.enable = false;
        };
      }
    ];
  }) "test" ''"network_enable":false'' true;

  # ── CLOISTER env var set to sandbox name ──────────────────────────

  cloister-env-name = mkConfigCheck "sandbox-cloister-env-name" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.mybox = { };
        };
      }
    ];
  }) "mybox" ''"name":"mybox"'' true;

  # ── Git enable/disable tests ──────────────────────────────────────

  git-enable = mkConfigCheck "sandbox-git-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.git.enable = true;
        };
      }
    ];
  }) "test" ''"git_enable":true'' true;

  git-disable = mkConfigCheck "sandbox-git-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.git.enable = false;
        };
      }
    ];
  }) "test" ''"git_enable":false'' true;

  # ── Newline unsafe char test ──────────────────────────────────────

  unsafe-path-newline = mkAssertionCheck "sandbox-unsafe-path-newline" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.dirs = lib.mkForce [
          "/safe"
          ''
            /has
            newline''
        ];
      };
    }
  ] "variable expansions";

  # ── Env value char tests ─────────────────────────────────────────

  env-value-dollar = mkConfigCheck "sandbox-env-value-dollar" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.env.BAD = lib.mkForce "value$[code]";
        };
      }
    ];
  }) "test" "value$[code]" true;

  env-value-prompt = mkConfigCheck "sandbox-env-value-prompt" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.env.BAD = lib.mkForce "value\${code@P}";
        };
      }
    ];
  }) "test" "value\${code@P}" true;

  # ── passthroughEnv test ─────────────────────────────────────────────

  passthrough-env = mkConfigCheck "sandbox-passthrough-env" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.passthroughEnv = [ "MY_CUSTOM_VAR" ];
        };
      }
    ];
  }) "test" "MY_CUSTOM_VAR" true;

  passthrough-env-invalid = mkAssertionCheck "sandbox-passthrough-env-invalid" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.passthroughEnv = [
          "MY_CUSTOM_VAR"
          "1BAD"
          "BAD-NAME"
        ];
      };
    }
  ] "sandbox.passthroughEnv contains invalid variable names";

  # ── Shell name tests ─────────────────────────────────────────────────

  shell-name-zsh = mkConfigCheck "sandbox-shell-name-zsh" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.shell = {
            name = "zsh";
          };
        };
      }
    ];
  }) "test" ''"shell_name":"zsh"'' true;

  shell-name-bash = mkConfigCheck "sandbox-shell-name-bash" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.shell = {
            name = "bash";
          };
        };
      }
    ];
  }) "test" ''"shell_name":"bash"'' true;

  # ── Network namespace with network.enable=false ─────────────────────

  netns-no-share-net = mkConfigCheck "sandbox-netns-no-share-net" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            network.enable = false;
            network.namespace = "vpn";
          };
        };
      }
    ];
  }) "test" ''"network_enable":false'' true;

  # ── Network namespace resolv.conf tests ──────────────────────────────

  netns-resolv-conf = mkConfigCheck "sandbox-netns-resolv-conf" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.network.namespace = "vpn";
        };
      }
    ];
  }) "test" "/etc/netns/vpn/resolv.conf" true;

  netns-resolv-conf-dest = mkConfigCheck "sandbox-netns-resolv-conf-dest" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.network.namespace = "vpn";
        };
      }
    ];
  }) "test" ''"/etc/netns/vpn/resolv.conf","/etc/resolv.conf"'' true;

  netns-hosts = mkConfigCheck "sandbox-netns-hosts" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.network.namespace = "vpn";
        };
      }
    ];
  }) "test" "/etc/netns/vpn/hosts" true;

  netns-hosts-dest = mkConfigCheck "sandbox-netns-hosts-dest" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.network.namespace = "vpn";
        };
      }
    ];
  }) "test" ''"/etc/netns/vpn/hosts","/etc/hosts"'' true;

  no-netns-resolv-conf = mkConfigCheck "sandbox-no-netns-resolv-conf" (evalConfig {
    modules = [ baseModule ];
  }) "test" "/etc/netns" false;

  netns-still-entered = mkConfigCheck "sandbox-netns-still-entered" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            network.enable = false;
            network.namespace = "vpn";
          };
        };
      }
    ];
  }) "test" ''"network_namespace":"vpn"'' true;

  # ── GPU enable/disable tests ──────────────────────────────────────

  gpu-enable = mkConfigCheck "sandbox-gpu-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" ''"gpu_enable":true'' true;

  gpu-disable = mkConfigCheck "sandbox-gpu-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            gui.wayland = {
              enable = true;
              securityContext.enable = false;
            };
            gui.gpu.enable = false;
          };
        };
      }
    ];
  }) "test" ''"gpu_enable":false'' true;

  gpu-shm-default = mkConfigCheck "sandbox-gpu-shm-default" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" ''"gpu_shm":true'' true;

  gpu-shm-disable = mkConfigCheck "sandbox-gpu-shm-disable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            gui.wayland = {
              enable = true;
              securityContext.enable = false;
            };
            gui.gpu.shm = false;
          };
        };
      }
    ];
  }) "test" ''"gpu_shm":false'' true;

  # ── Device bind tests ─────────────────────────────────────────────

  dev-bind-single = mkConfigCheck "sandbox-dev-bind-single" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.devBinds = [ "/dev/video0" ];
        };
      }
    ];
  }) "test" "/dev/video0" true;

  # ── Desktop entry assertion tests ─────────────────────────────────

  desktop-entry-without-gui = mkAssertionCheck "sandbox-desktop-entry-without-gui" [
    {
      cloister = {
        enable = true;
        sandboxes.test.gui.desktopEntry.enable = true;
      };
    }
  ] "gui.desktopEntry.enable requires";

  # ── LOCALE_ARCHIVE tests ───────────────────────────────────────────

  locale-archive-present = mkConfigCheck "sandbox-locale-archive-present" (evalConfig {
    modules = [ baseModule ];
  }) "test" "LOCALE_ARCHIVE" true;

  locale-archive-overridable = mkConfigCheck "sandbox-locale-archive-overridable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.env.LOCALE_ARCHIVE = "/custom/locale-archive";
        };
      }
    ];
  }) "test" "/custom/locale-archive" true;

  # ── XDG_DATA_DIRS tests ───────────────────────────────────────────

  xdg-data-dirs-with-gui = mkConfigCheck "sandbox-xdg-data-dirs-with-gui" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" "XDG_DATA_DIRS" true;

  xdg-data-dirs-without-gui = mkConfigCheck "sandbox-xdg-data-dirs-without-gui" (evalConfig {
    modules = [ baseModule ];
  }) "test" "XDG_DATA_DIRS" false;

  xdg-data-dirs-has-share = mkConfigCheck "sandbox-xdg-data-dirs-has-share" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" "/share" true;

  gui-env-override-xdg-data-dirs = mkAssertionCheck "sandbox-gui-env-override-xdg-data-dirs" [
    {
      cloister = {
        enable = true;
        sandboxes.test = {
          gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
          sandbox.env.XDG_DATA_DIRS = lib.mkForce "/bad/share";
        };
      };
    }
  ] "managed by gui";

  # ── GTK theme tests ────────────────────────────────────────────────

  gtk-theme-default-with-gui = mkConfigCheck "sandbox-gtk-theme-default-with-gui" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" "GTK_THEME" true;

  gtk-theme-default-is-adwaita = mkConfigCheck "sandbox-gtk-theme-default-is-adwaita" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" "Adwaita" true;

  gtk-theme-custom-name = mkConfigCheck "sandbox-gtk-theme-custom-name" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            gui.wayland = {
              enable = true;
              securityContext.enable = false;
            };
            gui.gtk.theme = "Adwaita:dark";
          };
        };
      }
    ];
  }) "test" "Adwaita:dark" true;

  gtk-theme-absent-without-gui = mkConfigCheck "sandbox-gtk-theme-absent-without-gui" (evalConfig {
    modules = [ baseModule ];
  }) "test" "GTK_THEME" false;

  gui-env-override-gtk-theme = mkAssertionCheck "sandbox-gui-env-override-gtk-theme" [
    {
      cloister = {
        enable = true;
        sandboxes.test = {
          gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
          sandbox.env.GTK_THEME = lib.mkForce "Breeze";
        };
      };
    }
  ] "managed by gui";

  gtk3-in-xdg-data-dirs = mkConfigCheck "sandbox-gtk3-in-xdg-data-dirs" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" "gtk+3" true;

  gtk-disabled-no-gtk3 = mkConfigCheck "sandbox-gtk-disabled-no-gtk3" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            gui.wayland = {
              enable = true;
              securityContext.enable = false;
            };
            gui.gtk.enable = false;
          };
        };
      }
    ];
  }) "test" "gtk+3" false;

  gtk-disabled-no-gtk-theme = mkConfigCheck "sandbox-gtk-disabled-no-gtk-theme" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            gui.wayland = {
              enable = true;
              securityContext.enable = false;
            };
            gui.gtk.enable = false;
          };
        };
      }
    ];
  }) "test" "GTK_THEME" false;

  # ── Qt theme tests ─────────────────────────────────────────────────

  qt-platform-theme-with-enable = mkConfigCheck "sandbox-qt-platform-theme-with-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            gui.wayland = {
              enable = true;
              securityContext.enable = false;
            };
            gui.qt.enable = true;
          };
        };
      }
    ];
  }) "test" "QT_QPA_PLATFORMTHEME" true;

  qt-platform-theme-default-gtk3 = mkConfigCheck "sandbox-qt-platform-theme-default-gtk3" (evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test = {
              gui.wayland = {
                enable = true;
                securityContext.enable = false;
              };
              gui.qt.enable = true;
            };
          };
        }
      ];
    }
  ) "test" "gtk3" true;

  qt-absent-without-enable = mkConfigCheck "sandbox-qt-absent-without-enable" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" "QT_QPA_PLATFORMTHEME" false;

  qt-style-override = mkConfigCheck "sandbox-qt-style-override" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            gui.wayland = {
              enable = true;
              securityContext.enable = false;
            };
            gui.qt = {
              enable = true;
              style = "adwaita-dark";
            };
          };
        };
      }
    ];
  }) "test" "adwaita-dark" true;

  qt-no-style-override-by-default = mkConfigCheck "sandbox-qt-no-style-override-by-default" (
    evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test = {
              gui.wayland = {
                enable = true;
                securityContext.enable = false;
              };
              gui.qt.enable = true;
            };
          };
        }
      ];
    }
  ) "test" "QT_STYLE_OVERRIDE" false;

  # ── bindWorkingDirectory tests ───────────────────────────────────────

  bind-working-dir-default-true = mkConfigCheck "sandbox-bind-working-dir-default-true" (evalConfig {
    modules = [ baseModule ];
  }) "test" ''"bind_working_directory":true'' true;

  bind-working-dir-false-json = mkConfigCheck "sandbox-bind-working-dir-false-json" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.bindWorkingDirectory = false;
        };
      }
    ];
  }) "test" ''"bind_working_directory":false'' true;

  bind-working-dir-false-no-sandbox-dir =
    mkConfigCheck "sandbox-bind-working-dir-false-no-sandbox-dir"
      (evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.sandbox.bindWorkingDirectory = false;
            };
          }
        ];
      })
      "test"
      "$SANDBOX_DIR"
      false;

  bind-working-dir-true-has-sandbox-dir =
    mkConfigCheck "sandbox-bind-working-dir-true-has-sandbox-dir"
      (evalConfig { modules = [ baseModule ]; })
      "test"
      "$SANDBOX_DIR"
      true;

  bind-working-dir-false-perdir-incompatible =
    mkAssertionCheck "sandbox-bind-working-dir-false-perdir"
      [
        {
          cloister = {
            enable = true;
            sandboxes.test.sandbox = {
              bindWorkingDirectory = false;
              extraBinds.perDir = [ ".cache" ];
            };
          };
        }
      ]
      "incompatible with sandbox.extraBinds.perDir";

  # ── shell.hostConfig tests ──────────────────────────────────────────

  host-config-default-true = mkConfigCheck "sandbox-host-config-default-true" (evalConfig {
    modules = [ baseModule ];
  }) "test" ''"shell_host_config":true'' true;

  host-config-false-json = mkConfigCheck "sandbox-host-config-false-json" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.shell.hostConfig = false;
        };
      }
    ];
  }) "test" ''"shell_host_config":false'' true;

  # When hostConfig=false (zsh), shell config binds should NOT appear
  host-config-false-no-zshrc = mkConfigCheck "sandbox-host-config-false-no-zshrc" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            shell.name = "zsh";
            shell.hostConfig = false;
          };
        };
      }
    ];
  }) "test" ".zshrc" false;

  # When hostConfig=false (zsh), ZDOTDIR should point to nix store
  host-config-false-zsh-zdotdir = mkConfigCheck "sandbox-host-config-false-zsh-zdotdir" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            shell.name = "zsh";
            shell.hostConfig = false;
          };
        };
      }
    ];
  }) "test" "ZDOTDIR" true;

  # When hostConfig=false (bash), shell config binds should NOT appear
  host-config-false-no-bashrc = mkConfigCheck "sandbox-host-config-false-no-bashrc" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            shell.name = "bash";
            shell.hostConfig = false;
          };
        };
      }
    ];
  }) "test" ".bashrc" false;

  # When hostConfig=false (bash), a minimal .bash_profile bind should appear
  host-config-false-bash-init = mkConfigCheck "sandbox-host-config-false-bash-init" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            shell.name = "bash";
            shell.hostConfig = false;
          };
        };
      }
    ];
  }) "test" "$HOME/.bash_profile" true;

  # When hostConfig=true (default), shell config binds should appear
  host-config-true-has-zshrc = mkConfigCheck "sandbox-host-config-true-has-zshrc" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.shell.name = "zsh";
        };
      }
    ];
  }) "test" ".zshrc" true;

  # When hostConfig=true (default), no ZDOTDIR override should appear in static args
  host-config-true-no-zdotdir = mkConfigCheck "sandbox-host-config-true-no-zdotdir" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.shell.name = "zsh";
        };
      }
    ];
  }) "test" "ZDOTDIR" false;

  # ── Nix store bind ──────────────────────────────────────────────────

  nix-store-full-default = mkConfigCheck "sandbox-nix-store-full-default" (evalConfig {
    modules = [ baseModule ];
  }) "test" ''"--ro-bind","/nix","/nix"'' true;

  # ── copyFiles mode validation ────────────────────────────────────────

  copyfiles-invalid-mode = mkAssertionCheck "sandbox-copyfiles-invalid-mode" [
    {
      cloister = {
        enable = true;
        sandboxes.test.sandbox.copyFiles = [
          {
            src = "/nix/store/fake-src";
            dest = "$HOME/.config/myapp/config.toml";
            mode = "999";
          }
        ];
      };
    }
  ] "invalid mode values";

  copyfiles-valid-mode = mkConfigCheck "sandbox-copyfiles-valid-mode" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.sandbox.copyFiles = [
            {
              src = "/nix/store/fake-src";
              dest = "$HOME/.config/myapp/config.toml";
              mode = "0644";
            }
          ];
        };
      }
    ];
  }) "test" "0644" true;

  # ── scaleFactor validation ───────────────────────────────────────────

  scalefactor-valid-passes = mkCheck "sandbox-scalefactor-valid-passes" (
    let
      config = evalConfig {
        modules = [
          {
            cloister = {
              enable = true;
              sandboxes.test.gui.scaleFactor = 1.5;
            };
          }
        ];
      };
      failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
      scaleFailures = builtins.filter (x: lib.hasInfix "scaleFactor" x.message) failedAssertions;
    in
    scaleFailures == [ ]
  );

  scalefactor-zero-fails = mkAssertionCheck "sandbox-scalefactor-zero-fails" [
    {
      cloister = {
        enable = true;
        sandboxes.test.gui.scaleFactor = 0.0;
      };
    }
  ] "scaleFactor must be a positive value";

  scalefactor-negative-fails = mkAssertionCheck "sandbox-scalefactor-negative-fails" [
    {
      cloister = {
        enable = true;
        sandboxes.test.gui.scaleFactor = -1.0;
      };
    }
  ] "scaleFactor must be a positive value";

  # ── Fontconfig tests ──────────────────────────────────────────────

  fontconfig-present-with-gui = mkConfigCheck "sandbox-fontconfig-present-with-gui" (evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test.gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
        };
      }
    ];
  }) "test" "FONTCONFIG_FILE" true;

  fontconfig-absent-without-gui = mkConfigCheck "sandbox-fontconfig-absent-without-gui" (evalConfig {
    modules = [ baseModule ];
  }) "test" "FONTCONFIG_FILE" false;

  fontconfig-absent-empty-packages = mkConfigCheck "sandbox-fontconfig-absent-empty-packages" (
    evalConfig
    {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.test = {
              gui.wayland = {
                enable = true;
                securityContext.enable = false;
              };
              gui.fonts.packages = lib.mkForce [ ];
            };
          };
        }
      ];
    }
  ) "test" "FONTCONFIG_FILE" false;

  fontconfig-custom-packages = mkConfigCheck "sandbox-fontconfig-custom-packages" (evalConfig {
    modules = [
      (
        { pkgs, ... }:
        {
          cloister = {
            enable = true;
            sandboxes.test = {
              gui.wayland = {
                enable = true;
                securityContext.enable = false;
              };
              gui.fonts.packages = lib.mkForce [ pkgs.noto-fonts ];
            };
          };
        }
      )
    ];
  }) "test" "FONTCONFIG_FILE" true;

  gui-env-override-fontconfig = mkAssertionCheck "sandbox-gui-env-override-fontconfig" [
    {
      cloister = {
        enable = true;
        sandboxes.test = {
          gui.wayland = {
            enable = true;
            securityContext.enable = false;
          };
          sandbox.env.FONTCONFIG_FILE = lib.mkForce "/bad/fonts.conf";
        };
      };
    }
  ] "managed by gui";

}
