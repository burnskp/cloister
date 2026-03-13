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

## Per-sandbox sockets

Filtered PipeWire sockets are scoped per sandbox (for example, `pipewire-cloister/zoom`). Even if two sandboxes use identical filter settings, each gets its own socket and WirePlumber policy so filtered graph visibility stays isolated to that sandbox.

## Validation

Inside the sandbox, run:

```bash
cloister-pipewire-validate      # summary
cloister-pipewire-validate -v   # per-object detail
```

For manual debugging, `wpctl status` shows visible devices and `wpctl set-volume <id> 5%+` can confirm whether `control` is effective.

For a quick policy check, `cloister-pipewire-validate -v` should show only `Audio/Sink` nodes plus the required internal linking objects (`Link` globals and the `client-node`, `adapter`, and `link-factory` factories) when only `audioOut` is enabled. The summary output should report `audioOut: true`, `factories: true`, and the remaining media toggles as `false`.
