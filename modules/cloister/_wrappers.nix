{
  config,
  lib,
  pkgs,
  ...
}:

let
  shells = import ./_mkShells.nix { inherit pkgs lib; };

  # Generates the wrapper init snippet for a given shell.
  # All cfg.sandboxes references are inside this thunk — they are only
  # forced when the option value is actually evaluated, not during
  # pushDownProperties (which only inspects attrset keys, not values).
  mkWrapperInitContent =
    shellLib:
    let
      cfg = config.cloister;
      allOutsideRendered = lib.concatStringsSep "\n\n" (
        lib.filter (s: s != "") (
          lib.mapAttrsToList (_name: sCfg: sCfg.registry.rendered.outside.${shellLib.key}) cfg.sandboxes
        )
      );
    in
    shellLib.renderWrapperInit {
      inherit (config.xdg) configHome;
      outsideRendered = allOutsideRendered;
    };
in
{
  # The config is a single flat attrset under lib.mkIf.  pushDownProperties
  # will see _type="if", recurse into the content (a plain attrset), and
  # wrap each VALUE in mkIf — but never force the values themselves.
  # This avoids the cfg.sandboxes evaluation that caused infinite recursion
  # when we used lib.mkMerge over a dynamically-computed list.
  config = lib.mkIf config.cloister.enable {

    # Per-sandbox init files (sourced inside the sandbox)
    xdg.configFile =
      let
        cfg = config.cloister;
      in
      lib.mapAttrs' (
        name: sCfg:
        let
          shellLib = shells.${sCfg.shell.name};
        in
        {
          name = "${shellLib.configDir}/cloister-${name}.${shellLib.initExt}";
          value.text = ''
            ${sCfg.init.rendered}
            ${sCfg.registry.rendered.inside}
          '';
        }
      ) cfg.sandboxes;

    programs =
      let
        cfg = config.cloister;
        enabled = shellKey: builtins.any (sCfg: sCfg.shell.name == shellKey) (lib.attrValues cfg.sandboxes);
      in
      {
        zsh.initContent = lib.mkIf (enabled "zsh") (lib.mkAfter (mkWrapperInitContent shells.zsh));
        bash.initExtra = lib.mkIf (enabled "bash") (lib.mkAfter (mkWrapperInitContent shells.bash));
      };

  };
}
