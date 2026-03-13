# PipeWire Filtering

When `audio.pipewire.enable = true` is set, the sandbox receives unrestricted access to the PipeWire graph — all audio devices, cameras, and metadata are visible and writable.

Enabling **PipeWire filters** (`audio.pipewire.filters.enable = true`) restricts that access. Cloister provisions a dedicated PipeWire socket and generates WirePlumber Lua policies so the sandbox can only see and interact with the device classes you explicitly allow.

## Configuration

With `filters.enable = true`, the sandbox starts with `audioOut` enabled and everything else is opt-in. The policy keeps the client on a link-only baseline, exposes only the configured sink/source classes, and grants the minimum internal PipeWire objects needed to create playback and capture streams:

```nix
cloister.sandboxes.zoom = {
  audio.pipewire = {
    enable = true;
    filters = {
      enable = true;
      audioOut = true;  # speakers (default)
      audioIn = true;   # microphones
      videoIn = true;   # webcams / V4L2
      control = false;  # volume / mute changes
      routing = false;  # default device / stream routing
    };
  };
  # videoIn still requires the physical /dev/video* nodes:
  video.enable = true;
};
```

### Discovery toggles

These control which `media.class` types are visible in the PipeWire registry.

| Toggle | media.class | Default | Notes |
|--------|------------|---------|-------|
| `audioOut` | `Audio/Sink` | `true` | Playback |
| `audioIn` | `Audio/Source` | `false` | Microphones |
| `videoIn` | `Video/Source` | `false` | Cameras. Also needs `video.enable = true` for `/dev/video*` binding |

### Management toggles

These grant additional WirePlumber permissions on objects the sandbox can already see.

| Toggle | Permission | Effect |
|--------|-----------|--------|
| `control` | `w` (write) | Change volume, mute state of visible nodes |
| `routing` | `m` (metadata) | Change default devices, move streams |

Hidden targets remain unavailable for discovery. The link-only baseline lets sandbox-created streams connect to the specific sinks or sources you explicitly expose without making the rest of the graph visible. Cloister grants read access to PipeWire `Link` objects only for links that touch sandbox-owned stream nodes, so `pipewire-pulse` can observe its own playback/record stream setup without learning about unrelated clients using the same device. `audioIn = true` is intended to allow real microphone capture, and `videoIn = true` is intended to allow real camera/video capture once the corresponding device nodes are also available.

With only `filters.enable = true`, the visible graph should be limited to the PipeWire core, the sandbox's own client object, `Audio/Sink` nodes, and only the `Link` objects associated with that sandbox's own streams. Microphones, cameras, metadata, and unrelated clients should stay hidden.

## `pipewire-pulse` compatibility

When `audio.pipewire.pulseCompat.enable = true` is set, Cloister generates a small wrapper around the sandbox entry command. That wrapper:

- starts `pipewire-pulse` inside the sandbox if `"$XDG_RUNTIME_DIR/pulse/native"` does not already exist
- waits for the local PulseAudio socket to appear
- exports `PULSE_SERVER=unix:$XDG_RUNTIME_DIR/pulse/native`
- launches the requested shell or command
- stops the transient `pipewire-pulse` process again when that command exits

This gives `libpulse` applications a normal PulseAudio endpoint without forwarding the host PulseAudio socket. `pipewire-pulse` still connects to the PipeWire native socket mounted into the sandbox, so all WirePlumber filter rules continue to apply. If the sandbox can only see `Audio/Sink`, PulseAudio-compatible clients only get playback. If `audioIn`, `videoIn`, `control`, or `routing` are enabled, `pipewire-pulse` can use those same capabilities too.

### Requirements

`pulseCompat` only works when the PipeWire native side is healthy. In practice that means:

- `audio.pipewire.enable = true`
- `audio.pulseaudio.enable = false` because direct PulseAudio forwarding and `pulseCompat` are mutually exclusive
- `XDG_RUNTIME_DIR` must be present, because both the forwarded PipeWire socket and the in-sandbox PulseAudio socket live there
- the mounted PipeWire socket must exist and be valid on the host (`pipewire-0` or the filtered `cloister/pipewire/<name>` socket)
- `pipewire` must be available in the sandbox package set so `pipewire-pulse` can start

For filtered setups, the usual media prerequisites still apply:

- `audioIn = true` for microphones
- `videoIn = true` plus `video.enable = true` for cameras
- `control = true` for volume / mute writes
- `routing = true` for default-device or stream-routing changes

If sandbox D-Bus is disabled, Cloister writes both `client.conf` and `pipewire-pulse.conf` with `support.dbus = false`. That keeps the bridge working without depending on a session bus, but D-Bus-backed PipeWire helpers remain unavailable.

### Operational notes

`pipewire-pulse` uses a sandbox-specific config generated by Cloister. That config removes `libpipewire-module-rt`, so the bridge does not try to acquire realtime scheduling from inside the sandbox. It also listens only on the local Unix socket (`unix:native`), not on TCP.

Because the bridge process is tied to the launched command, it is intentionally ephemeral. Child processes inherit `PULSE_SERVER` and keep working, but background jobs that outlive the wrapper should not assume the local PulseAudio socket will remain available after the original sandbox command exits.

## Per-sandbox sockets

Filtered PipeWire sockets are scoped per sandbox (for example, `cloister/pipewire/zoom`). Even if two sandboxes use identical filter settings, each gets its own socket and WirePlumber policy so filtered graph visibility stays isolated to that sandbox.

## Validation

Inside the sandbox, run:

```bash
cloister-pipewire-validate      # summary
cloister-pipewire-validate -v   # per-object detail
```

For manual debugging, `wpctl status` shows visible devices and `wpctl set-volume <id> 5%+` can confirm whether `control` is effective.

For a quick policy check, `cloister-pipewire-validate -v` should show only `Audio/Sink` nodes plus the required internal linking objects (`Link` globals and the `client-node`, `adapter`, and `link-factory` factories) when only `audioOut` is enabled. The summary output should report `audioOut: true`, `factories: true`, and the remaining media toggles as `false`.
