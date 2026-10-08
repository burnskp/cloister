{
  pkgs,
  home-manager,
  nixpkgs,
  cloister,
}:
let
  # Evaluate the actual README snippet so documentation cannot drift from a fixture.
  snippet = builtins.head (
    pkgs.lib.splitString "```" (
      builtins.elemAt (pkgs.lib.splitString "```nix\n# flake.nix\n" (builtins.readFile ../../README.md)) 1
    )
  );
  example = import (builtins.toFile "standalone-example.nix" snippet);
  outputs = example.outputs { inherit nixpkgs home-manager cloister; };
  home = outputs.homeConfigurations.yourname;
in
assert home.config.home.username == "yourname";
assert home.config.home.homeDirectory == "/home/yourname";
assert home.config.home.stateVersion == "25.05";
# Force the complete module evaluation, including Home Manager's assertions.
assert builtins.isString home.activationPackage.drvPath;
pkgs.runCommand "cloister-standalone-example" { } "touch $out"
