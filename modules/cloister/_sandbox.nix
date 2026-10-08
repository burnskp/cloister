# Namespace sandbox using bubblewrap
# Provides bare sandbox plumbing — personal tool preferences live in the consumer module
{
  config,
  pkgs,
  lib,
  osConfig ? null,
  ...
}:

let
  cfg = config.cloister;
  bwrapLib = import ./_bwrap.nix { inherit lib; };
  shells = import ./_mkShells.nix { inherit pkgs lib; };
  resolve = import ./_resolve.nix {
    inherit lib config;
    inherit (config.xdg) configHome;
  };
  cloister-seccomp-filter = pkgs.callPackage ../../helpers/cloister-seccomp-filter { };
  cloister-sandbox = pkgs.callPackage ../../helpers/cloister-sandbox { };
  buildRevision = lib.substring 0 12 (builtins.baseNameOf "${cloister-sandbox}");
  bubblewrap-subset-pid = pkgs.callPackage ../../pkgs/bubblewrap-subset-pid { };

  inherit (config.xdg) configHome;
  inherit (resolve) resolveConfigEntry resolveConfigEntryIfPresent resolveExplicitManagedBind;

  normalizePath =
    path:
    let
      isAbsolute = lib.hasPrefix "/" path;
      parts = lib.splitString "/" path;
      normalizedParts = builtins.foldl' (
        acc: part:
        if part == "" || part == "." then
          acc
        else if part == ".." then
          if acc == [ ] then [ ] else lib.init acc
        else
          acc ++ [ part ]
      ) [ ] parts;
      joined = lib.concatStringsSep "/" normalizedParts;
    in
    if isAbsolute then (if normalizedParts == [ ] then "/" else "/${joined}") else joined;

  # --- D-Bus proxy wrapper rendering ---

  dbusPolicyFlags =
    sCfg:
    let
      portalPolicies = sCfg.dbus._portalPolicies;
      inherit (sCfg.dbus) rawPolicies;
      talkFlags = map (name: "--talk=${name}") (portalPolicies.talk ++ rawPolicies.talk);
      ownFlags = map (name: "--own=${name}") (portalPolicies.own ++ rawPolicies.own);
      seeFlags = map (name: "--see=${name}") (portalPolicies.see ++ rawPolicies.see);
      mergeRuleAttrs = builtins.zipAttrsWith (_: values: lib.concatLists values) [
        portalPolicies.call
        rawPolicies.call
      ];
      mergeBroadcastAttrs = builtins.zipAttrsWith (_: values: lib.concatLists values) [
        portalPolicies.broadcast
        rawPolicies.broadcast
      ];
      callFlags = lib.concatLists (
        lib.mapAttrsToList (name: rules: map (rule: "--call=${name}=${rule}") rules) mergeRuleAttrs
      );
      broadcastFlags = lib.concatLists (
        lib.mapAttrsToList (
          name: rules: map (rule: "--broadcast=${name}=${rule}") rules
        ) mergeBroadcastAttrs
      );
    in
    talkFlags ++ ownFlags ++ seeFlags ++ callFlags ++ broadcastFlags;

  dbusEnabledSandboxes = lib.filterAttrs (_: sCfg: sCfg.dbus.enable) cfg.sandboxes;

  mkDbusProxyWrapper =
    name: sCfg:
    let
      policyFlags = dbusPolicyFlags sCfg;
      portalDesktopEnabled = builtins.any (x: x) [
        sCfg.dbus.portal.fileChooser
        sCfg.dbus.portal.openUri
        sCfg.dbus.portal.screencast
        sCfg.dbus.portal.camera
      ];
      proxyWrapper = pkgs.writeShellScript "cloister-dbus-proxy-${name}" ''
        set -eu

        runtime_dir="''${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR must be set}"
        proxy_socket="''${CLOISTER_DBUS_PROXY_SOCKET:-$runtime_dir/cloister/dbus/${name}}"

        ${lib.optionalString portalDesktopEnabled ''
          instance_id="''${CLOISTER_DBUS_PROXY_INSTANCE_ID:-}"
          if [ -z "$instance_id" ]; then
            echo "cloister-dbus-proxy-${name}: missing CLOISTER_DBUS_PROXY_INSTANCE_ID" >&2
            exit 1
          fi
          info_path="$runtime_dir/cloister/flatpak-info/$instance_id.ini"
          proxy_runtime_dir="$runtime_dir/.dbus-proxy"

          attempt=0
          while [ "$attempt" -lt 50 ]; do
            if [ -r "$info_path" ]; then
              break
            fi
            attempt=$((attempt + 1))
            ${pkgs.coreutils}/bin/sleep 0.1
          done

          if [ ! -r "$info_path" ]; then
            echo "cloister-dbus-proxy-${name}: missing Flatpak info at $info_path" >&2
            exit 1
          fi

          mkdir -p "$proxy_runtime_dir"
          chmod 700 "$proxy_runtime_dir"

          bwrap_args=(
            --die-with-parent
            --new-session
            --unshare-all
            --proc /proc
            --dev /dev
          )

          for entry in /*; do
            base="''${entry#/}"
            case "$base" in
              ""|.flatpak-info|dev|proc)
                continue
                ;;
              run|tmp|var)
                bwrap_args+=(--bind "$entry" "/$base")
                ;;
              *)
                if [ -L "$entry" ]; then
                  bwrap_args+=(--symlink "$(${pkgs.coreutils}/bin/readlink "$entry")" "/$base")
                else
                  bwrap_args+=(--ro-bind "$entry" "/$base")
                fi
                ;;
            esac
          done

          bwrap_args+=(--bind "$proxy_runtime_dir" "$proxy_runtime_dir")

          exec {info_fd}<"$info_path"

          exec ${pkgs.bubblewrap}/bin/bwrap \
            "''${bwrap_args[@]}" \
            --perms 0600 \
            --file "$info_fd" /.flatpak-info \
            -- ${pkgs.xdg-dbus-proxy}/bin/xdg-dbus-proxy \
            "unix:path=$runtime_dir/bus" \
            "$proxy_socket" \
            --filter \
            ${lib.escapeShellArgs (lib.optional sCfg.dbus.log "--log" ++ policyFlags)}
        ''}

        ${lib.optionalString (!portalDesktopEnabled) ''
          exec ${pkgs.xdg-dbus-proxy}/bin/xdg-dbus-proxy \
            "unix:path=$runtime_dir/bus" \
            "$proxy_socket" \
            --filter \
            ${lib.escapeShellArgs (lib.optional sCfg.dbus.log "--log" ++ policyFlags)}
        ''}
      '';
    in
    proxyWrapper;

  mkDbusService = name: sCfg: {
    Unit = {
      Description = "Cloister D-Bus proxy (${name})";
      StartLimitBurst = 5;
      StartLimitIntervalSec = 30;
    };
    Service = {
      ExecStart = mkDbusProxyWrapper name sCfg;
      Restart = "on-failure";
      RestartSec = 1;
      MemoryHigh = "64M";
      MemoryMax = "128M";
    };
  };

  dbusServices = lib.mapAttrs' (name: sCfg: {
    name = "cloister-dbus-proxy-${name}";
    value = mkDbusService name sCfg;
  }) dbusEnabledSandboxes;

  # --- Per-sandbox builder ---

  mkSandbox =
    name: sCfg:
    let
      shellLib = shells.${sCfg.shell.name};
      guiEnabled = sCfg.gui.enable;
      # --- Anonymization ---
      anonymize = sCfg.sandbox.anonymize.enable;
      sandboxHome = "/home/${sCfg.sandbox.anonymize.username}";

      # Transform bind destinations: replace $HOME with sandbox home
      remapBind =
        bind:
        if !anonymize then
          bind
        else
          let
            effectiveDest = if bind.dest != null then bind.dest else bind.src;
            newDest = builtins.replaceStrings [ "$HOME" ] [ sandboxHome ] effectiveDest;
          in
          bind // { dest = if newDest == bind.src then null else newDest; };

      remapBinds = map remapBind;

      flatpakAppId = "dev.cloister.${name}";

      portalDesktopEnabled = builtins.any (x: x) [
        sCfg.dbus.portal.fileChooser
        sCfg.dbus.portal.openUri
        sCfg.dbus.portal.screencast
        sCfg.dbus.portal.camera
      ];

      portalRoBinds = lib.optionals portalDesktopEnabled [
        {
          src = "$FLATPAK_INFO_PATH";
          dest = "/.flatpak-info";
          try = false;
        }
        {
          src = "$FLATPAK_INFO_PATH";
          dest = "$XDG_RUNTIME_DIR/flatpak-info";
          try = false;
        }
      ];

      portalRwBinds = lib.optionals sCfg.dbus.portal.fileChooser (
        let
          documentPortalPath = "$HOST_XDG_RUNTIME_DIR/doc/by-app/${flatpakAppId}";
        in
        [
          {
            src = documentPortalPath;
            dest = "/run/flatpak/doc";
            try = true;
          }
          {
            src = documentPortalPath;
            dest = "$XDG_RUNTIME_DIR/doc";
            try = true;
          }
        ]
      );

      workerBrokerNeedsNestedNamespaces = sCfg.workerBroker.profiles != { };
      effectiveNetworkEnabled = sCfg.network.enable || sCfg.network.namespace != null;

      seccompAllowNestedSandboxNamespaces =
        sCfg.sandbox.seccomp.allowChromiumSandbox || workerBrokerNeedsNestedNamespaces;

      # Worker-broker launches can invoke nested bubblewrap, which needs a
      # transient NETLINK_ROUTE socket while bringing up loopback.
      seccompDenyNetlink = !effectiveNetworkEnabled && !workerBrokerNeedsNestedNamespaces;

      seccompFilter = lib.optionalString sCfg.sandbox.seccomp.enable (
        pkgs.runCommand
          ("cloister-seccomp-${name}" + lib.optionalString seccompAllowNestedSandboxNamespaces "-chromium")
          { }
          ''
            ${cloister-seccomp-filter}/bin/cloister-seccomp-filter \
              --output "$out" \
              ${lib.optionalString seccompDenyNetlink "--deny-netlink"} \
              ${lib.optionalString seccompAllowNestedSandboxNamespaces "--allow-chromium-sandbox"}
          ''
      );

      sandboxHomeManagerConfig =
        if !sCfg.homeManager.enable then
          null
        else if sCfg.homeManager.config != null then
          sCfg.homeManager.config
        else if cfg.homeManager.builder != null then
          (cfg.homeManager.builder {
            inherit pkgs;
            modules = [
              {
                home.username = lib.mkDefault config.home.username;
                home.homeDirectory = lib.mkDefault config.home.homeDirectory;
                home.stateVersion = lib.mkDefault config.home.stateVersion;
              }
            ]
            ++ sCfg.homeManager.modules;
            inherit (sCfg.homeManager) extraSpecialArgs;
          }).config
        else
          throw "cloister.sandboxes.${name}.homeManager.modules requires cloister.homeManager.builder = inputs.home-manager.lib.homeManagerConfiguration, or homeManager.config";

      sandboxHomeManagerPackages = lib.optionals (
        sCfg.homeManager.enable && sCfg.homeManager.includePackages && sandboxHomeManagerConfig ? home
      ) (sandboxHomeManagerConfig.home.packages or [ ]);

      allPackages = sCfg._basePackages ++ sCfg.extraPackages ++ sandboxHomeManagerPackages;

      computedEnv = {
        PATH = lib.makeBinPath allPackages;
      }
      // lib.optionalAttrs hasCloisterInitContent {
        CLOISTER_SHELL_INIT = "${sandboxShellHookFile}";
      };

      # --- Internal low-level sandbox primitives ---

      pipewireDbusEnabled = sCfg.dbus.enable && sCfg.audio.pipewire.dbus.enable;
      pipewireNativeEnabled = sCfg.audio.pipewire.enable && !sCfg.audio.pipewire.pulseOnly;
      fileChooserPortalEnabled = sCfg.dbus.portal.fileChooser;

      pipewireContextProperties =
        if pipewireDbusEnabled then
          ""
        else
          ''
            context.properties = {
                support.dbus = false
            }
          '';

      pipewireClientConf = pkgs.writeText "cloister-pipewire-client.conf" ''
        # Cloister: disable D-Bus-dependent SPA support for sandboxed PipeWire clients
        ${pipewireContextProperties}
      '';

      customShellDest = "${
        if anonymize then sandboxHome else config.home.homeDirectory
      }/.config/cl-shell/${name}/custom";
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
          lib.optional (sCfg.shell.customRcPath.${field} != null) (
            mkCustomBind field sCfg.shell.customRcPath.${field}
          )
        ) rcFields;

      mkInternalBind =
        {
          src,
          dest ? null,
          try ? false,
        }:
        {
          inherit src dest try;
        };

      internalDirs = [
        "/nix"
        "/nix/store"
        "/var"
        "/run"
        "/run/current-system/sw/bin"
        "/usr/bin"
        "/bin"
        "/etc/ssl"
        "/etc/ssl/certs"
      ]
      ++ lib.optionals (customShellBinds != [ ]) [
        "${if anonymize then sandboxHome else config.home.homeDirectory}/.config/cl-shell/${name}"
      ]
      ++ lib.optionals fileChooserPortalEnabled [
        "/run/flatpak"
        "/run/flatpak/doc"
      ]
      ++ lib.optionals pipewireNativeEnabled [
        "${if anonymize then sandboxHome else config.home.homeDirectory}/.config/pipewire"
        "${if anonymize then sandboxHome else config.home.homeDirectory}/.config/pipewire/client.conf.d"
      ];

      internalTmpfs = [ "/tmp" ];

      internalSymlinks = [
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
      ++ lib.optionals pipewireNativeEnabled [
        {
          target = "${pipewireClientConf}";
          link = "${
            if anonymize then sandboxHome else config.home.homeDirectory
          }/.config/pipewire/client.conf.d/99-cloister.conf";
        }
      ];

      internalRoBinds =
        lib.filter (b: !(anonymize && (b.src == "/etc/passwd" || b.src == "/etc/group"))) [
          (mkInternalBind { src = "/etc/passwd"; })
          (mkInternalBind { src = "/etc/group"; })
          (mkInternalBind {
            src = "/etc/shells";
            try = true;
          })
          (mkInternalBind (
            if sCfg.network.namespace != null then
              {
                src = "/etc/netns/${sCfg.network.namespace}/hosts";
                dest = "/etc/hosts";
                try = true;
              }
            else
              {
                src = "/etc/hosts";
                try = true;
              }
          ))
          (mkInternalBind (
            if sCfg.network.dns.nameservers != [ ] then
              {
                src = toString (
                  pkgs.writeText "cloister-${name}-resolv.conf" (
                    lib.concatMapStrings (server: "nameserver ${server}\n") sCfg.network.dns.nameservers
                  )
                );
                dest = "/etc/resolv.conf";
              }
            else if sCfg.network.dns.resolvConf != null then
              {
                src = toString sCfg.network.dns.resolvConf;
                dest = "/etc/resolv.conf";
              }
            else if sCfg.network.namespace != null then
              {
                src = "/etc/netns/${sCfg.network.namespace}/resolv.conf";
                dest = "/etc/resolv.conf";
                try = true;
              }
            else
              {
                src = "/etc/resolv.conf";
                try = true;
              }
          ))
          (mkInternalBind {
            src = "/etc/ssh/ssh_known_hosts";
            try = true;
          })
          (mkInternalBind {
            src = "/etc/nix/nix.conf";
            try = true;
          })
          (mkInternalBind (
            if anonymize then
              {
                src = "${pkgs.tzdata}/share/zoneinfo/UTC";
                dest = "/etc/localtime";
              }
            else
              {
                src = "/etc/localtime";
                try = true;
              }
          ))
          (mkInternalBind {
            src = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
            dest = "/etc/ssl/certs/ca-bundle.crt";
          })
        ]
        ++ customShellBinds;

      # --- Resolution: convert semantic buckets → [{src, dest, try}] ---

      normalizeBindEntry =
        entry:
        let
          isString = builtins.isString entry;
          rawSrc = if isString then entry else toString entry.src;
          rawDest = if isString || entry.dest == null then null else entry.dest;
          optional = if isString then false else entry.optional;
          normalizeHome =
            path: if lib.hasPrefix "/" path || lib.hasPrefix "$" path then path else "$HOME/${path}";
        in
        {
          src = normalizeHome rawSrc;
          dest = if rawDest == null then null else normalizeHome rawDest;
          try = optional;
        };

      userRoBinds = map normalizeBindEntry sCfg.sandbox.readOnly;
      userRwBinds = map normalizeBindEntry sCfg.sandbox.readWrite;

      mkBindsFromAttr =
        attr:
        lib.concatLists (
          lib.mapAttrsToList (
            base: paths:
            map (p: {
              src = "${base}/cloister/${name}/${p}";
              dest = "$HOME/${p}";
              try = false;
            }) paths
          ) attr
        );

      mkMkdirSpecsFromAttr =
        attr:
        lib.concatLists (
          lib.mapAttrsToList (base: paths: map (p: { path = "${base}/cloister/${name}/${p}"; }) paths) attr
        );

      managedKeys = builtins.filter builtins.isString sCfg.sandbox.managed;
      explicitManagedEntries = builtins.filter (entry: !builtins.isString entry) sCfg.sandbox.managed;

      perDirBuckets = lib.filterAttrs (_: paths: paths != [ ]) sCfg.sandbox.state.projectDirs;
      perDirPaths = lib.concatLists (lib.attrValues perDirBuckets);
      workerBrokerDelegatedMounts = lib.concatMap (
        profile: builtins.attrValues profile.delegatedPerDirMounts
      ) (builtins.attrValues sCfg.workerBroker.profiles);

      dirBinds = mkBindsFromAttr sCfg.sandbox.state.dirs;
      fileBinds = mkBindsFromAttr sCfg.sandbox.state.files;

      perDirBinds = lib.concatLists (
        lib.mapAttrsToList (
          base: paths:
          map (p: {
            src = "${base}/$DIR_HASH/${p}";
            dest = "$HOME/${p}";
            try = false;
          }) paths
        ) perDirBuckets
      );

      # Directory-backed binds can shadow file mounts beneath them unless the
      # file bind is emitted after the directory bind in bubblewrap.
      dirBackedBinds = dirBinds ++ perDirBinds;

      normalizeCopyDest =
        path:
        let
          normalized = normalizePath path;
          homeDir = config.home.homeDirectory;
        in
        if lib.hasPrefix "$HOME/" normalized then
          normalized
        else if lib.hasPrefix "${homeDir}/" normalized then
          "$HOME/${lib.removePrefix "${homeDir}/" normalized}"
        else
          normalized;

      copyHostBase = sCfg.sandbox.copyBase;

      copyFileBinds = map (
        cf:
        let
          normalizedDest = normalizeCopyDest cf.dest;
        in
        {
          src = "${copyHostBase}/cloister/${name}/${lib.removePrefix "$HOME/" normalizedDest}";
          dest = normalizedDest;
          try = false;
        }
      ) sCfg.sandbox.copies;

      symlinkHome = if anonymize then sandboxHome else config.home.homeDirectory;
      normalizeSymlinkPath =
        path:
        if path == "$HOME" then
          symlinkHome
        else if lib.hasPrefix "$HOME/" path then
          "${symlinkHome}/${lib.removePrefix "$HOME/" path}"
        else if lib.hasPrefix "/" path then
          path
        else
          "${symlinkHome}/${path}";

      normalizeSymlinkTarget =
        target:
        if target == "$HOME" then
          symlinkHome
        else if lib.hasPrefix "$HOME/" target then
          "${symlinkHome}/${lib.removePrefix "$HOME/" target}"
        else
          target;

      userSymlinks = map (entry: {
        target = normalizeSymlinkTarget entry.target;
        link = normalizeSymlinkPath entry.link;
      }) sCfg.sandbox.symlinks;

      resolvedExtraRo = userRoBinds;
      resolvedExtraRw = userRwBinds ++ dirBinds ++ fileBinds ++ perDirBinds ++ copyFileBinds;

      # Resolve $HOME in managed file dests to the correct sandbox-side home
      # directory at eval time, so they work with both anonymize on and off.
      managedFileHome = if anonymize then sandboxHome else config.home.homeDirectory;
      resolvedManagedFileBinds = lib.concatMap resolveConfigEntry managedKeys;
      explicitManagedFileBinds = lib.concatMap resolveExplicitManagedBind explicitManagedEntries;
      sandboxHomeManagerFileBinds =
        let
          hmCfg = sandboxHomeManagerConfig;
          hmHomeDir = hmCfg.home.homeDirectory or config.home.homeDirectory;
          xdgFiles = lib.filterAttrs (_: entry: entry.enable or true) (hmCfg.xdg.configFile or { });
          homeFiles = lib.filterAttrs (_: entry: entry.enable or true) (hmCfg.home.file or { });
          normalizeHomeFileName =
            fileName:
            if lib.hasPrefix "/" fileName then
              if lib.hasPrefix "${hmHomeDir}/" fileName then
                lib.removePrefix "${hmHomeDir}/" fileName
              else
                throw "cloister.sandboxes.${name}.homeManager: home.file entry '${fileName}' is outside home.homeDirectory and cannot be mounted inside the sandbox home"
            else
              fileName;
          entryTarget =
            fileName: entry: if entry ? target && entry.target != null then entry.target else fileName;
          xdgDest = fileName: entry: "$HOME/.config/${entryTarget fileName entry}";
          homeDest = fileName: entry: "$HOME/${normalizeHomeFileName (entryTarget fileName entry)}";
        in
        lib.optionals (sCfg.homeManager.enable && sCfg.homeManager.includeFiles) (
          (lib.mapAttrsToList (fileName: entry: {
            src = toString entry.source;
            dest = xdgDest fileName entry;
            try = false;
          }) xdgFiles)
          ++ (lib.mapAttrsToList (fileName: entry: {
            src = toString entry.source;
            dest = homeDest fileName entry;
            try = false;
          }) homeFiles)
        );
      managedFileBinds = map (
        bind:
        bind
        // {
          dest = builtins.replaceStrings [ "$HOME" ] [ managedFileHome ] bind.dest;
        }
      ) (resolvedManagedFileBinds ++ explicitManagedFileBinds ++ sandboxHomeManagerFileBinds);
      managedFileDirs = lib.unique (map (bind: builtins.dirOf bind.dest) managedFileBinds);

      # Partition managedFileBinds: binds whose dest falls inside a
      # directory-backed bind mount must be applied AFTER the dir bind in
      # bwrap, otherwise the directory mount shadows the individual file
      # mounts.
      resolveManagedHome = builtins.replaceStrings [ "$HOME" ] [ managedFileHome ];
      managedFileOverlapsDir =
        bind:
        builtins.any (
          dirBind:
          let
            dirDest = resolveManagedHome (if dirBind.dest != null then dirBind.dest else dirBind.src);
          in
          bind.dest == dirDest || lib.hasPrefix "${dirDest}/" bind.dest
        ) dirBackedBinds;

      managedFileBindsNonOverlapping = builtins.filter (b: !managedFileOverlapsDir b) managedFileBinds;
      managedFileBindsOverlapping = builtins.filter managedFileOverlapsDir managedFileBinds;

      # Partition managedFileDirs: dirs inside a directory-backed bind mount
      # need host-side mkdir -p instead of bwrap --dir (which gets shadowed by
      # the bind).
      managedFileDirOverlap =
        dir:
        let
          matchingBinds = builtins.filter (
            bind:
            let
              dest = resolveManagedHome (if bind.dest != null then bind.dest else bind.src);
            in
            dir == dest || lib.hasPrefix "${dest}/" dir
          ) dirBackedBinds;
        in
        if matchingBinds == [ ] then
          null
        else
          let
            bind = builtins.head matchingBinds;
            dest = resolveManagedHome (if bind.dest != null then bind.dest else bind.src);
            relativePath = lib.removePrefix dest dir;
          in
          "${bind.src}${relativePath}";

      managedFileDirsNonOverlapping = builtins.filter (
        dir: managedFileDirOverlap dir == null
      ) managedFileDirs;
      managedFileDirsOverlapping = builtins.filter (
        dir: managedFileDirOverlap dir != null
      ) managedFileDirs;

      # D-Bus: conditional ro-bind for the proxy socket
      dbusBinds = lib.optionals sCfg.dbus.enable [
        {
          src = "$DBUS_PROXY_SOCKET";
          dest = "$XDG_RUNTIME_DIR/bus";
          try = true;
        }
      ];

      gitBinds = lib.optionals sCfg.git.enable [
        {
          src = "$HOME/.config/git/config";
          dest = null;
          try = true;
        }
        {
          src = "$HOME/.gitconfig";
          dest = null;
          try = true;
        }
      ];

      gtkSettingsIni = pkgs.writeText "cloister-gtk-settings-${name}.ini" ''
        [Settings]
        gtk-theme-name=${sCfg.gui.theme.gtk}
        gtk-icon-theme-name=${sCfg.gui.theme.icon}
      '';

      guiBinds = lib.optionals guiEnabled [
        {
          src = gtkSettingsIni;
          dest = "$HOME/.config/gtk-3.0/settings.ini";
          try = false;
        }
        {
          src = gtkSettingsIni;
          dest = "$HOME/.config/gtk-4.0/settings.ini";
          try = false;
        }
      ];

      shellConfigBinds = lib.optionals sCfg.shell.hostConfig (
        map (b: {
          inherit (b) src;
          dest = b.dest or null;
          try = b.try or false;
        }) shellLib.configBinds
      );

      shellConfigManagedFileBinds = lib.optionals sCfg.shell.hostConfig (
        lib.concatMap resolveConfigEntryIfPresent shellLib.managedConfigKeys
      );

      getBindDest = bind: if bind.dest != null then bind.dest else bind.src;

      shellConfigManagedDests = map getBindDest shellConfigManagedFileBinds;

      shellConfigHostBindOverlapsManaged =
        bind:
        let
          dest = getBindDest bind;
        in
        builtins.any (
          managedDest: managedDest == dest || lib.hasPrefix "${dest}/" managedDest
        ) shellConfigManagedDests;

      shellConfigHostBinds = builtins.filter (
        bind: !shellConfigHostBindOverlapsManaged bind
      ) shellConfigBinds;

      # --- Sandbox-local shell init ---
      cloisterInitContent = ''
        ${sCfg.init.rendered}
        ${sCfg.registry.rendered.inside}
      '';
      hasCloisterInitContent = builtins.match "[[:space:]]*" cloisterInitContent == null;

      # zsh: minimal ZDOTDIR with .zshrc (added via env, nix store always bound)
      cloisterZdotdir = pkgs.writeTextDir ".zshrc" cloisterInitContent;

      # bash: minimal .bash_profile (bound dynamically at $HOME/.bash_profile)
      cloisterBashProfile = pkgs.writeText "cloister-bash-profile-${name}" cloisterInitContent;

      sandboxShellHookFile = pkgs.writeText "cloister-${name}.${shellLib.initExt}" cloisterInitContent;

      noHostConfigBinds = lib.optionals (!sCfg.shell.hostConfig && sCfg.shell.name == "bash") [
        {
          src = "${cloisterBashProfile}";
          dest = "$HOME/.bash_profile";
          try = false;
        }
      ];

      noHostConfigEnv = lib.optionalAttrs (sCfg.shell.name == "zsh" && !sCfg.shell.hostConfig) {
        ZDOTDIR = "${cloisterZdotdir}";
      };

      sandboxDirBinds = lib.optionals sCfg.sandbox.bindWorkingDirectory [
        {
          src = "$SANDBOX_DIR";
          dest = if anonymize then "$SANDBOX_DEST" else null;
          try = false;
        }
      ];

      # Keys only — used for override/passthrough assertions.
      dbusEnv = lib.optionalAttrs sCfg.dbus.enable { DBUS_SESSION_BUS_ADDRESS = ""; };

      portalEnv = lib.optionalAttrs sCfg.dbus.portal.fileChooser { GTK_USE_PORTAL = "1"; };

      guiDataPackages = sCfg.gui.packages;

      qtPluginPaths = lib.concatMap (pkg: [
        "${pkg}/lib/qt-6/plugins"
        "${pkg}/lib/qt-5/plugins"
      ]) sCfg.gui.packages;

      guiEnv = lib.optionalAttrs guiEnabled (
        {
          NO_AT_BRIDGE = "1";
          NIXOS_OZONE_WL = "1";
          GTK_THEME = sCfg.gui.theme.gtk;
          GIO_EXTRA_MODULES = "${pkgs.dconf.lib}/lib/gio/modules";
          GSETTINGS_SCHEMA_DIR = lib.concatStringsSep ":" [
            "${pkgs.glib}/share/glib-2.0/schemas"
            "${pkgs.gsettings-desktop-schemas}/share/glib-2.0/schemas"
            "${pkgs.gtk3}/share/glib-2.0/schemas"
          ];
          GDK_PIXBUF_MODULE_FILE = "${pkgs.librsvg}/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache";
          QT_QPA_PLATFORMTHEME = sCfg.gui.theme.qt.platform;
        }
        // lib.optionalAttrs (sCfg.gui.theme.qt.style != null) {
          QT_STYLE_OVERRIDE = sCfg.gui.theme.qt.style;
        }
        // lib.optionalAttrs (qtPluginPaths != [ ]) {
          QT_PLUGIN_PATH = lib.concatStringsSep ":" qtPluginPaths;
        }
        // lib.optionalAttrs (guiDataPackages != [ ]) {
          XDG_DATA_DIRS = lib.concatStringsSep ":" (map (pkg: "${pkg}/share") guiDataPackages);
        }
        // lib.optionalAttrs (sCfg.gui.fonts != [ ]) {
          FONTCONFIG_FILE = pkgs.makeFontsConf {
            fontDirectories = sCfg.gui.fonts;
          };
        }
      );

      # --- Duplicate bind destination detection ---
      getDest = bind: if bind.dest != null then bind.dest else bind.src;

      normalizeDest =
        path:
        let
          homeDir = config.home.homeDirectory;
        in
        if path == "$HOME" || path == homeDir then
          "$HOME"
        else if lib.hasPrefix "$HOME/" path then
          path
        else if lib.hasPrefix "${homeDir}/" path then
          "$HOME/${lib.removePrefix "${homeDir}/" path}"
        else
          path;

      allDests = map (p: normalizeDest (getDest p)) (
        remapBinds (
          internalRoBinds
          ++ resolvedExtraRo
          ++ managedFileBinds
          ++ dbusBinds
          ++ gitBinds
          ++ guiBinds
          ++ shellConfigManagedFileBinds
          ++ shellConfigHostBinds
          ++ noHostConfigBinds
          ++ resolvedExtraRw
          ++ sandboxDirBinds
          ++ portalRwBinds
        )
        ++ portalRoBinds
      );

      findDuplicates =
        xs: lib.attrNames (lib.filterAttrs (_: v: builtins.length v > 1) (builtins.groupBy (x: x) xs));

      duplicateDests = findDuplicates allDests;

      # --- Overlapping dirs and tmpfs detection ---
      userSymlinkDirs = lib.unique (
        builtins.filter (dir: dir != "/") (map (entry: builtins.dirOf entry.link) userSymlinks)
      );
      allDirs = internalDirs ++ managedFileDirsNonOverlapping ++ userSymlinkDirs;
      dirTmpfsOverlap = lib.intersectLists allDirs internalTmpfs;

      # --- Duplicate symlink link paths ---
      allSymlinks = internalSymlinks ++ userSymlinks;
      allSymlinkLinks = map (s: normalizeDest s.link) allSymlinks;
      userSymlinkLinks = map (s: normalizeDest s.link) userSymlinks;
      duplicateLinks = findDuplicates allSymlinkLinks;

      # User symlinks are emitted before binds, so links that collide with or sit
      # underneath a bind destination would either be hidden by the later mount or
      # risk changing what the bind operation targets.
      userSymlinkBindConflicts = lib.unique (
        builtins.filter (
          link: builtins.any (dest: link == dest || (dest != "/" && lib.hasPrefix "${dest}/" link)) allDests
        ) userSymlinkLinks
      );

      # --- Duplicate managed file destinations ---
      duplicateManagedFiles = findDuplicates (map (bind: bind.dest) managedFileBinds);

      # --- Unsafe character assertion for user-provided paths ---
      userPaths =
        let
          bindEntryPaths =
            entry:
            if builtins.isString entry then
              [ entry ]
            else
              [ (toString entry.src) ] ++ lib.optional (entry.dest != null) entry.dest;
          managedEntryPaths =
            entry:
            if builtins.isString entry then
              [ entry ]
            else
              [
                (toString entry.src)
                entry.dest
              ];
          bindPaths = lib.concatMap bindEntryPaths (sCfg.sandbox.readOnly ++ sCfg.sandbox.readWrite);
          statePaths =
            lib.concatLists (lib.attrValues sCfg.sandbox.state.dirs)
            ++ lib.concatLists (lib.attrValues sCfg.sandbox.state.files)
            ++ perDirPaths
            ++ lib.attrNames sCfg.sandbox.state.dirs
            ++ lib.attrNames sCfg.sandbox.state.files;
          managedPaths = lib.concatMap managedEntryPaths sCfg.sandbox.managed;
          copySrcPaths = map (cf: cf.src) sCfg.sandbox.copies;
          unsafeCopyDestPaths = builtins.filter (
            dest:
            lib.hasInfix "\n" dest
            || lib.hasInfix "\r" dest
            || (lib.hasInfix "$" dest && !lib.hasPrefix "$HOME/" dest)
          ) (map (cf: cf.dest) sCfg.sandbox.copies);
          workerBrokerKeyPaths = lib.concatMap (profile: lib.attrNames profile.delegatedPerDirMounts) (
            builtins.attrValues sCfg.workerBroker.profiles
          );
          workerBrokerPaths = lib.concatMap (
            mount: [ mount.path ] ++ lib.optional (mount.subPath != null) mount.subPath
          ) workerBrokerDelegatedMounts;
          unsafeSymlinkPaths = lib.concatMap (
            entry:
            builtins.filter
              (
                path:
                lib.hasInfix "\n" path
                || lib.hasInfix "\r" path
                || (lib.hasInfix "$" path && path != "$HOME" && !lib.hasPrefix "$HOME/" path)
              )
              [
                entry.target
                entry.link
              ]
          ) sCfg.sandbox.symlinks;
        in
        bindPaths
        ++ statePaths
        ++ managedPaths
        ++ copySrcPaths
        ++ unsafeCopyDestPaths
        ++ unsafeSymlinkPaths
        ++ workerBrokerKeyPaths
        ++ workerBrokerPaths
        ++ lib.attrNames sCfg.sandbox.env
        ++ sCfg.sandbox.devices
        ++ sCfg.sandbox.disallowedPaths
        ++ lib.attrNames perDirBuckets
        ++ [ sCfg.sandbox.copyBase ];

      unsafePaths = builtins.filter (
        p: lib.hasInfix "$" p || lib.hasInfix "\n" p || lib.hasInfix "\r" p
      ) userPaths;

      # --- Computed env var override detection ---
      computedEnvKeys = lib.unique (
        [
          "XDG_RUNTIME_DIR"
          "HOST_XDG_RUNTIME_DIR"
        ]
        ++ lib.attrNames computedEnv
      );
      overriddenEnvKeys = lib.intersectLists computedEnvKeys (lib.attrNames sCfg.sandbox.env);

      dbusEnvKeys = lib.attrNames dbusEnv;
      overriddenDbusKeys = lib.intersectLists dbusEnvKeys (lib.attrNames sCfg.sandbox.env);

      guiEnvKeys = lib.attrNames guiEnv;
      overriddenGuiKeys = lib.intersectLists guiEnvKeys (lib.attrNames sCfg.sandbox.env);

      portalEnvKeys = lib.attrNames portalEnv;
      overriddenPortalKeys = lib.intersectLists portalEnvKeys (lib.attrNames sCfg.sandbox.env);

      passthroughBlockedKeys = lib.unique (computedEnvKeys ++ dbusEnvKeys ++ guiEnvKeys ++ portalEnvKeys);
      blockedPassthroughEnv = lib.intersectLists passthroughBlockedKeys sCfg.sandbox.passthroughEnv;

      # --- passthroughEnv validation ---
      invalidPassthroughEnv = builtins.filter (
        v: builtins.match "^[A-Za-z_][A-Za-z0-9_]*$" v == null
      ) sCfg.sandbox.passthroughEnv;

      # --- Assertions (extracted to _assertions.nix) ---
      assertions = import ./_assertions.nix {
        inherit
          lib
          name
          sCfg
          duplicateDests
          dirTmpfsOverlap
          duplicateLinks
          userSymlinkBindConflicts
          duplicateManagedFiles
          unsafePaths
          overriddenEnvKeys
          overriddenDbusKeys
          overriddenGuiKeys
          overriddenPortalKeys
          blockedPassthroughEnv
          invalidPassthroughEnv
          guiEnabled
          normalizeCopyDest
          normalizePath
          ;
        sandboxNames = builtins.attrNames cfg.sandboxes;
        hasPerDirBinds = perDirPaths != [ ];
      };

      renderedWorkerBroker = {
        profiles = lib.mapAttrs (_: profile: {
          inherit (profile) sandbox;
          workspace = {
            inherit (profile.workspace) mode;
          };
          delegated_per_dir_mounts = lib.mapAttrs (_: mount: {
            inherit (mount) mode path;
            sub_path = mount.subPath;
          }) profile.delegatedPerDirMounts;
        }) sCfg.workerBroker.profiles;
        generated_launchers = lib.mapAttrs (_: launcher: {
          inherit (launcher) profile sandbox;
        }) sCfg.workerBroker.generatedLaunchers;
      };

      # --- Compiled binary: JSON config and makeWrapper package ---

      # Partition binds: static (no $ variables) go into static_bwrap_args,
      # dynamic (contain $ references) go into dynamic_binds for runtime resolution.
      isStaticPath = path: !(lib.hasInfix "$" path);

      isStaticBind =
        bind:
        let
          dest = if bind.dest != null then bind.dest else bind.src;
        in
        isStaticPath bind.src && isStaticPath dest;

      # Static binds: fully resolved at Nix eval time
      staticRoBinds = builtins.filter isStaticBind (
        remapBinds (
          internalRoBinds
          ++ managedFileBindsNonOverlapping
          ++ guiBinds
          ++ shellConfigHostBinds
          ++ shellConfigManagedFileBinds
          ++ resolvedExtraRo
          ++ dbusBinds
          ++ gitBinds
          ++ noHostConfigBinds
        )
      );

      staticRwBinds = builtins.filter isStaticBind (remapBinds (resolvedExtraRw ++ sandboxDirBinds));

      # Dynamic binds: need runtime variable substitution
      mkDynamicBind =
        mode: bind:
        let
          dest = if bind.dest != null then bind.dest else bind.src;
        in
        {
          inherit (bind) src;
          inherit dest mode;
          try_bind = bind.try;
        };

      dynamicRoBinds = builtins.filter (b: !isStaticBind b) (
        remapBinds (
          resolvedExtraRo
          ++ guiBinds
          ++ dbusBinds
          ++ gitBinds
          ++ portalRoBinds
          ++ shellConfigHostBinds
          ++ shellConfigManagedFileBinds
          ++ noHostConfigBinds
        )
      );

      dynamicRwBinds = builtins.filter (b: !isStaticBind b) (
        remapBinds (resolvedExtraRw ++ sandboxDirBinds ++ portalRwBinds)
      );

      dynamicBindsList =
        map (mkDynamicBind "ro") dynamicRoBinds
        ++ map (mkDynamicBind "rw") dynamicRwBinds
        # Overlapping managed file binds must come after dir binds so the
        # read-only file mounts overlay on top of the writable directory.
        ++ map (mkDynamicBind "ro") managedFileBindsOverlapping;

      storeRootSources = lib.unique (
        map (b: b.src) (staticRoBinds ++ staticRwBinds ++ dynamicBindsList)
        ++ map (cf: cf.src) sCfg.sandbox.copies
      );

      # File operation specs for the compiled binary
      dirMkdirSpecs = mkMkdirSpecsFromAttr sCfg.sandbox.state.dirs;
      fileMkdirSpecs = mkMkdirSpecsFromAttr sCfg.sandbox.state.files;

      copyFileSpecs = map (
        cf:
        let
          normalizedDest = normalizeCopyDest cf.dest;
        in
        {
          inherit (cf) src mode overwrite;
          host_dest = "${copyHostBase}/cloister/${name}/${lib.removePrefix "$HOME/" normalizedDest}";
        }
      ) sCfg.sandbox.copies;

      pipewireSocketName =
        if sCfg.audio.pipewire.enable && !sCfg.audio.pipewire.pulseOnly then
          "cloister/pipewire/${name}"
        else
          null;

      # --- JSON config + package (extracted to package-config.nix) ---
      dbusProxyWrapper = if sCfg.dbus.enable then mkDbusProxyWrapper name sCfg else null;

      jsonResult = import ./package-config.nix {
        inherit
          pkgs
          lib
          bwrapLib
          cloister-sandbox
          name
          sCfg
          config
          shellLib
          anonymize
          sandboxHome
          seccompFilter
          allDirs
          internalTmpfs
          allSymlinks
          managedFileDirsOverlapping
          managedFileDirOverlap
          noHostConfigEnv
          guiEnv
          portalEnv
          computedEnv
          staticRoBinds
          staticRwBinds
          dynamicBindsList
          storeRootSources
          perDirBuckets
          dirMkdirSpecs
          fileMkdirSpecs
          copyFileSpecs
          dbusProxyWrapper
          flatpakAppId
          pipewireSocketName
          renderedWorkerBroker
          buildRevision
          bubblewrap-subset-pid
          osConfig
          ;
      };
    in
    {
      inherit (jsonResult) internal;
      inherit (jsonResult) package;
      inherit (jsonResult) sandboxConfig;
      inherit (jsonResult) imageStoreInfo;
      inherit assertions;
    };

  # Build all sandboxes
  allSandboxes = lib.mapAttrs mkSandbox cfg.sandboxes;

  imageStoreInfos = builtins.filter (info: info != null) (
    map (sb: sb.imageStoreInfo) (builtins.attrValues allSandboxes)
  );

in
{
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = pkgs.stdenv.hostPlatform.isLinux;
        message = "cloister: requires Linux (bubblewrap is not available on macOS).";
      }
    ]
    ++ lib.concatLists (lib.mapAttrsToList (_: sb: sb.assertions) allSandboxes);

    home.packages = lib.mapAttrsToList (_: sb: sb.package) allSandboxes;

    cloister._internal.imageStoreInfos = imageStoreInfos;
    cloister._internal.sandboxConfigs = lib.mapAttrs (_: sb: sb.sandboxConfig) allSandboxes;
    cloister._internal.sandboxInternals = lib.mapAttrs (_: sb: sb.internal) allSandboxes;

    systemd =
      let
        pipewireFilteredSandboxes = lib.filterAttrs (_: sCfg: sCfg.audio.pipewire.enable) cfg.sandboxes;
      in
      {
        user = {
          services = dbusServices;
          timers = { };
          sockets = { };
          tmpfiles.rules =
            lib.optional (dbusEnabledSandboxes != { }) "d %t/cloister/dbus 0700 - -"
            ++ lib.optional (pipewireFilteredSandboxes != { }) "d %t/cloister/pipewire 0700 - -";
        };
      };

    xdg.configFile =
      let
        pipewireEnabledSandboxes = lib.filterAttrs (_: sCfg: sCfg.audio.pipewire.enable) cfg.sandboxes;

        getNodePerms = filters: "rxl" + lib.optionalString filters.control "w";
        getObjectPerms =
          filters: "rx" + lib.optionalString filters.control "w" + lib.optionalString filters.routing "m";

        getMediaClasses =
          filters:
          lib.optional filters.audioOut "Audio/Sink"
          ++ lib.optional filters.audioIn "Audio/Source"
          ++ lib.optional filters.videoIn "Video/Source";

        mkPipewireSocketEntry =
          name: sCfg:
          let
            sandboxHome = "/home/${sCfg.sandbox.anonymize.username}";
            anonymizedIdentity =
              let
                match = builtins.match "^/home/([^/]+)$" sandboxHome;
              in
              if match == null then
                throw ''
                  cloister.sandboxes.${name}: anonymized sandboxHome must be exactly /home/<username>, got ${sandboxHome}
                ''
              else
                builtins.elemAt match 0;
            anonymizedPipewireProps = lib.optionalString sCfg.sandbox.anonymize.enable ''
              props = {
                "context.user-name" = "${anonymizedIdentity}"
                "context.host-name" = "${anonymizedIdentity}"
              }
            '';
          in
          ''
              {
                name = "cloister/pipewire/${name}"
            ${anonymizedPipewireProps}    }
          '';

        pipewireSocketEntries = lib.concatMapStrings (
          name: mkPipewireSocketEntry name pipewireEnabledSandboxes.${name}
        ) (builtins.attrNames pipewireEnabledSandboxes);

        pipewireAccessEntries = lib.concatMapStrings (name: ''
          cloister/pipewire/${name} = "cloister-${name}"
        '') (builtins.attrNames pipewireEnabledSandboxes);

        pipewireConfigs = lib.optionalAttrs (pipewireEnabledSandboxes != { }) {
          "pipewire/pipewire.conf.d/99-cloister.conf" = {
            text = ''
              module.protocol-native.args = {
                sockets = [
                  { name = "pipewire-0" }
                  { name = "pipewire-0-manager" }
              ${pipewireSocketEntries}    ]
              }

              module.access.args = {
                access.socket = {
                  pipewire-0-manager = "unrestricted"
              ${pipewireAccessEntries}    }
              }
            '';
          };
        };

        mkWireplumberConfig =
          name: filters:
          let
            luaScript = mkWireplumberLuaScript name filters;
          in
          {
            name = "wireplumber/wireplumber.conf.d/99-cloister-${name}.conf";
            value = {
              text = ''
                access.rules = [
                  {
                    matches = [
                      {
                        access = "cloister-${name}"
                      }
                    ]
                    actions = {
                      update-props = {
                        default_permissions = "l"
                      }
                    }
                  }
                ]

                wireplumber.components = [
                  {
                    name = ${luaScript}, type = script/lua
                    provides = custom.access-cloister-${name}
                  }
                ]

                wireplumber.profiles = {
                  main = {
                    custom.access-cloister-${name} = required
                  }
                }
              '';
            };
          };

        mkWireplumberLuaScript =
          name: filters:
          let
            nodePerms = getNodePerms filters;
            objectPerms = getObjectPerms filters;
            classes = getMediaClasses filters;
            allowedClassesLua = lib.concatStringsSep ", " (map builtins.toJSON classes);
            allowedFactoriesLua = lib.concatStringsSep ", " (
              map builtins.toJSON [
                "client-node"
                "adapter"
                "link-factory"
              ]
            );
          in
          pkgs.writeText "access-cloister-${name}.lua" (
            ''
              local log = Log.open_topic("s-client")
              local base_permissions = "l"
              local self_permissions = "rx"
              local factory_permissions = "rx"

              local function grant(client, object_id, permissions)
                client:update_permissions { [object_id] = permissions }
              end

              local allowed_media_classes = {}
              for _, media_class in ipairs({ ${allowedClassesLua} }) do
                allowed_media_classes[media_class] = true
              end

              local allowed_factory_names = {}
              for _, factory_name in ipairs({ ${allowedFactoriesLua} }) do
                allowed_factory_names[factory_name] = true
              end

              local node_objects
              local port_objects
              local link_objects

              local function is_allowed_node(node)
                local properties = node.properties
                if properties == nil then
                  return false
                end

                local media_class = properties["media.class"]
                return media_class ~= nil and allowed_media_classes[media_class] == true
              end

              local function node_matches_client(node, client_id)
                local properties = node.properties
                if properties == nil then
                  return false
                end

                local owner_client_id = properties["client.id"]
                return owner_client_id ~= nil and tostring(owner_client_id) == tostring(client_id)
              end

              local function is_visible_node_for_client(node, client_id)
                return is_allowed_node(node) or node_matches_client(node, client_id)
              end

              local function node_id_matches_client(node_id, client_id)
                for node in node_objects:iterate() do
                  if node_matches_client(node, client_id) and tostring(node["bound-id"]) == tostring(node_id) then
                    return true
                  end
                end

                return false
              end

              local function port_matches_client(port, client_id)
                local properties = port.properties
                if properties == nil then
                  return false
                end

                local port_node_id = properties["node.id"]
                if port_node_id == nil then
                  return false
                end

                for node in node_objects:iterate() do
                  if is_visible_node_for_client(node, client_id) and tostring(node["bound-id"]) == tostring(port_node_id) then
                    return true
                  end
                end

                return false
              end

              local function link_matches_client(link, client_id)
                local properties = link.properties
                if properties == nil then
                  return false
                end

                local output_node_id = properties["link.output.node"]
                local input_node_id = properties["link.input.node"]
                if output_node_id == nil or input_node_id == nil then
                  return false
                end

                return node_id_matches_client(output_node_id, client_id)
                  or node_id_matches_client(input_node_id, client_id)
              end

              local function is_allowed_factory(factory)
                local properties = factory.properties
                if properties == nil then
                  return false
                end

                local factory_name = properties["factory.name"]
                return factory_name ~= nil and allowed_factory_names[factory_name] == true
              end

              local function is_cloister_client(client)
                local properties = client.properties
                if properties == nil then
                  return false
                end

                local access = properties["pipewire.access.effective"] or properties["access"]
                return access == "cloister-${name}"
              end

              local cloister_clients = ObjectManager {
                Interest {
                  type = "client"
                }
              }

              node_objects = ObjectManager {
                Interest {
                  type = "node"
                }
              }

              port_objects = ObjectManager {
                Interest {
                  type = "port"
                }
              }

              link_objects = ObjectManager {
                Interest {
                  type = "link"
                }
              }

              local factory_objects = ObjectManager {
                Interest {
                  type = "factory"
                }
              }

            ''
            + lib.optionalString filters.routing ''
              local metadata_objects = ObjectManager {
                Interest {
                  type = "metadata"
                }
              }
            ''
            + ''

              local function sync_client_permissions(client)
                local client_id = client["bound-id"]
                log:info(client, "Syncing cloister-${name} client " .. client_id .. " permissions")
                client:update_permissions({
                  ["all"] = base_permissions,
                })

                local permissions = {
                  [0] = self_permissions,
                  [client_id] = self_permissions,
                }

                for node in node_objects:iterate() do
                  if is_visible_node_for_client(node, client_id) then
                    local node_id = node["bound-id"]
                    if is_allowed_node(node) then
                      permissions[node_id] = "${nodePerms}"
                    else
                      permissions[node_id] = self_permissions
                    end
                  end
                end

                for port in port_objects:iterate() do
                  if port_matches_client(port, client_id) then
                    local port_id = port["bound-id"]
                    permissions[port_id] = "rx"
                  end
                end

                for link in link_objects:iterate() do
                  if link_matches_client(link, client_id) then
                    local link_id = link["bound-id"]
                    permissions[link_id] = "rx"
                  end
                end

                for factory in factory_objects:iterate() do
                  if is_allowed_factory(factory) then
                    local factory_id = factory["bound-id"]
                    permissions[factory_id] = factory_permissions
                  end
                end

            ''
            + lib.optionalString filters.routing ''
              for metadata in metadata_objects:iterate() do
                local metadata_id = metadata["bound-id"]
                permissions[metadata_id] = "${objectPerms}"
              end
            ''
            + ''
                client:update_permissions(permissions)
              end

              node_objects:connect("object-added", function(om, node)
                local node_id = node["bound-id"]
                for client in cloister_clients:iterate() do
                  if is_cloister_client(client) then
                    local client_id = client["bound-id"]
                    if is_visible_node_for_client(node, client_id) then
                      local permissions = is_allowed_node(node) and "${nodePerms}" or self_permissions
                      log:info(client, "Granting '" .. permissions .. "' for node " .. node_id .. " to cloister-${name} client")
                      grant(client, node_id, permissions)
                    end
                  end
                end
              end)

              port_objects:connect("object-added", function(om, port)
                local port_id = port["bound-id"]
                for client in cloister_clients:iterate() do
                  if is_cloister_client(client) then
                    local client_id = client["bound-id"]
                    if port_matches_client(port, client_id) then
                      log:info(client, "Granting 'rx' for port " .. port_id .. " to cloister-${name} client")
                      grant(client, port_id, "rx")
                    end
                  end
                end
              end)

              link_objects:connect("object-added", function(om, link)
                local link_id = link["bound-id"]
                for client in cloister_clients:iterate() do
                  if is_cloister_client(client) then
                    local client_id = client["bound-id"]
                    if link_matches_client(link, client_id) then
                      log:info(client, "Granting 'rx' for link " .. link_id .. " to cloister-${name} client")
                      grant(client, link_id, "rx")
                    end
                  end
                end
              end)

              factory_objects:connect("object-added", function(om, factory)
                if is_allowed_factory(factory) then
                  local factory_id = factory["bound-id"]
                  for client in cloister_clients:iterate() do
                    if is_cloister_client(client) then
                      log:info(client, "Granting '" .. factory_permissions .. "' for factory " .. factory_id .. " to cloister-${name} client")
                      grant(client, factory_id, factory_permissions)
                    end
                  end
                end
              end)

            ''
            + lib.optionalString filters.routing ''
              metadata_objects:connect("object-added", function(om, metadata)
                local metadata_id = metadata["bound-id"]
                for client in cloister_clients:iterate() do
                  if is_cloister_client(client) then
                    log:info(client, "Granting '${objectPerms}' for metadata " .. metadata_id .. " to cloister-${name} client")
                    grant(client, metadata_id, "${objectPerms}")
                  end
                end
              end)
            ''
            + ''

              cloister_clients:connect("object-added", function(om, client)
                if is_cloister_client(client) then
                  sync_client_permissions(client)
                end
              end)

              node_objects:activate()
              port_objects:activate()
              link_objects:activate()
              factory_objects:activate()
            ''
            + lib.optionalString filters.routing ''
              metadata_objects:activate()
            ''
            + ''
              cloister_clients:activate()
            ''
          );
        wireplumberConfigs = lib.mapAttrs' (
          name: sCfg: mkWireplumberConfig name sCfg.audio.pipewire.filters
        ) pipewireEnabledSandboxes;
      in
      pipewireConfigs // wireplumberConfigs;

    # Desktop entries for sandboxes with gui.desktopEntry set
    xdg.desktopEntries =
      let
        desktopSandboxes = lib.filterAttrs (_: sCfg: sCfg.gui.desktopEntry != null) cfg.sandboxes;
      in
      lib.mapAttrs' (
        name: sCfg:
        let
          de = sCfg.gui.desktopEntry;
          pkg = allSandboxes.${name}.package;
          entryName = if de.name != "" then de.name else "cl-${name}";
          entryExec = "${pkg}/bin/cl-${name}" + lib.optionalString (de.execArgs != "") " ${de.execArgs}";
        in
        {
          name = "cl-${name}";
          value = {
            name = entryName;
            exec = entryExec;
            inherit (de) terminal;
            type = "Application";
          }
          // lib.optionalAttrs (de.icon != "") { inherit (de) icon; }
          // lib.optionalAttrs (de.categories != [ ]) {
            inherit (de) categories;
          }
          // lib.optionalAttrs (de.mimeTypes != [ ]) { mimeType = de.mimeTypes; }
          // lib.optionalAttrs (de.genericName != "") {
            inherit (de) genericName;
          }
          // lib.optionalAttrs (de.comment != "") { inherit (de) comment; }
          // lib.optionalAttrs de.startupNotify { startupNotify = true; };
        }
      ) desktopSandboxes;

    # Defaults are set inside the submodule config in _options.nix
  };
}
