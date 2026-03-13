# PipeWire Filtering

When `audio.pipewire.enable = true` is set, the sandbox receives unrestricted access to the PipeWire graph — all audio devices, cameras, and metadata are visible and writable.

Enabling **PipeWire filters** (`audio.pipewire.filters.enable = true`) restricts that access. Cloister provisions a dedicated PipeWire socket and generates WirePlumber Lua policies so the sandbox can only see and interact with the device classes you explicitly allow.

## Configuration

With `filters.enable = true`, the sandbox starts with `audioOut` enabled and everything else is opt-in. The policy keeps the client on a link-only baseline, exposes only the configured sink/source classes, and grants the minimum internal PipeWire factories needed to create playback and capture streams:

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

Hidden targets remain unavailable for discovery. The link-only baseline lets sandbox-created streams connect to the specific sinks or sources you explicitly expose without making the rest of the graph visible. `audioIn = true` is intended to allow real microphone capture, and `videoIn = true` is intended to allow real camera/video capture once the corresponding device nodes are also available.

With only `filters.enable = true`, the visible graph should be limited to the PipeWire core, the sandbox's own client object, and `Audio/Sink` nodes. Microphones, cameras, metadata, and unrelated clients should stay hidden.

## Deduplication

The socket name is derived from a hash of the filter config (e.g., `pipewire-cloister-a1b2c3d4`). Sandboxes with identical filter settings share a single socket and WirePlumber policy automatically.

## Validation

Inside the sandbox, run:

```bash
cloister-pipewire-validate      # summary
cloister-pipewire-validate -v   # per-object detail
```

For manual debugging, `wpctl status` shows visible devices and `wpctl set-volume <id> 5%+` can confirm whether `control` is effective.

For a quick policy check, `cloister-pipewire-validate -v` should show only `Audio/Sink` nodes plus the required internal factories when only `audioOut` is enabled. The summary output should report `audioOut: true`, `factories: true`, and the remaining media toggles as `false`.
