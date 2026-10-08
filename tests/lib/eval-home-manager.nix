{ lib, pkgs }:
let
  fileSubmodule = lib.types.submodule {
    options = {
      source = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
      };

      text = lib.mkOption {
        type = lib.types.lines;
        default = "";
      };

      target = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
      };

      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
      };
    };
  };

  harnessModule = _: {
    options = {
      assertions = lib.mkOption {
        type = lib.types.listOf lib.types.anything;
        default = [ ];
        internal = true;
      };

      home = {
        username = lib.mkOption {
          type = lib.types.str;
          default = "tester";
        };

        homeDirectory = lib.mkOption {
          type = lib.types.str;
          default = "/home/tester";
        };

        packages = lib.mkOption {
          type = lib.types.listOf lib.types.package;
          default = [ ];
        };

        stateVersion = lib.mkOption {
          type = lib.types.str;
          default = "25.05";
        };

        file = lib.mkOption {
          type = lib.types.attrsOf fileSubmodule;
          default = { };
        };
      };

      xdg = {
        configHome = lib.mkOption {
          type = lib.types.str;
          default = "/home/tester/.config";
        };

        stateHome = lib.mkOption {
          type = lib.types.str;
          default = "/home/tester/.local/state";
        };

        cacheHome = lib.mkOption {
          type = lib.types.str;
          default = "/home/tester/.cache";
        };

        dataHome = lib.mkOption {
          type = lib.types.str;
          default = "/home/tester/.local/share";
        };

        configFile = lib.mkOption {
          type = lib.types.attrsOf fileSubmodule;
          default = { };
        };

        desktopEntries = lib.mkOption {
          type = lib.types.attrsOf lib.types.anything;
          default = { };
        };
      };

      programs = {
        zsh.initContent = lib.mkOption {
          type = lib.types.lines;
          default = "";
        };

        bash.initExtra = lib.mkOption {
          type = lib.types.lines;
          default = "";
        };
      };

      systemd.user = {
        services = lib.mkOption {
          type = lib.types.attrsOf lib.types.anything;
          default = { };
        };

        timers = lib.mkOption {
          type = lib.types.attrsOf lib.types.anything;
          default = { };
        };

        sockets = lib.mkOption {
          type = lib.types.attrsOf lib.types.anything;
          default = { };
        };

        tmpfiles.rules = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
        };
      };
    };
  };
in
module:
let
  result = lib.evalModules {
    modules = [
      harnessModule
      ../../modules/cloister
      module
    ];
    specialArgs = {
      inherit pkgs;
    };
  };

  checkedAssertions = builtins.deepSeq (map (
    assertion: if assertion.assertion then true else throw assertion.message
  ) result.config.assertions) true;
in
result
// {
  inherit (result.config) assertions;
  config = builtins.deepSeq checkedAssertions result.config;
}
