{ testLib }:
let
  inherit (testLib) mkCheck lib;

  bwrapLib = import ../modules/cloister/_bwrap.nix { inherit lib; };
  inherit (bwrapLib) mkBwrapArgs;
in
{
  # Empty input produces empty output
  empty-args = mkCheck "bwrap-empty-args" (mkBwrapArgs { } == [ ]);

  # --dir arguments
  dirs = mkCheck "bwrap-dirs" (
    mkBwrapArgs {
      dirs = [
        "/var"
        "/run"
      ];
    } == [
      "--dir"
      "/var"
      "--dir"
      "/run"
    ]
  );

  # --tmpfs arguments
  tmpfs = mkCheck "bwrap-tmpfs" (
    mkBwrapArgs { tmpfs = [ "/tmp" ]; } == [
      "--tmpfs"
      "/tmp"
    ]
  );

  # --symlink arguments
  symlinks = mkCheck "bwrap-symlinks" (
    mkBwrapArgs {
      symlinks = [
        {
          target = "t";
          link = "l";
        }
      ];
    } == [
      "--symlink"
      "t"
      "l"
    ]
  );

  # --ro-bind (try=false)
  ro-bind = mkCheck "bwrap-ro-bind" (
    mkBwrapArgs {
      binds.ro = [
        {
          src = "/nix";
          dest = null;
          try = false;
        }
      ];
    } == [
      "--ro-bind"
      "/nix"
      "/nix"
    ]
  );

  # --ro-bind-try (try=true)
  ro-bind-try = mkCheck "bwrap-ro-bind-try" (
    mkBwrapArgs {
      binds.ro = [
        {
          src = "/etc/shells";
          dest = null;
          try = true;
        }
      ];
    } == [
      "--ro-bind-try"
      "/etc/shells"
      "/etc/shells"
    ]
  );

  # --bind (rw, try=false)
  rw-bind = mkCheck "bwrap-rw-bind" (
    mkBwrapArgs {
      binds.rw = [
        {
          src = "/rw";
          dest = null;
          try = false;
        }
      ];
    } == [
      "--bind"
      "/rw"
      "/rw"
    ]
  );

  # --bind-try (rw, try=true)
  rw-bind-try = mkCheck "bwrap-rw-bind-try" (
    mkBwrapArgs {
      binds.rw = [
        {
          src = "/rw";
          dest = null;
          try = true;
        }
      ];
    } == [
      "--bind-try"
      "/rw"
      "/rw"
    ]
  );

  # Explicit dest overrides src for second argument
  explicit-dest = mkCheck "bwrap-explicit-dest" (
    mkBwrapArgs {
      binds.ro = [
        {
          src = "/nix/store/abc";
          dest = "/etc/ssl/certs/ca-bundle.crt";
          try = false;
        }
      ];
    } == [
      "--ro-bind"
      "/nix/store/abc"
      "/etc/ssl/certs/ca-bundle.crt"
    ]
  );

  # --setenv arguments
  env-vars = mkCheck "bwrap-env-vars" (
    mkBwrapArgs {
      env = {
        HOME = "$HOME";
      };
    } == [
      "--setenv"
      "HOME"
      "$HOME"
    ]
  );

  # env vars are ordered alphabetically (lib.attrNames sorts)
  env-ordering = mkCheck "bwrap-env-ordering" (
    let
      result = mkBwrapArgs {
        env = {
          ZZZ = "z";
          AAA = "a";
        };
      };
    in
    result == [
      "--setenv"
      "AAA"
      "a"
      "--setenv"
      "ZZZ"
      "z"
    ]
  );

  # Combined ordering: dirs → tmpfs → symlinks → rw-binds → ro-binds → env
  combined-ordering = mkCheck "bwrap-combined-ordering" (
    let
      result = mkBwrapArgs {
        dirs = [ "/var" ];
        tmpfs = [ "/tmp" ];
        symlinks = [
          {
            target = "t";
            link = "l";
          }
        ];
        binds = {
          rw = [
            {
              src = "/rw";
              dest = null;
              try = false;
            }
          ];
          ro = [
            {
              src = "/ro";
              dest = null;
              try = false;
            }
          ];
        };
        env = {
          FOO = "bar";
        };
      };
    in
    result == [
      # dirs
      "--dir"
      "/var"
      # tmpfs
      "--tmpfs"
      "/tmp"
      # symlinks
      "--symlink"
      "t"
      "l"
      # rw binds (before ro)
      "--bind"
      "/rw"
      "/rw"
      # ro binds
      "--ro-bind"
      "/ro"
      "/ro"
      # env
      "--setenv"
      "FOO"
      "bar"
    ]
  );

  # Backslashes in paths are preserved as-is (no double-escaping)
  backslash-in-path = mkCheck "bwrap-backslash-in-path" (
    mkBwrapArgs { dirs = [ "/path\\with\\backslashes" ]; } == [
      "--dir"
      "/path\\with\\backslashes"
    ]
  );

  # Double quotes in paths are preserved as-is
  quote-in-path = mkCheck "bwrap-quote-in-path" (
    mkBwrapArgs { dirs = [ ''/path"with"quotes'' ]; } == [
      "--dir"
      ''/path"with"quotes''
    ]
  );
}
