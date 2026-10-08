# Assertion builder: produces the list of assertion attrsets for a sandbox.
{
  lib,
  name,
  sCfg,
  duplicateDests,
  dirTmpfsOverlap,
  duplicateLinks,
  userSymlinkBindConflicts,
  duplicateManagedFiles,
  unsafePaths,
  overriddenEnvKeys,
  overriddenDbusKeys,
  overriddenGuiKeys,
  overriddenPortalKeys,
  blockedPassthroughEnv,
  invalidPassthroughEnv,
  guiEnabled,
  hasPerDirBinds,
  normalizeCopyDest,
  normalizePath,
  sandboxNames,
}:
let
  workerBrokerCfg = sCfg.workerBroker;
  isRelativeDescendantPath =
    path: path != "" && !lib.hasPrefix "/" path && normalizePath path == path;
  isAbsoluteTraversalFreeHostPath =
    path:
    path != ""
    && lib.hasPrefix "/" path
    && normalizePath path == path
    && !(builtins.any (segment: segment == "..") (lib.splitString "/" path));
  isSafeBindPath =
    path:
    if lib.hasPrefix "/" path then
      isAbsoluteTraversalFreeHostPath path
    else
      isRelativeDescendantPath path;
  isSafeBindEntry =
    entry:
    if builtins.isString entry then
      isRelativeDescendantPath entry
    else
      isSafeBindPath (toString entry.src) && (entry.dest == null || isSafeBindPath entry.dest);
  invalidBindEntries = builtins.filter (entry: !isSafeBindEntry entry) (
    sCfg.sandbox.readOnly ++ sCfg.sandbox.readWrite
  );
  invalidStateBases = builtins.filter (base: !isAbsoluteTraversalFreeHostPath base) (
    lib.attrNames sCfg.sandbox.state.dirs
    ++ lib.attrNames sCfg.sandbox.state.files
    ++ lib.attrNames sCfg.sandbox.state.projectDirs
  );
  invalidStatePaths = builtins.filter (path: !isRelativeDescendantPath path) (
    lib.concatLists (lib.attrValues sCfg.sandbox.state.dirs)
    ++ lib.concatLists (lib.attrValues sCfg.sandbox.state.files)
    ++ lib.concatLists (lib.attrValues sCfg.sandbox.state.projectDirs)
  );
  invalidCopySources = builtins.filter (
    cf: !isAbsoluteTraversalFreeHostPath cf.src
  ) sCfg.sandbox.copies;
  isSafeSymlinkLink =
    path:
    if lib.hasPrefix "$HOME/" path then
      normalizeCopyDest path == path
    else if lib.hasPrefix "/" path then
      isAbsoluteTraversalFreeHostPath path
    else
      isRelativeDescendantPath path;
  invalidSymlinks = builtins.filter (
    entry: entry.target == "" || !isSafeSymlinkLink entry.link
  ) sCfg.sandbox.symlinks;
  workerBrokerEnabled = workerBrokerCfg.profiles != { };
  profileDelegatedMounts = lib.concatMapAttrs (
    profileName: profile:
    lib.mapAttrs' (
      mountName: mount:
      lib.nameValuePair "${profileName}:${mountName}" {
        inherit profileName mountName mount;
      }
    ) profile.delegatedPerDirMounts
  ) workerBrokerCfg.profiles;
  invalidReferencedDelegatedMountKeys = lib.filterAttrs (
    _: profile:
    builtins.any (mountName: !isRelativeDescendantPath mountName) (
      lib.attrNames profile.delegatedPerDirMounts
    )
  ) workerBrokerCfg.profiles;
  invalidSpawnableProfiles = lib.filterAttrs (
    _: profile: !(builtins.elem profile.sandbox sandboxNames)
  ) workerBrokerCfg.profiles;
  invalidSpawnableProfileMessages = lib.concatMapStringsSep "; " (
    profileName:
    let
      profile = invalidSpawnableProfiles.${profileName};
    in
    "workerBroker.profiles.${profileName}.sandbox references unknown sandbox '${profile.sandbox}'"
  ) (lib.attrNames invalidSpawnableProfiles);
  invalidDelegatedMountSubPaths = lib.filterAttrs (
    _: entry:
    entry.mount.subPath != null
    && (
      entry.mount.subPath == ""
      || lib.hasPrefix "/" entry.mount.subPath
      || normalizeCopyDest entry.mount.subPath != entry.mount.subPath
    )
  ) profileDelegatedMounts;
  invalidDelegatedMountBasePaths = lib.filterAttrs (
    _: entry: !isAbsoluteTraversalFreeHostPath entry.mount.path
  ) profileDelegatedMounts;
  invalidDelegatedMountSubPathMessages = lib.concatStringsSep "; " (
    lib.mapAttrsToList (
      _: entry:
      "workerBroker.profiles.${entry.profileName}.delegatedPerDirMounts.${entry.mountName}.subPath must be a relative descendant path without traversal"
    ) invalidDelegatedMountSubPaths
  );
  invalidDelegatedMountBasePathMessages = lib.concatStringsSep "; " (
    lib.mapAttrsToList (
      _: entry:
      "workerBroker.profiles.${entry.profileName}.delegatedPerDirMounts.${entry.mountName}.path must be an absolute traversal-free host path"
    ) invalidDelegatedMountBasePaths
  );
  inherit (workerBrokerCfg) invalidGeneratedLauncherNames;
  generatedLauncherNames = lib.attrNames workerBrokerCfg.generatedLaunchers;
  generatedLauncherAliasCollisions = lib.intersectLists generatedLauncherNames (
    lib.attrNames sCfg.registry.aliases
  );
  generatedLauncherFunctionCollisions = lib.intersectLists generatedLauncherNames (
    lib.attrNames sCfg.registry.functions
  );
  generatedLauncherCommandCollisions = lib.intersectLists generatedLauncherNames sCfg.registry.commands;
  invalidDelegatedMountKeyMessages = lib.concatStringsSep "; " (
    map (_: "workerBroker delegatedPerDirMounts keys must be sandbox-relative descendant paths") (
      lib.attrNames invalidReferencedDelegatedMountKeys
    )
  );
in
[
  {
    assertion = sCfg.network.dns.nameservers == [ ] || sCfg.network.dns.resolvConf == null;
    message = "cloister.sandboxes.${name}: network.dns.nameservers and network.dns.resolvConf are mutually exclusive.";
  }
  {
    assertion = builtins.all (
      server: builtins.match "[^[:space:]]+" server != null
    ) sCfg.network.dns.nameservers;
    message = "cloister.sandboxes.${name}: network.dns.nameservers entries must be nonempty and contain no whitespace.";
  }
  {
    assertion = sCfg.sandbox.bindWorkingDirectory || !hasPerDirBinds;
    message = "cloister.sandboxes.${name}: sandbox.bindWorkingDirectory = false is incompatible with sandbox.state.projectDirs. Per-project state requires the working directory to be detected.";
  }
  {
    assertion = invalidBindEntries == [ ];
    message = "cloister.sandboxes.${name}: sandbox.readOnly/readWrite entries must use traversal-free paths; string entries must be home-relative descendants: ${builtins.toJSON invalidBindEntries}";
  }
  {
    assertion = invalidStateBases == [ ];
    message = "cloister.sandboxes.${name}: sandbox.state base directories must be absolute traversal-free host paths: ${lib.concatStringsSep ", " invalidStateBases}";
  }
  {
    assertion = invalidStatePaths == [ ];
    message = "cloister.sandboxes.${name}: sandbox.state paths must be home-relative descendants without traversal: ${lib.concatStringsSep ", " invalidStatePaths}";
  }
  {
    assertion = isAbsoluteTraversalFreeHostPath sCfg.sandbox.copyBase;
    message = "cloister.sandboxes.${name}: sandbox.copyBase must be an absolute traversal-free host path";
  }
  {
    assertion = invalidCopySources == [ ];
    message = "cloister.sandboxes.${name}: sandbox.copies src paths must be absolute traversal-free host paths: ${
      lib.concatMapStringsSep ", " (cf: cf.src) invalidCopySources
    }";
  }
  {
    assertion = builtins.all (
      bind: bind.dest != "" && !lib.hasPrefix "/" bind.dest && normalizeCopyDest bind.dest == bind.dest
    ) (builtins.filter (entry: !builtins.isString entry) sCfg.sandbox.managed);
    message = "cloister.sandboxes.${name}: all attr-form sandbox.managed dest paths must be home-relative descendant paths without traversal";
  }
  {
    assertion = builtins.all (
      cf: lib.hasPrefix "$HOME/" (normalizeCopyDest cf.dest)
    ) sCfg.sandbox.copies;
    message = "cloister.sandboxes.${name}: all sandbox.copies dest paths must start with $HOME/ (after normalization)";
  }
  (
    let
      invalidModes = builtins.filter (
        cf: builtins.match "^[0-7]{3,4}$" cf.mode == null
      ) sCfg.sandbox.copies;
    in
    {
      assertion = invalidModes == [ ];
      message = "cloister.sandboxes.${name}: sandbox.copies contains invalid mode values: ${
        lib.concatMapStringsSep ", " (cf: "'${cf.mode}' (for ${cf.dest})") invalidModes
      }. Modes must be 3 or 4 octal digits (e.g. '0644', '755').";
    }
  )
  {
    assertion = invalidSymlinks == [ ];
    message = "cloister.sandboxes.${name}: sandbox.symlinks entries must have non-empty targets and traversal-free link paths: ${builtins.toJSON invalidSymlinks}";
  }
  {
    assertion = duplicateDests == [ ];
    message = "cloister.sandboxes.${name}: duplicate bind mount destinations: ${lib.concatStringsSep ", " duplicateDests}";
  }
  {
    assertion = dirTmpfsOverlap == [ ];
    message = "cloister.sandboxes.${name}: paths appear in both sandbox dirs and tmpfs: ${lib.concatStringsSep ", " dirTmpfsOverlap}";
  }
  {
    assertion = duplicateLinks == [ ];
    message = "cloister.sandboxes.${name}: duplicate symlink destinations: ${lib.concatStringsSep ", " duplicateLinks}";
  }
  {
    assertion = userSymlinkBindConflicts == [ ];
    message = "cloister.sandboxes.${name}: sandbox.symlinks link paths must not collide with or be hidden by bind mount destinations: ${lib.concatStringsSep ", " userSymlinkBindConflicts}";
  }
  {
    assertion = duplicateManagedFiles == [ ];
    message = "cloister.sandboxes.${name}: duplicate managed file destinations: ${lib.concatStringsSep ", " duplicateManagedFiles}";
  }
  {
    assertion = overriddenEnvKeys == [ ];
    message = "cloister.sandboxes.${name}: sandbox.env sets keys that are computed and cannot be overridden: ${lib.concatStringsSep ", " overriddenEnvKeys}";
  }
  {
    assertion = overriddenDbusKeys == [ ];
    message = "cloister.sandboxes.${name}: sandbox.env sets keys managed by dbus and cannot be overridden when dbus is enabled: ${lib.concatStringsSep ", " overriddenDbusKeys}";
  }
  {
    assertion = overriddenGuiKeys == [ ];
    message = "cloister.sandboxes.${name}: sandbox.env sets keys managed by gui and cannot be overridden when gui is enabled: ${lib.concatStringsSep ", " overriddenGuiKeys}";
  }
  {
    assertion = overriddenPortalKeys == [ ];
    message = "cloister.sandboxes.${name}: sandbox.env sets keys managed by dbus.portal and cannot be overridden when portal is enabled: ${lib.concatStringsSep ", " overriddenPortalKeys}";
  }
  {
    assertion =
      !(builtins.any (x: x) [
        sCfg.dbus.portal.fileChooser
        sCfg.dbus.portal.openUri
        sCfg.dbus.portal.screencast
        sCfg.dbus.portal.camera
        sCfg.dbus.notifications
      ])
      || sCfg.dbus.enable;
    message = "cloister.sandboxes.${name}: dbus portal options and dbus.notifications require dbus.enable = true.";
  }
  {
    assertion = invalidPassthroughEnv == [ ];
    message = "cloister.sandboxes.${name}: sandbox.passthroughEnv contains invalid variable names: ${lib.concatStringsSep ", " invalidPassthroughEnv}";
  }
  {
    assertion = blockedPassthroughEnv == [ ];
    message = "cloister.sandboxes.${name}: sandbox.passthroughEnv cannot include computed/managed keys: ${lib.concatStringsSep ", " blockedPassthroughEnv}";
  }
  {
    assertion = unsafePaths == [ ];
    message = "cloister.sandboxes.${name}: sandbox path and environment keys cannot contain unsafe variable expansions ($) or newlines: ${lib.concatStringsSep ", " unsafePaths}";
  }
  {
    assertion = sCfg.gui.desktopEntry == null || guiEnabled;
    message = "cloister.sandboxes.${name}: gui.desktopEntry requires gui.enable = true.";
  }
  {
    assertion = !workerBrokerEnabled || sCfg.sandbox.bindWorkingDirectory;
    message = "cloister.sandboxes.${name}: workerBroker.profiles requires sandbox.bindWorkingDirectory = true.";
  }
  {
    assertion = invalidSpawnableProfiles == { };
    message = "cloister.sandboxes.${name}: ${invalidSpawnableProfileMessages}.";
  }
  {
    assertion = invalidReferencedDelegatedMountKeys == { };
    message = "cloister.sandboxes.${name}: ${invalidDelegatedMountKeyMessages}.";
  }
  {
    assertion = invalidDelegatedMountSubPaths == { };
    message = "cloister.sandboxes.${name}: ${invalidDelegatedMountSubPathMessages}.";
  }
  {
    assertion = invalidDelegatedMountBasePaths == { };
    message = "cloister.sandboxes.${name}: ${invalidDelegatedMountBasePathMessages}.";
  }
  {
    assertion = invalidGeneratedLauncherNames == [ ];
    message = "cloister.sandboxes.${name}: workerBroker.profiles keys must produce safe generated launcher names: ${
      lib.concatStringsSep ", " (
        map (launcherName: lib.removePrefix "clb-" launcherName) invalidGeneratedLauncherNames
      )
    }";
  }
  {
    assertion = generatedLauncherAliasCollisions == [ ];
    message = "cloister.sandboxes.${name}: generated worker broker launcher names collide with registry.aliases: ${lib.concatStringsSep ", " generatedLauncherAliasCollisions}";
  }
  {
    assertion = generatedLauncherFunctionCollisions == [ ];
    message = "cloister.sandboxes.${name}: generated worker broker launcher names collide with registry.functions: ${lib.concatStringsSep ", " generatedLauncherFunctionCollisions}";
  }
  {
    assertion = generatedLauncherCommandCollisions == [ ];
    message = "cloister.sandboxes.${name}: generated worker broker launcher names collide with registry.commands: ${lib.concatStringsSep ", " generatedLauncherCommandCollisions}";
  }
  {
    assertion =
      sCfg.gui.desktopEntry == null || (sCfg.defaultCommand != null && sCfg.defaultCommand != [ ]);
    message = "cloister.sandboxes.${name}: gui.desktopEntry requires defaultCommand to be set so the launcher does not open an interactive shell.";
  }
  {
    assertion =
      sCfg.gui.desktopEntry == null
      || sCfg.gui.desktopEntry.execArgs == ""
      ||
        builtins.match ''
          .*['";|&
          `$].*'' sCfg.gui.desktopEntry.execArgs == null;
    message = ''cloister.sandboxes.${name}: gui.desktopEntry.execArgs must not contain shell metacharacters (', ", ;, |, &, `, $) or newlines.'';
  }
  (
    let
      dbusNamePattern = ''^([A-Za-z_][A-Za-z0-9_-]*)(\.([A-Za-z_][A-Za-z0-9_-]*))+$'';
      dbusWildcardPattern = ''^([A-Za-z_][A-Za-z0-9_-]*)(\.([A-Za-z_][A-Za-z0-9_-]*))+\.\*$'';
      validDbusName =
        n: builtins.match dbusNamePattern n != null || builtins.match dbusWildcardPattern n != null;
      allDbusNames =
        sCfg.dbus.rawPolicies.talk
        ++ sCfg.dbus.rawPolicies.own
        ++ sCfg.dbus.rawPolicies.see
        ++ lib.attrNames sCfg.dbus.rawPolicies.call
        ++ lib.attrNames sCfg.dbus.rawPolicies.broadcast
        ++ sCfg.dbus._portalPolicies.talk
        ++ sCfg.dbus._portalPolicies.own
        ++ sCfg.dbus._portalPolicies.see
        ++ lib.attrNames sCfg.dbus._portalPolicies.call
        ++ lib.attrNames sCfg.dbus._portalPolicies.broadcast;
      invalidDbusNames = builtins.filter (n: !validDbusName n) allDbusNames;
    in
    {
      assertion = !sCfg.dbus.enable || invalidDbusNames == [ ];
      message = "cloister.sandboxes.${name}: D-Bus policy names are invalid: ${lib.concatStringsSep ", " invalidDbusNames}. Names must be dot-separated well-known bus names, with optional .*.";
    }
  )
  {
    assertion = !sCfg.audio.pipewire.pulseOnly || sCfg.audio.pipewire.enable;
    message = "cloister.sandboxes.${name}: audio.pipewire.pulseOnly requires audio.pipewire.enable = true so Cloister can create the filtered PipeWire backend socket used by the PulseAudio proxy.";
  }
  {
    assertion = !(sCfg.audio.pipewire.pulseOnly && sCfg.audio.pipewire.filters.videoIn);
    message = "cloister.sandboxes.${name}: audio.pipewire.pulseOnly is audio-only and does not support audio.pipewire.filters.videoIn.";
  }
  {
    assertion =
      !(sCfg.sandbox.anonymize.enable && sCfg.audio.pipewire.enable && !sCfg.audio.pipewire.pulseOnly);
    message = "cloister.sandboxes.${name}: it is not possible to anonymize a PipeWire socket. If sandbox.anonymize.enable = true and audio.pipewire.enable = true, you must also enable audio.pipewire.pulseOnly.";
  }
]
