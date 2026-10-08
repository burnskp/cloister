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
    gui.enable = true;
    extraPackages = with pkgs; [ zathura imv ];
    registry.commands = [ "zathura" "imv" ];
  };
};
```

This produces `cl-dev` and `cl-pdf` binaries. Wrapped commands route to the correct sandbox automatically (e.g., `nvim` -> `cl-dev nvim`, `zathura` -> `cl-pdf zathura`).

Cross-sandbox name collisions are detected at build time - two sandboxes cannot wrap the same command name outside.

### Presets

See `docs/presets.md` for the available presets (`hardened`, `developer`, `gui`, `chromium`), examples, and override behavior.

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

### Nix store modes

Cloister supports two ways to expose `/nix/store` inside a sandbox.

- `sandbox.nixStore.mode = "host"` bind-mounts the host `/nix/store` directly.
- `sandbox.nixStore.mode = "image-store"` mounts a prebuilt immutable store image at `/nix/store`.

`image-store` shifts store preparation into `nixos-rebuild`: Cloister computes a stable store ID from the store paths the sandbox needs, builds a `squashfs` image for that closure, publishes it under `cloister.imageStore.base`, and mounts it system-wide under `cloister.imageStore.mountBase`.

When using `image-store`:

- the internally managed base packages and configured `extraPackages` are available on `PATH` and their runtime closures are exposed automatically
- some other store paths are included when Cloister references them directly, such as shell binaries, generated config files, or symlink targets

```nix
cloister.sandboxes.editor = {
  extraPackages = with pkgs; [
    helix
    nil
    lua-language-server
  ];

  sandbox = {
    nixStore.mode = "image-store";

    managed = [ "helix/languages.toml" ];
  };
};
```

To use `image-store`, also import the NixOS `cloister-image-store` module and set the global storage location if you want something other than the defaults:

```nix
{
  imports = [ inputs.cloister.nixosModules.cloister-image-store ];

  cloister.imageStore = {
    base = "/var/lib/cloister/images";
    mountBase = "/run/cloister/images";
  };
}
```

To clean up stale published image links periodically, set `cloister.imageStore.enable = true;` and optionally adjust `cloister.imageStore.interval` in the NixOS module.

If you want a complete example, see `examples/image-store.nix`.

### Validator helpers

Wayland, D-Bus, seccomp, and PipeWire validators are available in the separate `cloister-diagnostics` package. They are not installed in sandboxes by default.

```nix
environment.systemPackages = [ cloister.packages.${pkgs.system}.cloister-diagnostics ];
```

### Shell

```nix
cloister.sandboxes.dev.shell.name = "zsh";  # default is cloister.defaultShell ("zsh"); also supports "bash"
```

This controls the interactive shell inside the sandbox and the wrapper integration outside.

Host shell config files are not bound by default. To bind them and/or add sandbox-specific rc files:

```nix
cloister.sandboxes.dev.shell = {
  hostConfig = true; # opt in to host shell startup files
  customRcPath = {
    zshenv = ./configs/zsh/dev.zshenv;
    zshrc = ./configs/zsh/dev.zshrc;
  };
};
```

Host config binds include the shell-specific config directories as well as the usual rc files (`~/.config/zsh`, `~/.zshenv`, `~/.zshrc` for zsh; `~/.config/bash`, `~/.bash_profile`, `~/.bashrc` for bash).

Custom rc files are bound into `~/.config/cl-shell/<name>/custom/` and sourced in this order:

1. Host shell startup files (when `hostConfig = true`, via the shell's normal startup path)
1. Custom rc files (when set)
1. `init.text`
1. Registry snippet (always last, so registry wins)

Notes:

- `customRcPath` entries are Nix paths (e.g. `./configs/zsh/dev.zshrc`), so they land in the Nix store and are bind-mounted read-only into the sandbox.
- Leave `hostConfig = false` (the default) to avoid binding any host shell config files.

Example: two sandboxes, two different zshrc subsets

See `examples/shell-custom-rc.nix` and the companion files under `examples/configs/zsh/`.

> **Note:** When enabled, shell config files are bound read-only into the sandbox.
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

When worker broker is enabled for a sandbox, Cloister also prepends generated `clb-<profile>` launchers to that sandbox's `PATH`, so they are available by plain name inside the parent sandbox.

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

Cloister strictly disallows bash environment variable expansions (like `$HOME`, `$XDG_RUNTIME_DIR`, or `$USER`) in host-side path options such as `sandbox.state.projectDirs`, `sandbox.readOnly` / `sandbox.readWrite` attr sources, `sandbox.copyBase`, `sandbox.copies.*.src`, device paths, and similar values. This is a security measure to prevent shell injection and path evaluation vulnerabilities during wrapper script execution.

Instead of bash variables in host paths, use **Nix-native path evaluations**. Since this module is configured within `home-manager`, you have full access to your home directory, runtime directories, and config paths through `config`.

**❌ Incorrect (shell variables in host paths, will fail validation):**

```nix
cloister.sandboxes.dev.sandbox = {
  state.projectDirs."$HOME/.local/state/cloister" = [ ".cache/nix" ];
  copies = [
    {
      src = "$XDG_CONFIG_HOME/task/taskrc";
      dest = "$HOME/.config/task/taskrc";
      mode = "0644";
    }
  ];
};
```

This fails because `state.projectDirs` keys and `copies.*.src` are host-side paths. The `copies.*.dest` value shown here is sandbox-side and is valid; see the note below.

**✅ Correct (Nix-native host pathing):**

```nix
# Ensure you inherit 'config' in your module arguments
{ config, pkgs, ... }:

cloister.sandboxes.dev.sandbox = {
  state.projectDirs."${config.home.homeDirectory}/.local/state/cloister" = [ ".cache/nix" ];
  copies = [
    {
      src = "${config.xdg.configHome}/task/taskrc";
      dest = "${config.home.homeDirectory}/.config/task/taskrc";
      mode = "0644";
    }
  ];
};
```

`copies.*.dest` is sandbox-side and must resolve under the sandbox home. It may be written as `$HOME/<path>` or as `${config.home.homeDirectory}/<path>`; Cloister normalizes the latter to `$HOME/<path>` internally so anonymized sandboxes can remap it to `/home/<username>`.

### Common Nix replacements for host paths:

- `$HOME` ➔ `config.home.homeDirectory`
- `$XDG_CONFIG_HOME` ➔ `config.xdg.configHome`
- `$XDG_DATA_HOME` ➔ `config.xdg.dataHome`
- `$XDG_STATE_HOME` ➔ `config.xdg.stateHome`
- `$XDG_CACHE_HOME` ➔ `config.xdg.cacheHome`

> **Note:** String entries in `sandbox.readOnly` and `sandbox.readWrite`, plus state/managed destinations, are home-relative and do not require `$HOME` prefixes.

## Registry

The registry system defines shell aliases, functions, and wrapped commands that work both inside and outside the sandbox:

```nix
cloister.sandboxes.dev.registry = {
  # Simple argv-style aliases (available inside sandbox, wrapped outside)
  aliases = {
    ll = "ls -la";
    gs = "git status";
  };

  # Shell functions (available inside sandbox, wrapped outside)
  # Use these for pipes, redirects, quoting, conditionals, or builtins.
  functions = {
    mkcd = ''
      mkdir -p "$1" && cd "$1"
    '';
  };

  # Commands to wrap outside the sandbox (typing these
  # in your normal shell routes them through cl-dev)
  commands = [ "nvim" "cargo" "claude" ];

  # Commands that should run through the shell startup path first
  # (useful for direnv, init.text, shell functions, etc.)
  interactiveCommands = [ "lazygit" "opencode" ];

  # Names that should NOT be wrapped outside
  noWrap = [ "git" ];
};
```

When `registry.commands` is set, typing the command in your normal shell transparently runs it through the sandbox. For example, `nvim` becomes `cl-dev nvim`. This means you never need to think about entering the sandbox - your tools just work, but sandboxed.

If a wrapped command depends on shell setup inside the sandbox (for example `direnv` hooks, shell functions, or environment initialization added by `init.text`), put it in `registry.interactiveCommands`. Cloister will then emit wrappers that invoke `cl-<name> -i ...`, which runs the target command through the interactive shell startup path before execution. When `direnv` is installed inside the sandbox, that `-i` path also delegates to `direnv exec "$PWD" ...` before launching the target so one-shot commands inherit the repo environment without requiring a custom wrapper.

Outside-wrapped aliases are intentionally limited to argv-safe strings. If an alias needs shell syntax such as pipes, redirects, variable expansion, quoting, or builtins, put it in `registry.functions` instead.

Outside wrapping is installed through the matching Home Manager shell module. Enable `programs.zsh.enable = true` for zsh-based sandboxes or `programs.bash.enable = true` for bash-based sandboxes so the generated init hook is sourced in your normal shell.

## State persistence

By default, the sandbox is ephemeral - only the working directory survives. Use semantic sandbox buckets to declare host binds and persisted state:

```nix
cloister.sandboxes.dev.sandbox = {
  # Host-home paths bound directly into the sandbox.
  readOnly = [
    ".config/starship.toml"
    { src = ".cache/bat"; optional = true; }
  ];
  readWrite = [
    ".local/share/atuin"
    { src = ".cargo/registry"; optional = true; }
  ];

  state = {
    # Volume-backed directories: {base}/cloister/{name}/{path} -> $HOME/{path}
    dirs."/persist" = [ ".local/share/notes" ];

    # Volume-backed files use the same path scheme but are touch'd before bwrap.
    files."/persist" = [ ".local/share/myapp/config.db" ];

    # Per-project state is isolated by a hash of the working directory path.
    projectDirs = {
      "/ephemeral" = [ ".local/state/cargo-target" ];
      "/local/worktrees" = [ ".local/worktrees/project" ];
    };
  };
};
```

### Writable config file copies

If you need a writable copy of a host configuration file inside the sandbox (e.g. to modify it without affecting the host), use `sandbox.copies`:

```nix
cloister.sandboxes.dev.sandbox = {
  copies = [
    {
      src = "${config.home.homeDirectory}/.config/task/home-manager-taskrc";
      dest = "${config.home.homeDirectory}/.config/task/taskrc";
      mode = "0644";
      overwrite = false; # if false, only copies when dest doesn't exist
    }
  ];

  # Optional; defaults to "${config.xdg.stateHome}/cloister".
  copyBase = "/local/ephemeral";
};
```

This automatically creates a volume-backed file bind and performs the host-side copy before the sandbox launches.
If `src` is missing or is not a regular file, sandbox startup fails immediately so misconfigured copies are visible.

### Home-manager managed files

If you manage config files through home-manager (`xdg.configFile`, `home.file`), you can bind their Nix store sources directly into the sandbox - read-only and tamper-proof:

```nix
cloister.sandboxes.dev.sandbox.managed = [
  "bat"             # prefix - binds bat/config, bat/themes/*, etc.
  "gh"              # prefix - binds gh/config.yml
  "starship.toml"   # exact key
  ".claude"          # prefix outside ~/.config/ - binds .claude/* from home.file
];
```

For sandbox-only Home Manager profiles that generate files and packages without managing the host, see [Sandbox-local Home Manager Profiles](home-manager.md).

### Explicit store-backed managed files

If you want to bind files directly from the Nix store without managing them in Home Manager, use attr-form `sandbox.managed` entries:

```nix
let
  opencodeAssetsDir = ./assets/opencode;
  opencodeFiles = [
    "tui.json"
    "opencode.json"
  ];
in
{
  cloister.sandboxes.opencode.sandbox = {
    state.projectDirs."/ephemeral" = [ ".config/opencode" ];
    managed = map (relativePath: {
      src = opencodeAssetsDir + "/${relativePath}";
      dest = ".config/opencode/${relativePath}";
    }) opencodeFiles;
  };
}
```

These binds are read-only and overlay correctly on top of `state.dirs` and `state.projectDirs` mounts.

### Sandbox symlinks

Use `sandbox.symlinks` to create symlinks inside the sandbox. Relative `link` paths are resolved under the sandbox home; `target` is the symlink contents and may be relative to the link's parent or absolute inside the sandbox.

```nix
cloister.sandboxes.dev.sandbox = {
  managed = [
    {
      src = config.home.file.".ssh/config".source;
      dest = ".ssh/.config";
    }
  ];

  symlinks = [
    {
      target = ".config";
      link = ".ssh/config";
    }
  ];
};
```

This is useful when a program cares whether a sandbox path is a symlink. For example, OpenSSH accepts a user-owned `~/.ssh/config` symlink to an immutable store file on the host, but rejects a root-owned store file bound directly at `~/.ssh/config`.

### Disabling working directory binding

App-specific sandboxes (like Discord or Chromium) don't need access to the host directory they're launched from. Disable the working directory bind for tighter isolation:

```nix
cloister.sandboxes.discord.sandbox = {
  bindWorkingDirectory = false;
  state.dirs."/persist" = [
    ".config/discord"
    ".cache/discord"
  ];
};
```

When `bindWorkingDirectory` is false, the sandbox skips directory detection entirely and starts in the sandbox home directory. This is incompatible with non-empty `state.projectDirs` (which requires a working directory hash).

Each `state.projectDirs` key gets its own `manifest.json` and its own `${DIR_HASH}` namespace, so you can split ephemeral caches and reboot-persistent project state across different host roots.

## Security & isolation

### Network isolation

```nix
cloister.sandboxes.dev.network.enable = true;   # default - share host network
cloister.sandboxes.pdf.network.enable = false; # no network access
```

When `network.enable` is `true`, the sandbox shares the active network namespace (`--share-net`). When `false`, the sandbox does not share networking and seccomp also denies new `AF_NETLINK` sockets unless `network.namespace` is set.

### DNS servers

Set DNS servers for an individual sandbox:

```nix
cloister.sandboxes.dev.network.dns.nameservers = [
  "1.1.1.1"
  "2606:4700:4700::1111"
];
```

Alternatively, use an existing resolver file, including NetworkManager's:

```nix
cloister.sandboxes.dev.network.dns.resolvConf =
  "/run/NetworkManager/resolv.conf";
```

Both options default to unset (`nameservers = []`, `resolvConf = null`).
A nonempty nameserver list and a resolver file are mutually exclusive.
Nameserver entries must be nonempty and contain no whitespace; IPv4 and IPv6
addresses are supported.

Explicit DNS settings override the host or network namespace resolver and bind
the selected file read-only at `/etc/resolv.conf`. Use a quoted absolute path
for runtime files: the file is accessed at launch, and a missing file fails
startup. Relaunch the sandbox to pick up file replacements, such as those
made by NetworkManager. The file's other resolver directives are preserved.

Without an explicit DNS setting, existing host/namespace DNS selection and
fallback behavior apply. DNS configuration does not enable networking or make
a server reachable from an isolated namespace.

### Network namespace

To route all sandbox network traffic through a specific Linux network namespace (for example, a VPN namespace), set:

```nix
cloister.sandboxes.dev.network.namespace = "vpn";
```

Setting `network.namespace` implies networking for that sandbox even if a preset or override set `network.enable = false`, because bubblewrap must preserve the joined namespace with `--share-net`. This requires the `cloister-netns` NixOS module on the host system plus a declarative namespace definition:

```nix
{
  imports = [ cloister.nixosModules.cloister-netns ];
  cloister-netns.networks.vpn.type = "isolated";
}
```

For full details (declarative namespace types, WireGuard and LAN examples, file-based secret options, and all `cloister-netns.*` options), see [Network Namespaces](network-namespace.md).
For veth-based namespace types (`localhost`, `lan`), addresses are auto-assigned from `cloister-netns.veth.addressPool`.
For localhost namespaces, Cloister always opens host firewall ports on `veth-<name>` and adds matching accepts in cloister-netns localhost nft `input` rules for `allowedPorts`.
If `/etc/netns/<name>/hosts` or `/etc/netns/<name>/resolv.conf` is missing, Cloister falls back to host `/etc/hosts` and `/etc/resolv.conf`. Explicit `network.dns` settings replace resolver selection and disable its fallback.

### Git configuration

```nix
cloister.sandboxes.dev.git.enable = true;   # bind .gitconfig and .config/git/config read-only
cloister.sandboxes.pdf.git.enable = false; # default - no git config inside this sandbox
```

When enabled, `.gitconfig` and `.config/git/config` are bound read-only. This includes credential helper configuration. Disabled by default to avoid exposing credential helper configuration.

## Desktop integration

### GUI display and rendering

```nix
cloister.sandboxes.dev.gui.enable = true;
```

`gui.enable` is the single switch for graphical applications. It forwards Wayland through `wp-security-context-v1`; the compositor filters which protocol globals are advertised to the sandbox, hiding privileged extensions such as screencopy, foreign-toplevel, and virtual keyboard protocols. The sandbox refuses to start if the compositor does not support the security context protocol.

GUI sandboxes also enable GPU rendering support and mount a private tmpfs at `/dev/shm` for GPU drivers and multi-process applications such as Chromium and Firefox. The host's `/dev/shm` is not exposed.

In addition to `/dev/dri`, the sandbox binary automatically detects and binds the following paths when they exist (all as `--ro-bind`, not `--dev-bind`):

- **`/run/opengl-driver`** - NixOS-specific Mesa driver libraries. Without this, GPU apps fail to find `libGL`, `libEGL`, and driver backends.
- **`/sys/dev/char`** - character device node resolution. Allows `libdrm` to map `/dev/dri/cardN` major:minor numbers to their sysfs device nodes.
- **GPU PCI sysfs paths** - auto-detected from `/sys/class/drm/card*` symlinks. These provide vendor/device IDs that Mesa and Vulkan drivers query to identify the GPU hardware.

These binds are detected at runtime by the compiled sandbox binary, so they work across different hardware configurations without per-sandbox configuration.

### Theme

```nix
cloister.sandboxes.dev.gui.theme = {
  gtk = "Adwaita";
  icon = "Adwaita";
  qt = {
    platform = "gtk3";
    style = null;
  };
};
```

When GUI is enabled, cloister sets GTK and Qt theme environment inside the sandbox and writes GTK settings files. `gui.theme.gtk` sets `GTK_THEME` and `gtk-theme-name`, `gui.theme.icon` sets `gtk-icon-theme-name`, `gui.theme.qt.platform` sets `QT_QPA_PLATFORMTHEME`, and `gui.theme.qt.style` sets `QT_STYLE_OVERRIDE` when non-null.

For alternative themes, add the theme or icon package to `gui.packages` and set the matching theme names:

```nix
cloister.sandboxes.myapp.gui = {
  theme = {
    gtk = "Adwaita:dark";
    icon = "Adwaita";
  };
  packages = with pkgs; [ adwaita-icon-theme adw-gtk3 ];
};
```

`GTK_THEME`, `QT_QPA_PLATFORMTHEME`, `QT_STYLE_OVERRIDE`, and `QT_PLUGIN_PATH` cannot be set directly via `sandbox.env` when GUI is enabled - use `gui.theme.*` and `gui.packages` instead.

### GUI packages

```nix
cloister.sandboxes.dev.gui.packages = with pkgs; [
  hicolor-icon-theme
  adwaita-icon-theme
  gtk3
  gtk4
  gsettings-desktop-schemas
];
```

When GUI is enabled, `XDG_DATA_DIRS` is computed from `gui.packages`; each package's `/share` directory is included. Qt plugin directories from the same packages are added to `QT_PLUGIN_PATH`.

The default GUI package set is:

- **`hicolor-icon-theme`** - freedesktop fallback icon theme required by GTK and Qt
- **`adwaita-icon-theme`** - default Adwaita icon theme
- **`gtk3`** / **`gtk4`** - built-in Adwaita theme assets
- **`gsettings-desktop-schemas`** - GSettings schemas for desktop settings

To add MIME data, icon themes, GTK themes, or Qt plugins:

```nix
cloister.sandboxes.evince.gui.packages = with pkgs; [
  hicolor-icon-theme
  adwaita-icon-theme
  gtk3
  gtk4
  gsettings-desktop-schemas
  shared-mime-info
  qt6ct
];
```

`XDG_DATA_DIRS` cannot be set directly via `sandbox.env` when GUI is enabled - use `gui.packages` instead.

### Fonts

```nix
cloister.sandboxes.dev.gui.fonts = with pkgs; [ noto-fonts noto-fonts-color-emoji ];
```

When GUI is enabled, a self-contained fontconfig configuration is generated via `pkgs.makeFontsConf` and injected into the sandbox as `FONTCONFIG_FILE`. This replaces the previous host `/etc/fonts` bind mount, making font rendering a declared sandbox property instead of a host-dependent side-effect.

The default provides **`noto-fonts`** plus **`noto-fonts-color-emoji`**. To add additional fonts:

```nix
cloister.sandboxes.myapp.gui.fonts = with pkgs; [
  noto-fonts
  noto-fonts-color-emoji
  noto-fonts-cjk-sans
];
```

Set to an empty list to disable the generated fontconfig entirely (e.g., if the application bundles its own fonts):

```nix
cloister.sandboxes.myapp.gui.fonts = lib.mkForce [ ];
```

`FONTCONFIG_FILE` cannot be set directly via `sandbox.env` when GUI is enabled - use `gui.fonts` instead.

### Desktop entries

```nix
cloister.sandboxes.chromium.gui.desktopEntry = {
  name = "Chromium (Sandboxed)";
  execArgs = "%U";
  icon = "chromium";
  categories = [ "Network" "WebBrowser" ];
  mimeTypes = [ "text/html" "x-scheme-handler/http" "x-scheme-handler/https" ];
};
```

Generates an XDG `.desktop` file so the sandbox appears in your app launcher. Set `gui.desktopEntry = null` to disable launcher generation. A desktop entry requires `gui.enable = true` and `defaultCommand` to be set. The `Exec` line launches `cl-<name>` with any `execArgs` appended, so `defaultCommand` is what makes that wrapper behave like an application launcher instead of opening an interactive shell. When `name` is empty, it falls back to `cl-<name>`.

### Device integrations

Pass explicit device nodes with `sandbox.devices` when a sandbox needs direct device access:

```nix
cloister.sandboxes.dev.sandbox.devices = [ "/dev/input/js0" ];
```

Device paths are mounted with `--dev-bind`. Missing devices are warned about at runtime rather than failing startup.

### Examples

See the [examples/](../examples/) directory for complete, importable sandbox configurations.

### PipeWire audio modes

Cloister supports two PipeWire-backed audio modes. Both use a per-sandbox filtered PipeWire backend socket; direct host PulseAudio passthrough, unfiltered PipeWire forwarding, in-sandbox `pipewire-pulse` compatibility, and ALSA compatibility are not supported.

| Option | Protocol exposed inside sandbox | Audio | Screen sharing / cameras | Filtering |
|--------|---------------------------------|-------|--------------------------|-----------|
| `audio.pipewire.enable` | PipeWire native | Yes | Yes | Always (`filters.*`) |
| `audio.pipewire.pulseOnly` | PulseAudio-compatible proxy | Yes | No | Always (`filters.audioOut`, `audioIn`, `control`) |

Use `audio.pipewire.pulseOnly = true` for audio-only sandboxes that need playback, optional microphone access, and volume control without exposing native PipeWire inside the sandbox. Use filtered native `audio.pipewire` when the sandbox also needs cameras, screen sharing, or native PipeWire clients.

### Filtered native PipeWire

```nix
cloister.sandboxes.dev.audio.pipewire = {
  enable = true;
  filters = {
    audioOut = true;  # speakers (default)
    # audioIn = true; # microphones
    # videoIn = true; # cameras exposed through filtered PipeWire
  };
};
```

Creates a dedicated restricted PipeWire socket, exposing only the device classes and capabilities you specify while still allowing clients to create the playback/capture streams needed for enabled sinks, microphones, and cameras. See the [audio guide](audio.md) for the full option reference.

### Pulse-only proxy

```nix
cloister.sandboxes.dev.audio.pipewire = {
  enable = true;
  pulseOnly = true;
  filters = {
    audioOut = true;
    audioIn = true;
    control = false;
  };
};
```

This keeps the backend on filtered PipeWire, but exposes only a PulseAudio-compatible proxy socket inside the sandbox. `audioOut`, `audioIn`, and `control` still apply. Native PipeWire tools and camera/screencast flows are intentionally unavailable in this mode.

### D-Bus notifications

```nix
cloister.sandboxes.dev.dbus = {
  enable = true;
  notifications = true;
};
```

Allows sandboxed tools to send desktop notifications through a filtered, per-launch D-Bus proxy. See [dbus.md](dbus.md) for policy examples and setup details.

### Worker broker

`workerBroker` config renders the host-side policy the parent launcher needs to register worker sessions and advertise spawnable profiles.

```nix
cloister.sandboxes = {
  dev.workerBroker.profiles.project = {
    sandbox = "worker";
    workspace.mode = "project-rw";
    delegatedPerDirMounts.worktrees = {
      mode = "rw";
      path = "/local/worktrees/dev";
    };
  };

  worker = {
    preset = "hardened";
    shell.hostConfig = false;
  };
};
```

This also installs generated launchers in the parent sandbox package named `clb-<profile>`. Run them inside the parent sandbox as `clb-<profile> <command> [args...]`.

Trust model:

- parent sandboxes receive only opaque capability info
- child launches resolve policy from trusted mounted session records / host-side session store
- session record cleanup is attempted on normal parent exit
- env-carried full session JSON is not trusted

The rendered sandbox JSON still includes the worker broker profile and delegated mount metadata because the parent launcher needs that host-authored policy when registering launches. That does not make child-side environment variables authoritative: child launches re-resolve the selected profile from the trusted mounted session record instead of trusting env-carried policy blobs.

For an end-to-end setup and manual testing walkthrough, see [worker-broker.md](worker-broker.md).

Session-record cleanup is currently best-effort on normal parent exit. Records left behind by crashes or hard kills are not yet pruned automatically.

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
cloister.sandboxes.evince = {
  extraPackages = with pkgs; [ evince ];
  defaultCommand = [ "evince" ];
  gui.enable = true;
};
```

Now `cl-evince` launches evince directly instead of opening a shell. Additional arguments are appended to the default command, so `cl-evince document.pdf` runs `evince document.pdf`. To run a different command explicitly, use `-c`: `cl-evince -c some-other-command`. To run a command through the interactive shell startup path first, use `-i`: `cl-evince -i some-other-command`. When `direnv` is present in the sandbox, that path also uses `direnv exec "$PWD" ...` before the final `exec`, which makes one-shot launches pick up the active repo environment. Bare `-c` and bare `-i` are invalid and exit with a usage error. To pass values like `--version`, `--build-info`, `-c`, or `-i` through to the sandboxed default command instead of the launcher, insert `--` first: `cl-evince -- --version`. Generated wrapped command aliases also pass their leading arguments through to the app now, so sandbox control flags like `--shell` should be used on `cl-<name>` itself. This is especially useful with `gui.desktopEntry`, because the desktop entry launches `cl-<name>` and relies on `defaultCommand` to start the application.

## Identity anonymization

See `docs/anonymize.md` for anonymized identity behavior, proc privacy details, audio constraints, and limitations.

# Options reference

See the sections above for usage examples and explanations.

## Global options

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `cloister.enable` | bool | `false` | Gate the entire module |
| `cloister.defaultShell` | enum | `"zsh"` | Default interactive shell for sandboxes (`"zsh"` or `"bash"`) |
| `cloister.homeManager.builder` | nullOr function | `null` | Home Manager evaluation function for sandbox-local `homeManager.modules` profiles |

## NixOS host options (`cloister.imageStore.*`)

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `cloister.imageStore.base` | string | `"/var/lib/cloister/images"` | NixOS global publish directory for immutable store images |
| `cloister.imageStore.mountBase` | string | `"/run/cloister/images"` | NixOS global mount directory for immutable store images |
| `cloister.imageStore.compression.enable` | bool | `true` | Compress generated squashfs images with zstd level 10 and 1 MiB blocks |
| `cloister.imageStore.enable` | bool | `false` | Enable periodic cleanup of published image-store links |
| `cloister.imageStore.interval` | string | `"weekly"` | systemd timer schedule for image-store cleanup |

## Per-sandbox options (`cloister.sandboxes.<name>.*`)

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `defaultCommand` | nullOr (listOf str) | `null` | Default command prefix used when invoked without args, or when appending positional args |
| `extraPackages` | list of package | `[]` | Additional packages appended to the internally managed base PATH |
| `homeManager.enable` | bool | `false` | Evaluate a sandbox-local Home Manager profile and consume only files/packages |
| `homeManager.modules` | list of module | `[]` | Home Manager modules evaluated for this sandbox when `cloister.homeManager.builder` is set |
| `homeManager.extraSpecialArgs` | attrs | `{}` | Extra `specialArgs` passed to the nested Home Manager evaluation |
| `homeManager.config` | nullOr attrs | `null` | Pre-evaluated Home Manager config to consume instead of `homeManager.modules` |
| `homeManager.includeFiles` | bool | `true` | Bind `xdg.configFile` and `home.file` entries from the sandbox profile |
| `homeManager.includePackages` | bool | `true` | Add `home.packages` from the sandbox profile to the sandbox PATH |
| `preset` | nullOr enum | `null` | Apply one of `hardened`, `developer`, `gui`, or `chromium` default profiles |
| `shell.name` | enum | `cloister.defaultShell` | Interactive shell (`"zsh"` or `"bash"`) |
| `shell.hostConfig` | bool | `false` | Bind host shell config files into the sandbox |
| `shell.customRcPath.zshenv` | nullOr path | `null` | Custom zshenv file to source inside the sandbox |
| `shell.customRcPath.zshrc` | nullOr path | `null` | Custom zshrc file to source inside the sandbox |
| `shell.customRcPath.bashenv` | nullOr path | `null` | Custom bashenv file to source inside the sandbox |
| `shell.customRcPath.bashrc` | nullOr path | `null` | Custom bashrc file to source inside the sandbox |
| `shell.customRcPath.profile` | nullOr path | `null` | Custom profile file to source inside the sandbox |
| `workerBroker.profiles` | attrsOf submodule | `{}` | Worker broker profiles keyed by profile name; non-empty profiles enable worker broker support |
| `workerBroker.profiles.<name>.sandbox` | str | *(required)* | Child sandbox name this broker profile may launch |
| `workerBroker.profiles.<name>.workspace.mode` | enum | *(required)* | Workspace exposure mode: `project-rw` or `project-overlay` |
| `workerBroker.profiles.<name>.delegatedPerDirMounts` | attrsOf submodule | `{}` | Delegated mounts keyed by sandbox-relative path, with `mode`, `path`, and optional `subPath` |
| `network.enable` | bool | `true` | Share host network namespace |
| `network.namespace` | nullOr str | `null` | Linux network namespace to join (localhost-netns host services are reachable as `host.internal:<port>`) |
| `sandbox.bindWorkingDirectory` | bool | `true` | Bind-mount the working directory (git root or CWD) into the sandbox. Disable for app-specific sandboxes |
| `sandbox.nixStore.mode` | enum | `"host"` | How Cloister exposes `/nix/store`: bind the host store or a mounted immutable image store |
| `sandbox.env` | attrsOf str | *(base vars)* | Environment variables inside sandbox |
| `sandbox.passthroughEnv` | list of str | *(locale vars)* | Host env vars to pass through when set |
| `sandbox.readOnly` | list of str or {src, dest, optional} | `[]` | Read-only host path binds; string entries are home-relative |
| `sandbox.readWrite` | list of str or {src, dest, optional} | `[]` | Read-write host path binds; string entries are home-relative |
| `sandbox.managed` | list of str or {src, dest} | `[]` | Home Manager managed keys/prefixes or explicit read-only source binds |
| `sandbox.state.dirs` | attrsOf (list of str) | `{}` | Volume-backed writable directories keyed by host base directory |
| `sandbox.state.files` | attrsOf (list of str) | `{}` | Volume-backed writable files keyed by host base directory |
| `sandbox.state.projectDirs` | attrsOf (list of str) | `{}` | Per-project writable directories keyed by host base directory |
| `sandbox.enforceStrictHomePolicy` | bool | `true` | Prevent sandboxing home dirs and dot-dirs |
| `sandbox.disallowedPaths` | list of str | `["/", "/root"]` | Paths disallowed as sandbox directory |
| `sandbox.copyBase` | str | `${config.xdg.stateHome}/cloister` | Host base directory where `sandbox.copies` writable state is stored |
| `sandbox.copies` | list of {src, dest, mode, overwrite} | `[]` | Files to copy writable into sandbox state; `src` is a literal absolute host path and `dest` resolves under `$HOME` |
| `sandbox.symlinks` | list of {target, link} | `[]` | Symlinks to create inside the sandbox; relative `link` paths resolve under `$HOME` |
| `sandbox.anonymize.enable` | bool | `false` | Present generic identity (username/hostname `ubuntu`, synthetic `/proc` files, blocked `/proc/sys`) |
| `sandbox.anonymize.username` | str | `"ubuntu"` | Username and home directory name used by anonymized sandboxes |
| `gui.enable` | bool | `false` | Enable Wayland GUI integration with security-context forwarding, GPU rendering support, and private `/dev/shm` |
| `gui.fonts` | list of package | `[]`\* | Font packages for fontconfig (*`noto-fonts` and `noto-fonts-color-emoji` added when GUI enabled) |
| `gui.packages` | list of package | `[]`* | GUI asset/plugin packages for `XDG_DATA_DIRS` and `QT_PLUGIN_PATH` (\*Adwaita defaults added when GUI enabled) |
| `gui.theme.gtk` | str | `"Adwaita"` | GTK theme name (sets `GTK_THEME` env var and GTK settings) |
| `gui.theme.icon` | str | `"Adwaita"` | Icon theme name for GTK settings |
| `gui.theme.qt.platform` | str | `"gtk3"` | Qt platform theme plugin (sets `QT_QPA_PLATFORMTHEME`) |
| `gui.theme.qt.style` | nullOr str | `null` | Qt style override (sets `QT_STYLE_OVERRIDE` when non-null) |
| `gui.desktopEntry` | nullOr submodule | `null` | Generate XDG .desktop file for app launchers when set |
| `gui.desktopEntry.name` | str | `""` | Display name (falls back to `cl-<name>`) |
| `gui.desktopEntry.execArgs` | str | `""` | Extra arguments appended after the sandbox binary path (e.g. `%U`) |
| `gui.desktopEntry.icon` | str | `""` | Icon name or path |
| `gui.desktopEntry.categories` | list of str | `[]` | XDG categories |
| `gui.desktopEntry.mimeTypes` | list of str | `[]` | MIME types handled |
| `gui.desktopEntry.terminal` | bool | `false` | Run in terminal |
| `gui.desktopEntry.genericName` | str | `""` | Generic name (e.g. "Web Browser") |
| `gui.desktopEntry.comment` | str | `""` | Tooltip/comment |
| `gui.desktopEntry.startupNotify` | bool | `false` | Startup notification support |
| `sandbox.seccomp.enable` | bool | `true` | Apply seccomp-bpf filter blocking dangerous syscalls |
| `sandbox.seccomp.allowChromiumSandbox` | bool | `false` | Allow Chromium/Electron internal sandbox syscalls (chroot, namespaces) |
| `ssh.enable` | bool | `false` | Forward SSH agent socket |
| `ssh.allowFingerprints` | list of str | `[]` | Restrict visible SSH keys to these fingerprints |
| `ssh.filterTimeoutSeconds` | unsigned int | `60` | SSH filter read/write timeout; set `0` to disable |
| `git.enable` | bool | `false` | Bind `.gitconfig` and `.config/git/config` read-only |
| `dbus.enable` | bool | `false` | Per-launch D-Bus proxy |
| `dbus.log` | bool | `false` | Enable xdg-dbus-proxy logging |
| `dbus.portal.fileChooser` | bool | `false` | Enable portal file chooser via D-Bus; adds `.flatpak-info`, `GTK_USE_PORTAL`, and optional binds for `/run/flatpak/doc` and `$XDG_RUNTIME_DIR/doc` from the per-app document portal subtree when it already exists; this improves file chooser integration but is not a host-path privacy boundary |
| `dbus.portal.openUri` | bool | `false` | Allow the OpenURI portal |
| `dbus.portal.screencast` | bool | `false` | Allow the ScreenCast portal |
| `dbus.portal.camera` | bool | `false` | Allow the Camera portal |
| `dbus.notifications` | bool | `false` | Allow native desktop notifications via `org.freedesktop.Notifications` |
| `dbus.rawPolicies.talk` | list of str | `[]` | Raw D-Bus TALK allowlist |
| `dbus.rawPolicies.own` | list of str | `[]` | Raw D-Bus OWN allowlist |
| `dbus.rawPolicies.see` | list of str | `[]` | Raw D-Bus SEE allowlist |
| `dbus.rawPolicies.call` | attrsOf (list of str) | `{}` | Raw per-name call rules |
| `dbus.rawPolicies.broadcast` | attrsOf (list of str) | `{}` | Raw per-name broadcast rules |
| `audio.pipewire.enable` | bool | `false` | Enable filtered PipeWire-backed audio |
| `audio.pipewire.pulseOnly` | bool | `false` | Expose only a filtered PulseAudio-compatible proxy socket while keeping PipeWire hidden from the sandbox |
| `audio.pipewire.filters.audioOut` | bool | `true` | Allow playback to speakers/sinks when PipeWire audio is enabled |
| `audio.pipewire.filters.audioIn` | bool | `false` | Allow microphone access when PipeWire audio is enabled |
| `audio.pipewire.filters.videoIn` | bool | `false` | Allow camera nodes when PipeWire audio is enabled |
| `audio.pipewire.filters.control` | bool | `false` | Allow volume and mute changes on visible nodes |
| `audio.pipewire.filters.routing` | bool | `false` | Allow default-device and stream-routing changes |
| `audio.pipewire.dbus.enable` | bool | `true` | Let sandboxed PipeWire clients use D-Bus support when a filtered bus is available |
| `sandbox.devices` | list of str | `[]` | Additional device paths for `--dev-bind` passthrough |
| `registry.aliases` | attrsOf str | `{}` | Shell aliases |
| `registry.functions` | attrsOf lines | `{}` | Shell functions |
| `registry.commands` | list of str | `[]` | Commands to wrap outside sandbox |
| `registry.interactiveCommands` | list of str | `[]` | Commands to wrap outside sandbox via `cl-<name> -i ...` |
| `registry.noWrap` | list of str | `[]` | Names to exclude from wrapping |
| `init.text` | lines | `""` | Shell snippet sourced inside the sandbox |
