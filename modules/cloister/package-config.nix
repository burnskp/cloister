# JSON config generation and compiled binary package builder.
{
  pkgs,
  lib,
  bwrapLib,
  cloister-sandbox,
  name,
  sCfg,
  config,
  shellLib,
  anonymize,
  sandboxHome,
  seccompFilter,
  allDirs,
  internalTmpfs,
  allSymlinks,
  managedFileDirsOverlapping,
  managedFileDirOverlap,
  noHostConfigEnv,
  guiEnv,
  portalEnv,
  computedEnv,
  staticRoBinds,
  staticRwBinds,
  dynamicBindsList,
  storeRootSources,
  perDirBuckets,
  dirMkdirSpecs,
  fileMkdirSpecs,
  copyFileSpecs,
  dbusProxyWrapper,
  flatpakAppId,
  pipewireSocketName,
  renderedWorkerBroker,
  buildRevision,
  bubblewrap-subset-pid,
  osConfig ? null,
}:
let
  bubblewrapPackage = if anonymize then bubblewrap-subset-pid else pkgs.bubblewrap;

  storeRootFromEntry =
    entry:
    let
      entryString = toString entry;
      match = builtins.match "^(/nix/store/[^/:]+)(/.*)?$" entryString;
    in
    if match == null then
      null
    else
      builtins.appendContext (builtins.elemAt match 0) (builtins.getContext entryString);

  storeRootsFromValue =
    value: builtins.filter (p: p != null) (map storeRootFromEntry (lib.splitString ":" value));

  imageStoreMode = sCfg.sandbox.nixStore.mode == "image-store";

  portalDesktopEnabled = builtins.any (x: x) [
    sCfg.dbus.portal.fileChooser
    sCfg.dbus.portal.openUri
    sCfg.dbus.portal.screencast
    sCfg.dbus.portal.camera
  ];

  imageStoreConfig =
    if osConfig != null && osConfig ? cloister && osConfig.cloister ? imageStore then
      osConfig.cloister.imageStore
    else
      null;

  hostGraphicsConfig =
    if osConfig == null then
      null
    else if lib.hasAttrByPath [ "hardware" "graphics" ] osConfig then
      lib.getAttrFromPath [ "hardware" "graphics" ] osConfig
    else if lib.hasAttrByPath [ "hardware" "opengl" ] osConfig then
      lib.getAttrFromPath [ "hardware" "opengl" ] osConfig
    else
      null;

  hostGpuStoreInputs =
    if hostGraphicsConfig == null then
      [ ]
    else
      lib.unique (
        lib.filter (pkg: pkg != null) (
          lib.optional (hostGraphicsConfig ? package) hostGraphicsConfig.package
          ++ lib.optional (hostGraphicsConfig ? package32) hostGraphicsConfig.package32
          ++ (hostGraphicsConfig.extraPackages or [ ])
          ++ (hostGraphicsConfig.extraPackages32 or [ ])
        )
      );

  _imageStoreConfigCheck =
    if imageStoreMode && imageStoreConfig == null then
      throw ''
        cloister.sandboxes.${name}: sandbox.nixStore.mode = "image-store" requires the NixOS
        cloister-image-store module so osConfig.cloister.imageStore is available.
      ''
    else
      null;

  storeRoots = lib.unique (lib.sort builtins.lessThan (map toString storeInputs));

  storeRootsText = lib.concatMapStringsSep " " lib.escapeShellArg storeRoots;

  storeId = builtins.hashString "sha256" (
    builtins.toJSON {
      version = 1;
      inherit (sCfg.sandbox.nixStore) mode;
      roots = storeRoots;
    }
  );

  storeImagePublishedPath =
    if imageStoreConfig == null then null else "${imageStoreConfig.base}/${storeId}.squashfs";

  storeMetaPublishedPath =
    if imageStoreConfig == null then null else "${imageStoreConfig.base}/${storeId}.json";

  storeMountPath =
    if imageStoreConfig == null then null else "${imageStoreConfig.mountBase}/${storeId}";

  storeMetaJson = builtins.toJSON {
    version = 1;
    mode = "image-store";
    inherit storeId storeRoots;
  };

  storeMeta = pkgs.writeText "cloister-image-store-${storeId}.json" "${storeMetaJson}\n";

  closureInfo = pkgs.closureInfo { rootPaths = storeRoots; };

  squashfsCompressionArgs =
    if imageStoreConfig != null && (imageStoreConfig.compression.enable or true) then
      "-comp zstd -Xcompression-level 10 -b 1M"
    else
      "-no-compression";

  storeImage =
    pkgs.runCommand "cloister-image-store-${storeId}.squashfs"
      {
        nativeBuildInputs = [ pkgs.squashfsTools ];
      }
      ''
        set -eu

        root="$TMPDIR/root"
        mkdir -p "$root/nix/store"

        for path in ${storeRootsText}; do
          if [ ! -e "$path" ]; then
            echo "missing declared store root: $path" >&2
            exit 1
          fi
        done

        LC_ALL=C sort "${closureInfo}/store-paths" | while IFS= read -r path; do
          cp -a "$path" "$root/nix/store/$(basename "$path")"
        done
        cp ${storeMeta} "$root/meta.json"

        mksquashfs "$root" "$out" -noappend -all-root ${squashfsCompressionArgs} >/dev/null
      '';

  pipewireBackendSocketName =
    if sCfg.audio.pipewire.pulseOnly then "cloister/pipewire/${name}" else null;

  anonymizedIdentity =
    if anonymize then
      let
        match = builtins.match "^/home/([^/]+)$" sandboxHome;
      in
      if match == null then
        throw ''
          cloister.sandboxes.${name}: anonymized sandboxHome must be exactly /home/<username>, got ${sandboxHome}
        ''
      else
        builtins.elemAt match 0
    else
      null;

  pipewirePulseOnlyConfText =
    let
      anonymizedPulseOnlyProps = lib.optionalString sCfg.sandbox.anonymize.enable ''
        "context.user-name" = "${anonymizedIdentity}"
        "context.host-name" = "${anonymizedIdentity}"
      '';
      pulseOnlyQuirks =
        lib.optional (!sCfg.audio.pipewire.filters.audioOut) "block-playback-stream"
        ++ lib.optional (!sCfg.audio.pipewire.filters.audioIn) "block-record-stream"
        ++ lib.optionals (!sCfg.audio.pipewire.filters.control) [
          "block-sink-volume"
          "block-source-volume"
        ];
      pulseOnlyQuirksText =
        if pulseOnlyQuirks == [ ] then
          ""
        else
          ''
            pulse.rules = [
              {
                matches = [ { client.api = "pulse" } ]
                actions = {
                  quirks = [ ${lib.concatStringsSep " " pulseOnlyQuirks} ]
                }
              }
            ]
          '';
    in
    ''
      context.properties = {
          support.dbus = ${
            if (sCfg.dbus.enable && sCfg.audio.pipewire.dbus.enable) then "true" else "false"
          }
      ${anonymizedPulseOnlyProps}
      }
      context.spa-libs = {
          audio.convert.* = audioconvert/libspa-audioconvert
          support.*       = support/libspa-support
      }
      context.modules = [
          { name = libpipewire-module-protocol-native }
          { name = libpipewire-module-client-node }
          { name = libpipewire-module-adapter }
          { name = libpipewire-module-metadata }
          { name = libpipewire-module-protocol-pulse
              args = { }
          }
      ]
      context.exec = [ ]
      pulse.cmd = [ ]
      stream.properties = { }
      pulse.properties = {
          server.address = [ "unix:native" ]
          server.dbus-name = "org.pulseaudio.Server.cloister.${name}"
          pulse.allow-module-loading = false
      }
      ${pulseOnlyQuirksText}
    '';

  pipewirePulseOnlyConf =
    if sCfg.audio.pipewire.pulseOnly then
      pkgs.writeText "cloister-pipewire-pulse-only-${name}.conf" pipewirePulseOnlyConfText
    else
      null;

  workerBrokerLauncherTexts = lib.mapAttrs (
    launcherName: launcher:
    let
      launcherConfigArg =
        if launcher.sandbox == name then
          ''"''${CLOISTER_CONFIG_PATH:?CLOISTER_CONFIG_PATH must be set}"''
        else
          lib.escapeShellArg config.cloister._internal.sandboxInternals.${launcher.sandbox}.configJsonPath;
    in
    ''
      set -eu

      if [ "$#" -eq 0 ]; then
        echo "${launcherName}: expected a command to run" >&2
        exit 2
      fi

      if [ -n "''${CLOISTER_BROKER_TRUSTED_RECORD:-}" ]; then
        exec ${cloister-sandbox}/bin/cloister-sandbox \
          --config ${launcherConfigArg} \
          --broker-launch-profile ${lib.escapeShellArg launcher.profile} \
          --broker-launch-sandbox ${lib.escapeShellArg launcher.sandbox} \
          --broker-trusted-record "$CLOISTER_BROKER_TRUSTED_RECORD" \
          -- "$@"
      fi

      exec ${cloister-sandbox}/bin/cloister-sandbox \
        --config ${launcherConfigArg} \
        --broker-launch-profile ${lib.escapeShellArg launcher.profile} \
        --broker-launch-sandbox ${lib.escapeShellArg launcher.sandbox} \
        -- "$@"
    ''
  ) renderedWorkerBroker.generated_launchers;

  workerBrokerLauncherPackage =
    if workerBrokerLauncherTexts == { } then
      null
    else
      pkgs.runCommand "cloister-worker-broker-launchers-${name}" { } ''
        mkdir -p "$out/bin"
        ${lib.concatStringsSep "\n" (
          lib.mapAttrsToList (launcherName: launcherText: ''
            script_path=${pkgs.writeShellScript "${launcherName}" launcherText}
            cp "$script_path" "$out/bin/${launcherName}"
            chmod 0555 "$out/bin/${launcherName}"
          '') workerBrokerLauncherTexts
        )}
      '';

  workerBrokerRenderedLauncherTexts = lib.mapAttrs (_: text: text) workerBrokerLauncherTexts;

  baseEnvAttrs = sCfg.sandbox.env // guiEnv // portalEnv // computedEnv // noHostConfigEnv;

  envAttrs =
    baseEnvAttrs
    // lib.optionalAttrs (workerBrokerLauncherPackage != null) {
      PATH = "${lib.makeBinPath [ workerBrokerLauncherPackage ]}:${baseEnvAttrs.PATH}";
    };

  symlinkTargets = map (entry: entry.target) allSymlinks;

  storeInputs = lib.unique (
    builtins.filter (p: p != null) (
      sCfg._basePackages
      ++ sCfg.extraPackages
      ++ [ pkgs.tini ]
      ++ lib.optional sCfg.gui.enable pkgs.mesa
      ++ lib.optionals sCfg.gui.enable hostGpuStoreInputs
      ++ lib.optional sCfg.audio.pipewire.pulseOnly pkgs.pipewire
      ++ lib.optional (pipewirePulseOnlyConf != null) pipewirePulseOnlyConf
      ++ map storeRootFromEntry symlinkTargets
      ++ map storeRootFromEntry storeRootSources
      ++ lib.concatMap storeRootsFromValue (builtins.attrValues envAttrs)
    )
  );

  effectiveNetworkEnabled = sCfg.network.enable || sCfg.network.namespace != null;

  # The JSON config for the compiled binary
  sandboxConfigJsonBase =
    assert _imageStoreConfigCheck == null;
    builtins.toJSON {
      inherit name;
      bwrap_path = "${bubblewrapPackage}/bin/bwrap";
      shell_bin = shellLib.bin;
      shell_interactive_args = shellLib.interactiveArgs;
      wrapped_command_shell_args = shellLib.wrappedCommandShellArgs or shellLib.interactiveArgs;
      shell_name = sCfg.shell.name;
      shell_host_config = sCfg.shell.hostConfig;
      default_command = sCfg.defaultCommand;

      network_enable = effectiveNetworkEnabled;
      network_namespace = sCfg.network.namespace;
      network_dns_override = sCfg.network.dns.nameservers != [ ] || sCfg.network.dns.resolvConf != null;
      gui_enable = sCfg.gui.enable;
      ssh_enable = sCfg.ssh.enable;
      pipewire_backend_socket_name = pipewireBackendSocketName;
      pipewire_socket_name = pipewireSocketName;
      pipewire_pulse_binary_path =
        if sCfg.audio.pipewire.pulseOnly then "${pkgs.pipewire}/bin/pipewire-pulse" else null;
      pipewire_pulse_config_path =
        if sCfg.audio.pipewire.pulseOnly then "${pipewirePulseOnlyConf}" else null;
      dbus_enable = sCfg.dbus.enable;
      seccomp_enable = sCfg.sandbox.seccomp.enable;
      git_enable = sCfg.git.enable;
      bind_working_directory = sCfg.sandbox.bindWorkingDirectory;
      store_mode = sCfg.sandbox.nixStore.mode;
      store_roots = if imageStoreMode then storeRoots else [ ];
      store_id = if imageStoreMode then storeId else null;
      store_image_path = if imageStoreMode then storeImagePublishedPath else null;
      store_mount_path = if imageStoreMode then storeMountPath else null;
      inherit anonymize;

      ssh_allow_fingerprints = sCfg.ssh.allowFingerprints;
      ssh_filter_timeout_seconds = sCfg.ssh.filterTimeoutSeconds;

      home_directory = config.home.homeDirectory;
      sandbox_home = if anonymize then sandboxHome else config.home.homeDirectory;
      seccomp_filter_path = if seccompFilter != "" then seccompFilter else null;
      per_dir = perDirBuckets;
      copy_file_base = sCfg.sandbox.copyBase;
      netns_helper_path =
        if sCfg.network.namespace != null then "/run/wrappers/bin/cloister-netns" else null;
      git_path = "${pkgs.git}/bin/git";
      init_path = "${pkgs.tini}/bin/tini";

      static_bwrap_args = bwrapLib.mkBwrapArgs {
        dirs =
          allDirs
          ++ (lib.optionals anonymize [
            "/home"
            sandboxHome
          ]);
        tmpfs = internalTmpfs;
        symlinks = allSymlinks;
        binds = {
          ro = staticRoBinds;
          rw = staticRwBinds;
        };
        env = envAttrs;
      };
      dynamic_binds = dynamicBindsList;

      passthrough_env = sCfg.sandbox.passthroughEnv;
      disallowed_paths = sCfg.sandbox.disallowedPaths;
      dev_binds = sCfg.sandbox.devices;

      dir_mkdirs = dirMkdirSpecs;
      file_mkdirs = fileMkdirSpecs;
      managed_file_host_mkdirs = map managedFileDirOverlap managedFileDirsOverlapping;
      copy_files = copyFileSpecs;

      enforce_strict_home_policy = sCfg.sandbox.enforceStrictHomePolicy;
      dbus_proxy_socket_name = if sCfg.dbus.enable then "cloister/dbus/${name}" else null;
      dbus_proxy_path = if sCfg.dbus.enable then dbusProxyWrapper else null;
      flatpak_app_id = if portalDesktopEnabled then flatpakAppId else null;
      worker_broker = renderedWorkerBroker;
      build_revision = buildRevision;
    };

  configJsonPath = pkgs.writeText "cloister-config-${name}.json" sandboxConfigJsonBase;

  package =
    pkgs.runCommand "cl-${name}"
      {
        nativeBuildInputs = [ pkgs.makeWrapper ];
      }
      ''
        mkdir -p $out/bin
        makeWrapper ${cloister-sandbox}/bin/cloister-sandbox $out/bin/cl-${name} \
          --set-default CLOISTER_BUILD_REV ${lib.escapeShellArg buildRevision} \
          --set-default CLOISTER_CONFIG_PATH ${lib.escapeShellArg configJsonPath} \
          --set-default CLOISTER_WORKER_BROKER_LAUNCHER_PACKAGE ${
            lib.escapeShellArg (
              if workerBrokerLauncherPackage != null then toString workerBrokerLauncherPackage else ""
            )
          } \
          --add-flags "--config ${configJsonPath} --"
      '';
in
{
  inherit configJsonPath package;
  sandboxConfig = builtins.fromJSON (builtins.unsafeDiscardStringContext sandboxConfigJsonBase);
  internal = {
    inherit configJsonPath;
    inherit pipewirePulseOnlyConfText;
    inherit imageStoreMode squashfsCompressionArgs;
    inherit workerBrokerLauncherPackage;
    workerBrokerLauncherTexts = workerBrokerRenderedLauncherTexts;
  };
  imageStoreInfo =
    if imageStoreMode then
      {
        inherit name storeId storeRoots;
        imagePath = toString storeImage;
        metaPath = toString storeMeta;
        publishedImagePath = storeImagePublishedPath;
        publishedMetaPath = storeMetaPublishedPath;
        mountPath = storeMountPath;
      }
    else
      null;
  sandboxConfigJson = builtins.readFile configJsonPath;
}
