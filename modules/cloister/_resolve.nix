# Managed file resolution: converts managedFile keys into bind mount specs
# by looking them up in xdg.configFile and home.file.
{
  lib,
  config,
  configHome,
}:
{
  resolveConfigEntry =
    key:
    let
      hasExact = config.xdg.configFile ? ${key};
      prefixEntries = lib.filterAttrs (name: _: lib.hasPrefix "${key}/" name) config.xdg.configFile;
      absKey = "${configHome}/${key}";
      hasHome = config.home.file ? ${absKey};
      homePrefixEntries = lib.filterAttrs (name: _: lib.hasPrefix "${absKey}/" name) config.home.file;
      # Direct home.file lookup (for entries outside ~/.config/)
      hasDirectHome = config.home.file ? ${key};
      directHomePrefixEntries = lib.filterAttrs (name: _: lib.hasPrefix "${key}/" name) config.home.file;
    in
    if hasExact then
      [
        {
          src = toString config.xdg.configFile.${key}.source;
          dest = "$HOME/.config/${key}";
          try = false;
        }
      ]
    else if prefixEntries != { } then
      lib.mapAttrsToList (name: entry: {
        src = toString entry.source;
        dest = "$HOME/.config/${name}";
        try = false;
      }) prefixEntries
    else if hasHome then
      [
        {
          src = toString config.home.file.${absKey}.source;
          dest = "$HOME/.config/${key}";
          try = false;
        }
      ]
    else if homePrefixEntries != { } then
      lib.mapAttrsToList (name: entry: {
        src = toString entry.source;
        dest = "$HOME/.config/${lib.removePrefix "${configHome}/" name}";
        try = false;
      }) homePrefixEntries
    else if hasDirectHome then
      [
        {
          src = toString config.home.file.${key}.source;
          dest = "$HOME/${key}";
          try = false;
        }
      ]
    else if directHomePrefixEntries != { } then
      lib.mapAttrsToList (name: entry: {
        src = toString entry.source;
        dest = "$HOME/${name}";
        try = false;
      }) directHomePrefixEntries
    else
      throw "cloister.sandbox.extraBinds.managedFile: '${key}' not found in xdg.configFile or home.file";
}
