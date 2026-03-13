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

When `gui.wayland.securityContext.enable` is set, the sandbox connects through `cloister-sandbox`'s built-in Wayland support, which creates a new Wayland connection using the `wp-security-context-v1` protocol. The compositor can then apply per-client restrictions (e.g., preventing screenshot capture or input simulation). This requires compositor support (sway 1.9+, Hyprland, niri, labwc 0.8.2+).

### D-Bus Proxy

When `dbus.enable` is set, a per-sandbox `xdg-dbus-proxy` instance filters session bus access. Policies control which bus names the sandbox can `talk` to, `own`, `see`, `call`, or receive `broadcast` signals from. The proxy socket is activated via systemd.

### Dangerous Path Detection

The `dangerousPaths` mechanism prevents accidental exposure of credential files. At Nix evaluation time, all bind mount paths are checked against a list of known sensitive locations. This list is not comprehensive and is designed to be a feedback mechanism and will not prevent leaking of sensitive data due to misconfiguration.

If a bind mount path overlaps any of these, the build fails with an assertion error listing the offending paths.

**Suppressing warnings for specific paths:**

```nix
cloister.sandboxes.<name>.sandbox.allowDangerousPaths = [ ".ssh" ];
```

**Disabling all checks:**

```nix
cloister.sandboxes.<name>.sandbox.dangerousPathWarnings = false;
```

## Accepted Risks

Each of these is a deliberate trade-off between security and developer experience:

| Risk | Rationale |
|------|-----------|
| **Shell init files (`.zshrc`/`.bashrc`) bound read-only by default** | May contain exported secrets (e.g., `export API_KEY=...`). Necessary for dev UX - shell configuration, aliases, prompt, and tool initialization. Disable with `shell.hostConfig = false`. |
| **Network enabled by default** | Needed for package managers, LSPs, git fetch, and API access. Disable with `network.enable = false`. |
| **`nix`, `curl`, `openssh` in default packages** | Core dev toolchain. SSH access is mediated by the fingerprint filter when configured. |
| **X11 passthrough has no client isolation** | X11's security model allows any client to read any other client's input/output. Prefer Wayland with security context for GUI workloads. |
| **PulseAudio grants full audio including microphone** | PulseAudio has no per-client restriction mechanism. The socket gives access to both playback and recording. |
| **Git config bound read-only** | May expose credential helper configuration (e.g., `credential.helper = store`). Cloister binds only `.gitconfig` and `.config/git/config`; actual credential files (`.git-credentials`, `.config/git/credentials`) are not mounted and are still blocked by dangerous path detection. |
| **GPU sysfs paths exposed read-only** | When GPU is enabled, GPU-specific `/sys/dev/char/MAJ:MIN` entries, `/run/opengl-driver`, and auto-detected PCI sysfs device paths are ro-bound. This reveals GPU hardware identifiers (vendor/device IDs) and driver metadata. Required for Mesa/Vulkan to identify and initialize the GPU. All binds use `--ro-bind` (not `--dev-bind`) and are gated on path existence. |
| **Video device access exposes all V4L2 devices** | When `video.enable` is set, all `/dev/video*` devices are bound into the sandbox. V4L2 has no per-client restriction mechanism - any process with device access can capture video from any connected camera. The sysfs paths exposed also reveal USB/PCI device identifiers. |
| **CUPS socket grants full print access** | When `printing.enable` is set, the CUPS socket is bound read-only. Any process inside the sandbox can submit print jobs and query printer information. The socket is read-only so the sandbox cannot modify CUPS configuration. |
| **PipeWire native socket grants full media access** | When `audio.pipewire.enable` is set, the PipeWire socket provides access to all media streams including camera, screen capture, and audio. This is broader than PulseAudio alone and should only be enabled for trusted sandboxes that need portal/camera integration. |

## Additional Privacy

### Identity Anonymization

The `sandbox.anonymize.enable` option provides an optional identity-masking layer. When enabled, the sandbox presents a generic `ubuntu` username and hostname, uses `/home/ubuntu` as the home directory, and masks `/proc` entries that reveal host identity (hostname, domain name, kernel release, boot ID).

This is **not a security boundary** - a determined attacker inside the sandbox could still fingerprint the host through timing, hardware characteristics, or network metadata. It is designed to prevent **casual identity leakage** to untrusted tools that may phone home with environment metadata (e.g., telemetry in editors, AI assistants, or language toolchains).

See [Configuration - Identity anonymization](configuration.md#identity-anonymization) for usage.
