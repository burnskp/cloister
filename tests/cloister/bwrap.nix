{
  checks,
  lib,
  pkgs,
  ...
}:
let
  bwrapLib = import ../../modules/cloister/_bwrap.nix { inherit lib; };
  subsetPidPackage = pkgs.callPackage ../../pkgs/bubblewrap-subset-pid { };

  # Exercise the package's actual source and patch list without compiling it.
  patchedSource = pkgs.applyPatches {
    name = "bubblewrap-subset-pid-patched-source";
    inherit (subsetPidPackage) src;
    patches = subsetPidPackage.patches or [ ];
    prePatch = subsetPidPackage.prePatch or "";
    postPatch = subsetPidPackage.postPatch or "";
  };

  # Cover both upstream layouts without fetching another Bubblewrap release.
  patchedLayouts = pkgs.applyPatches {
    name = "bubblewrap-subset-pid-source-layouts";
    src = pkgs.runCommand "bubblewrap-source-layouts" { } ''
      mkdir -p "$out"
      cat > "$out/bubblewrap.c" <<'EOF'
      if (mount ("proc", arg1, "proc", MS_NOSUID | MS_NOEXEC | MS_NODEV, NULL) != 0)
        die_with_mount_error ("Can't mount proc on %s", arg1);
      if (mount ("proc", dest_path, "proc", MS_NOSUID | MS_NOEXEC | MS_NODEV, NULL) != 0)
        die_with_mount_error ("Can't mount proc on %s", op->dest);
      EOF
    '';
    inherit
      (
        (pkgs.callPackage ../../pkgs/bubblewrap-subset-pid {
          bubblewrap = pkgs.bubblewrap.overrideAttrs { postPatch = ""; };
        })
      )
      postPatch
      ;
  };

  emptyArgs = bwrapLib.mkBwrapArgs { };

  renderedArgs = bwrapLib.mkBwrapArgs {
    dirs = [ "/sandbox" ];
    tmpfs = [ "/tmp" ];
    symlinks = [
      {
        target = "/nix/store/source";
        link = "/link";
      }
    ];
    binds = {
      rw = [
        {
          src = "/rw-src";
          dest = "/rw-dest";
          try = false;
        }
        {
          src = "/rw-optional";
          dest = null;
          try = true;
        }
      ];
      ro = [
        {
          src = ''/path\with\slashes'';
          dest = "/dest\"quote";
          try = false;
        }
        {
          src = "/ro-optional";
          dest = "/opt";
          try = true;
        }
      ];
    };
    env = {
      Z_VAR = "z";
      A_VAR = "a";
    };
  };
in
(checks.mkCheck "test-cloister-bwrap" [
  (checks.expectEq "empty args stay empty" [ ] emptyArgs)
  (checks.expectEq "bubblewrap args render in stable order" [
    "--dir"
    "/sandbox"
    "--tmpfs"
    "/tmp"
    "--symlink"
    "/nix/store/source"
    "/link"
    "--bind"
    "/rw-src"
    "/rw-dest"
    "--bind-try"
    "/rw-optional"
    "/rw-optional"
    "--ro-bind"
    ''/path\with\slashes''
    "/dest\"quote"
    "--ro-bind-try"
    "/ro-optional"
    "/opt"
    "--setenv"
    "A_VAR"
    "a"
    "--setenv"
    "Z_VAR"
    "z"
  ] renderedArgs)
]).overrideAttrs
  (old: {
    buildCommand = ''
      grep -F '"proc", MS_NOSUID | MS_NOEXEC | MS_NODEV, "subset=pid")' \
        ${patchedSource}/bubblewrap.c
      for dest in arg1 dest_path; do
        grep -F "mount (\"proc\", $dest, \"proc\", MS_NOSUID | MS_NOEXEC | MS_NODEV, \"subset=pid\")" \
          ${patchedLayouts}/bubblewrap.c
      done
    ''
    + old.buildCommand;
  })
