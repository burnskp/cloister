# Namespace sandbox using bubblewrap
# Provides bare sandbox plumbing — personal tool preferences live in the consumer module
{
  config,
  pkgs,
  lib,
  ...
}:

let
  cfg = config.cloister;
  bwrapLib = import ./_bwrap.nix { inherit lib; };
  shells = import ./_mkShells.nix { inherit pkgs lib; };
  dangerous = import ./_dangerous.nix { inherit lib; };
  resolve = import ./_resolve.nix {
    inherit lib config;
    inherit (config.xdg) configHome;
  };

  cloister-seccomp-filter = pkgs.callPackage ../../helpers/cloister-seccomp-filter { };
  cloister-sandbox = pkgs.callPackage ../../helpers/cloister-sandbox { };

  inherit (config.xdg) configHome;
  inherit (dangerous)
    normalizeDangerousPath
    normalizedDangerousPaths
    pathsOverlap
    isDangerousPath
    ;
  inherit (resolve) resolveConfigEntry;

  # 8-char SHA256 hash of a sandbox's PipeWire filter config, used to
  # deduplicate sockets and WirePlumber policies across sandboxes.
  pipewireFilterHash =
    sCfg:
    builtins.substring 0 8 (builtins.hashString "sha256" (builtins.toJSON sCfg.audio.pipewire.filters));

  # --- D-Bus proxy unit rendering (per-sandbox, socket-activated) ---

  dbusPolicyFlags =
    sCfg:
    let
      talkFlags = map (name: "--talk=${name}") sCfg.dbus.policies.talk;
      ownFlags = map (name: "--own=${name}") sCfg.dbus.policies.own;
      seeFlags = map (name: "--see=${name}") sCfg.dbus.policies.see;
      callFlags = lib.concatLists (
        lib.mapAttrsToList (name: rules: map (rule: "--call=${name}=${rule}") rules) sCfg.dbus.policies.call
      );
      broadcastFlags = lib.concatLists (
        lib.mapAttrsToList (
          name: rules: map (rule: "--broadcast=${name}=${rule}") rules
        ) sCfg.dbus.policies.broadcast
      );
    in
    talkFlags ++ ownFlags ++ seeFlags ++ callFlags ++ broadcastFlags;

  dbusEnabledSandboxes = lib.filterAttrs (_: sCfg: sCfg.dbus.enable) cfg.sandboxes;

  mkDbusService =
    name: sCfg:
    let
      policyFlags = dbusPolicyFlags sCfg;
      execArgs = [
        "${pkgs.xdg-dbus-proxy}/bin/xdg-dbus-proxy"
        "unix:path=%t/bus"
        "%t/dbus-proxy-${name}"
        "--filter"
      ]
      ++ lib.optional sCfg.dbus.log "--log"
      ++ policyFlags;
    in
    {
      Unit = {
        Description = "Cloister D-Bus proxy (${name})";
        StartLimitBurst = 5;
        StartLimitIntervalSec = 30;
      };
      Service = {
        ExecStart = lib.escapeShellArgs execArgs;
        Restart = "on-failure";
        RestartSec = 1;
        MemoryHigh = "64M";
        MemoryMax = "128M";
      };
    };

  mkDbusSocket = name: {
    Unit = {
      Description = "Cloister D-Bus proxy socket (${name})";
    };
    Socket = {
      ListenStream = "%t/dbus-proxy-${name}";
    };
    Install = {
      WantedBy = [ "sockets.target" ];
    };
  };

  dbusServices = lib.mapAttrs' (name: sCfg: {
    name = "cloister-dbus-proxy-${name}";
    value = mkDbusService name sCfg;
  }) dbusEnabledSandboxes;

  dbusSockets = lib.mapAttrs' (name: _: {
    name = "cloister-dbus-proxy-${name}";
    value = mkDbusSocket name;
  }) dbusEnabledSandboxes;

  # --- Per-sandbox builder ---

  mkSandbox =
    name: sCfg:
    let
      resolveHome = path: builtins.replaceStrings [ "$HOME" ] [ config.home.homeDirectory ] path;
      shellLib = shells.${sCfg.shell.name};
      guiEnabled = sCfg.gui.wayland.enable || sCfg.gui.x11.enable;
      gpuEnabled = sCfg.gui.gpu.enable;
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

      syntheticFlatpakInfo = pkgs.writeText "cloister-${name}-flatpak-info" ''
        [Application]
        name=dev.cloister.${name}
      '';

      portalRoBinds = lib.optionals sCfg.dbus.portal [
        {
          src = "${syntheticFlatpakInfo}";
          dest = "/.flatpak-info";
          try = false;
        }
      ];

      portalRwBinds = lib.optionals sCfg.dbus.portal [
        {
          src = "$XDG_RUNTIME_DIR/doc";
          dest = "/run/flatpak/doc";
          try = true;
        }
      ];

      # /proc entries to mask with /dev/null
      procMaskPaths = [
        "/proc/version"
        "/proc/cmdline"
        "/proc/uptime"
        "/proc/loadavg"
        "/proc/stat"
        "/proc/diskstats"
        "/proc/vmstat"
        "/proc/sys/kernel/osrelease"
        "/proc/sys/kernel/version"
        "/proc/sys/kernel/random/boot_id"
        "/proc/self/mountinfo"
        "/proc/self/mounts"
      ];

      procMaskBinds = lib.optionals anonymize (
        map (p: {
          src = "/dev/null";
          dest = p;
          try = true;
        }) procMaskPaths
      );

      seccompFilter = lib.optionalString sCfg.sandbox.seccomp.enable (
        pkgs.runCommand
          (
            "cloister-seccomp-${name}"
            + lib.optionalString sCfg.sandbox.seccomp.allowChromiumSandbox "-chromium"
          )
          { }
          ''
            ${cloister-seccomp-filter}/bin/cloister-seccomp-filter \
              --output "$out" \
              ${lib.optionalString sCfg.sandbox.seccomp.allowChromiumSandbox "--allow-chromium-sandbox"}
          ''
      );

      allPackages = sCfg.packages ++ sCfg.extraPackages;

      computedEnv = {
        PATH = lib.makeBinPath allPackages;
      };

      # --- Resolution: convert semantic extraBinds → [{src, dest, try}] ---

      mkHomeBinds =
        try: paths:
        map (p: {
          src = "$HOME/${p}";
          dest = null;
          inherit try;
        }) paths;

      requiredRo = mkHomeBinds false sCfg.sandbox.extraBinds.required.ro;
      optionalRo = mkHomeBinds true sCfg.sandbox.extraBinds.optional.ro;
      requiredRw = mkHomeBinds false sCfg.sandbox.extraBinds.required.rw;
      optionalRw = mkHomeBinds true sCfg.sandbox.extraBinds.optional.rw;

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

      dirBinds = mkBindsFromAttr sCfg.sandbox.extraBinds.dir;
      fileBinds = mkBindsFromAttr sCfg.sandbox.extraBinds.file;

      perDirBinds = map (p: {
        src = "${sCfg.sandbox.perDirBase}/$DIR_HASH/${p}";
        dest = "$HOME/${p}";
        try = false;
      }) sCfg.sandbox.extraBinds.perDir;

      normalizeCopyDest =
        path:
        let
          normalized = normalizeDangerousPath path;
          homeDir = config.home.homeDirectory;
        in
        if lib.hasPrefix "$HOME/" normalized then
          normalized
        else if lib.hasPrefix "${homeDir}/" normalized then
          "$HOME/${lib.removePrefix "${homeDir}/" normalized}"
        else
          normalized;

      copyFileBinds = map (
        cf:
        let
          normalizedDest = normalizeCopyDest cf.dest;
        in
        {
          src = "${sCfg.sandbox.copyFileBase}/cloister/${name}/${lib.removePrefix "$HOME/" normalizedDest}";
          dest = normalizedDest;
          try = false;
        }
      ) sCfg.sandbox.copyFiles;

      resolvedExtraRo = requiredRo ++ optionalRo;
      resolvedExtraRw = requiredRw ++ optionalRw ++ dirBinds ++ fileBinds ++ perDirBinds ++ copyFileBinds;

      managedFileBinds = lib.concatMap resolveConfigEntry sCfg.sandbox.extraBinds.managedFile;
      managedFileDirs = lib.unique (map (bind: builtins.dirOf bind.dest) managedFileBinds);

      # Partition managedFileBinds: binds whose dest falls inside a dir-backed
      # bind mount must be applied AFTER the dir bind in bwrap, otherwise the
      # directory mount shadows the individual file mounts.
      managedFileOverlapsDir =
        bind:
        builtins.any (
          dirBind:
          let
            dirDest = resolveHome (if dirBind.dest != null then dirBind.dest else dirBind.src);
          in
          bind.dest == dirDest || lib.hasPrefix "${dirDest}/" bind.dest
        ) dirBinds;

      managedFileBindsNonOverlapping = builtins.filter (b: !managedFileOverlapsDir b) managedFileBinds;
      managedFileBindsOverlapping = builtins.filter managedFileOverlapsDir managedFileBinds;

      # Partition managedFileDirs: dirs inside a dir-backed bind mount need
      # host-side mkdir -p instead of bwrap --dir (which gets shadowed by the bind).
      managedFileDirOverlap =
        dir:
        let
          # dirBinds use "$HOME/..." while managedFileDirs use concrete homeDirectory;
          # resolve $HOME so comparison works.
          matchingBinds = builtins.filter (
            bind:
            let
              dest = resolveHome (if bind.dest != null then bind.dest else bind.src);
            in
            dir == dest || lib.hasPrefix "${dest}/" dir
          ) dirBinds;
        in
        if matchingBinds == [ ] then
          null
        else
          let
            bind = builtins.head matchingBinds;
            dest = resolveHome (if bind.dest != null then bind.dest else bind.src);
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
          src = "$HOME/.config/git";
          dest = null;
          try = true;
        }
        {
          src = "$HOME/.gitconfig";
          dest = null;
          try = true;
        }
      ];

      guiBinds = [ ];

      shellConfigBinds = lib.optionals sCfg.shell.hostConfig (
        map (b: {
          inherit (b) src;
          dest = b.dest or null;
          try = b.try or false;
        }) shellLib.configBinds
      );

      # --- Minimal shell init for hostConfig = false ---
      cloisterInitContent = ''
        ${sCfg.init.rendered}
        ${sCfg.registry.rendered.inside}
      '';

      # zsh: minimal ZDOTDIR with .zshrc (added via env, nix store always bound)
      cloisterZdotdir = pkgs.writeTextDir ".zshrc" cloisterInitContent;

      # bash: minimal .bash_profile (bound dynamically at $HOME/.bash_profile)
      cloisterBashProfile = pkgs.writeText "cloister-bash-profile-${name}" cloisterInitContent;

      noHostConfigBinds = lib.optionals (!sCfg.shell.hostConfig) (
        if sCfg.shell.name == "bash" then
          [
            {
              src = "${cloisterBashProfile}";
              dest = "$HOME/.bash_profile";
              try = false;
            }
          ]
        else
          [ ]
      );

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

      portalEnv = lib.optionalAttrs sCfg.dbus.portal { GTK_USE_PORTAL = "1"; };

      guiDataPackages = sCfg.gui.dataPackages ++ lib.optionals sCfg.gui.gtk.enable sCfg.gui.gtk.packages;

      qtPluginPaths = lib.concatMap (pkg: [
        "${pkg}/lib/qt-6/plugins"
        "${pkg}/lib/qt-5/plugins"
      ]) sCfg.gui.qt.packages;

      guiEnv = lib.optionalAttrs guiEnabled (
        {
          NO_AT_BRIDGE = "1";
        }
        // lib.optionalAttrs sCfg.gui.wayland.enable { NIXOS_OZONE_WL = "1"; }
        // lib.optionalAttrs sCfg.gui.gtk.enable {
          GTK_THEME = sCfg.gui.gtk.theme;
          GIO_EXTRA_MODULES = "${pkgs.dconf.lib}/lib/gio/modules";
          GSETTINGS_SCHEMA_DIR = lib.concatStringsSep ":" [
            "${pkgs.glib}/share/glib-2.0/schemas"
            "${pkgs.gsettings-desktop-schemas}/share/glib-2.0/schemas"
            "${pkgs.gtk3}/share/glib-2.0/schemas"
          ];
          GDK_PIXBUF_MODULE_FILE = "${pkgs.librsvg}/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache";
        }
        // lib.optionalAttrs sCfg.gui.qt.enable {
          QT_QPA_PLATFORMTHEME = sCfg.gui.qt.platformTheme;
        }
        // lib.optionalAttrs (sCfg.gui.qt.enable && sCfg.gui.qt.style != null) {
          QT_STYLE_OVERRIDE = sCfg.gui.qt.style;
        }
        // lib.optionalAttrs (sCfg.gui.qt.enable && qtPluginPaths != [ ]) {
          QT_PLUGIN_PATH = lib.concatStringsSep ":" qtPluginPaths;
        }
        // lib.optionalAttrs (guiDataPackages != [ ]) {
          XDG_DATA_DIRS = lib.concatStringsSep ":" (map (pkg: "${pkg}/share") guiDataPackages);
        }
        // lib.optionalAttrs (sCfg.gui.scaleFactor != null) (
          let
            gdkScale = builtins.ceil sCfg.gui.scaleFactor;
            gdkDpiScale = sCfg.gui.scaleFactor / gdkScale;
          in
          {
            GDK_SCALE = toString gdkScale;
            GDK_DPI_SCALE = toString gdkDpiScale;
            QT_SCALE_FACTOR = toString sCfg.gui.scaleFactor;
          }
        )
        // lib.optionalAttrs (sCfg.gui.fonts.packages != [ ]) {
          FONTCONFIG_FILE = pkgs.makeFontsConf {
            fontDirectories = sCfg.gui.fonts.packages;
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
          sCfg.sandbox.binds.ro
          ++ resolvedExtraRo
          ++ managedFileBinds
          ++ dbusBinds
          ++ gitBinds
          ++ guiBinds
          ++ shellConfigBinds
          ++ noHostConfigBinds
          ++ sCfg.sandbox.binds.rw
          ++ resolvedExtraRw
          ++ sandboxDirBinds
          ++ portalRwBinds
        )
        ++ procMaskBinds
        ++ portalRoBinds
      );

      findDuplicates =
        xs: lib.attrNames (lib.filterAttrs (_: v: builtins.length v > 1) (builtins.groupBy (x: x) xs));

      duplicateDests = findDuplicates allDests;

      # --- Overlapping dirs and tmpfs detection ---
      allDirs = sCfg.sandbox.dirs ++ sCfg.sandbox.extraDirs ++ managedFileDirsNonOverlapping;
      dirTmpfsOverlap = lib.intersectLists allDirs sCfg.sandbox.tmpfs;

      # --- Duplicate symlink link paths ---
      allSymlinks = sCfg.sandbox.symlinks ++ sCfg.sandbox.extraSymlinks;
      allSymlinkLinks = map (s: s.link) allSymlinks;
      duplicateLinks = findDuplicates allSymlinkLinks;

      # --- Duplicate managedFile entries ---
      duplicateManagedFiles = findDuplicates sCfg.sandbox.extraBinds.managedFile;

      # --- Unsafe character assertion for user-provided paths ---
      userPaths =
        let
          bindPaths = lib.concatMap (bind: [
            bind.src
            (if bind.dest != null then bind.dest else bind.src)
          ]) (sCfg.sandbox.binds.ro ++ sCfg.sandbox.binds.rw);
          dirPaths = sCfg.sandbox.dirs ++ sCfg.sandbox.extraDirs;
          tmpfsPaths = sCfg.sandbox.tmpfs;
          symlinkPaths = lib.concatMap (s: [
            s.target
            s.link
          ]) (sCfg.sandbox.symlinks ++ sCfg.sandbox.extraSymlinks);
          extraBindPaths =
            sCfg.sandbox.extraBinds.required.ro
            ++ sCfg.sandbox.extraBinds.required.rw
            ++ sCfg.sandbox.extraBinds.optional.ro
            ++ sCfg.sandbox.extraBinds.optional.rw
            ++ lib.concatLists (lib.attrValues sCfg.sandbox.extraBinds.dir)
            ++ lib.concatLists (lib.attrValues sCfg.sandbox.extraBinds.file)
            ++ sCfg.sandbox.extraBinds.perDir
            ++ sCfg.sandbox.extraBinds.managedFile;
          copyFileSrcPaths = map (cf: cf.src) sCfg.sandbox.copyFiles;
        in
        bindPaths
        ++ dirPaths
        ++ tmpfsPaths
        ++ symlinkPaths
        ++ extraBindPaths
        ++ copyFileSrcPaths
        ++ lib.attrNames sCfg.sandbox.env
        ++ sCfg.sandbox.devBinds
        ++ sCfg.sandbox.disallowedPaths
        ++ [
          sCfg.sandbox.perDirBase
          sCfg.sandbox.copyFileBase
        ];

      unsafePaths = builtins.filter (
        p: lib.hasInfix "$" p || lib.hasInfix "\n" p || lib.hasInfix "\r" p
      ) userPaths;

      # --- Dangerous path detection ---
      toDangerousPath =
        path:
        let
          homeDir = config.home.homeDirectory;
          normalized = normalizeDangerousPath path;
          genericHomeMatch = builtins.match "^/(home|var/home)/[^/]+/(.+)$" normalized;
          rootHomeMatch = builtins.match "^/root/(.+)$" normalized;
        in
        if normalized == "" || normalized == "$HOME" || normalized == homeDir then
          ""
        else if lib.hasPrefix "$HOME/" normalized then
          lib.removePrefix "$HOME/" normalized
        else if lib.hasPrefix "${homeDir}/" normalized then
          lib.removePrefix "${homeDir}/" normalized
        else if genericHomeMatch != null then
          builtins.elemAt genericHomeMatch 1
        else if rootHomeMatch != null then
          builtins.elemAt rootHomeMatch 0
        else
          normalized;

      rawBindPaths = lib.concatMap (
        bind:
        map toDangerousPath (
          lib.filter (p: p != null) [
            bind.src
            bind.dest
          ]
        )
      ) (sCfg.sandbox.binds.ro ++ sCfg.sandbox.binds.rw);

      managedFileBindPaths = map (bind: toDangerousPath bind.dest) managedFileBinds;

      allDangerousBindPaths =
        sCfg.sandbox.extraBinds.required.ro
        ++ sCfg.sandbox.extraBinds.required.rw
        ++ sCfg.sandbox.extraBinds.optional.ro
        ++ sCfg.sandbox.extraBinds.optional.rw
        ++ lib.concatLists (lib.attrValues sCfg.sandbox.extraBinds.dir)
        ++ lib.concatLists (lib.attrValues sCfg.sandbox.extraBinds.file)
        ++ sCfg.sandbox.extraBinds.perDir
        ++ managedFileBindPaths
        ++ rawBindPaths;

      normalizedAllowDangerousPaths = builtins.filter (p: p != "") (
        map normalizeDangerousPath sCfg.sandbox.allowDangerousPaths
      );

      matchedDangerousPaths = lib.unique (
        builtins.filter (
          path:
          let
            normalizedPath = normalizeDangerousPath path;
            isAllowedPath = builtins.any (
              allowed: pathsOverlap normalizedPath allowed
            ) normalizedAllowDangerousPaths;
          in
          normalizedPath != "" && isDangerousPath normalizedPath && !isAllowedPath
        ) allDangerousBindPaths
      );

      # --- Computed env var override detection ---
      computedEnvKeys = lib.attrNames computedEnv;
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
          duplicateManagedFiles
          unsafePaths
          matchedDangerousPaths
          overriddenEnvKeys
          overriddenDbusKeys
          overriddenGuiKeys
          overriddenPortalKeys
          blockedPassthroughEnv
          invalidPassthroughEnv
          guiEnabled
          normalizeCopyDest
          ;
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
          sCfg.sandbox.binds.ro
          ++ managedFileBindsNonOverlapping
          ++ guiBinds
          ++ shellConfigBinds
          ++ resolvedExtraRo
          ++ dbusBinds
          ++ gitBinds
          ++ noHostConfigBinds
        )
        ++ procMaskBinds
        ++ portalRoBinds
      );

      staticRwBinds = builtins.filter isStaticBind (
        remapBinds (sCfg.sandbox.binds.rw ++ resolvedExtraRw ++ sandboxDirBinds)
      );

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
        remapBinds (resolvedExtraRo ++ dbusBinds ++ gitBinds ++ shellConfigBinds ++ noHostConfigBinds)
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

      # Feature-generated bind sources are excluded from dangerous path
      # validation: they are intentional binds added by the module, not
      # user-provided paths. This prevents e.g. git.enable's .config/git
      # bind from tripping the .config/git/credentials overlap check while
      # still catching user-supplied extraBinds that touch the same tree.
      featureBindSrcs = map (b: b.src) (gitBinds ++ dbusBinds);
      bindSources = lib.unique (
        builtins.filter (s: !builtins.elem s featureBindSrcs) (
          map (b: b.src) (staticRoBinds ++ staticRwBinds ++ dynamicBindsList)
        )
      );

      # File operation specs for the compiled binary
      dirMkdirSpecs = mkMkdirSpecsFromAttr sCfg.sandbox.extraBinds.dir;
      fileMkdirSpecs = mkMkdirSpecsFromAttr sCfg.sandbox.extraBinds.file;

      copyFileSpecs = map (
        cf:
        let
          normalizedDest = normalizeCopyDest cf.dest;
        in
        {
          inherit (cf) src mode overwrite;
          host_dest = "${sCfg.sandbox.copyFileBase}/cloister/${name}/${lib.removePrefix "$HOME/" normalizedDest}";
        }
      ) sCfg.sandbox.copyFiles;

      pipewireSocketName =
        if sCfg.audio.pipewire.enable then
          if sCfg.audio.pipewire.filters.enable then
            "pipewire-cloister-${pipewireFilterHash sCfg}"
          else
            "pipewire-0"
        else
          null;

      # --- JSON config + package (extracted to _json.nix) ---
      jsonResult = import ./_json.nix {
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
          gpuEnabled
          seccompFilter
          allDirs
          managedFileDirsOverlapping
          managedFileDirOverlap
          noHostConfigEnv
          guiEnv
          portalEnv
          computedEnv
          staticRoBinds
          staticRwBinds
          dynamicBindsList
          bindSources
          dirMkdirSpecs
          fileMkdirSpecs
          copyFileSpecs
          normalizedDangerousPaths
          normalizedAllowDangerousPaths
          pipewireSocketName
          ;
      };
    in
    {
      inherit (jsonResult) package;
      inherit assertions;
    };

  # Build all sandboxes
  allSandboxes = lib.mapAttrs mkSandbox cfg.sandboxes;

in
{
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = pkgs.stdenv.isLinux;
        message = "cloister: requires Linux (bubblewrap is not available on macOS).";
      }
    ]
    ++ lib.concatLists (lib.mapAttrsToList (_: sb: sb.assertions) allSandboxes);

    home.packages = lib.mapAttrsToList (_: sb: sb.package) allSandboxes;

    systemd.user.services = dbusServices;
    systemd.user.sockets = dbusSockets;

    xdg.configFile =
      let
        pipewireEnabledSandboxes = lib.filterAttrs (
          _: sCfg: sCfg.audio.pipewire.enable && sCfg.audio.pipewire.filters.enable
        ) cfg.sandboxes;

        uniqueFilters = builtins.listToAttrs (
          lib.mapAttrsToList (_: sCfg: {
            name = pipewireFilterHash sCfg;
            value = sCfg.audio.pipewire.filters;
          }) pipewireEnabledSandboxes
        );

        getPerms =
          filters: "rx" + lib.optionalString filters.control "w" + lib.optionalString filters.routing "m";

        getMediaClasses =
          filters:
          lib.optional filters.audioOut "Audio/Sink"
          ++ lib.optional filters.audioIn "Audio/Source"
          ++ lib.optional filters.videoIn "Video/Source";

        pipewireSocketEntries = lib.concatMapStrings (hash: ''
          { name = "pipewire-cloister-${hash}" }
        '') (builtins.attrNames uniqueFilters);

        pipewireAccessEntries = lib.concatMapStrings (hash: ''
          pipewire-cloister-${hash} = "cloister-${hash}"
        '') (builtins.attrNames uniqueFilters);

        pipewireConfigs = lib.optionalAttrs (uniqueFilters != { }) {
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
          hash: filters:
          let
            luaScript = mkWireplumberLuaScript hash filters;
          in
          {
            name = "wireplumber/wireplumber.conf.d/99-cloister-${hash}.conf";
            value = {
              text = ''
                access.rules = [
                  {
                    matches = [
                      {
                        access = "cloister-${hash}"
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
                    provides = custom.access-cloister-${hash}
                  }
                ]

                wireplumber.profiles = {
                  main = {
                    custom.access-cloister-${hash} = required
                  }
                }
              '';
            };
          };

        mkWireplumberLuaScript =
          hash: filters:
          let
            perms = getPerms filters;
            classes = getMediaClasses filters;
            allowedClassesLua = lib.concatStringsSep ", " (map builtins.toJSON classes);
          in
          pkgs.writeText "access-cloister-${hash}.lua" (
            ''
              local log = Log.open_topic("s-client")
              local base_permissions = "l"
              local self_permissions = "rx"

              local function grant(client, object_id, permissions)
                client:update_permissions { [object_id] = permissions }
              end

              local allowed_media_classes = {}
              for _, media_class in ipairs({ ${allowedClassesLua} }) do
                allowed_media_classes[media_class] = true
              end

              local function is_allowed_node(node)
                local properties = node.properties
                if properties == nil then
                  return false
                end

                local media_class = properties["media.class"]
                return media_class ~= nil and allowed_media_classes[media_class] == true
              end

              local function is_cloister_client(client)
                local properties = client.properties
                if properties == nil then
                  return false
                end

                local access = properties["pipewire.access.effective"] or properties["access"]
                return access == "cloister-${hash}"
              end

              local cloister_clients = ObjectManager {
                Interest {
                  type = "client"
                }
              }

              local node_objects = ObjectManager {
                Interest {
                  type = "node"
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
                log:info(client, "Syncing cloister-${hash} client " .. client_id .. " permissions")
                client:update_permissions({
                  ["all"] = base_permissions,
                })

                local permissions = {
                  [0] = self_permissions,
                  [client_id] = self_permissions,
                }

                for node in node_objects:iterate() do
                  if is_allowed_node(node) then
                    local node_id = node["bound-id"]
                    permissions[node_id] = "${perms}"
                  end
                end

            ''
            + lib.optionalString filters.routing ''
              for metadata in metadata_objects:iterate() do
                local metadata_id = metadata["bound-id"]
                permissions[metadata_id] = "${perms}"
              end
            ''
            + ''
                client:update_permissions(permissions)
              end

              node_objects:connect("object-added", function(om, node)
                if is_allowed_node(node) then
                  local node_id = node["bound-id"]
                  for client in cloister_clients:iterate() do
                    if is_cloister_client(client) then
                      log:info(client, "Granting '${perms}' for node " .. node_id .. " to cloister-${hash} client")
                      grant(client, node_id, "${perms}")
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
                    log:info(client, "Granting '${perms}' for metadata " .. metadata_id .. " to cloister-${hash} client")
                    grant(client, metadata_id, "${perms}")
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
            ''
            + lib.optionalString filters.routing ''
              metadata_objects:activate()
            ''
            + ''
              cloister_clients:activate()
            ''
          );
        wireplumberConfigs = lib.mapAttrs' mkWireplumberConfig uniqueFilters;
      in
      pipewireConfigs // wireplumberConfigs;

    # Desktop entries for sandboxes with gui.desktopEntry.enable = true
    xdg.desktopEntries =
      let
        desktopSandboxes = lib.filterAttrs (_: sCfg: sCfg.gui.desktopEntry.enable) cfg.sandboxes;
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
          // lib.optionalAttrs (de.mimeType != [ ]) { inherit (de) mimeType; }
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
