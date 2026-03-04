{
  pkgs,
  home-manager,
  cloister-module,
}:
let
  testLib = import ./lib.nix { inherit pkgs home-manager cloister-module; };

  bwrap = import ./bwrap.nix { inherit testLib; };
  sandbox = import ./sandbox.nix { inherit testLib; };
  registry = import ./registry.nix { inherit testLib; };
  wrappers = import ./wrappers.nix { inherit testLib; };
  netns = import ./netns.nix { inherit testLib; };

  # Flatten nested attrsets into a single level for nix flake check.
  # Each test suite returns an attrset of derivations; prefix with suite name.
  flatten =
    prefix: attrs:
    builtins.listToAttrs (
      map (name: {
        name = "${prefix}-${name}";
        value = attrs.${name};
      }) (builtins.attrNames attrs)
    );
in
flatten "bwrap" bwrap
// flatten "sandbox" sandbox
// flatten "registry" registry
// flatten "wrappers" wrappers
// flatten "netns" netns
