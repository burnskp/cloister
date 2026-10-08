# Sandbox Presets

Presets provide opinionated bundles of sandbox defaults under `cloister.sandboxes.<name>.preset`. They are defaults only: any explicit sandbox option you set still overrides the preset.

## Available presets

### `hardened`

Low-trust CLI sandbox for untrusted tools, AI agents, and unknown scripts.

- disables network, SSH agent forwarding, git config binds, D-Bus, GUI, and audio
- keeps host shell config disabled
- keeps the base sandbox protections such as seccomp enabled

```nix
cloister.sandboxes.untrusted = {
  preset = "hardened";
};
```

### `developer`

Trusted day-to-day development sandbox.

- enables host shell config
- enables network, git config binds, SSH agent forwarding, and D-Bus
- enables desktop notifications on D-Bus
- keeps GUI and audio off unless you opt in

```nix
cloister.sandboxes.dev = {
  preset = "developer";
  extraPackages = with pkgs; [ git ripgrep fd ];
};
```

### `gui`

General desktop-application sandbox.

- enables Wayland, network, D-Bus, and desktop notifications
- keeps host shell config, git config binds, and SSH agent forwarding off
- keeps audio off unless you opt in

```nix
cloister.sandboxes.viewer = {
  preset = "gui";
  defaultCommand = [ "evince" ];
};
```

### `chromium`

Browser-oriented preset for Chromium and similar apps.

- enables the `gui` baseline
- enables PipeWire
- enables `audio.pipewire.pulseOnly = true`
- enables filtered PipeWire audio with `audio.pipewire.enable = true`
- enables `sandbox.seccomp.allowChromiumSandbox = true`

```nix
cloister.sandboxes.browser = {
  preset = "chromium";
  defaultCommand = [ "chromium" ];
};
```

## Override behavior

Presets intentionally use defaults, not forced values. That means you can start from a preset and loosen or tighten individual settings as needed:

```nix
cloister.sandboxes.browser = {
  preset = "chromium";

  audio.pipewire.filters.audioIn = true;
  dbus.portal.fileChooser = true;
};
```

Likewise, you can start from `hardened` and opt one thing back in:

```nix
cloister.sandboxes.agent = {
  preset = "hardened";
  network.enable = true;
};
```

## Picking a preset

- Choose `hardened` for low-trust tools
- Choose `developer` for trusted daily development
- Choose `gui` for desktop apps that need Wayland and notifications
- Choose `chromium` for browsers and Chromium-family apps

See also:

- `examples/hardened.nix`
- `examples/developer.nix`
- `examples/gui.nix`
- `examples/chromium.nix`
