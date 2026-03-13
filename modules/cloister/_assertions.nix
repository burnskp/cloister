# Assertion builder: produces the list of assertion attrsets for a sandbox.
{
  lib,
  name,
  sCfg,
  duplicateDests,
  dirTmpfsOverlap,
  duplicateLinks,
  duplicateManagedFiles,
  unsafePaths,
  matchedDangerousPaths,
  overriddenEnvKeys,
  overriddenDbusKeys,
  overriddenGuiKeys,
  overriddenPortalKeys,
  blockedPassthroughEnv,
  invalidPassthroughEnv,
  guiEnabled,
  normalizeCopyDest,
}:
[
  {
    assertion = sCfg.sandbox.bindWorkingDirectory || sCfg.sandbox.extraBinds.perDir == [ ];
    message = "cloister.sandboxes.${name}: sandbox.bindWorkingDirectory = false is incompatible with sandbox.extraBinds.perDir. Per-directory isolation requires the working directory to be detected.";
  }
  {
    assertion = builtins.all (
      cf: lib.hasPrefix "$HOME/" (normalizeCopyDest cf.dest)
    ) sCfg.sandbox.copyFiles;
    message = "cloister.sandboxes.${name}: all copyFiles dest paths must start with $HOME/ (after normalization)";
  }
  (
    let
      invalidModes = builtins.filter (
        cf: builtins.match "^[0-7]{3,4}$" cf.mode == null
      ) sCfg.sandbox.copyFiles;
    in
    {
      assertion = invalidModes == [ ];
      message = "cloister.sandboxes.${name}: copyFiles contains invalid mode values: ${
        lib.concatMapStringsSep ", " (cf: "'${cf.mode}' (for ${cf.dest})") invalidModes
      }. Modes must be 3 or 4 octal digits (e.g. '0644', '755').";
    }
  )
  {
    assertion =
      sCfg.gui.scaleFactor == null
      || (
        sCfg.gui.scaleFactor > 0.0
        && (
          let
            scaled = sCfg.gui.scaleFactor * 4.0;
          in
          builtins.floor scaled == builtins.ceil scaled
        )
      );
    message = "cloister.sandboxes.${name}: gui.scaleFactor must be a positive value in 0.25 increments (e.g. 1.0, 1.25, 1.5, 1.75, 2.0).";
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
    assertion = duplicateManagedFiles == [ ];
    message = "cloister.sandboxes.${name}: duplicate managedFile entries: ${lib.concatStringsSep ", " duplicateManagedFiles}";
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
    assertion = !sCfg.dbus.portal.enable || sCfg.dbus.enable;
    message = "cloister.sandboxes.${name}: dbus.portal requires dbus.enable = true.";
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
    message = "cloister.sandboxes.${name}: bind/dir/tmpfs/symlink/env paths cannot contain variable expansions ($) or newlines: ${lib.concatStringsSep ", " unsafePaths}";
  }
  {
    assertion = !sCfg.gui.desktopEntry.enable || guiEnabled;
    message = "cloister.sandboxes.${name}: gui.desktopEntry.enable requires gui.wayland.enable or gui.x11.enable.";
  }
  {
    assertion =
      sCfg.gui.desktopEntry.execArgs == ""
      ||
        builtins.match ''
          .*['";|&
          `$].*'' sCfg.gui.desktopEntry.execArgs == null;
    message = ''cloister.sandboxes.${name}: gui.desktopEntry.execArgs must not contain shell metacharacters (', ", ;, |, &, `, $) or newlines.'';
  }
  (
    let
      dbusNamePattern = "^[a-zA-Z_][a-zA-Z0-9._-]*(\\.[*])?$";
      allDbusNames =
        sCfg.dbus.policies.talk
        ++ sCfg.dbus.policies.own
        ++ sCfg.dbus.policies.see
        ++ lib.attrNames sCfg.dbus.policies.call
        ++ lib.attrNames sCfg.dbus.policies.broadcast;
      invalidDbusNames = builtins.filter (n: builtins.match dbusNamePattern n == null) allDbusNames;
    in
    {
      assertion = !sCfg.dbus.enable || invalidDbusNames == [ ];
      message = "cloister.sandboxes.${name}: D-Bus policy names contain invalid characters: ${lib.concatStringsSep ", " invalidDbusNames}. Names must match ${dbusNamePattern}.";
    }
  )
  {
    assertion = !sCfg.sandbox.dangerousPathWarnings || matchedDangerousPaths == [ ];
    message = lib.concatStringsSep "\n" (
      [
        "cloister.sandboxes.${name}: binds/extraBinds/managedFile contains paths that expose credentials or secrets:"
      ]
      ++ map (p: "  - ${p}") matchedDangerousPaths
      ++ [
        ""
        "These paths contain sensitive data (SSH keys, cloud credentials, keyrings, etc.)"
        "that the sandbox is designed to protect. Binding them in defeats the purpose."
        ""
        "To suppress this warning for specific paths:"
        "  cloister.sandboxes.${name}.sandbox.allowDangerousPaths = [ ${
            lib.concatMapStringsSep " " (p: ''"${p}"'') matchedDangerousPaths
          } ];"
        ""
        "To disable all dangerous path checks:"
        "  cloister.sandboxes.${name}.sandbox.dangerousPathWarnings = false;"
      ]
    );
  }
]
