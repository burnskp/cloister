{
  lib,
  pkgs,
}:
let
  checks = import ./lib/checks.nix { inherit lib pkgs; };
  hm = import ./lib/eval-home-manager.nix { inherit lib pkgs; };
  nixos = import ./lib/eval-nixos.nix { inherit lib pkgs; };

  evalChecks = {
    "test-cloister-bwrap" = import ./cloister/bwrap.nix {
      inherit checks lib pkgs;
    };

    "test-cloister-examples" = import ./cloister/examples.nix {
      inherit
        checks
        hm
        lib
        pkgs
        ;
    };

    "test-cloister-registry" = import ./cloister/registry.nix {
      inherit
        checks
        hm
        lib
        pkgs
        ;
    };

    "test-cloister-presets" = import ./cloister/presets.nix {
      inherit
        checks
        hm
        lib
        pkgs
        ;
    };

    "test-cloister-sandbox-core" = import ./cloister/sandbox-core.nix {
      inherit
        checks
        hm
        lib
        pkgs
        ;
    };

    "test-cloister-gui-dbus-audio" = import ./cloister/gui-dbus-audio.nix {
      inherit
        checks
        hm
        lib
        pkgs
        ;
    };

    "test-cloister-rendered-config" = import ./cloister/rendered-config.nix {
      inherit
        checks
        hm
        lib
        pkgs
        ;
    };

    "test-cloister-worker-broker" = import ./cloister/worker-broker.nix {
      inherit
        checks
        hm
        lib
        pkgs
        ;
    };

    "test-cloister-image-store" = import ./cloister-image-store/default.nix {
      inherit
        checks
        lib
        nixos
        pkgs
        ;
    };

    "test-cloister-netns" = import ./cloister-netns/default.nix {
      inherit
        checks
        lib
        nixos
        pkgs
        ;
    };
  };

  runtimeChecks = {
    "test-runtime-sandbox-core" = import ./runtime/sandbox-core.nix {
      inherit pkgs;
    };

    "test-runtime-worker-broker" = import ./runtime/worker-broker.nix {
      inherit pkgs;
    };

    "test-runtime-gui-dbus-audio" = import ./runtime/gui-dbus-audio.nix {
      inherit pkgs;
    };

    "test-runtime-image-store" = import ./runtime/image-store.nix {
      inherit pkgs;
    };

    "test-runtime-netns" = import ./runtime/netns.nix {
      inherit pkgs;
    };
  };
in
evalChecks
// {
  tests-runtime = pkgs.linkFarm "cloister-tests-runtime" (
    lib.mapAttrsToList (name: path: {
      inherit name path;
    }) runtimeChecks
  );
}
// runtimeChecks
