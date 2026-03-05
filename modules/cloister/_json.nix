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
  gpuEnabled,
  seccompFilter,
  allDirs,
  managedFileDirsOverlapping,
  managedFileDirOverlap,
  noHostConfigEnv,
  guiEnv,
  portalEnv,
  computedEnv,
  staticRoBinds,
  staticRwBinds,
  dynamicBindsList,
  bindSources,
  dirMkdirSpecs,
  fileMkdirSpecs,
  copyFileSpecs,
  normalizedDangerousPaths,
  normalizedAllowDangerousPaths,
}:
let
  # The JSON config for the compiled binary
  sandboxConfigJson = builtins.toJSON {
    inherit name;
    bwrap_path = "${pkgs.bubblewrap}/bin/bwrap";
    shell_bin = shellLib.bin;
    shell_interactive_args = shellLib.interactiveArgs;
    shell_name = sCfg.shell.name;
    shell_host_config = sCfg.shell.hostConfig;
    default_command = sCfg.defaultCommand;

    network_enable = sCfg.network.enable;
    network_namespace = sCfg.network.namespace;
    wayland_enable = sCfg.gui.wayland.enable;
    wayland_security_context = sCfg.gui.wayland.securityContext.enable;
    x11_enable = sCfg.gui.x11.enable;
    gpu_enable = gpuEnabled;
    gpu_shm = sCfg.gui.gpu.shm;
    ssh_enable = sCfg.ssh.enable;
    pulseaudio_enable = sCfg.audio.pulseaudio.enable;
    pipewire_enable = sCfg.audio.pipewire.enable;
    fido2_enable = sCfg.fido2.enable;
    video_enable = sCfg.video.enable;
    printing_enable = sCfg.printing.enable;
    dbus_enable = sCfg.dbus.enable;
    seccomp_enable = sCfg.sandbox.seccomp.enable;
    git_enable = sCfg.git.enable;
    bind_working_directory = sCfg.sandbox.bindWorkingDirectory;
    inherit anonymize;

    ssh_allow_fingerprints = sCfg.ssh.allowFingerprints;
    ssh_filter_timeout_seconds = sCfg.ssh.filterTimeoutSeconds;

    home_directory = config.home.homeDirectory;
    sandbox_home = if anonymize then sandboxHome else config.home.homeDirectory;
    seccomp_filter_path = if seccompFilter != "" then seccompFilter else null;
    per_dir_base = sCfg.sandbox.perDirBase;
    copy_file_base = sCfg.sandbox.copyFileBase;
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
      inherit (sCfg.sandbox) tmpfs;
      symlinks = sCfg.sandbox.symlinks ++ sCfg.sandbox.extraSymlinks;
      binds = {
        ro = staticRoBinds;
        rw = staticRwBinds;
      };
      env = sCfg.sandbox.env // guiEnv // portalEnv // computedEnv // noHostConfigEnv;
    };
    dynamic_binds = dynamicBindsList;

    passthrough_env = sCfg.sandbox.passthroughEnv;
    disallowed_paths = sCfg.sandbox.disallowedPaths;
    dangerous_paths = normalizedDangerousPaths;
    allow_dangerous_paths = normalizedAllowDangerousPaths;
    dangerous_path_warnings = sCfg.sandbox.dangerousPathWarnings;
    dev_binds = sCfg.sandbox.devBinds;
    per_dir_paths = sCfg.sandbox.extraBinds.perDir;
    bind_sources = bindSources;

    dir_mkdirs = dirMkdirSpecs;
    file_mkdirs = fileMkdirSpecs;
    managed_file_host_mkdirs = map managedFileDirOverlap managedFileDirsOverlapping;
    copy_files = copyFileSpecs;

    enforce_strict_home_policy = sCfg.sandbox.enforceStrictHomePolicy;
    dbus_proxy_socket_name = if sCfg.dbus.enable then "dbus-proxy-${name}" else null;
  };

  configJsonPath = pkgs.writeText "cloister-config-${name}.json" sandboxConfigJson;

  package =
    pkgs.runCommand "cl-${name}"
      {
        nativeBuildInputs = [ pkgs.makeWrapper ];
      }
      ''
        mkdir -p $out/bin
        makeWrapper ${cloister-sandbox}/bin/cloister-sandbox $out/bin/cl-${name} \
          --add-flags "--config ${configJsonPath} --"
      '';
in
{
  inherit package sandboxConfigJson configJsonPath;
}
