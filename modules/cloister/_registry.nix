{ config, lib, ... }:
let
  cfg = config.cloister;
  patterns = import ./_patterns.nix;

  # --- Per-sandbox registry assertions ---
  mkRegistryAssertions =
    name: sCfg:
    let
      regCfg = sCfg.registry;
      allCommands = regCfg.commands ++ regCfg.interactiveCommands;
      aliasNames = lib.attrNames regCfg.aliases;
      functionNames = lib.attrNames regCfg.functions;
      wrappableAliasNames = lib.filter (n: !builtins.elem n regCfg.noWrap) aliasNames;

      safeAliasPattern = patterns.safeAlias;
      safeFunctionPattern = patterns.safeFunction;
      safeCommandPattern = patterns.safeCommand;
      argvAliasPattern = "^[A-Za-z0-9_@%+=:,./#-]+( [A-Za-z0-9_@%+=:,./#-]+)*$";

      invalidAliasNames = builtins.filter (n: builtins.match safeAliasPattern n == null) aliasNames;
      invalidFunctionNames = builtins.filter (
        n: builtins.match safeFunctionPattern n == null
      ) functionNames;
      invalidCommandNames = builtins.filter (n: builtins.match safeCommandPattern n == null) allCommands;
      invalidOutsideAliasValues = builtins.filter (
        n: builtins.match argvAliasPattern regCfg.aliases.${n} == null
      ) wrappableAliasNames;

      aliasCommandOverlap = lib.intersectLists aliasNames allCommands;
      functionCommandOverlap = lib.intersectLists functionNames allCommands;
      aliasFunctionOverlap = lib.intersectLists aliasNames functionNames;
      commandModeOverlap = lib.intersectLists regCfg.commands regCfg.interactiveCommands;
    in
    [
      {
        assertion = invalidAliasNames == [ ];
        message = "cloister.sandboxes.${name}.registry: alias names must match ${safeAliasPattern}: ${lib.concatStringsSep ", " invalidAliasNames}";
      }
      {
        assertion = invalidFunctionNames == [ ];
        message = "cloister.sandboxes.${name}.registry: function names must match ${safeFunctionPattern}: ${lib.concatStringsSep ", " invalidFunctionNames}";
      }
      {
        assertion = invalidCommandNames == [ ];
        message = "cloister.sandboxes.${name}.registry: command names must match ${safeCommandPattern}: ${lib.concatStringsSep ", " invalidCommandNames}";
      }
      {
        assertion = aliasFunctionOverlap == [ ];
        message = "cloister.sandboxes.${name}.registry: names defined as both alias and function: ${lib.concatStringsSep ", " aliasFunctionOverlap}";
      }
      {
        assertion = aliasCommandOverlap == [ ];
        message = "cloister.sandboxes.${name}.registry: names defined as both alias and command: ${lib.concatStringsSep ", " aliasCommandOverlap}";
      }
      {
        assertion = functionCommandOverlap == [ ];
        message = "cloister.sandboxes.${name}.registry: names defined as both function and command: ${lib.concatStringsSep ", " functionCommandOverlap}";
      }
      {
        assertion = commandModeOverlap == [ ];
        message = "cloister.sandboxes.${name}.registry: names defined as both command and interactiveCommand: ${lib.concatStringsSep ", " commandModeOverlap}";
      }
      {
        assertion = invalidOutsideAliasValues == [ ];
        message = "cloister.sandboxes.${name}.registry: aliases wrapped outside the sandbox must be argv-safe (letters, numbers, /, ., _, -, and space separators only): ${lib.concatStringsSep ", " invalidOutsideAliasValues}. Use registry.functions for shell syntax, quoting, pipes, redirects, variable expansion, or put the alias in registry.noWrap.";
      }
    ];

  # --- Cross-sandbox name collision detection ---
  getOutsideNames =
    _name: sCfg:
    let
      regCfg = sCfg.registry;
      allCommands = regCfg.commands ++ regCfg.interactiveCommands;
      aliasNames = lib.attrNames regCfg.aliases;
      functionNames = lib.attrNames regCfg.functions;

      wrappableAliasNames = lib.filter (n: !builtins.elem n regCfg.noWrap) aliasNames;
      wrappableCommands = lib.filter (cmd: !builtins.elem cmd regCfg.noWrap) allCommands;
      wrappableFunctions = lib.filter (n: !builtins.elem n regCfg.noWrap) functionNames;
    in
    wrappableAliasNames ++ wrappableCommands ++ wrappableFunctions;

  allOutsideNamesWithSource = lib.concatLists (
    lib.mapAttrsToList (
      name: sCfg:
      map (n: {
        name = n;
        sandbox = name;
      }) (getOutsideNames name sCfg)
    ) cfg.sandboxes
  );

  outsideNameGroups = builtins.groupBy (x: x.name) allOutsideNamesWithSource;

  duplicateOutsideNames = lib.filterAttrs (_: v: builtins.length v > 1) outsideNameGroups;

  duplicateOutsideNamesList = lib.mapAttrsToList (
    name: entries: "${name} (from: ${lib.concatMapStringsSep ", " (e: e.sandbox) entries})"
  ) duplicateOutsideNames;

  sandboxNamePattern = patterns.sandboxName;
  invalidSandboxNames = builtins.filter (name: builtins.match sandboxNamePattern name == null) (
    lib.attrNames cfg.sandboxes
  );

in
{
  config = lib.mkIf cfg.enable {
    assertions = lib.concatLists (lib.mapAttrsToList mkRegistryAssertions cfg.sandboxes) ++ [
      {
        assertion = invalidSandboxNames == [ ];
        message = "cloister: sandbox names must match ${sandboxNamePattern}: ${lib.concatStringsSep ", " invalidSandboxNames}";
      }
      {
        assertion = duplicateOutsideNames == { };
        message = "cloister: cross-sandbox name collision — multiple sandboxes wrap the same name outside: ${lib.concatStringsSep "; " duplicateOutsideNamesList}";
      }
    ];
    # Registry rendering is computed inside the submodule config in _options.nix
  };
}
