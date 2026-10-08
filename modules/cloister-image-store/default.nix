{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.cloister.imageStore;

  hmUsers =
    if lib.hasAttrByPath [ "home-manager" "users" ] config then config.home-manager.users else { };

  rawImageInfos = lib.concatLists (
    lib.mapAttrsToList (
      _: userCfg:
      if userCfg ? cloister && userCfg.cloister ? _internal then
        userCfg.cloister._internal.imageStoreInfos
      else
        [ ]
    ) hmUsers
  );

  imageInfoMap = builtins.listToAttrs (
    map (info: {
      name = info.storeId;
      value = info;
    }) rawImageInfos
  );

  imageInfos = builtins.attrValues imageInfoMap;

  keepIds = map (info: info.storeId) imageInfos;
  keepIdsText = lib.concatStringsSep " " (map lib.escapeShellArg keepIds);

  cleanupScript = pkgs.writeShellScriptBin "cloister-image-store-clean" ''
    set -eu

    base=${lib.escapeShellArg cfg.base}
    mount_base=${lib.escapeShellArg cfg.mountBase}
    keep_ids=" ${keepIdsText} "

    mkdir -p "$base" "$mount_base"

    for entry in "$base"/*; do
      [ -e "$entry" ] || continue
      name=$(basename "$entry")
      case "$name" in
        *.squashfs) id=''${name%.squashfs} ;;
        *.json) id=''${name%.json} ;;
        *) continue ;;
      esac
      case "$keep_ids" in
        *" $id "*) ;;
        *) rm -f "$entry" ;;
      esac
    done

    for entry in "$mount_base"/*; do
      [ -e "$entry" ] || continue
      id=$(basename "$entry")
      case "$keep_ids" in
        *" $id "*) ;;
        *)
          if ! ${pkgs.util-linux}/bin/mountpoint -q "$entry" 2>/dev/null; then
            rmdir "$entry" 2>/dev/null || true
          fi
          ;;
      esac
    done
  '';
in
{
  options.cloister.imageStore = {
    base = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/cloister/images";
      description = "Global directory where Cloister publishes immutable store images and metadata.";
    };

    mountBase = lib.mkOption {
      type = lib.types.str;
      default = "/run/cloister/images";
      description = "Global directory where Cloister mounts immutable store images by store hash.";
    };

    enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Enable periodic cleanup for Cloister image-store links and mount directories.";
    };

    interval = lib.mkOption {
      type = lib.types.str;
      default = "weekly";
      description = "systemd OnCalendar expression for Cloister image-store cleanup.";
    };

    compression.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Compress generated image-store squashfs files with zstd level 10 and 1 MiB blocks.
        Disable this to build uncompressed squashfs images.
      '';
    };
  };

  config = lib.mkMerge [
    {
      boot.supportedFilesystems = [ "squashfs" ];
    }
    (lib.mkIf (imageInfos != [ ]) {
      assertions = [
        {
          assertion = lib.hasAttrByPath [ "home-manager" "users" ] config;
          message = "cloister-image-store: requires the Home Manager NixOS module so sandbox image metadata can be collected from home-manager.users.";
        }
      ];

      environment.systemPackages = [ cleanupScript ];

      systemd = {
        tmpfiles.rules = [
          "d ${cfg.base} 0755 root root -"
          "d ${cfg.mountBase} 0755 root root -"
        ];

        mounts = map (info: {
          description = "Cloister image store ${info.storeId}";
          what = info.publishedImagePath;
          where = info.mountPath;
          type = "squashfs";
          options = "loop,ro,nodev,nosuid";
          wantedBy = [ "multi-user.target" ];
        }) imageInfos;

        services.cloister-image-store-clean = lib.mkIf cfg.enable {
          description = "Cloister image-store cleanup";
          serviceConfig = {
            Type = "oneshot";
            ExecStart = "${cleanupScript}/bin/cloister-image-store-clean";
          };
        };

        timers.cloister-image-store-clean = lib.mkIf cfg.enable {
          description = "Cloister image-store cleanup timer";
          wantedBy = [ "timers.target" ];
          timerConfig = {
            OnCalendar = cfg.interval;
            Persistent = true;
          };
        };
      };

      system.activationScripts.cloisterImageStore = lib.stringAfter [ "users" ] ''
        mkdir -p ${lib.escapeShellArg cfg.base} ${lib.escapeShellArg cfg.mountBase}
        ${lib.concatMapStringsSep "\n" (info: ''
          ln -sfn ${lib.escapeShellArg info.imagePath} ${lib.escapeShellArg info.publishedImagePath}
          ln -sfn ${lib.escapeShellArg info.metaPath} ${lib.escapeShellArg info.publishedMetaPath}
          mkdir -p ${lib.escapeShellArg info.mountPath}
        '') imageInfos}
        ${cleanupScript}/bin/cloister-image-store-clean
      '';
    })
  ];
}
