{ config, lib, ... }:
let
  cfg = config.cloister;
  patterns = import ./_patterns.nix;

  # --- Per-sandbox registry assertions ---
  mkRegistryAssertions =
    name: sCfg:
    let
      regCfg = sCfg.registry;
      allCommands = regCfg.commands ++ regCfg.extraCommands;
      aliasNames = lib.attrNames regCfg.aliases;
      functionNames = lib.attrNames regCfg.functions;

      safeAliasPattern = patterns.safeAlias;
      safeFunctionPattern = patterns.safeFunction;
      safeCommandPattern = patterns.safeCommand;

      invalidAliasNames = builtins.filter (n: builtins.match safeAliasPattern n == null) aliasNames;
      invalidFunctionNames = builtins.filter (
        n: builtins.match safeFunctionPattern n == null
      ) functionNames;
      invalidCommandNames = builtins.filter (n: builtins.match safeCommandPattern n == null) allCommands;

      aliasCommandOverlap = lib.intersectLists aliasNames allCommands;
      functionCommandOverlap = lib.intersectLists functionNames allCommands;
      aliasFunctionOverlap = lib.intersectLists aliasNames functionNames;
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
    ];

  # --- Cross-sandbox name collision detection ---
  getOutsideNames =
    _name: sCfg:
    let
      regCfg = sCfg.registry;
      allCommands = regCfg.commands ++ regCfg.extraCommands;
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
