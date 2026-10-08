# Audio

Cloister supports two PipeWire-backed audio modes:

- **filtered native PipeWire** for native PipeWire clients, cameras, and screen-sharing capable apps
- **pulse-only proxy** for audio-only applications that need a PulseAudio-compatible socket

Both modes use a per-sandbox filtered PipeWire backend socket (`cloister/pipewire/<name>`). Direct host PulseAudio passthrough, unfiltered PipeWire forwarding, in-sandbox `pipewire-pulse` compatibility, and ALSA compatibility are not supported.

## Filtered native PipeWire access

When `audio.pipewire.enable = true` is set, Cloister provisions a dedicated PipeWire socket and generates WirePlumber policy so the sandbox can only see and interact with the audio or video device classes you explicitly allow. Unfiltered host `pipewire-0` forwarding is rejected.

```nix
cloister.sandboxes.zoom = {
  audio.pipewire = {
    enable = true;
    filters = {
      audioOut = true;  # speakers (default)
      audioIn = true;   # microphones
      videoIn = true;   # webcams / V4L2
      control = false;  # volume / mute changes
      routing = false;  # default device / stream routing
    };
  };
};
```

### Discovery toggles

These control which `media.class` types are visible in the PipeWire registry.

| Toggle | media.class | Default | Notes |
|--------|------------|---------|-------|
| `audioOut` | `Audio/Sink` | `true` | Playback |
| `audioIn` | `Audio/Source` | `false` | Microphones |
| `videoIn` | `Video/Source` | `false` | Cameras exposed through the filtered PipeWire graph |

### Management toggles

These grant additional WirePlumber permissions on objects the sandbox can already see.

| Toggle | Permission | Effect |
|--------|-----------|--------|
| `control` | `w` (write) | Change volume or mute state of visible nodes |
| `routing` | `m` (metadata) | Change default devices or move streams |

The link-only baseline lets sandbox-created streams connect to explicitly exposed sinks or sources without making the rest of the graph visible. `audioIn = true` allows microphone capture, and `videoIn = true` allows camera capture once the corresponding device nodes are available.

## Pulse-only proxy mode

When `audio.pipewire.pulseOnly = true` is set, Cloister still uses filtered PipeWire as the backend, but it does not mount any native PipeWire socket into the sandbox. Instead, Cloister starts a transient `pipewire-pulse` bridge outside the sandbox, points it at the sandbox's filtered `cloister/pipewire/<name>` remote, and binds only the resulting `pulse/native` socket into the sandbox.

This mode is intended for audio-only sandboxes that want:

- playback without exposing native PipeWire
- optional microphone access via `filters.audioIn = true`
- optional volume/mute control via `filters.control = true`
- no access to cameras, screen sharing, or native PipeWire tooling inside the sandbox

```nix
cloister.sandboxes.music.audio.pipewire = {
  enable = true;
  pulseOnly = true;
  filters = {
    audioOut = true;
    audioIn = false;
    control = false;
  };
};
```

`pulseOnly` is audio-only by design and cannot be combined with `filters.videoIn = true`.

## Per-sandbox filtered sockets

Filtered PipeWire sockets are scoped per sandbox (for example, `cloister/pipewire/zoom`). Even if two sandboxes use identical filter settings, each gets its own socket and WirePlumber policy so filtered graph visibility stays isolated.

## Anonymized audio sandboxes

When `sandbox.anonymize.enable = true` is also set, Cloister does not allow exposing a native PipeWire socket inside the sandbox. Nix evaluation fails if `audio.pipewire.enable = true` is combined with anonymization without also setting `audio.pipewire.pulseOnly = true`.

For anonymized audio sandboxes, `pulseOnly` launches `pipewire-pulse` inside a transient helper bubblewrap with a synthetic hostname and passwd/group view, then forwards only that helper's `pulse/native` socket into the final sandbox.

## Validation

Inside the sandbox, run:

```bash
cloister-pipewire-validate      # summary
cloister-pipewire-validate -v   # per-object detail
```

The helper exits `0` on `RESULT: PASS` and `1` on `RESULT: FAIL` or connection errors, so it is safe to use in scripts and CI.

For manual debugging, `wpctl status` shows visible devices and `wpctl set-volume <id> 5%+` can confirm whether `control` is effective.

If `pulseOnly` is enabled, `pw-cli` should not have any native PipeWire socket to talk to inside the sandbox. For audio-only validation, use PulseAudio-compatible applications or `pactl` instead.
