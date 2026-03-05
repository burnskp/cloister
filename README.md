# cloister

A bubblewrap namespace sandbox for shell and GUI applications. Every tool - editors, AI assistants, language toolchains, git - runs inside an isolated namespace with access only to the current directory and explicitly declared state paths. Multiple sandboxes can coexist with different security profiles.

## Why

The sandbox limits what a compromised or misbehaving tool can access. A prompt-injected AI assistant, a compromised GUI app, a supply-chain attack in an npm package, or a malicious build script cannot read your SSH keys, cloud credentials, other projects, or personal files. Each sandbox sees only what you explicitly grant it.

## Requirements

- **Linux only** - bubblewrap uses Linux namespaces; macOS is not supported
- **home-manager >= 25.05** - uses `programs.zsh.initContent`
- **NixOS not required** - works on any Linux system with Nix + home-manager

## Quick start

Add `cloister` as a flake input and import the home-manager module:

```nix
cloister.url = "github:burnskp/cloister";
```

<details>
<summary>Standalone home-manager flake.nix</summary>

```nix
# flake.nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    home-manager.url = "github:nix-community/home-manager";
    cloister.url = "github:burnskp/cloister";
  };

  outputs = { nixpkgs, home-manager, cloister, ... }: {
    homeConfigurations."yourname" = home-manager.lib.homeManagerConfiguration {
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      modules = [
        cloister.homeManagerModules.default
        {
          cloister = {
            enable = true;
            sandboxes.dev = { };
          };
        }
      ];
    };
  };
}
```

</details>

<details>
<summary>NixOS with home-manager module flake.nix</summary>

```nix
# flake.nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    home-manager.url = "github:nix-community/home-manager";
    cloister.url = "github:burnskp/cloister";
  };

  outputs = { nixpkgs, home-manager, cloister, ... }: {
    nixosConfigurations.yourhostname = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        home-manager.nixosModules.home-manager
        {
          home-manager.users.yourname = {
            imports = [ cloister.homeManagerModules.default ];
            cloister = {
              enable = true;
              sandboxes.dev = { };
            };
          };
        }
      ];
    };
  };
}
```

</details>

After adding the input, run `nix flake lock --update-input cloister` to fetch it, then rebuild.

### Usage

Each sandbox defined under `cloister.sandboxes.<name>` produces a `cl-<name>` binary.

```sh
cl-dev              # interactive shell (detects git root automatically)
cl-dev cargo build  # run a single command and exit
cl-dev -c cargo build  # -c is accepted but optional
```

You can use host shell config or provide sandbox-specific rc files (sourced before registry snippets). Custom rc files are mounted under `~/.config/cl-shell/<name>/custom/`.

```nix
cloister.sandboxes.dev.shell = {
  name = "zsh";
  hostConfig = true; # default
  customRcPath.zshrc = ./configs/zsh/dev.zshrc;
};
```

Example: two sandboxes with different zshrc subsets

See `examples/shell-custom-rc.nix`.

Directory detection rules:

- Inside a **git repo** -> uses the repo root
- In a **non-git directory** -> uses the current directory
- In **`$HOME`** -> error (ambiguous - set `CLOISTER_DIR` or cd into a directory)

Override the sandbox directory with `CLOISTER_DIR`:

```sh
CLOISTER_DIR=/path/to/project cl-dev
```

## Features

- **Multiple sandboxes** - different security profiles side by side (`cl-dev`, `cl-pdf`, etc.)
- **Command wrapping** - typing `nvim` in your normal shell transparently routes through the sandbox
- **Shell choice** - zsh or bash as the interactive shell
- **Network control** - full network, no network, or routed through a VPN namespace
- **State persistence** - bind mount categories for caches, config, volume-backed storage, and per-directory isolation
- **Home-manager integration** - bind Nix store-backed config files directly into the sandbox
- **Wayland forwarding** - with `wp-security-context-v1` to filter privileged protocols
- **X11 forwarding** - `DISPLAY` passthrough (no client isolation - prefer Wayland)
- **PulseAudio** - audio playback via PulseAudio/PipeWire socket forwarding
- **D-Bus notifications** - per-sandbox filtered proxy with configurable policies
- **SSH agent** - forward `SSH_AUTH_SOCK` into the sandbox (optional fingerprint filtering + timeout)
- **Dangerous path detection** - build-time checks prevent accidentally binding credential locations
- **Validator helpers** - install Wayland/D-Bus/seccomp validators and wrap them outside the sandbox

## Options at a glance

All per-sandbox options live under `cloister.sandboxes.<name>.*`. See [Configuration & Options Reference](docs/configuration.md) for full details.

| Category | Key options | Type | Default | Purpose |
|----------|-----------|------|---------|---------|
| **Global** | `cloister.enable` | bool | `false` | Gate the entire module |
| | `cloister.defaultShell` | enum | `"zsh"` | Default interactive shell |
| **Packages** | `extraPackages` | list of package | `[]` | Additional packages on sandbox PATH |
| **Command** | `defaultCommand` | nullOr (listOf str) | `null` | Command to run when invoked without arguments |
| **Shell** | `shell.name` | enum | `defaultShell` | Interactive shell (`"zsh"` or `"bash"`) |
| | `shell.hostConfig` | bool | `true` | Bind host shell config files |
| | `shell.customRcPath.*` | nullOr path | `null` | Custom rc files to source |
| **Sandbox** | `sandbox.bindWorkingDirectory` | bool | `true` | Bind-mount working directory |
| | `sandbox.env` | attrsOf str | *(base vars)* | Environment variables inside sandbox |
| | `sandbox.perDirBase` | str | `"${config.xdg.stateHome}/cloister"` | Per-directory state base |
| | `sandbox.copyFiles` | list of {…} | `[]` | Writable config file copies |
| | `sandbox.anonymize.enable` | bool | `false` | Generic identity (ubuntu user/hostname) |
| | `sandbox.extraBinds.*` | various | - | State persistence bind mounts |
| | `sandbox.seccomp.enable` | bool | `true` | Seccomp-bpf syscall filter |
| | `sandbox.devBinds` | list of str | `[]` | Device passthrough |
| **Network** | `network.enable` | bool | `true` | Share host network |
| | `network.namespace` | nullOr str | `null` | Linux network namespace to join |
| **GUI** | `gui.wayland.enable` | bool | `false` | Forward Wayland socket |
| | `gui.x11.enable` | bool | `false` | Forward X11 DISPLAY |
| | `gui.gpu.enable` | bool | `false`\* | Bind /dev/dri (*auto-enabled with Wayland/X11) |
| | `gui.gpu.shm` | bool | `true` | Private tmpfs at /dev/shm |
| | `gui.gtk.enable` | bool | `false`* | GTK theming (*auto-enabled with Wayland/X11) |
| | `gui.scaleFactor` | nullOr float | `null` | HiDPI scale (sets GDK_SCALE, QT_SCALE_FACTOR) |
| | `gui.qt.enable` | bool | `false` | Qt theming |
| | `gui.desktopEntry.enable` | bool | `false` | Generate .desktop file |
| | `gui.dataPackages` | list of package | `[hicolor-icon-theme]`* | XDG_DATA_DIRS packages |
| | `gui.fonts.packages` | list of package | `[]`* | Font packages for fontconfig (*`dejavu_fonts` with GUI) |
| **Audio** | `audio.pulseaudio.enable` | bool | `false` | Forward PulseAudio socket |
| | `audio.pipewire.enable` | bool | `false` | Forward PipeWire socket |
| **Hardware** | `video.enable` | bool | `false` | Bind webcam/camera devices |
| | `fido2.enable` | bool | `false` | Bind FIDO2/U2F devices |
| | `printing.enable` | bool | `false` | Forward CUPS socket |
| **SSH** | `ssh.enable` | bool | `false` | Forward SSH agent socket |
| | `ssh.allowFingerprints` | list of str | `[]` | Restrict visible keys |
| **Git** | `git.enable` | bool | `false` | Bind git config read-only |
| **D-Bus** | `dbus.enable` | bool | `false` | Per-sandbox D-Bus proxy |
| | `dbus.log` | bool | `false` | Proxy logging |
| | `dbus.portal` | bool | `false` | xdg-desktop-portal integration |
| | `dbus.policies.*` | various | - | Talk/own/see/call/broadcast rules |
| **Registry** | `registry.commands` | list of str | `[]` | Commands to wrap outside sandbox |
| | `registry.aliases` | attrsOf str | `{}` | Shell aliases |
| | `registry.functions` | attrsOf lines | `{}` | Shell functions |
| **Init** | `init.text` | lines | `""` | Shell snippet sourced on session start |

## Documentation

For deep details, see the following documents:

- **[Configuration & Options Reference](docs/configuration.md)** - how to configure state persistence, networking, desktop integration, and more.
- **[Network Namespaces](docs/network-namespace.md)** - route sandboxes through VPN, LAN, localhost-only, or isolated namespaces.
- **[Security Model](docs/security.md)** - threat model and isolation layers.
- **[D-Bus Proxy](docs/dbus.md)** - how the per-sandbox D-Bus filtering works.
- **[Diagnostics](docs/diagnostics.md)** - tools for validating your setup (Wayland, D-Bus, etc).

## Examples

See the [examples/](examples/) directory for complete, importable sandbox configurations:

- **[chromium.nix](examples/chromium.nix)** - Sandboxed browser with GPU, audio, notifications, and desktop entry
- **[discord.nix](examples/discord.nix)** - Sandboxed Discord with Wayland, audio, and Flatpak-aligned D-Bus policies
- **[evince.nix](examples/evince.nix)** - Network-isolated PDF viewer with desktop entry and MIME type registration
- **[nixdev.nix](examples/nixdev.nix)** - Nix configuration development with editor, LSP, formatters, and persistent caches
- **[shell-custom-rc.nix](examples/shell-custom-rc.nix)** - Shell rc subset configuration with per-sandbox zshrc files
