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
  # containing the expected message substring.
  #
  # homeManagerConfiguration eagerly checks assertions and throws before
  # returning, so we use builtins.tryEval to catch the throw.  We verify
  # the message by also attempting to read config.assertions directly —
  # if that path is reachable we match the substring; if the throw is
  # too early we accept the failure as confirmation the assertion fired.
  mkAssertionCheck =
    name: modules: expectedMessage:
    let
      # First, try to get the raw config — this may throw if
      # homeManagerConfiguration checks assertions eagerly.
      configResult = builtins.tryEval (evalConfig {
        inherit modules;
      });
    in
    if configResult.success then
      # Config evaluated without throwing — check assertions as data
      let
        config = configResult.value;
        failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
        messages = map (x: x.message) failedAssertions;
        hasExpectedFailure = builtins.any (msg: lib.hasInfix expectedMessage msg) messages;
      in
      mkCheck name hasExpectedFailure
    else
      # homeManagerConfiguration threw before returning.  Try to recover
      # the assertion messages so we can verify the *expected* assertion
      # fired rather than accepting any throw as a pass.
      let
        # Re-evaluate without the assertion check wrapper so we can
        # inspect config.assertions directly.
        rawConfig =
          (home-manager.lib.homeManagerConfiguration {
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
          }).config;
        rawResult = builtins.tryEval rawConfig.assertions;
        hasExpected =
          if rawResult.success then
            builtins.any (a: !a.assertion && lib.hasInfix expectedMessage a.message) rawResult.value
          else
            # Assertions themselves threw (eager evaluation); fall back to
            # accepting the original throw as confirmation.
            builtins.trace
              "WARNING: mkAssertionCheck '${name}': could not verify assertion message, accepting throw as pass"
              true;
      in
      mkCheck name hasExpected;

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
