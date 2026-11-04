# Sandboxed services: long-running commands supervised by systemd user units.
#
# A service reuses an existing sandbox definition and may only add to it
# (working directory, binds, env, network namespace). The derived sandbox is
# rendered through the same mkSandbox pipeline, so it keeps the sandbox's
# name, state paths, and assertions, but gets its own JSON config.
{
  lib,
  pkgs,
  config,
  mkSandbox,
  cloister-sandbox,
}:
let
  cfg = config.cloister;
  patterns = import ./_patterns.nix;
  homeDir = config.home.homeDirectory;

  toHostPath = path: if lib.hasPrefix "/" path then path else "${homeDir}/${path}";

  bindSrc = entry: if builtins.isString entry then entry else toString entry.src;
  bindOptional = entry: if builtins.isString entry then false else entry.optional;

  isStorePath = path: lib.hasPrefix "/nix/store" (toHostPath path);

  hasUnsafeChars = path: lib.hasInfix "$" path || lib.hasInfix "\n" path || lib.hasInfix "\r" path;

  deriveSandboxConfig =
    svc: base:
    base
    // {
      sandbox = base.sandbox // {
        bindWorkingDirectory = true;
        readWrite = base.sandbox.readWrite ++ svc.readWrite;
        readOnly = base.sandbox.readOnly ++ svc.readOnly;
        env = base.sandbox.env // svc.env;
      };
      network =
        base.network
        // lib.optionalAttrs (svc.network.namespace != null) {
          inherit (svc.network) namespace;
        };
    };

  mkServiceAssertions =
    name: svc:
    let
      prefix = "cloister.services.${name}";
      bindPaths = map bindSrc (svc.readWrite ++ svc.readOnly);
    in
    [
      {
        assertion = builtins.match patterns.sandboxName name != null;
        message = "${prefix}: service name must match ${patterns.sandboxName}.";
      }
      {
        assertion = cfg.sandboxes ? ${svc.sandbox};
        message = "${prefix}.sandbox: unknown sandbox '${svc.sandbox}'. Known sandboxes: ${lib.concatStringsSep ", " (builtins.attrNames cfg.sandboxes)}.";
      }
      {
        assertion = svc.command != [ ] && builtins.head svc.command != "";
        message = "${prefix}.command: must be a non-empty argv list.";
      }
      {
        assertion = svc.workingDirectory != "" && !hasUnsafeChars svc.workingDirectory;
        message = "${prefix}.workingDirectory: must be a non-empty absolute or home-relative path without '$' or newlines.";
      }
      {
        assertion =
          let
            p = toHostPath svc.workingDirectory;
          in
          p != "/" && p != homeDir && p != "${homeDir}/";
        message = "${prefix}.workingDirectory: must not be / or the home directory itself.";
      }
      {
        assertion = !(builtins.any isStorePath ([ svc.workingDirectory ] ++ bindPaths));
        message = "${prefix}: workingDirectory and bind sources must not be Nix store paths.";
      }
      {
        assertion = builtins.all (k: builtins.match "^[A-Za-z_][A-Za-z0-9_]*$" k != null) (
          builtins.attrNames svc.env
        );
        message = "${prefix}.env: variable names must match ^[A-Za-z_][A-Za-z0-9_]*$.";
      }
    ];

  mkService =
    name: svc:
    let
      ownAssertions = mkServiceAssertions name svc;
      valid = builtins.all (a: a.assertion) ownAssertions;
      base = cfg.sandboxes.${svc.sandbox};
      derived = mkSandbox svc.sandbox (deriveSandboxConfig svc base);
      inherit (derived.internal) configJsonPath workerBrokerLauncherPackage;

      workingDirectory = toHostPath svc.workingDirectory;
      createDirs = lib.unique (
        [ workingDirectory ]
        ++ map (entry: toHostPath (bindSrc entry)) (
          builtins.filter (entry: !bindOptional entry) svc.readWrite
        )
      );

      unitName = "cloister-service-${name}";

      launcherText = ''
        set -eu
        umask 077

        for dir in ${lib.escapeShellArgs createDirs}; do
          if [ ! -e "$dir" ]; then
            mkdir -p -- "$dir"
          fi
        done

        cd -- ${lib.escapeShellArg workingDirectory}
        export CLOISTER_DIR=${lib.escapeShellArg workingDirectory}

        export CLOISTER_BUILD_REV="''${CLOISTER_BUILD_REV:-${derived.sandboxConfig.build_revision}}"
        export CLOISTER_CONFIG_PATH="''${CLOISTER_CONFIG_PATH:-${configJsonPath}}"
        export CLOISTER_WORKER_BROKER_LAUNCHER_PACKAGE="''${CLOISTER_WORKER_BROKER_LAUNCHER_PACKAGE:-${
          if workerBrokerLauncherPackage != null then toString workerBrokerLauncherPackage else ""
        }}"

        exec ${cloister-sandbox}/bin/cloister-sandbox --config ${configJsonPath} --service-name ${lib.escapeShellArg name} -- \
          -c ${lib.escapeShellArgs svc.command} "$@"
      '';

      launcher = pkgs.writeShellScriptBin unitName launcherText;

      baseUnit = {
        Unit = {
          Description = "Cloister service ${name} (sandbox ${svc.sandbox})";
          StartLimitBurst = 5;
          StartLimitIntervalSec = 300;
        };
        Service = {
          Type = "simple";
          ExecStart = "${launcher}/bin/${unitName}";
          Restart = "on-failure";
          RestartSec = 5;
          KillMode = "mixed";
          # cloister-sandbox forwards SIGTERM and exits 128+15 on a clean stop.
          SuccessExitStatus = "143";
          # cloister-sandbox escalates to SIGKILL after 10s; bound the worst case
          # if its own escalation cannot reach the sandbox.
          TimeoutStopSec = 20;
        };
      }
      // lib.optionalAttrs svc.autoStart {
        Install.WantedBy = [ "default.target" ];
      };

      unit = lib.recursiveUpdate baseUnit svc.extraServiceConfig;
    in
    {
      assertions =
        ownAssertions
        ++ lib.optionals valid (
          map (
            a:
            a
            // {
              message = "cloister.services.${name} (derived from sandbox ${svc.sandbox}): ${a.message}";
            }
          ) derived.assertions
        );
    }
    // lib.optionalAttrs valid {
      inherit launcher unit unitName;
      inherit (derived) imageStoreInfo sandboxConfig;
      internal = {
        inherit configJsonPath createDirs workingDirectory;
        inherit launcherText;
      };
    };

  allServices = lib.mapAttrs mkService cfg.services;
  validServices = lib.filterAttrs (_: s: s ? unit) allServices;
in
{
  assertions = lib.concatLists (lib.mapAttrsToList (_: s: s.assertions) allServices);
  packages = lib.mapAttrsToList (_: s: s.launcher) validServices;
  units = lib.mapAttrs' (_: s: lib.nameValuePair s.unitName s.unit) validServices;
  imageStoreInfos = builtins.filter (i: i != null) (
    lib.mapAttrsToList (_: s: s.imageStoreInfo) validServices
  );
  internals = lib.mapAttrs (_: s: s.internal // { inherit (s) sandboxConfig unit; }) validServices;
}
