# Cloister Security Model

## Threat Model

Cloister defends against **compromised or misbehaving tools** running inside the sandbox:

- **Supply-chain attacks** - malicious code in npm/pip/cargo dependencies that attempts to exfiltrate credentials, SSH keys, or personal files.
- **Untrusted build scripts** - `postinstall`, `setup.py`, Makefiles, and similar hooks that execute with the user's full permissions.

Cloister does **not** defend against:

- **Kernel exploits** - a sandbox escape via a kernel vulnerability bypasses all userspace isolation.
- **Hardware attacks** - side-channel, rowhammer, or physical access attacks.
- **Root-level attackers** - an attacker with root on the host can trivially bypass namespace isolation.
- **Untrusted Configuration** - While there are some protections built in to help prevent misconfiguration, they are there more as feedback mechanisms than prevention and can be overriden. As with anything, make sure you read and understand any third party NixOS configs before you apply them.

## Isolation Layers

### Namespace Unshare

Every sandbox runs under `bwrap --unshare-all`, creating separate mount, PID, IPC, UTS, and (optionally) network namespaces. The sandbox only sees files that are explicitly bound in. `/dev` is a private devtmpfs with its own devpts instance for PTY isolation.

### Seccomp Filter

A BPF seccomp filter blocks dangerous syscalls including:

- Kernel module loading, reboot, kexec
- Mount/unmount, pivot_root, chroot, namespace creation via `clone(CLONE_NEW*)`, `unshare`, `setns`, `clone3` (unless `--allow-chromium-sandbox`)
- `io_uring` (bypasses seccomp on submitted operations)
- `ioctl(TIOCSTI)` / `ioctl(TIOCLINUX)` (PTY injection attacks)
- Socket families outside `{AF_UNSPEC, AF_LOCAL, AF_INET, AF_INET6, AF_NETLINK}`; `AF_NETLINK` is also denied when `network.enable = false`
- `prctl(PR_SET_SECUREBITS)` and `prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE)` (capability model manipulation)
- `ptrace`, `process_vm_readv/writev`, BPF, perf, userfaultfd

The filter uses a default-allow policy - only explicitly listed syscalls are blocked.

### SSH Fingerprint Filtering

When `ssh.allowFingerprints` is configured, an SSH agent filter proxy (built into `cloister-sandbox`) sits between the sandbox and the host's `SSH_AUTH_SOCK`. It:

- Strips keys from `SSH_AGENT_IDENTITIES_ANSWER` whose fingerprint is not in the allowlist.
- Rejects `SSH_AGENTC_SIGN_REQUEST` for non-allowed keys.
- Enforces a per-operation timeout to prevent hung connections from blocking the agent.

This prevents a compromised sandbox from using SSH keys it shouldn't have access to.

### Wayland Security Context

When `gui.enable` is set, the sandbox connects through `cloister-sandbox`'s built-in Wayland support, which creates a new Wayland connection using the `wp-security-context-v1` protocol. The compositor can then apply per-client restrictions (e.g., preventing screenshot capture or input simulation). This requires compositor support (sway 1.9+, Hyprland, niri, labwc 0.8.2+).

### D-Bus Proxy

When `dbus.enable` is set, a per-launch `xdg-dbus-proxy` instance filters session bus access. Policies control which bus names the sandbox can `talk` to, `own`, `see`, `call`, or receive `broadcast` signals from. Portal toggles are layered on top of this proxy and are not available when D-Bus is disabled.

## Accepted Risks

Each of these is a deliberate trade-off between security and developer experience:

| Risk | Rationale |
|------|-----------|
| **Shell init files (`.zshrc`/`.bashrc`) can be bound read-only** | May contain exported secrets (e.g., `export API_KEY=...`). Host shell config is off by default; enable it only for trusted sandboxes with `shell.hostConfig = true`. |
| **Network enabled by default** | Needed for package managers, LSPs, git fetch, and API access. Disable with `network.enable = false`. |
| **`nix`, `curl`, `openssh` in default packages** | Core dev toolchain. SSH access is mediated by the fingerprint filter when configured. |
| **Git config bound read-only** | May expose credential helper configuration (e.g., `credential.helper = store`). Cloister binds only `.gitconfig` and `.config/git/config`; actual credential files (`.git-credentials`, `.config/git/credentials`) are not mounted by this feature. |
| **GPU sysfs paths exposed read-only** | When GPU is enabled, GPU-specific `/sys/dev/char/MAJ:MIN` entries, `/run/opengl-driver`, and auto-detected PCI sysfs device paths are ro-bound. This reveals GPU hardware identifiers (vendor/device IDs) and driver metadata. Required for Mesa/Vulkan to identify and initialize the GPU. All binds use `--ro-bind` (not `--dev-bind`) and are gated on path existence. |
| **Device passthrough grants direct kernel-device access** | `sandbox.devices` mounts device nodes with `--dev-bind`. Any process inside the sandbox can use those devices with the same host permissions as the launching user/group. Only bind devices you trust the sandboxed workload to access. |
| **`/proc/*/mountinfo` can reveal host source paths for bind mounts** | Linux mount metadata records the source subtree root for bind mounts. If Cloister bind-mounts the working directory or other host paths, processes inside the sandbox may still infer portions of the host path from `/proc/self/mountinfo`, `/proc/thread-self/mountinfo`, or `/proc/<pid>/mountinfo`. Cloister masks common mountinfo aliases in anonymized mode where practical, but this is not a complete fix with plain bind mounts; fully hiding these source paths generally requires exposing a copied/snapshotted workspace or moving to a stronger isolation boundary such as a VM. |
| **PipeWire media access follows filter policy** | When `audio.pipewire.enable = true`, Cloister exposes only a per-sandbox filtered PipeWire socket. Enabling `audio.pipewire.filters.audioIn`, `videoIn`, `control`, or `routing` broadens what sandbox processes can capture or control within that filtered policy. |
| **`audio.pipewire.pulseOnly` is audio-only by design** | When `audio.pipewire.pulseOnly = true` is set, Cloister exposes only a PulseAudio-compatible socket inside the sandbox. Playback, microphone, and volume-control access can still be gated by `audio.pipewire.filters.*`, but native PipeWire clients, camera access, and PipeWire portal flows are not available. |

## Additional Privacy

### Identity Anonymization

The `sandbox.anonymize.enable` option provides an optional identity-masking layer. When enabled, the sandbox presents the configured anonymized username and hostname (default `ubuntu`), uses `/home/<username>` as the home directory, and switches to a Cloister-patched bubblewrap build that mounts procfs with `subset=pid`. That hides non-task top-level `/proc` entries such as kernel, memory, module, interrupt, and sysctl views before the sandbox starts. Cloister still overlays common task-related mount metadata aliases like `/proc/self/mountinfo`, `/proc/self/mounts`, `/proc/thread-self/mountinfo`, and `/proc/thread-self/mounts`, because `subset=pid` intentionally leaves those visible.

For PipeWire, anonymization and native PipeWire socket exposure are intentionally incompatible. Cloister fails evaluation unless anonymized sandboxes use `audio.pipewire.pulseOnly = true`, so the sandbox only sees a PulseAudio-compatible socket instead of a native PipeWire endpoint.

This is **not a security boundary** - a determined attacker inside the sandbox could still fingerprint the host through timing, hardware characteristics, or network metadata. It is designed to prevent **casual identity leakage** to untrusted tools that may phone home with environment metadata (e.g., telemetry in editors, AI assistants, or language toolchains).

See [Configuration - Identity anonymization](configuration.md#identity-anonymization) for usage.

## Writable projects

The default writable project mount includes project secrets such as `.env` and repository metadata in `.git`. Sandbox processes can read and alter these files. Modified Git hooks, build scripts, editor tasks, or other project code may subsequently execute with host permissions. Review changes, including hooks and configuration, before running the project outside the sandbox. Use isolated workspaces without host secrets for low-trust tasks.

## Dependency audit exceptions

The shared `scripts/rust-check.sh` audit denies unsoundness warnings. Lockfiles use patched `anyhow >=1.0.103` ([RUSTSEC-2026-0190](https://rustsec.org/advisories/RUSTSEC-2026-0190)) and `event-listener >=5.4.2` ([RUSTSEC-2026-0221](https://rustsec.org/advisories/RUSTSEC-2026-0221)).

Only `helpers/cloister-sandbox` and `helpers/cloister-wayland-validate` temporarily ignore [RUSTSEC-2026-0194](https://rustsec.org/advisories/RUSTSEC-2026-0194) and [RUSTSEC-2026-0195](https://rustsec.org/advisories/RUSTSEC-2026-0195). Their `wayrs-client` dependency (also used by the sandbox’s `wayrs-protocols`) uses the `wayrs-scanner` procedural macro → `wayrs-proto-parser` → `quick-xml` dependency chain. It parses bundled protocol XML during compilation, not XML supplied by sandbox clients at runtime. The exceptions cover XML denial-of-service findings in this compile-time dependency only; other workspaces and other advisories still fail validation. Remove both exceptions and update the lockfiles when upstream `wayrs-proto-parser` supports patched `quick-xml >=0.41.0`. Reassess immediately if XML inputs become untrusted or runtime parsing is introduced.
