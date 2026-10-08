{ lib, pkgs }:
let
  harnessModule = _: {
    options = {
      assertions = lib.mkOption {
        type = lib.types.listOf lib.types.anything;
        default = [ ];
        internal = true;
      };

      boot = {
        supportedFilesystems = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
        };

        kernel.sysctl = lib.mkOption {
          type = lib.types.attrsOf lib.types.anything;
          default = { };
        };
      };

      environment.systemPackages = lib.mkOption {
        type = lib.types.listOf lib.types.package;
        default = [ ];
      };

      home-manager.users = lib.mkOption {
        type = lib.types.attrsOf lib.types.anything;
        default = { };
      };

      networking.firewall.interfaces = lib.mkOption {
        type = lib.types.attrsOf lib.types.anything;
        default = { };
      };

      security.wrappers = lib.mkOption {
        type = lib.types.attrsOf lib.types.anything;
        default = { };
      };

      system.activationScripts = lib.mkOption {
        type = lib.types.attrsOf lib.types.anything;
        default = { };
      };

      systemd = {
        mounts = lib.mkOption {
          type = lib.types.listOf lib.types.anything;
          default = [ ];
        };

        services = lib.mkOption {
          type = lib.types.attrsOf lib.types.anything;
          default = { };
        };

        timers = lib.mkOption {
          type = lib.types.attrsOf lib.types.anything;
          default = { };
        };

        tmpfiles.rules = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
        };
      };

      users.groups = lib.mkOption {
        type = lib.types.attrsOf lib.types.anything;
        default = { };
      };
    };
  };
in
{
  imageStore =
    module:
    let
      result = lib.evalModules {
        modules = [
          harnessModule
          ../../modules/cloister-image-store
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
    };

  netns =
    module:
    let
      result = lib.evalModules {
        modules = [
          harnessModule
          ../../modules/cloister-netns
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
    };
}
