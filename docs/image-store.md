# Image Store

`image-store` is Cloister's immutable `/nix/store` mode.

Instead of exposing the host store directly, Cloister computes the store paths a sandbox needs, builds a `squashfs` image for that closure during `nixos-rebuild`, publishes the image under `cloister.imageStore.base`, and mounts it read-only under `cloister.imageStore.mountBase`.

## Why use it

Use `image-store` when you want stricter store visibility than the host `/nix/store` provides and want that store view to be immutable at runtime.

Benefits:

- only the store paths Cloister knows about are visible inside the sandbox
- immutable image artifacts avoid mutable runtime cache poisoning
- identical closure sets share the same image and mount by store hash
- sandbox startup avoids `nix copy` or first-launch cache population work

## Basic usage

In Home Manager:

```nix
cloister.sandboxes.editor = {
  extraPackages = with pkgs; [
    helix
    nil
    lua-language-server
  ];

  sandbox = {
    nixStore.mode = "image-store";
  };
};
```

In NixOS:

```nix
{
  imports = [ inputs.cloister.nixosModules.cloister-image-store ];

  cloister.imageStore = {
    base = "/var/lib/cloister/images";
    mountBase = "/run/cloister/images";
  };
}
```

See `examples/image-store.nix` for a complete sandbox example.

## What gets copied

When `image-store` is enabled, Cloister builds the store input set from paths it already knows the sandbox needs, including:

- internally managed base packages and configured `extraPackages`
- shell and runtime helpers Cloister injects
- symlink targets inside the sandbox config
- bind sources that resolve into `/nix/store`
- environment variable values that contain `/nix/store/...`
- Home Manager-managed files whose sources live in the Nix store

## How it works

For each sandbox configuration in `image-store` mode:

1. Cloister computes a sorted, unique list of store roots.
1. It hashes that list to derive a stable store ID.
1. It builds a zstd level 10-compressed `squashfs` image with 1 MiB blocks containing `nix/store` plus metadata.
1. The NixOS `cloister-image-store` module publishes the image under `cloister.imageStore.base/<store-id>.squashfs`.
1. NixOS mounts that image read-only at `cloister.imageStore.mountBase/<store-id>`.
1. At launch, Cloister verifies the mountpoint, confirms it is read-only, and checks the embedded metadata before bind-mounting `<mount>/nix/store` to `/nix/store` inside the sandbox.

Multiple sandboxes with the same computed store roots reuse the same image and mounted path automatically.

## Cleanup

Published image links and empty mount directories can be cleaned periodically with:

```nix
cloister.imageStore = {
  enable = true;
  interval = "weekly";
};
```

The underlying image artifacts live in `/nix/store` and are still cleaned by normal Nix garbage collection.

By default, generated `squashfs` files use zstd compression level 10 with 1 MiB blocks. To build uncompressed images instead:

```nix
cloister.imageStore.compression.enable = false;
```

Known limitation: image-store validation now checks that the configured mount exists, is a real read-only mountpoint, and carries the expected metadata identity, but it still depends on the host mount existing correctly. If the `cloister-image-store` NixOS module is not enabled or the mount has not come up yet, sandbox launch will fail closed instead of silently falling back to the host store.

## When to prefer host mode

Prefer `sandbox.nixStore.mode = "host"` when:

- you do not care about limiting store visibility
- you want the sandbox to see the full host `/nix/store`
- you do not want to enable the NixOS `cloister-image-store` module
