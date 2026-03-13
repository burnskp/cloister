# D-Bus Proxy for Sandboxes

## Purpose

Sandboxed tools (build systems, GUI apps, AI assistants, editors) often want to send desktop notifications - build complete, task finished, error occurred. Since the sandbox isolates all namespaces except network, the session D-Bus bus is not available inside the sandbox by default.

Rather than exposing the full D-Bus session bus (which would allow sandboxed tools to interact with any D-Bus service), a filtered proxy exposes only the services you explicitly allow per sandbox.

## The proxy pattern

`xdg-dbus-proxy` creates a filtered Unix socket that forwards only allowed D-Bus traffic:

```
Host D-Bus session bus ──► xdg-dbus-proxy ──► Filtered socket ──► Sandbox
                            (per-sandbox policy)
```

Each sandbox gets its own socket-activated proxy. The cloister module creates a per-sandbox systemd user socket and service, binds that socket into the sandbox, and sets `DBUS_SESSION_BUS_ADDRESS` to point to it.

## Socket path contract

Both the systemd socket and sandbox must agree on the socket path:

```
$XDG_RUNTIME_DIR/dbus-proxy-<name>
```

This is `%t/dbus-proxy-<name>` in systemd unit notation.

## Setup

Enable D-Bus per sandbox and configure policies:

```nix
cloister.sandboxes.gui = {
  dbus.enable = true;
  dbus.policies = {
    talk = [ "org.freedesktop.Notifications" ];
  };
};
```

Cloister generates a per-sandbox socket-activated systemd user unit:

- Socket: `cloister-dbus-proxy-<name>.socket` (listens on `%t/dbus-proxy-<name>`)
- Service: `cloister-dbus-proxy-<name>.service` (runs `xdg-dbus-proxy` with your policy)

## Module integration

`cloister.sandboxes.<name>.dbus.enable` (default `false`) controls whether the proxy socket is bound into the sandbox:

- Adds an ro-bind-try for the proxy socket to `$XDG_RUNTIME_DIR/bus` inside the sandbox
- Sets `DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus`

## Runtime behavior

At sandbox startup, the cloister script checks whether the proxy socket exists:

- **Socket present**: D-Bus access works normally
- **Socket missing**: prints a warning to stderr, sandbox starts without D-Bus access - graceful degradation, no hard failure

This handles the case where the desktop session hasn't started yet, or the proxy service failed.

## Debugging

### Enabling proxy logging

Set `dbus.log = true` on a sandbox to pass `--log` to `xdg-dbus-proxy`. This prints every filtering decision (allowed/denied bus names, method calls, signals) to stderr, which systemd captures in the journal:

```nix
cloister.sandboxes.myapp.dbus.log = true;
```

After rebuilding, watch the log with:

```
journalctl --user -u cloister-dbus-proxy-myapp -f
```

### Common issue: file picker not working

Chromium-based browsers need portal access for file pickers (open/save dialogs). The `org.freedesktop.portal.Documents` service handles file descriptor transfer between the picker and app.

If you see errors like `Failed to call method: org.freedesktop.DBus.NameHasOwner: unknown error type`, the proxy is denying `NameHasOwner` queries for bus names the sandbox lacks visibility for. Enable `dbus.log = true` to identify which names need to be added.

Flatpak solves this with wildcard portal rules (no `--see` or `--talk` for portals, just `--call` and `--broadcast`):

```nix
dbus.policies = {
  talk = [ "org.freedesktop.Notifications" ];
  call."org.freedesktop.portal.*" = [ "*" ];
  broadcast."org.freedesktop.portal.*" = [ "*@/org/freedesktop/portal/*" ];
};
```

See `examples/chromium.nix` for a complete Chromium policy matching Flatpak's approach.

## Policy configuration

### Policy levels

The proxy supports three base policy levels, plus optional call/signal rules:

- `see`: make the name visible (ListNames, NameOwnerChanged, etc.)
- `talk`: allow method calls and signals to the name
- `own`: allow RequestName/ReleaseName for the name

Optional fine-grained rules:

- `call`: allow method calls on a name, constrained by rule
- `broadcast`: allow broadcast signals from a name, constrained by rule

`RULE` syntax is `[METHOD][@PATH]`, where `METHOD` can be `*`, an interface (optionally with `.*`), or a fully-qualified method, and `PATH` is an object path (optionally with `/*`).

### Common policy sets

Notifications only (default):

```nix
dbus.policies.talk = [ "org.freedesktop.Notifications" ];
```

Notifications + portals (file pickers, open/save dialogs - matches Flatpak's approach):

```nix
dbus.policies = {
  talk = [ "org.freedesktop.Notifications" ];
  call."org.freedesktop.portal.*" = [ "*" ];
  broadcast."org.freedesktop.portal.*" = [ "*@/org/freedesktop/portal/*" ];
};
```

### Keep blocked

Avoid exposing sensitive session services unless you are certain you need them:

- Keyrings/secrets: `org.freedesktop.secrets`, `org.gnome.keyring`
- System/session control: `org.freedesktop.systemd1`, `org.freedesktop.login1`
- Network control: `org.freedesktop.NetworkManager`
- Power/session manager services (varies by desktop environment)

## Portal integration

### What `dbus.portal.enable` enables

Setting `dbus.portal.enable = true` on a sandbox configures everything needed for `xdg-desktop-portal` to recognize the sandbox and provide portal services (file pickers, open/save dialogs, screenshot requests, etc.):

1. **Synthetic `.flatpak-info`** - A read-only file at `/.flatpak-info` inside the sandbox with `[Application]\nname=dev.cloister.<name>`. The portal daemon reads `/proc/<pid>/root/.flatpak-info` to detect sandboxed clients and activate portal behavior.

1. **Document portal FUSE mount** - When `dbus.portal.documentFUSE.enable = true` (default), binds `$XDG_RUNTIME_DIR/doc` (host) to `/run/flatpak/doc` (sandbox). This is where `xdg-document-portal` exposes its FUSE filesystem.

1. **Portal D-Bus policies** - Auto-merges `--call=org.freedesktop.portal.*=*` and `--broadcast=org.freedesktop.portal.*=*@/org/freedesktop/portal/*` into the D-Bus proxy. User-specified policies override these defaults.

1. **`GTK_USE_PORTAL=1`** - Tells GTK to prefer portal dialogs over native ones.

### How portal FUSE works

The `xdg-document-portal` daemon (part of `xdg-desktop-portal`) creates a FUSE filesystem at `$XDG_RUNTIME_DIR/doc/` on the host. When a user picks a file through a portal dialog:

1. The portal daemon receives the file selection on the host side
1. It registers the file with the document store and creates an opaque document ID
1. The app receives a path like `/run/flatpak/doc/DOCID/filename`
1. File access through this path is proxied by the FUSE daemon using `O_PATH` file descriptors, so the sandboxed app can read/write the file without direct host filesystem access

Inside Flatpak sandboxes (and cloister sandboxes with `dbus.portal.enable = true` and document FUSE enabled), the host path `$XDG_RUNTIME_DIR/doc/` is remapped to `/run/flatpak/doc/`.

### Requirements

- `xdg-desktop-portal` and `xdg-desktop-portal-gtk` (or your DE's portal backend) must be running on the host. This is standard on desktop NixOS - most desktop environments include it.
- `dbus.enable = true` is required (the portal option sets an assertion for this).

### Limitations

- Portal FUSE paths use opaque document IDs (`/run/flatpak/doc/abc123/file.txt`), not human-readable host paths. Applications see these IDs, not the original file location.
- The document portal FUSE mount only appears if `xdg-document-portal` is running on the host. If it's not running, the bind uses `--bind-try` and degrades gracefully.
- If `dbus.portal.documentFUSE.enable = false`, portal D-Bus integration still works, but document portal paths are not mounted into the sandbox.

### Example

```nix
cloister.sandboxes.chromium.dbus = {
  enable = true;
  portal = {
    enable = true;
    documentFUSE.enable = true;
  };
};
```

## Headless systems

D-Bus proxy support is disabled by default, so no configuration is needed on headless systems (servers, CI). The socket bind and `DBUS_SESSION_BUS_ADDRESS` are only added when `cloister.sandboxes.<name>.dbus.enable = true`.
