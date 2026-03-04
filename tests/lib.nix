{
  pkgs,
  home-manager,
  cloister-module,
}:
let
  inherit (pkgs) lib;

  # Evaluate a home-manager configuration with the cloister module loaded.
  # Returns the full `config` attrset.
  evalConfig =
    { modules }:
    let
      result = home-manager.lib.homeManagerConfiguration {
        inherit pkgs;
        modules = [
          cloister-module
          {
            home = {
              username = "testuser";
              homeDirectory = "/home/testuser";
              stateVersion = "25.05";
            };
          }
        ]
        ++ modules;
      };
    in
    result.config;

  # Like evalConfig but bypasses homeManagerConfiguration's eager assertion
  # checking (moduleChecks).  This lets callers inspect config.assertions as
  # data instead of catching a throw.
  evalConfigUnchecked =
    { modules }:
    let
      extendedLib = import "${home-manager}/modules/lib/stdlib-extended.nix" lib;
      hmModules = import "${home-manager}/modules/modules.nix" {
        check = true;
        inherit pkgs;
        lib = extendedLib;
      };
      configuration =
        { ... }:
        {
          imports = [
            cloister-module
            {
              home = {
                username = "testuser";
                homeDirectory = "/home/testuser";
                stateVersion = "25.05";
              };
            }
          ]
          ++ modules
          ++ [
            { programs.home-manager.path = "${home-manager}"; }
          ];
          nixpkgs = {
            config = lib.mkDefault pkgs.config;
            inherit (pkgs) overlays;
          };
        };
    in
    (extendedLib.evalModules {
      modules = [ configuration ] ++ hmModules;
      class = "homeManager";
      specialArgs = {
        modulesPath = builtins.toString "${home-manager}/modules";
      };
    }).config;

  # Turn a boolean eval-time check into a derivation.
  # Passes: touch $out. Fails: exit 1 with message.
  mkCheck =
    name: pass:
    pkgs.runCommand "check-${name}" { } (
      if pass then
        "touch $out"
      else
        ''
          echo "FAIL: ${name}"
          exit 1
        ''
    );

  # Find a sandbox package (cl-<sandboxName>) in config.home.packages, then
  # extract the JSON config path from the makeWrapper shim and grep for
  # `pattern` in that JSON config file.
  # `positive` = true means pattern MUST be found; false means MUST NOT.
  mkConfigCheck =
    name: config: sandboxName: pattern: positive:
    let
      scriptName = "cl-${sandboxName}";
      sandboxPkg = lib.findFirst (p: (p.pname or p.name or "") == scriptName) null config.home.packages;
    in
    assert sandboxPkg != null;
    pkgs.runCommand "check-${name}" { } ''
      # Extract the JSON config path from the makeWrapper shim
      config_path=$(${pkgs.gnugrep}/bin/grep -oP '/nix/store/[^ ]+\.json' ${sandboxPkg}/bin/${scriptName})
      found=false
      if ${pkgs.gnugrep}/bin/grep -qF -- ${lib.escapeShellArg pattern} "$config_path"; then
        found=true
      fi
      if [[ "$found" == "${if positive then "true" else "false"}" ]]; then
        touch $out
      else
        echo "FAIL: ${name}"
        echo "Expected pattern to be ${if positive then "present" else "absent"}:"
        echo ${lib.escapeShellArg pattern}
        echo "--- config content ---"
        cat "$config_path"
        exit 1
      fi
    '';

  # Evaluate config with given modules and check that an assertion fires
  # containing the expected message substring.  Uses evalConfigUnchecked
  # to bypass homeManagerConfiguration's eager assertion throw so we can
  # inspect config.assertions as data.
  mkAssertionCheck =
    name: modules: expectedMessage:
    let
      config = evalConfigUnchecked { inherit modules; };
      failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
      messages = map (x: x.message) failedAssertions;
      hasExpectedFailure = builtins.any (msg: lib.hasInfix expectedMessage msg) messages;
    in
    mkCheck name hasExpectedFailure;

in
{
  inherit
    pkgs
    evalConfig
    mkCheck
    mkConfigCheck
    mkAssertionCheck
    lib
    ;
}
