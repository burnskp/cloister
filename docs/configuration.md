# Configuration

All per-sandbox configuration lives under `cloister.sandboxes.<name>`. The module provides sensible defaults for a bare functional sandbox, and you layer on your preferences.

## Sandbox basics

### Multiple sandboxes

Define multiple sandboxes with different security profiles:

```nix
cloister = {
  enable = true;
  sandboxes.dev = {
    ssh.enable = true;
    network.enable = true;  # has network (default)
    registry.commands = [ "nvim" "cargo" "claude" ];
  };
  sandboxes.pdf = {
    network.enable = false;  # no network
    gui.wayland.enable = true;
    extraPackages = with pkgs; [ zathura imv ];
    registry.commands = [ "zathura" "imv" ];
  };
};
```

This produces `cl-dev` and `cl-pdf` binaries. Wrapped commands route to the correct sandbox automatically (e.g., `nvim` -> `cl-dev nvim`, `zathura` -> `cl-pdf zathura`).

Cross-sandbox name collisions are detected at build time - two sandboxes cannot wrap the same command name outside.

### Adding packages

The sandbox includes a core set of 14 packages: bash, coreutils, curl, findutils, gawk, git, gnugrep, gnused, gnutar, gzip, less, nix, openssh, which, plus the configured shell. Add your tools with `extraPackages`:

```nix
cloister.sandboxes.dev.extraPackages = with pkgs; [
  cargo
  rustc
  nodejs
  neovim
];
```

### Validator helpers

Install the Wayland, D-Bus, and seccomp validators inside the sandbox and wrap them outside:

```nix
cloister.sandboxes.dev.validators.enable = true;
```

### Shell

```nix
cloister.sandboxes.dev.shell.name = "zsh";  # default is cloister.defaultShell ("zsh"); also supports "bash"
```

This controls the interactive shell inside the sandbox and the wrapper integration outside.

To bind host shell config files and/or add sandbox-specific ones:

```nix
cloister.sandboxes.dev.shell = {
  hostConfig = true; # default
  customRcPath = {
    zshenv = ./configs/zsh/dev.zshenv;
    zshrc = ./configs/zsh/dev.zshrc;
  };
};
```

Custom rc files are bound into `~/.config/cl-shell/<name>/custom/` and sourced in this order:

1. Host config files (when `hostConfig = true`)
1. Custom rc files (when set)
1. Registry snippet (always last, so registry wins)

Notes:

- `customRcPath` entries are Nix paths (e.g. `./configs/zsh/dev.zshrc`), so they land in the Nix store and are bind-mounted read-only into the sandbox.
- Set `hostConfig = false` to avoid binding any host shell config files.

Example: two sandboxes, two different zshrc subsets

See `examples/shell-custom-rc.nix`.

> **Note:** Shell config files are bound read-only into the sandbox by default.
> Avoid storing secrets (API tokens, credentials) directly in shell config files,
> as they will be visible inside every sandbox. Use a credential manager or
> environment variable passthrough instead.

### Environment variables

```nix
cloister.sandboxes.dev.sandbox.env = {
  EDITOR = "nvim";
  RUST_BACKTRACE = "1";
};
```

`PATH` is computed from packages and cannot be overridden here. Base variables (`HOME`, `USER`, `SHELL`, `TERM`, `LOCALE_ARCHIVE`, etc.) use `mkDefault` so your values take precedence.

The `CLOISTER` env var is set to the sandbox name inside the sandbox (e.g., `CLOISTER=dev`).

`LOCALE_ARCHIVE` is set to the `glibcLocales` store path by default, providing glibc locale support inside the sandbox. This prevents "cannot set LC_ALL" warnings and ensures correct date formatting, sorting, and Unicode handling.

To pass through host environment variables when they are set, use `sandbox.passthroughEnv`:

```nix
cloister.sandboxes.dev.sandbox.passthroughEnv = [
  "LANG"
  "LC_ALL"
  "NIX_PATH"
];
```

Locale variables (`LANG` and `LC_*`) are included by default.

### Shell init snippet

Run arbitrary shell code inside the sandbox on session start:

```nix
cloister.sandboxes.dev.init.text = ''
  export GREETING="hello from the sandbox"
'';
```

## Paths and Environment Variables

Cloister strictly disallows bash environment variable expansions (like `$HOME`, `$XDG_RUNTIME_DIR`, or `$USER`) in configuration options related to paths (such as `sandbox.perDirBase`, `copyFiles`, `binds`, etc.). This is a security measure to prevent shell injection and path evaluation vulnerabilities during wrapper script execution.

Instead of bash variables, you should use **Nix-native path evaluations**. Since this module is configured within `home-manager`, you have full access to your home directory, runtime directories, and config paths through `config`.

**❌ Incorrect (Shell variables, will fail validation):**

```nix
cloister.sandboxes.dev.sandbox = {
  perDirBase = "$HOME/.local/state/cloister";
  copyFiles = [
    {
      src = "$XDG_CONFIG_HOME/task/taskrc";
      dest = "$HOME/.config/task/taskrc";
      mode = "0644";
    }
  ];
};
```

**✅ Correct (Nix-native pathing):**

```nix
# Ensure you inherit 'config' in your module arguments
{ config, pkgs, ... }:

cloister.sandboxes.dev.sandbox = {
  perDirBase = "${config.home.homeDirectory}/.local/state/cloister";
  copyFiles = [
    {
      src = "${config.xdg.configHome}/task/taskrc";
      dest = "${config.home.homeDirectory}/.config/task/taskrc";
      mode = "0644";
    }
  ];
};
```

### Common Nix replacements:

- `$HOME` ➔ `config.home.homeDirectory`
- `$XDG_CONFIG_HOME` ➔ `config.xdg.configHome`
- `$XDG_DATA_HOME` ➔ `config.xdg.dataHome`
- `$XDG_STATE_HOME` ➔ `config.xdg.stateHome`
- `$XDG_CACHE_HOME` ➔ `config.xdg.cacheHome`

> **Note:** The `dest` paths for home-relative extra binds (`sandbox.extraBinds.required.rw`, `sandbox.extraBinds.optional.ro`, etc.) are implicitly relative to the sandbox home and do not require `$HOME` prefixes.

## Registry

The registry system defines shell aliases, functions, and wrapped commands that work both inside and outside the sandbox:

```nix
cloister.sandboxes.dev.registry = {
  # Simple aliases (available inside sandbox, wrapped outside)
  aliases = {
    ll = "ls -la";
    gs = "git status";
  };

  # Shell functions (available inside sandbox, wrapped outside)
  functions = {
    mkcd = ''
      mkdir -p "$1" && cd "$1"
    '';
  };

  # Commands to wrap outside the sandbox (typing these
  # in your normal shell routes them through cl-dev)
  commands = [ "nvim" "cargo" "claude" "lazygit" ];

  # Names that should NOT be wrapped outside
  noWrap = [ "git" ];
};
```

When `registry.commands` is set, typing the command in your normal shell transparently runs it through the sandbox. For example, `nvim` becomes `cl-dev nvim`. This means you never need to think about entering the sandbox - your tools just work, but sandboxed.

## State persistence

By default, the sandbox is ephemeral - only the working directory survives. To persist tool state (caches, history, databases), declare bind mounts using `sandbox.extraBinds`:

```nix
cloister.sandboxes.dev.sandbox.extraBinds = {
  # Read-write home-relative paths (must exist)
  required.rw = [ ".local/share/atuin" ];

  # Read-write home-relative paths (ok if missing)
  optional.rw = [ ".cargo/registry" ];

  # Read-only home-relative paths (ok if missing)
  optional.ro = [ ".config/starship.toml" ];

  # Volume-backed directories: key is a base directory on the host,
  # values are home-relative paths inside the sandbox.
  # Source: {key}/cloister/{name}/{path} -> dest: $HOME/{path}
  # Directories are created automatically.
  dir."/persist" = [ ".local/share/notes" ];

  # Volume-backed files: same path scheme as dir, but creates
  # individual files instead of directories (touch'd before bwrap).
  file."/persist" = [ ".local/share/myapp/config.db" ];

  # Per-directory state (isolated by a hash of the sandbox directory path)
  # Source: {perDirBase}/$HASH/{path} -> dest: $HOME/{path}
  perDir = [ ".local/state/cargo-target" ];
};
```

### Writable config file copies

If you need a writable copy of a configuration file that only exists inside the sandbox (e.g. to modify it without affecting the host), you can use `copyFiles`:

```nix
cloister.sandboxes.dev.sandbox = {
  copyFileBase = "/local/ephemeral"; # optional, defaults to "${config.xdg.stateHome}/cloister"
  copyFiles = [
    {
      src = "${config.home.homeDirectory}/.config/task/home-manager-taskrc";
      dest = "${config.home.homeDirectory}/.config/task/taskrc";
      mode = "0644";
      overwrite = false; # if false, only copies when dest doesn't exist
    }
  ];
};
```

This automatically creates a volume-backed file bind and performs the host-side copy before the sandbox launches.

### Home-manager managed files

If you manage config files through home-manager (`xdg.configFile`, `home.file`), you can bind their Nix store sources directly into the sandbox - read-only and tamper-proof:

```nix
cloister.sandboxes.dev.sandbox.extraBinds.managedFile = [
  "bat"             # prefix - binds bat/config, bat/themes/*, etc.
  "gh"              # prefix - binds gh/config.yml
  "starship.toml"   # exact key
  ".claude"          # prefix outside ~/.config/ - binds .claude/* from home.file
];
```

### Disabling working directory binding

App-specific sandboxes (like Discord or Chromium) don't need access to the host directory they're launched from. Disable the working directory bind for tighter isolation:

```nix
cloister.sandboxes.discord.sandbox = {
  bindWorkingDirectory = false;
  extraBinds.dir."/persist" = [
    ".config/discord"
    ".cache/discord"
  ];
};
```

When `bindWorkingDirectory` is false, the sandbox skips directory detection entirely and starts in the sandbox home directory. This is incompatible with `extraBinds.perDir` (which requires a working directory hash).

### Per-directory state base

Per-directory state is stored under a configurable base directory, with subdirectories named by a hash of the sandbox directory path:

```nix
cloister.sandboxes.dev.sandbox.perDirBase = "${config.xdg.stateHome}/cloister";  # default
```

## Security & isolation

### Network isolation

```nix
cloister.sandboxes.dev.network.enable = true;   # default - share host network
cloister.sandboxes.pdf.network.enable = false; # no network access
```

When `network.enable` is `true`, the sandbox shares the host network namespace (`--share-net`). When `false`, the sandbox does not share host networking and seccomp also denies new `AF_NETLINK` sockets.

### Network namespace

To route all sandbox network traffic through a specific Linux network namespace (for example, a VPN namespace), set:

```nix
cloister.sandboxes.dev.network.namespace = "vpn";
```

This requires the `cloister-netns` NixOS module on the host system:

```nix
{
  imports = [ cloister.nixosModules.cloister-netns ];
  cloister-netns.enable = true;
}
```

For full details (declarative namespace types, WireGuard and LAN examples, file-based secret options, and all `cloister-netns.*` options), see [Network Namespaces](network-namespace.md).
For veth-based namespace types (`localhost`, `lan`), addresses are auto-assigned from host-level pools (`cloister-netns.addressPools.localhost` and `cloister-netns.addressPools.lan`).
For localhost namespaces, `cloister-netns.firewall.autoOpenLocalhostPorts = true` (default) auto-opens host firewall ports on `veth-<name>` and adds matching accepts in cloister-netns localhost nft `input` rules. Setting it to `false` disables both auto-open paths.

### Git configuration

```nix
cloister.sandboxes.dev.git.enable = true;   # bind .gitconfig and .config/git/config read-only
cloister.sandboxes.pdf.git.enable = false; # default - no git config inside this sandbox
```

When enabled, `.gitconfig` and `.config/git/config` are bound read-only. This includes credential helper configuration. Disabled by default to avoid exposing credential helper configuration.

### Dangerous path detection

The module checks all bind paths at build time against a list of known credential locations (`.ssh`, `.gnupg`, `.aws`, `.kube`, `.docker/config.json`, keyrings, etc.). If any match, the build fails with a clear error explaining the risk.

> **Note:** This is a best-effort, informational check designed to prevent accidental exposure of common secrets. It is not a strict security boundary, as it relies on static analysis and cannot detect if a user binds a symlink pointing to a sensitive location.

To acknowledge specific paths as intentionally bound:

```nix
cloister.sandboxes.dev.sandbox.allowDangerousPaths = [ ".config/gh/hosts.yml" ];
```

To disable all path checks: `sandbox.dangerousPathWarnings = false`.

## Desktop integration

### Wayland

```nix
cloister.sandboxes.dev.gui.wayland.enable = true;
```

By default, `wp-security-context-v1` is required - the compositor filters which protocol globals are advertised to the sandbox, hiding privileged extensions (screencopy, virtual keyboard injection, etc.). Disable with `gui.wayland.securityContext.enable = false` for raw socket passthrough.

### X11

```nix
cloister.sandboxes.dev.gui.x11.enable = true;
```

> **Warning:** X11 provides no client isolation. Any X11 client can keylog, take screenshots, and inject input into other clients on the same display. Prefer Wayland with `securityContext` for GUI applications.

### GPU acceleration

```nix
cloister.sandboxes.dev.gui.gpu.enable = true;  # auto-enabled when Wayland or X11 is on
cloister.sandboxes.dev.gui.gpu.shm = true;     # default - private tmpfs at /dev/shm for GPU drivers
```

Binds `/dev/dri` into the sandbox for hardware-accelerated rendering. Auto-enabled when Wayland or X11 is active, but can be explicitly disabled with `gui.gpu.enable = false`. A private tmpfs is mounted at `/dev/shm` by default (not the host's `/dev/shm`) since most GPU drivers and multi-process applications (Chromium, Firefox) require POSIX shared memory.

In addition to `/dev/dri`, the sandbox binary automatically detects and binds the following paths when they exist (all as `--ro-bind`, not `--dev-bind`):

- **`/run/opengl-driver`** - NixOS-specific Mesa driver libraries. Without this, GPU apps fail to find `libGL`, `libEGL`, and driver backends.
- **`/sys/dev/char`** - character device node resolution. Allows `libdrm` to map `/dev/dri/cardN` major:minor numbers to their sysfs device nodes.
- **GPU PCI sysfs paths** - auto-detected from `/sys/class/drm/card*` symlinks. These provide vendor/device IDs that Mesa and Vulkan drivers query to identify the GPU hardware.

These binds are detected at runtime by the compiled sandbox binary, so they work across different hardware configurations without per-sandbox configuration.

### HiDPI scaling

```nix
cloister.sandboxes.chromium.gui.scaleFactor = 2.0;
```

When set, `GDK_SCALE`, `GDK_DPI_SCALE`, and `QT_SCALE_FACTOR` are configured inside the sandbox so that GUI applications render at the correct size on HiDPI displays. Set this to the host's display scale (e.g. `2.0` for a 2× HiDPI display). When `null` (default), no scaling variables are set and applications use their own defaults.

### GTK theme

```nix
cloister.sandboxes.dev.gui.gtk = {
  enable = true;    # default - auto-enabled when Wayland or X11 is on
  theme = "Adwaita"; # default
};
```

When `gui.gtk.enable` is true, `GTK_THEME` is set inside the sandbox and `gtk3`/`gtk4` are added to the default `gui.dataPackages`, providing built-in Adwaita theme assets. GTK is auto-enabled whenever a GUI display protocol is active, but can be explicitly disabled with `gui.gtk.enable = false` (e.g., for Qt-only apps that don't need GTK).

For alternative themes, add the theme package and set the name:

```nix
cloister.sandboxes.myapp.gui.gtk = {
  theme = "Adwaita:dark";
  packages = with pkgs; [ adw-gtk3 ];  # merged into XDG_DATA_DIRS
};
```

`GTK_THEME` cannot be set directly via `sandbox.env` when GUI is enabled - use `gui.gtk.theme` instead.

### Qt theme

```nix
cloister.sandboxes.dev.gui.qt = {
  enable = true;
  # platformTheme = "gtk3";  # default - reads GTK_THEME, built into qtbase
  # style = null;            # default - no QT_STYLE_OVERRIDE
};
```

When `gui.qt.enable` is true, `QT_QPA_PLATFORMTHEME` is set inside the sandbox. The default `"gtk3"` platform theme is built into qtbase and reads `GTK_THEME`, so Qt apps follow the GTK theme automatically when `gui.gtk` is also enabled.

For apps needing additional Qt plugins (e.g., `qt6ct` for fine-grained control):

```nix
cloister.sandboxes.myapp.gui.qt = {
  enable = true;
  platformTheme = "qt6ct";
  packages = with pkgs; [ qt6ct ];  # adds to QT_PLUGIN_PATH (both qt-5 and qt-6 paths)
};
```

To force a specific Qt style (sets `QT_STYLE_OVERRIDE`):

```nix
cloister.sandboxes.myapp.gui.qt.style = "Fusion";
```

`QT_QPA_PLATFORMTHEME`, `QT_STYLE_OVERRIDE`, and `QT_PLUGIN_PATH` cannot be set directly via `sandbox.env` when Qt is enabled - use the `gui.qt.*` options instead.

### Icon themes and XDG data

```nix
cloister.sandboxes.dev.gui.dataPackages = with pkgs; [ hicolor-icon-theme gtk3 gtk4 gsettings-desktop-schemas ];  # default when GTK is enabled
```

When a GUI display protocol is enabled, `XDG_DATA_DIRS` is computed from `gui.dataPackages` (plus `gui.gtk.packages`) - each package's `/share` directory is included. The defaults provide:

- **`hicolor-icon-theme`** - the freedesktop fallback icon theme required by GTK and Qt (always included when GUI is enabled)
- **`gtk3`** / **`gtk4`** - built-in Adwaita theme assets (only included when `gui.gtk.enable` is true)
- **`gsettings-desktop-schemas`** - GSettings schemas for desktop settings (only included when `gui.gtk.enable` is true)

To add additional icon themes or MIME type databases:

```nix
cloister.sandboxes.geeqie.gui.dataPackages = with pkgs; [
  hicolor-icon-theme
  gtk3
  gtk4
  gsettings-desktop-schemas
  adwaita-icon-theme
  shared-mime-info
];
```

`XDG_DATA_DIRS` cannot be set directly via `sandbox.env` when GUI is enabled - use `gui.dataPackages` instead.

### Fonts

```nix
cloister.sandboxes.dev.gui.fonts.packages = with pkgs; [ dejavu_fonts ];  # default when GUI is enabled
```

When a GUI display protocol is enabled, a self-contained fontconfig configuration is generated via `pkgs.makeFontsConf` and injected into the sandbox as `FONTCONFIG_FILE`. This replaces the previous host `/etc/fonts` bind mount, making font rendering a declared sandbox property instead of a host-dependent side-effect.

The default provides **`dejavu_fonts`** - a widely-compatible font family covering Latin, Greek, Cyrillic, and more. To add additional fonts:

```nix
cloister.sandboxes.myapp.gui.fonts.packages = with pkgs; [
  dejavu_fonts
  noto-fonts
  noto-fonts-cjk-sans
];
```

Set to an empty list to disable the generated fontconfig entirely (e.g., if the application bundles its own fonts):

```nix
cloister.sandboxes.myapp.gui.fonts.packages = lib.mkForce [ ];
```

`FONTCONFIG_FILE` cannot be set directly via `sandbox.env` when GUI is enabled - use `gui.fonts.packages` instead.

### Desktop entries

```nix
cloister.sandboxes.chromium.gui.desktopEntry = {
  enable = true;
  name = "Chromium (Sandboxed)";
  execArgs = "%U";
  icon = "chromium";
  categories = [ "Network" "WebBrowser" ];
  mimeType = [ "text/html" "x-scheme-handler/http" "x-scheme-handler/https" ];
};
```

Generates an XDG `.desktop` file so the sandbox appears in your app launcher. Requires a GUI display protocol to be enabled. The `Exec` line is built from `defaultCommand` (or `cl-<name>` when unset) with `execArgs` appended. When `name` is empty, it falls back to `cl-<name>`.

### Device passthrough

```nix
cloister.sandboxes.dev.sandbox.devBinds = [ "/dev/video0" ];
```

Passes arbitrary device nodes into the sandbox with `--dev-bind`. Missing devices are warned about at runtime rather than failing. Useful for webcams, hardware tokens, or other device access.

### Examples

See the [examples/](../examples/) directory and the [README](../README.md#examples) for complete, importable sandbox configurations.

### PipeWire vs PulseAudio

On modern NixOS the default sound server is PipeWire, which also exposes a PulseAudio-compatible socket. Cloister can forward either socket into the sandbox:

| Option | Protocol | Audio | Screen sharing / cameras | Filtering |
|--------|----------|-------|--------------------------|-----------|
| `audio.pipewire.enable` | PipeWire native | Yes | Yes | Yes (`filters.*`) |
| `audio.pipewire.pulseCompat.enable` | PipeWire + PulseAudio bridge | Yes | Yes | Yes (`filters.*`) |
| `audio.pulseaudio.enable` | PulseAudio | Yes | No | No |

**Recommended approach**: use `audio.pipewire` with filtering for most sandboxes. This gives apps the native PipeWire protocol (needed for portal-based screen sharing and camera access) while restricting exactly which devices are visible. The `pulseCompat` bridge (enabled by default with PipeWire) runs `pipewire-pulse` inside the sandbox so that apps using `libpulse` (Firefox, Chromium, Electron apps, mpv, VLC, etc.) work transparently — all traffic still flows through the filtered PipeWire socket, and filtered mode now preserves the internal PipeWire factories needed for playback, microphone capture, and camera/video capture, including link creation for Pulse-compatible streams.

ALSA compatibility (`alsa.enable`) is opt-in and only needed for software that speaks raw ALSA (e.g. some games, Wine, JACK bridges). Most applications use PulseAudio or PipeWire natively.

Fall back to `audio.pulseaudio` only for sandboxes that need direct host PulseAudio socket forwarding without PipeWire. Note that `pulseCompat` and `pulseaudio` are mutually exclusive.

### PipeWire

```nix
cloister.sandboxes.dev.audio.pipewire = {
  enable = true;
  # alsa.enable = true;  # only needed for raw-ALSA software (games, Wine)
  filters = {
    enable = true;
    audioOut = true;  # speakers (default)
    # audioIn = true; # microphones
    # videoIn = true; # webcams (also needs video.enable)
  };
};
```

Forwards the PipeWire native socket (`$XDG_RUNTIME_DIR/pipewire-0`) into the sandbox. When `filters.enable = true`, a dedicated restricted socket is created instead, exposing only the device classes and capabilities you specify while still allowing clients to create the playback/capture streams needed for enabled sinks, microphones, and cameras. See the [PipeWire filtering guide](pipewire.md) for the full option reference.

### PulseAudio

```nix
cloister.sandboxes.dev.audio.pulseaudio.enable = true;
```

Forwards the PulseAudio socket for audio playback and recording. Works with both PulseAudio and PipeWire's PulseAudio compatibility layer. Does not support filtering — the sandbox gets unrestricted audio access.

### Webcam/Camera

```nix
cloister.sandboxes.dev.video.enable = true;
```

Binds `/dev/video*` devices and related sysfs paths for webcam/camera access. At runtime, the sandbox binary scans `/sys/class/video4linux/` to discover all V4L2 video devices and their USB/PCI parent device paths, binding each one into the sandbox. Useful for video calls in sandboxed browsers or video capture applications.

### Printing

```nix
cloister.sandboxes.dev.printing.enable = true;
```

Forwards the CUPS printing socket (`/run/cups/cups.sock`) into the sandbox. Sets `CUPS_SERVER` to the socket path so applications can discover the printer. The socket is bound read-only.

### D-Bus notifications

```nix
cloister.sandboxes.dev.dbus = {
  enable = true;
  policies.talk = [ "org.freedesktop.Notifications" ];
};
```

Allows sandboxed tools to send desktop notifications through a filtered, per-sandbox D-Bus proxy. See [docs/dbus.md](docs/dbus.md) for policy examples and setup details.

### SSH agent

```nix
cloister.sandboxes.dev.ssh.enable = true;
```

Forwards `SSH_AUTH_SOCK` into the sandbox when set on the host.

To filter which keys are visible, set `ssh.allowFingerprints`. When filtering is enabled,
the proxy uses a read/write timeout (default 60s) that can be tuned for interactive agents:

```nix
cloister.sandboxes.dev.ssh = {
  enable = true;
  allowFingerprints = [ "SHA256:..." ];
  filterTimeoutSeconds = 60; # set 0 to disable timeouts
};
```

> **Security Note:** Forwarding the SSH agent socket allows any process inside the sandbox to use your loaded keys to authenticate or sign commits. To mitigate the risk of a compromised tool misusing your agent, it is highly recommended to use hardware-backed keys that require physical touch (e.g., FIDO2) or add keys to the agent with confirmation required (`ssh-add -c`). Alternatively, you can run a separate, restricted `ssh-agent` specifically for lower-trust sandboxes.

## Default command

For app-specific sandboxes, `defaultCommand` specifies the command to run when the sandbox binary is invoked without arguments. This turns `cl-<name>` from an interactive shell into a direct app launcher:

```nix
cloister.sandboxes.geeqie = {
  extraPackages = with pkgs; [ geeqie ];
  defaultCommand = [ "geeqie" ];
  gui.wayland.enable = true;
};
```

Now `cl-geeqie` launches geeqie directly instead of opening a shell. Additional arguments are appended to the default command, so `cl-geeqie photo.jpg` runs `geeqie photo.jpg`. To run a different command explicitly, use `-c`: `cl-geeqie -c some-other-command`. Bare `-c` is invalid and exits with a usage error. This is especially useful with `gui.desktopEntry` - the desktop entry's `Exec` line uses `defaultCommand` automatically.

## Identity anonymization

Enable `sandbox.anonymize.enable` to present a generic identity inside the sandbox:

```nix
cloister.sandboxes.untrusted.sandbox.anonymize.enable = true;
```

When enabled, the sandbox:

- Sets username and hostname to `ubuntu` (synthetic `/etc/passwd` and `/etc/group`)
- Uses `/home/ubuntu` as the home directory instead of your real home path
- Masks `/proc` entries that would reveal host identity (`/proc/sys/kernel/hostname`, `/proc/sys/kernel/domainname`, `/proc/sys/kernel/osrelease`, `/proc/sys/kernel/random/boot_id`)
- Remaps all bind mount destinations from your real home to `/home/ubuntu`

This prevents casual identity leakage to untrusted tools but is not a strict security boundary - see [Security Model](security.md) for details.

# Options reference

See the sections above for usage examples and explanations.

## Global options

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `cloister.enable` | bool | `false` | Gate the entire module |
| `cloister.defaultShell` | enum | `"zsh"` | Default interactive shell for sandboxes (`"zsh"` or `"bash"`) |

## Per-sandbox options (`cloister.sandboxes.<name>.*`)

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `defaultCommand` | nullOr (listOf str) | `null` | Default command prefix used when invoked without args, or when appending positional args |
| `packages` | list of package | *(core set)* | Base packages on the sandbox PATH |
| `extraPackages` | list of package | `[]` | Additional packages appended to PATH |
| `shell.name` | enum | `cloister.defaultShell` | Interactive shell (`"zsh"` or `"bash"`) |
| `shell.hostConfig` | bool | `true` | Bind host shell config files into the sandbox |
| `shell.customRcPath.zshenv` | nullOr path | `null` | Custom zshenv file to source inside the sandbox |
| `shell.customRcPath.zshrc` | nullOr path | `null` | Custom zshrc file to source inside the sandbox |
| `shell.customRcPath.bashenv` | nullOr path | `null` | Custom bashenv file to source inside the sandbox |
| `shell.customRcPath.bashrc` | nullOr path | `null` | Custom bashrc file to source inside the sandbox |
| `shell.customRcPath.profile` | nullOr path | `null` | Custom profile file to source inside the sandbox |
| `validators.enable` | bool | `false` | Install cloister validator helpers and wrap them outside |
| `network.enable` | bool | `true` | Share host network namespace |
| `network.namespace` | nullOr str | `null` | Linux network namespace to join (localhost-netns host services are reachable as `host.internal:<port>`) |
| `sandbox.bindWorkingDirectory` | bool | `true` | Bind-mount the working directory (git root or CWD) into the sandbox. Disable for app-specific sandboxes |
| `sandbox.env` | attrsOf str | *(base vars)* | Environment variables inside sandbox |
| `sandbox.passthroughEnv` | list of str | *(locale vars)* | Host env vars to pass through when set |
| `sandbox.dirs` | list of str | *(system dirs)* | Directories to create inside the sandbox |
| `sandbox.extraDirs` | list of str | `[]` | Additional directories appended to sandbox dirs |
| `sandbox.tmpfs` | list of str | `["/tmp"]` | Tmpfs mounts inside the sandbox |
| `sandbox.symlinks` | list of {target, link} | `[]` | Symlinks to create inside the sandbox |
| `sandbox.extraSymlinks` | list of {target, link} | `[]` | Additional symlinks appended to sandbox symlinks |
| `sandbox.binds.ro` | list of bind | *(system paths)* | Read-only bind mounts |
| `sandbox.binds.rw` | list of bind | *(working dir)* | Read-write bind mounts |
| `sandbox.extraBinds.required.ro` | list of str | `[]` | Home-relative required read-only binds |
| `sandbox.extraBinds.required.rw` | list of str | `[]` | Home-relative required read-write binds |
| `sandbox.extraBinds.optional.ro` | list of str | `[]` | Home-relative optional read-only binds |
| `sandbox.extraBinds.optional.rw` | list of str | `[]` | Home-relative optional read-write binds |
| `sandbox.extraBinds.dir` | attrsOf (list of str) | `{}` | Volume-backed directory binds (auto-created) |
| `sandbox.extraBinds.file` | attrsOf (list of str) | `{}` | Volume-backed file binds (auto-created) |
| `sandbox.extraBinds.perDir` | list of str | `[]` | Per-directory binds (isolated by dir hash) |
| `sandbox.extraBinds.managedFile` | list of str | `[]` | Home-manager managed file keys bound read-only |
| `sandbox.perDirBase` | str | `"${config.xdg.stateHome}/cloister"` | Per-directory state base directory |
| `sandbox.dangerousPathWarnings` | bool | `true` | Fail on binds to known credential locations |
| `sandbox.allowDangerousPaths` | list of str | `[]` | Acknowledged credential paths to allow |
| `sandbox.enforceStrictHomePolicy` | bool | `true` | Prevent sandboxing home dirs and dot-dirs |
| `sandbox.disallowedPaths` | list of str | `["/", "/root"]` | Paths disallowed as sandbox directory |
| `sandbox.copyFileBase` | str | `"${config.xdg.stateHome}/cloister"` | Base directory on the host where copyFiles are stored |
| `sandbox.copyFiles` | list of {src, dest, mode, overwrite} | `[]` | Files to copy writable into the sandbox state |
| `sandbox.anonymize.enable` | bool | `false` | Present generic identity (username/hostname `ubuntu`, masked `/proc` entries) |
| `gui.wayland.enable` | bool | `false` | Forward Wayland display socket |
| `gui.wayland.securityContext.enable` | bool | `true` | Require wp-security-context-v1 for Wayland |
| `gui.x11.enable` | bool | `false` | Forward X11 DISPLAY variable |
| `gui.gpu.enable` | bool | `false`\* | Bind /dev/dri for GPU acceleration (*auto-enabled with Wayland/X11) |
| `gui.gpu.shm` | bool | `true` | Mount a private tmpfs at /dev/shm when GPU is enabled (does not expose host shared memory) |
| `gui.scaleFactor` | nullOr float | `null` | Display scale factor for HiDPI (sets `GDK_SCALE`, `GDK_DPI_SCALE`, `QT_SCALE_FACTOR`) |
| `gui.dataPackages` | list of package | `[hicolor-icon-theme]`* | Packages whose `/share` dirs form `XDG_DATA_DIRS` (*`gtk3`/`gtk4`/`gsettings-desktop-schemas` added when `gui.gtk.enable`) |
| `gui.fonts.packages` | list of package | `[]`* | Font packages for fontconfig (*`dejavu_fonts` added when Wayland/X11 enabled) |
| `gui.gtk.enable` | bool | `false`* | Enable GTK theming (*auto-enabled with Wayland/X11) |
| `gui.gtk.theme` | str | `"Adwaita"` | GTK theme name (sets `GTK_THEME` env var) |
| `gui.gtk.packages` | list of package | `[]` | Additional GTK theme packages merged into `XDG_DATA_DIRS` |
| `gui.qt.enable` | bool | `false` | Enable Qt theming (`QT_QPA_PLATFORMTHEME`, etc.) |
| `gui.qt.platformTheme` | str | `"gtk3"` | Qt platform theme plugin (sets `QT_QPA_PLATFORMTHEME`) |
| `gui.qt.style` | nullOr str | `null` | Qt style override (sets `QT_STYLE_OVERRIDE` when non-null) |
| `gui.qt.packages` | list of package | `[]` | Qt plugin packages (added to `QT_PLUGIN_PATH`) |
| `gui.desktopEntry.enable` | bool | `false` | Generate XDG .desktop file for app launchers |
| `gui.desktopEntry.name` | str | `""` | Display name (falls back to `cl-<name>`) |
| `gui.desktopEntry.execArgs` | str | `""` | Extra arguments appended after the sandbox binary path (e.g. `%U`) |
| `gui.desktopEntry.icon` | str | `""` | Icon name or path |
| `gui.desktopEntry.categories` | list of str | `[]` | XDG categories |
| `gui.desktopEntry.mimeType` | list of str | `[]` | MIME types handled |
| `gui.desktopEntry.terminal` | bool | `false` | Run in terminal |
| `gui.desktopEntry.genericName` | str | `""` | Generic name (e.g. "Web Browser") |
| `gui.desktopEntry.comment` | str | `""` | Tooltip/comment |
| `gui.desktopEntry.startupNotify` | bool | `false` | Startup notification support |
| `sandbox.devBinds` | list of str | `[]` | Device paths for --dev-bind passthrough |
| `sandbox.seccomp.enable` | bool | `true` | Apply seccomp-bpf filter blocking dangerous syscalls |
| `sandbox.seccomp.allowChromiumSandbox` | bool | `false` | Allow Chromium/Electron internal sandbox syscalls (chroot, namespaces) |
| `ssh.enable` | bool | `false` | Forward SSH agent socket |
| `ssh.allowFingerprints` | list of str | `[]` | Restrict visible SSH keys to these fingerprints |
| `ssh.filterTimeoutSeconds` | unsigned int | `60` | SSH filter read/write timeout; set `0` to disable |
| `git.enable` | bool | `false` | Bind `.gitconfig` and `.config/git/config` read-only |
| `dbus.enable` | bool | `false` | Per-sandbox D-Bus proxy |
| `dbus.log` | bool | `false` | Enable xdg-dbus-proxy logging |
| `dbus.portal.enable` | bool | `false` | Enable xdg-desktop-portal integration (.flatpak-info, portal policies, GTK_USE_PORTAL) |
| `dbus.portal.documentFUSE.enable` | bool | `true` | Bind the document portal FUSE mount at `/run/flatpak/doc` |
| `dbus.policies.talk` | list of str | `["org.freedesktop.Notifications"]` | D-Bus TALK allowlist |
| `dbus.policies.own` | list of str | `[]` | D-Bus OWN allowlist |
| `dbus.policies.see` | list of str | `[]` | D-Bus SEE allowlist |
| `dbus.policies.call` | attrsOf (list of str) | `{}` | Per-name call rules |
| `dbus.policies.broadcast` | attrsOf (list of str) | `{}` | Per-name broadcast rules |
| `audio.pulseaudio.enable` | bool | `false` | Forward PulseAudio socket for audio |
| `audio.pipewire.enable` | bool | `false` | Forward PipeWire native socket |
| `audio.pipewire.alsa.enable` | bool | `false` | Expose ALSA compatibility via PipeWire's ALSA plugin (only needed for raw-ALSA software) |
| `audio.pipewire.pulseCompat.enable` | bool | `pipewire.enable && !pulseaudio.enable` | Run in-sandbox pipewire-pulse bridge for PulseAudio protocol compatibility |
| `video.enable` | bool | `false` | Bind /dev/video* devices for webcam/camera access |
| `printing.enable` | bool | `false` | Forward CUPS printing socket |
| `fido2.enable` | bool | `false` | Bind /dev/hidraw\* devices for FIDO2/U2F security key access |
| `registry.aliases` | attrsOf str | `{}` | Shell aliases |
| `registry.functions` | attrsOf lines | `{}` | Shell functions |
| `registry.commands` | list of str | `[]` | Commands to wrap outside sandbox |
| `registry.extraCommands` | list of str | `[]` | Additional commands appended to wrapped commands |
| `registry.noWrap` | list of str | `[]` | Names to exclude from wrapping |
| `init.text` | lines | `""` | Shell snippet sourced inside the sandbox |
