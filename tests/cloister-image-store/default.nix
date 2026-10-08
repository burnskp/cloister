{
  checks,
  nixos,
  ...
}:
let
  eval = nixos.imageStore {
    cloister.imageStore = {
      enable = true;
      interval = "daily";
    };

    home-manager.users.alice.cloister._internal.imageStoreInfos = [
      {
        name = "dev";
        storeId = "abc123";
        imagePath = "/nix/store/abc123-image.squashfs";
        metaPath = "/nix/store/abc123-meta.json";
        publishedImagePath = "/var/lib/cloister/images/abc123.squashfs";
        publishedMetaPath = "/var/lib/cloister/images/abc123.json";
        mountPath = "/run/cloister/images/abc123";
      }
    ];
  };

  customBaseEval = nixos.imageStore {
    cloister.imageStore = {
      enable = true;
      base = "/srv/cloister/images";
      mountBase = "/mnt/cloister/images";
      interval = "hourly";
    };

    home-manager.users.alice.cloister._internal.imageStoreInfos = [
      {
        name = "dev";
        storeId = "abc123";
        imagePath = "/nix/store/abc123-image.squashfs";
        metaPath = "/nix/store/abc123-meta.json";
        publishedImagePath = "/srv/cloister/images/abc123.squashfs";
        publishedMetaPath = "/srv/cloister/images/abc123.json";
        mountPath = "/mnt/cloister/images/abc123";
      }
    ];
  };

  disabledCleanupEval = nixos.imageStore {
    cloister.imageStore = {
      enable = false;
      base = "/srv/cloister/images";
      mountBase = "/mnt/cloister/images";
    };

    home-manager.users.alice.cloister._internal.imageStoreInfos = [
      {
        name = "dev";
        storeId = "abc123";
        imagePath = "/nix/store/abc123-image.squashfs";
        metaPath = "/nix/store/abc123-meta.json";
        publishedImagePath = "/srv/cloister/images/abc123.squashfs";
        publishedMetaPath = "/srv/cloister/images/abc123.json";
        mountPath = "/mnt/cloister/images/abc123";
      }
    ];
  };

  emptyInfosEval = nixos.imageStore {
    cloister.imageStore = {
      enable = true;
      base = "/srv/cloister/images";
      mountBase = "/mnt/cloister/images";
    };

    home-manager.users.alice.cloister._internal.imageStoreInfos = [ ];
  };

  duplicateInfoEval = nixos.imageStore {
    cloister.imageStore.enable = true;

    home-manager.users.alice.cloister._internal.imageStoreInfos = [
      {
        name = "dev";
        storeId = "abc123";
        imagePath = "/nix/store/abc123-image.squashfs";
        metaPath = "/nix/store/abc123-meta.json";
        publishedImagePath = "/var/lib/cloister/images/abc123.squashfs";
        publishedMetaPath = "/var/lib/cloister/images/abc123.json";
        mountPath = "/run/cloister/images/abc123";
      }
      {
        name = "dev-copy";
        storeId = "abc123";
        imagePath = "/nix/store/other-image.squashfs";
        metaPath = "/nix/store/other-meta.json";
        publishedImagePath = "/var/lib/cloister/images/abc123.squashfs";
        publishedMetaPath = "/var/lib/cloister/images/abc123.json";
        mountPath = "/run/cloister/images/abc123";
      }
    ];
  };

  mount = builtins.head eval.config.systemd.mounts;
  cleanupService = eval.config.systemd.services.cloister-image-store-clean;
  customMount = builtins.head customBaseEval.config.systemd.mounts;
  customCleanup = customBaseEval.config.systemd.services.cloister-image-store-clean;
  duplicateMounts = duplicateInfoEval.config.systemd.mounts;
in
checks.mkCheck "test-cloister-image-store" [
  (checks.expectContains "image store enables squashfs" "squashfs" (
    builtins.toJSON eval.config.boot.supportedFilesystems
  ))
  (checks.expectEq "image store mount path" "/run/cloister/images/abc123" mount.where)
  (checks.expectEq "image store mount options are hardened" "loop,ro,nodev,nosuid" mount.options)
  (checks.expectEq "image store mount remains tied to multi-user target" [
    "multi-user.target"
  ] mount.wantedBy)
  (checks.expectEq "image store mount no longer blocks multi-user target" [ ] (mount.before or [ ]))
  (checks.expectEq "cleanup timer calendar" "daily"
    eval.config.systemd.timers.cloister-image-store-clean.timerConfig.OnCalendar
  )
  (checks.expectEq "cleanup timer is persistent" true
    eval.config.systemd.timers.cloister-image-store-clean.timerConfig.Persistent
  )
  (checks.expectEq "image store compression defaults enabled" true
    eval.config.cloister.imageStore.compression.enable
  )
  (checks.expectContains "cleanup service uses cleaner binary" "/bin/cloister-image-store-clean"
    cleanupService.serviceConfig.ExecStart
  )
  (checks.expectContains "tmpfiles creates image base dir"
    "d /var/lib/cloister/images 0755 root root -"
    (builtins.toJSON eval.config.systemd.tmpfiles.rules)
  )
  (checks.expectContains "tmpfiles creates mount base dir" "d /run/cloister/images 0755 root root -" (
    builtins.toJSON eval.config.systemd.tmpfiles.rules
  ))
  (checks.expectContains "activation script publishes image" "abc123.squashfs"
    eval.config.system.activationScripts.cloisterImageStore.text
  )
  (checks.expectContains "activation script publishes metadata" "abc123.json"
    eval.config.system.activationScripts.cloisterImageStore.text
  )
  (checks.expectContains "activation script runs cleanup pass" "cloister-image-store-clean"
    eval.config.system.activationScripts.cloisterImageStore.text
  )
  (checks.expectContains "cleanup service points at generated cleaner" "cloister-image-store-clean"
    cleanupService.serviceConfig.ExecStart
  )
  (checks.expectEq "custom base changes mount path" "/mnt/cloister/images/abc123" customMount.where)
  (checks.expectEq "custom interval is rendered" "hourly"
    customBaseEval.config.systemd.timers.cloister-image-store-clean.timerConfig.OnCalendar
  )
  (checks.expectContains "custom tmpfiles use overridden base"
    "d /srv/cloister/images 0755 root root -"
    (builtins.toJSON customBaseEval.config.systemd.tmpfiles.rules)
  )
  (checks.expectContains "custom tmpfiles use overridden mount base"
    "d /mnt/cloister/images 0755 root root -"
    (builtins.toJSON customBaseEval.config.systemd.tmpfiles.rules)
  )
  (checks.expectContains "custom activation script uses overridden base" "/srv/cloister/images"
    customBaseEval.config.system.activationScripts.cloisterImageStore.text
  )
  (checks.expectContains "custom cleanup service still uses cleaner binary"
    "cloister-image-store-clean"
    customCleanup.serviceConfig.ExecStart
  )
  (checks.expectFalse "cleanup service is omitted when disabled" (
    disabledCleanupEval.config.systemd.services ? "cloister-image-store-clean"
  ))
  (checks.expectFalse "cleanup timer is omitted when disabled" (
    disabledCleanupEval.config.systemd.timers ? "cloister-image-store-clean"
  ))
  (checks.expectEq "image store compression can be disabled" false
    (nixos.imageStore { cloister.imageStore.compression.enable = false; })
    .config.cloister.imageStore.compression.enable
  )
  (checks.expectEq "no image infos means no mounts" [ ] emptyInfosEval.config.systemd.mounts)
  (checks.expectFalse "no image infos means no activation script" (
    emptyInfosEval.config.system.activationScripts ? "cloisterImageStore"
  ))
  (checks.expectEq "duplicate store ids deduplicate mount entries" 1 (
    builtins.length duplicateMounts
  ))
  (checks.expectFalse "no image infos means no cleanup package" (
    emptyInfosEval.config.environment.systemPackages != [ ]
  ))
]
