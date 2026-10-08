# Identity Anonymization

Enable `sandbox.anonymize.enable` to present a generic identity inside the sandbox:

```nix
cloister.sandboxes.untrusted.sandbox.anonymize.enable = true;
```

## What changes

When enabled, the sandbox:

- sets username and hostname to the configured anonymized username (default `ubuntu`)
- uses `/home/<username>` as the home directory instead of your real home path
- remaps bind mount destinations from your real home into `/home/<username>`
- uses a Cloister-patched bubblewrap build that mounts procfs with `subset=pid`
- hides non-task top-level `/proc` entries such as `/proc/cpuinfo`, `/proc/meminfo`, `/proc/modules`, and `/proc/sys`
- still overlays task-related mount metadata aliases such as `/proc/self/mountinfo`, `/proc/self/mounts`, `/proc/thread-self/mountinfo`, and `/proc/thread-self/mounts`

## Configuration

```nix
cloister.sandboxes.untrusted.sandbox.anonymize = {
  enable = true;
  username = "ubuntu";
};
```

The anonymized username must match `^[a-z_][a-z0-9_-]*$`.

## Audio interaction

Anonymized sandboxes do not permit exposing a native PipeWire socket. If an anonymized sandbox needs audio, use PulseAudio-compatible filtered mode instead:

```nix
cloister.sandboxes.browser = {
  sandbox.anonymize.enable = true;
  audio.pipewire = {
    enable = true;
    pulseOnly = true;
  };
};
```

## Limitations

This is best-effort privacy only, not a strict security boundary.

- bind source paths may still remain inferable through other proc aliases such as `/proc/<pid>/mountinfo`
- portal file chooser integration is also not a host-path privacy boundary; upstream portals may return host `file://` URIs directly, and document-portal FUSE can expose the original host path via the read-only `user.document-portal.host-path` xattr
- fully hiding host path details likely requires copying or snapshotting the workspace instead of bind-mounting it
- anonymization reduces casual identity leakage to tools, but it does not make an otherwise powerful sandbox safe by itself

See `docs/security.md` for the broader security model.

## Implementation note

Only anonymized sandboxes use the patched bubblewrap package. Non-anonymized sandboxes continue to use nixpkgs' stock `bubblewrap`.

The package at `pkgs/bubblewrap-subset-pid` extends the consuming configuration's nixpkgs `bubblewrap` package. It replaces the proc mount options shared by older and newer Bubblewrap source layouts, and fails the build if the expected options are absent. When updating nixpkgs, rebuild `bubblewrap-subset-pid` to verify compatibility.
