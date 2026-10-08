{
  checks,
  hm,
  lib,
  pkgs,
  ...
}:
let
  eval = hm {
    cloister = {
      enable = true;
      sandboxes.browser = {
        defaultCommand = [ "firefox" ];
        gui = {
          enable = true;
          theme = {
            gtk = "Adwaita";
            icon = "Adwaita";
            qt.style = "fusion";
          };
          packages = [ pkgs.qt6.qtbase ];
          desktopEntry = {
            execArgs = "%U";
            icon = "firefox";
            categories = [ "Network" ];
            mimeTypes = [ "x-scheme-handler/https" ];
            terminal = true;
            genericName = "Web Browser";
            comment = "Sandboxed browser";
            startupNotify = true;
          };
        };
        dbus = {
          enable = true;
          log = true;
          portal.openUri = true;
          rawPolicies = {
            talk = [ "org.example.Service" ];
            own = [ "org.example.Owner" ];
            see = [ "org.example.Observer" ];
            call."org.example.Calls" = [ "Method@/org/example" ];
            broadcast."org.example.Signals" = [ "Signal@/org/example" ];
          };
        };
        audio.pipewire = {
          enable = true;
          filters = {
            audioIn = true;
            videoIn = true;
            control = true;
            routing = true;
          };
        };
      };
    };
  };

  guiOffEval = hm {
    cloister = {
      enable = true;
      sandboxes.browser = {
        gui.enable = false;
      };
    };
  };

  guiEnvOverride = hm {
    cloister = {
      enable = true;
      sandboxes.browser = {
        gui.enable = true;
        sandbox.env.FONTCONFIG_FILE = "/tmp/fonts.conf";
      };
    };
  };

  dbusEnvOverride = hm {
    cloister = {
      enable = true;
      sandboxes.browser = {
        dbus.enable = true;
        sandbox.env.DBUS_SESSION_BUS_ADDRESS = "unix:path=/tmp/bus";
      };
    };
  };

  xdgRuntimeDirPassthrough = hm {
    cloister = {
      enable = true;
      sandboxes.browser = {
        gui.enable = true;
        sandbox.passthroughEnv = [ "XDG_RUNTIME_DIR" ];
      };
    };
  };

  allPortalsEval = hm {
    cloister = {
      enable = true;
      sandboxes.browser = {
        gui.enable = true;
        dbus = {
          enable = true;
          notifications = true;
          portal = {
            fileChooser = true;
            openUri = true;
            screencast = true;
            camera = true;
          };
        };
      };
    };
  };

  sshOnlyEval = hm {
    cloister = {
      enable = true;
      sandboxes.browser.ssh.enable = true;
    };
  };

  dbusDisabledEval = hm {
    cloister = {
      enable = true;
      sandboxes.browser.dbus.enable = false;
    };
  };

  pipewireDisabledEval = hm {
    cloister = {
      enable = true;
      sandboxes.browser.audio.pipewire.enable = false;
    };
  };

  portalWithoutDbus = hm {
    cloister = {
      enable = true;
      sandboxes.browser.dbus.portal.openUri = true;
    };
  };

  desktopWithoutCommand = hm {
    cloister = {
      enable = true;
      sandboxes.browser.gui = {
        enable = true;
        desktopEntry = { };
      };
    };
  };

  anonymizeNativePipewire = hm {
    cloister = {
      enable = true;
      sandboxes.browser = {
        sandbox.anonymize.enable = true;
        audio.pipewire.enable = true;
      };
    };
  };

  pulseOnlyEval = hm {
    cloister = {
      enable = true;
      sandboxes.listener = {
        sandbox.anonymize.enable = true;
        dbus.enable = true;
        audio.pipewire = {
          enable = true;
          pulseOnly = true;
        };
      };
    };
  };

  nativeDefaultEval = hm {
    cloister = {
      enable = true;
      sandboxes.player.audio.pipewire.enable = true;
    };
  };

  pulseOnlyVideoConflict = hm {
    cloister = {
      enable = true;
      sandboxes.player.audio.pipewire = {
        enable = true;
        pulseOnly = true;
        filters.videoIn = true;
      };
    };
  };

  invalidDesktopExecArgs = hm {
    cloister = {
      enable = true;
      sandboxes.browser = {
        defaultCommand = [ "firefox" ];
        gui = {
          enable = true;
          desktopEntry = {
            execArgs = "%U; touch /tmp/pwned";
          };
        };
      };
    };
  };

  pulseOnlyNoDbusEval = hm {
    cloister = {
      enable = true;
      sandboxes.listener = {
        dbus.enable = false;
        audio.pipewire = {
          enable = true;
          pulseOnly = true;
        };
      };
    };
  };

  invalidDbusName = hm {
    cloister = {
      enable = true;
      sandboxes.browser.dbus = {
        enable = true;
        rawPolicies.talk = [ "invalid" ];
      };
    };
  };

  invalidDbusWildcard = hm {
    cloister = {
      enable = true;
      sandboxes.browser.dbus = {
        enable = true;
        rawPolicies.talk = [ "org.freedesktop.*.Desktop" ];
      };
    };
  };

  portalEnvOverride = hm {
    cloister = {
      enable = true;
      sandboxes.browser = {
        dbus = {
          enable = true;
          portal.fileChooser = true;
        };
        sandbox.env.GTK_USE_PORTAL = "0";
      };
    };
  };

  multiPipewireEval = hm {
    cloister = {
      enable = true;
      sandboxes = {
        alpha.audio.pipewire.enable = true;
        beta.audio.pipewire.enable = true;
      };
    };
  };

  inherit (eval.config.cloister.sandboxes) browser;
  desktop = eval.config.xdg.desktopEntries."cl-browser";
  sandboxConfig = eval.config.cloister._internal.sandboxConfigs.browser;
  pulseOnlyConfig = pulseOnlyEval.config.cloister._internal.sandboxConfigs.listener;
  pulseOnlyInternal = pulseOnlyEval.config.cloister._internal.sandboxInternals.listener;
  nativeDefaultConfig = nativeDefaultEval.config.cloister._internal.sandboxConfigs.player;
  pulseOnlyNoDbusInternal = pulseOnlyNoDbusEval.config.cloister._internal.sandboxInternals.listener;
  alphaConfig = multiPipewireEval.config.cloister._internal.sandboxConfigs.alpha;
  betaConfig = multiPipewireEval.config.cloister._internal.sandboxConfigs.beta;
  browserStaticArgs = builtins.toJSON sandboxConfig.static_bwrap_args;
  browserDynamicBinds = builtins.toJSON sandboxConfig.dynamic_binds;
  pipewireConf = eval.config.xdg.configFile."pipewire/pipewire.conf.d/99-cloister.conf".text;
  wireplumberConf =
    eval.config.xdg.configFile."wireplumber/wireplumber.conf.d/99-cloister-browser.conf".text;
  guiOffConfig = guiOffEval.config.cloister._internal.sandboxConfigs.browser;
  allPortalsConfig = allPortalsEval.config.cloister._internal.sandboxConfigs.browser;
  sshOnlyConfig = sshOnlyEval.config.cloister._internal.sandboxConfigs.browser;
  dbusDisabledConfig = dbusDisabledEval.config.cloister._internal.sandboxConfigs.browser;
  pipewireDisabledConfig = pipewireDisabledEval.config.cloister._internal.sandboxConfigs.browser;

  dbusProxyPath = sandboxConfig.dbus_proxy_path;
  allPortalsProxyPath = allPortalsConfig.dbus_proxy_path;
  dbusProxyScriptPath =
    eval.config.systemd.user.services.cloister-dbus-proxy-browser.Service.ExecStart;
  allPortalsProxyScriptPath =
    allPortalsEval.config.systemd.user.services.cloister-dbus-proxy-browser.Service.ExecStart;
  wireplumberLuaPath =
    let
      lines = lib.splitString "\n" wireplumberConf;
      matched = builtins.filter (
        line:
        builtins.match ".*name = /nix/store/.*-access-cloister-browser\\.lua, type = script/lua" line
        != null
      ) lines;
      line = if matched == [ ] then null else builtins.head matched;
    in
    if line == null then
      throw "failed to locate generated WirePlumber Lua script path"
    else
      builtins.elemAt (lib.splitString ", type = script/lua" (builtins.elemAt (lib.splitString "name = " line) 1)) 0;

  staticAssertions = [
    (checks.expectTrue "openUri adds portal call policy" (
      browser.dbus._portalPolicies.call ? "org.freedesktop.portal.Desktop"
    ))
    (checks.expectContains "desktop entry exec uses wrapper" "/bin/cl-browser" desktop.exec)
    (checks.expectContains "desktop entry exec args rendered" "%U" desktop.exec)
    (checks.expectEq "desktop entry icon rendered" "firefox" desktop.icon)
    (checks.expectEq "desktop entry category rendered" [ "Network" ] desktop.categories)
    (checks.expectEq "desktop entry mime type rendered" [ "x-scheme-handler/https" ] desktop.mimeType)
    (checks.expectEq "desktop entry terminal rendered" true desktop.terminal)
    (checks.expectEq "desktop entry generic name rendered" "Web Browser" desktop.genericName)
    (checks.expectEq "desktop entry comment rendered" "Sandboxed browser" desktop.comment)
    (checks.expectEq "desktop entry startup notify rendered" true desktop.startupNotify)
    (checks.expectEq "dbus enable renders" true sandboxConfig.dbus_enable)
    (checks.expectEq "dbus disable renders" false dbusDisabledConfig.dbus_enable)
    (checks.expectContains "dbus proxy wrapper path renders" "cloister-dbus-proxy-browser"
      dbusProxyPath
    )
    (checks.expectContains "gtk theme renders" ''"GTK_THEME","Adwaita"'' browserStaticArgs)
    (checks.expectContains "gtk settings gtk3 bind renders"
      ''"dest":"$HOME/.config/gtk-3.0/settings.ini"''
      browserDynamicBinds
    )
    (checks.expectContains "gtk settings gtk4 bind renders"
      ''"dest":"$HOME/.config/gtk-4.0/settings.ini"''
      browserDynamicBinds
    )
    (checks.expectContains "qt style renders" ''"QT_STYLE_OVERRIDE","fusion"'' browserStaticArgs)
    (checks.expectContains "fontconfig file renders" ''"FONTCONFIG_FILE"'' browserStaticArgs)
    (checks.expectContains "xdg data dirs render" ''"XDG_DATA_DIRS"'' browserStaticArgs)
    (checks.expectNotContains "ssh alone does not force xdg runtime passthrough" ''"XDG_RUNTIME_DIR"'' (
      builtins.toJSON sshOnlyConfig.passthrough_env
    ))
    (checks.expectNotContains "xdg runtime dir is absent without gui/dbus/audio" ''"XDG_RUNTIME_DIR"'' (
      builtins.toJSON guiOffConfig.passthrough_env
    ))
    (checks.expectNotContains "gui dbus audio no longer passthroughs xdg runtime dir"
      ''"XDG_RUNTIME_DIR"''
      (builtins.toJSON sandboxConfig.passthrough_env)
    )
    (checks.expectEq "gui toggle renders" true sandboxConfig.gui_enable)
    (checks.expectContains "portal flatpak marker bind renders" "/.flatpak-info" browserDynamicBinds)
    (checks.expectContains "portal runtime flatpak info bind renders" "$XDG_RUNTIME_DIR/flatpak-info"
      browserDynamicBinds
    )
    (checks.expectEq "portal flatpak app id renders" "dev.cloister.browser"
      allPortalsConfig.flatpak_app_id
    )
    (checks.expectContains "file chooser doc bind renders" "/run/flatpak/doc" (
      builtins.toJSON allPortalsConfig.dynamic_binds
    ))
    (checks.expectContains "file chooser runtime doc bind renders" "$XDG_RUNTIME_DIR/doc" (
      builtins.toJSON allPortalsConfig.dynamic_binds
    ))
    (checks.expectContains "portal wrapper path renders" "cloister-dbus-proxy-browser"
      allPortalsProxyPath
    )
    (checks.expectContains "GTK_USE_PORTAL is set for file chooser" ''"GTK_USE_PORTAL","1"'' (
      builtins.toJSON allPortalsConfig.static_bwrap_args
    ))
    (checks.expectEq "pipewire socket rendered" "cloister/pipewire/browser"
      sandboxConfig.pipewire_socket_name
    )
    (checks.expectEq "pipewire can be disabled" null pipewireDisabledConfig.pipewire_socket_name)
    (checks.expectContains "pipewire config declares per-sandbox socket" "cloister/pipewire/browser"
      pipewireConf
    )
    (checks.expectContains "wireplumber baseline permissions are restrictive"
      ''default_permissions = "l"''
      wireplumberConf
    )
    (checks.expectContains "wireplumber policy references sandbox access label"
      ''access = "cloister-browser"''
      wireplumberConf
    )
    (checks.expectContains "wireplumber policy installs custom script"
      "custom.access-cloister-browser = required"
      wireplumberConf
    )
    (checks.expectContains "pipewire config preserves unfiltered manager socket"
      ''pipewire-0-manager = "unrestricted"''
      pipewireConf
    )
    (checks.expectEq "pulse-only keeps backend socket distinct" "cloister/pipewire/listener"
      pulseOnlyConfig.pipewire_backend_socket_name
    )
    (checks.expectEq "pulse-only hides native pipewire socket" null
      pulseOnlyConfig.pipewire_socket_name
    )
    (checks.expectContains "pulse-only config blocks recording by default" "block-record-stream"
      pulseOnlyInternal.pipewirePulseOnlyConfText
    )
    (checks.expectContains "pulse-only config blocks volume control by default" "block-sink-volume"
      pulseOnlyInternal.pipewirePulseOnlyConfText
    )
    (checks.expectContains "pulse-only anonymize rewrites identity" ''"context.user-name" = "ubuntu"''
      pulseOnlyInternal.pipewirePulseOnlyConfText
    )
    (checks.expectContains "pulse-only disables dbus support when dbus is off" "support.dbus = false"
      pulseOnlyNoDbusInternal.pipewirePulseOnlyConfText
    )
    (checks.expectEq "pipewire defaults to filtered native socket" "cloister/pipewire/player"
      nativeDefaultConfig.pipewire_socket_name
    )
    (checks.expectEq "pipewire filter socket names stay per-sandbox" "cloister/pipewire/alpha"
      alphaConfig.pipewire_socket_name
    )
    (checks.expectEq "pipewire filter socket names differ across sandboxes" "cloister/pipewire/beta"
      betaConfig.pipewire_socket_name
    )
    (checks.expectAssertionMessage "portal requires dbus" portalWithoutDbus.assertions
      "dbus portal options and dbus.notifications require dbus.enable = true"
    )
    (checks.expectAssertionMessage "desktop entry requires default command"
      desktopWithoutCommand.assertions
      "gui.desktopEntry requires defaultCommand to be set"
    )
    (checks.expectAssertionMessage "anonymize rejects native pipewire"
      anonymizeNativePipewire.assertions
      "it is not possible to anonymize a PipeWire socket"
    )
    (checks.expectAssertionMessage "desktop entry execArgs rejects metacharacters"
      invalidDesktopExecArgs.assertions
      "gui.desktopEntry.execArgs must not contain shell metacharacters"
    )
    (checks.expectAssertionMessage "pulseOnly rejects video input" pulseOnlyVideoConflict.assertions
      "audio.pipewire.pulseOnly is audio-only and does not support audio.pipewire.filters.videoIn"
    )
    (checks.expectAssertionMessage "gui managed fontconfig cannot be overridden"
      guiEnvOverride.assertions
      "managed by gui"
    )
    (checks.expectAssertionMessage "dbus managed env cannot be overridden" dbusEnvOverride.assertions
      "managed by dbus"
    )
    (checks.expectAssertionMessage "xdg runtime dir passthrough is blocked"
      xdgRuntimeDirPassthrough.assertions
      "sandbox.passthroughEnv cannot include computed/managed keys: XDG_RUNTIME_DIR"
    )
    (checks.expectAssertionMessage "D-Bus names need dotted form" invalidDbusName.assertions
      "D-Bus policy names are invalid"
    )
    (checks.expectAssertionMessage "D-Bus wildcard position stays restricted"
      invalidDbusWildcard.assertions
      "D-Bus policy names are invalid"
    )
    (checks.expectAssertionMessage "portal-managed GTK_USE_PORTAL cannot be overridden"
      portalEnvOverride.assertions
      "managed by dbus.portal"
    )
  ];
in
builtins.deepSeq staticAssertions (
  pkgs.runCommand "test-cloister-gui-dbus-audio" { } ''
    grep -F -- '--log' '${dbusProxyScriptPath}'
    grep -F -- '--talk=org.example.Service' '${dbusProxyScriptPath}'
    grep -F -- '--own=org.example.Owner' '${dbusProxyScriptPath}'
    grep -F -- '--see=org.example.Observer' '${dbusProxyScriptPath}'
    grep -F -- '--call=org.example.Calls=Method@/org/example' '${dbusProxyScriptPath}'
    grep -F -- '--broadcast=org.example.Signals=Signal@/org/example' '${dbusProxyScriptPath}'

    grep -F -- 'CLOISTER_DBUS_PROXY_INSTANCE_ID' '${allPortalsProxyScriptPath}'
    grep -F -- '/.flatpak-info' '${allPortalsProxyScriptPath}'
    grep -F -- 'org.freedesktop.Notifications' '${allPortalsProxyScriptPath}'
    grep -F -- 'org.freedesktop.portal.Camera' '${allPortalsProxyScriptPath}'
    grep -F -- 'org.freedesktop.portal.ScreenCast' '${allPortalsProxyScriptPath}'
    grep -F -- 'org.freedesktop.portal.OpenURI' '${allPortalsProxyScriptPath}'

    grep -F -- 'permissions[node_id] = "rxlw"' '${wireplumberLuaPath}'
    grep -F -- 'permissions[metadata_id] = "rxwm"' '${wireplumberLuaPath}'
    grep -F -- '"Audio/Source"' '${wireplumberLuaPath}'
    grep -F -- '"Video/Source"' '${wireplumberLuaPath}'

    touch "$out"
  ''
)
