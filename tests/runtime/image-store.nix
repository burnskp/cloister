{ pkgs }:
let
  imagePath =
    pkgs.runCommand "cloister-runtime-image-store.squashfs"
      {
        nativeBuildInputs = [ pkgs.squashfsTools ];
      }
      ''
        mkdir -p root
        printf '%s\n' '{"fixture":"runtime-image-store"}' > root/meta.json
        mksquashfs root "$out" -noappend -all-root >/dev/null
      '';

  metaPath = pkgs.writeText "cloister-runtime-image-store-meta.json" ''
    {"fixture":"runtime-image-store"}
  '';

  homeManagerCompat =
    { lib, ... }:
    {
      options.home-manager.users = lib.mkOption {
        type = lib.types.attrsOf lib.types.anything;
        default = { };
      };
    };
in
pkgs.testers.runNixOSTest (_: {
  name = "cloister-runtime-image-store";

  nodes.machine =
    { ... }:
    {
      virtualisation.cores = 2;

      imports = [
        homeManagerCompat
        ../../modules/cloister-image-store
      ];

      cloister.imageStore = {
        enable = true;
        interval = "hourly";
      };

      home-manager.users.alice.cloister._internal.imageStoreInfos = [
        {
          name = "runtime";
          storeId = "abc123";
          inherit imagePath metaPath;
          publishedImagePath = "/var/lib/cloister/images/abc123.squashfs";
          publishedMetaPath = "/var/lib/cloister/images/abc123.json";
          mountPath = "/run/cloister/images/abc123";
        }
      ];
    };

  testScript = ''
    ${builtins.readFile ./lib.py}

    start_all()

    machine.wait_for_unit("cloister-image-store-clean.timer")
    machine.wait_until_succeeds("mountpoint -q /run/cloister/images/abc123")

    machine.succeed("test -L /var/lib/cloister/images/abc123.squashfs")
    machine.succeed("test -L /var/lib/cloister/images/abc123.json")
    assert_contains(
        machine,
        "cat /run/cloister/images/abc123/meta.json",
        "runtime-image-store",
        "published image metadata contains fixture marker",
    )

    machine.succeed("touch /var/lib/cloister/images/stale.json")
    machine.succeed("mkdir -p /run/cloister/images/stale")
    machine.succeed("systemctl start cloister-image-store-clean.service")
    machine.fail("test -e /var/lib/cloister/images/stale.json")
    machine.fail("test -e /run/cloister/images/stale")
  '';
})
