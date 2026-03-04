{ lib }:
let
  mkBindFlag =
    mode: try:
    assert mode == "ro" || mode == "rw";
    let
      base = if mode == "ro" then "ro-bind" else "bind";
    in
    if try then "--${base}-try" else "--${base}";

  mkListArgs =
    flag: items:
    lib.concatMap (item: [
      flag
      item
    ]) items;

  mkSymlinkArgs =
    symlinks:
    lib.concatMap (entry: [
      "--symlink"
      entry.target
      entry.link
    ]) symlinks;

  mkBindArgs =
    mode: binds:
    lib.concatMap (
      bind:
      let
        inherit (bind) src try;
        dest = if bind.dest != null then bind.dest else bind.src;
      in
      [
        (mkBindFlag mode try)
        src
        dest
      ]
    ) binds;

  mkEnvArgs =
    env:
    lib.concatMap (name: [
      "--setenv"
      name
      env.${name}
    ]) (lib.attrNames env);
in
{
  # Unquoted argument list for JSON serialization.
  # Produces plain strings without shell-level quoting — suitable for
  # passing to the compiled Rust binary via a JSON config file.
  mkBwrapArgs =
    {
      dirs ? [ ],
      tmpfs ? [ ],
      symlinks ? [ ],
      binds ? { },
      env ? { },
    }:
    let
      roBinds = binds.ro or [ ];
      rwBinds = binds.rw or [ ];
    in
    lib.concatLists [
      (mkListArgs "--dir" dirs)
      (mkListArgs "--tmpfs" tmpfs)
      (mkSymlinkArgs symlinks)
      (mkBindArgs "rw" rwBinds)
      (mkBindArgs "ro" roBinds)
      (mkEnvArgs env)
    ];
}
