{
  checks,
  hm,
  lib,
  ...
}:
let
  eval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        dbus.enable = true;
        sandbox.env.BASE = "1";
      };
      services.paseo = {
        sandbox = "dev";
        command = [
          "paseo"
          "daemon"
          "--listen"
          "127.0.0.1:6767"
        ];
        workingDirectory = "cloister/paseo";
        readWrite = [
          ".local/state/cloister/paseo"
          {
            src = "/srv/optional-cache";
            optional = true;
          }
        ];
        readOnly = [ { src = "/srv/reference"; } ];
        env.PASEO_HOME = "/home/tester/.local/state/cloister/paseo";
        network.namespace = "agents";
        extraServiceConfig.Service.RestartSec = 30;
      };
      services.manual = {
        sandbox = "dev";
        command = [ "sleep" ];
        workingDirectory = "/srv/manual";
        autoStart = false;
      };
    };
  };

  inherit (eval.config.cloister._internal) serviceInternals;
  inherit (serviceInternals) paseo;
  paseoUnit = eval.config.systemd.user.services.cloister-service-paseo;
  manualUnit = eval.config.systemd.user.services.cloister-service-manual;
  sandboxCfg = paseo.sandboxConfig;
  rwSrcs = map (b: b.src) (builtins.filter (b: b.mode == "rw") sandboxCfg.dynamic_binds);
  staticArgs = lib.concatStringsSep " " sandboxCfg.static_bwrap_args;

  failing =
    services:
    (hm {
      cloister = {
        enable = true;
        sandboxes.dev = { };
        inherit services;
      };
    }).assertions;
in
checks.mkCheck "test-cloister-services" [
  # Unit wiring
  (checks.expectContains "unit ExecStart uses service launcher" "/bin/cloister-service-paseo"
    paseoUnit.Service.ExecStart
  )
  (checks.expectEq "restart policy" "on-failure" paseoUnit.Service.Restart)
  (checks.expectEq "clean stop is success" "143" paseoUnit.Service.SuccessExitStatus)
  (checks.expectEq "bounded stop timeout" 20 paseoUnit.Service.TimeoutStopSec)
  (checks.expectEq "extraServiceConfig merged" 30 paseoUnit.Service.RestartSec)
  (checks.expectEq "autoStart wanted by default.target" [
    "default.target"
  ] paseoUnit.Install.WantedBy)
  (checks.expectFalse "autoStart=false omits Install" (manualUnit ? Install))
  (checks.expectFalse "no NoNewPrivileges" (paseoUnit.Service ? NoNewPrivileges))
  (checks.expectTrue "dbus proxy unit kept" (
    eval.config.systemd.user.services ? cloister-dbus-proxy-dev
  ))
  (checks.expectTrue "launcher installed" (
    builtins.any (p: lib.hasInfix "cloister-service-paseo" (toString p)) eval.config.home.packages
  ))

  # Launcher
  (checks.expectContains "launcher cds into working dir" "cd -- /home/tester/cloister/paseo"
    paseo.launcherText
  )
  (checks.expectContains "launcher pins project root"
    "export CLOISTER_DIR=/home/tester/cloister/paseo"
    paseo.launcherText
  )
  (checks.expectContains "launcher passes argv via -c" "-c paseo daemon --listen 127.0.0.1:6767"
    paseo.launcherText
  )
  (checks.expectContains "launcher uses derived config" (builtins.unsafeDiscardStringContext (
    toString paseo.configJsonPath
  )) (builtins.unsafeDiscardStringContext paseo.launcherText))
  (checks.expectEq "creates working dir and required rw binds" [
    "/home/tester/cloister/paseo"
    "/home/tester/.local/state/cloister/paseo"
  ] paseo.createDirs)

  # Derived sandbox config
  (checks.expectTrue "working directory bound" sandboxCfg.bind_working_directory)
  (checks.expectEq "netns override" "agents" sandboxCfg.network_namespace)
  (checks.expectTrue "netns enables network" sandboxCfg.network_enable)
  (checks.expectTrue "service rw bind present" (
    builtins.elem "$HOME/.local/state/cloister/paseo" rwSrcs
  ))
  (checks.expectContains "service ro bind present" "/srv/reference" staticArgs)
  (checks.expectContains "service env merged" "PASEO_HOME" staticArgs)
  (checks.expectContains "sandbox env kept" "BASE" staticArgs)
  (checks.expectEq "base sandbox keeps its own network" null
    eval.config.cloister._internal.sandboxConfigs.dev.network_namespace
  )
  (checks.expectTrue "derived config differs from base" (
    toString paseo.configJsonPath
    != toString eval.config.cloister._internal.sandboxInternals.dev.configJsonPath
  ))

  # Assertions
  (checks.expectAssertionMessage "unknown sandbox" (failing {
    x = {
      sandbox = "missing";
      command = [ "true" ];
      workingDirectory = "svc";
    };
  }) "unknown sandbox 'missing'")
  (checks.expectAssertionMessage "empty command" (failing {
    x = {
      sandbox = "dev";
      command = [ ];
      workingDirectory = "svc";
    };
  }) "non-empty argv")
  (checks.expectAssertionMessage "bad name" (failing {
    "bad name" = {
      sandbox = "dev";
      command = [ "true" ];
      workingDirectory = "svc";
    };
  }) "service name must match")
  (checks.expectAssertionMessage "store path rejected" (failing {
    x = {
      sandbox = "dev";
      command = [ "true" ];
      workingDirectory = "/nix/store/abc";
    };
  }) "must not be Nix store paths")
  (checks.expectAssertionMessage "home dir rejected" (failing {
    x = {
      sandbox = "dev";
      command = [ "true" ];
      workingDirectory = "/home/tester";
    };
  }) "must not be / or the home directory")
  (checks.expectAssertionMessage "derived sandbox assertions surface" (failing {
    x = {
      sandbox = "dev";
      command = [ "true" ];
      workingDirectory = "svc";
      readWrite = [ "../escape" ];
    };
  }) "cloister.services.x (derived from sandbox dev)")
]
