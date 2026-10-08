# D-Bus Proxy for Sandboxes

## Purpose

Sandboxed tools often want a small amount of desktop integration without exposing the full session bus. Cloister runs `xdg-dbus-proxy` per launch and only forwards the names and methods you explicitly allow.

## The proxy pattern

```
Host D-Bus session bus -> xdg-dbus-proxy -> Filtered socket -> Sandbox
```

Each sandbox launch gets its own proxy socket under `%t/cloister/dbus/<name>-<instance-id>`. Cloister starts the matching proxy wrapper before launching the app and prewarms that socket so the first client is less likely to block on proxy startup.

## Basic setup

```nix
cloister.sandboxes.gui = {
  dbus.enable = true;
  dbus.notifications = true;
};
```

When `dbus.enable = true`, Cloister binds the proxy socket to `$XDG_RUNTIME_DIR/bus` in the sandbox and sets `DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus`.

## Portal toggles

Use the built-in desktop integration toggles instead of broad wildcard portal rules:

```nix
cloister.sandboxes.chromium.dbus = {
  enable = true;
  notifications = true;
  portal = {
    fileChooser = true;
    openUri = true;
  };
};
```

Portal toggles are an add-on to the filtered D-Bus proxy and require `dbus.enable = true`.

Available toggles:

- `fileChooser` - enables the FileChooser portal, creates `/.flatpak-info`, attempts to bind the per-app document portal view to `/run/flatpak/doc` and `$XDG_RUNTIME_DIR/doc` when that subtree already exists, and sets `GTK_USE_PORTAL=1`
- `openUri` - allows the OpenURI portal
- `screencast` - allows the ScreenCast portal
- `camera` - allows the Camera portal

Use `dbus.notifications = true` for native session-bus notifications through `org.freedesktop.Notifications`.

## Chromium file chooser

Chromium-family browsers need the file chooser portal plus the document portal FUSE mount. `dbus.portal.fileChooser = true` sets up the pieces Chromium needs:

- `/.flatpak-info` so `xdg-desktop-portal` recognizes the sandbox
- `/run/flatpak/doc` and `$XDG_RUNTIME_DIR/doc` optionally bound from the app-specific document portal subtree under `$XDG_RUNTIME_DIR/doc/by-app/...` when that subtree already exists at launch time
- `GTK_USE_PORTAL=1`
- D-Bus allow rules for `org.freedesktop.portal.FileChooser.*` and request-response objects

If the per-app document portal subtree already exists, the selected files appear inside the sandbox through the document portal FUSE path.

If that subtree does not exist yet, Cloister still starts the sandbox and leaves those binds absent for that launch instead of failing during startup.

## Documents / host paths

Cloister does not intentionally expose the `org.freedesktop.portal.Documents` service on D-Bus as part of the built-in portal toggles.

That means `Documents.GetHostPaths` is not expected to be available through the generated proxy policy. File chooser access relies on `org.freedesktop.portal.Desktop` plus the document portal FUSE mount when the per-app document subtree is available at launch time.

This is not a guarantee that the original host path stays hidden from the sandboxed app:

- Upstream `org.freedesktop.portal.FileChooser` only says selected files will be made accessible and that this may involve the Documents portal. It does not guarantee that returned `file://` URIs will always point at `/run/flatpak/doc`.
- Upstream document-portal can also expose the original host path through the read-only `user.document-portal.host-path` extended attribute on files served from the FUSE mount.

In practice, `dbus.portal.fileChooser = true` should be treated as desktop integration for opening files, not as a path-privacy boundary. Blocking the Documents service avoids the direct `Documents.GetHostPaths` call path, but it does not fully prevent host path disclosure by the upstream portal stack.

## Policy configuration

Use high-level `dbus.portal.*` toggles for common desktop integration. When an application needs raw `xdg-dbus-proxy` rules beyond the built-in portal grants, add them through `dbus.rawPolicies.*`. Policy names must be well-formed dot-separated bus names such as `org.example.App`, with an optional trailing `.*` wildcard suffix.

Common base policy:

```nix
dbus = {
  enable = true;
  notifications = true;
};
```

Keep blocked unless you explicitly need them:

- `org.freedesktop.systemd1`
- `org.freedesktop.login1`
- `org.freedesktop.secrets`
- `org.gnome.keyring`
- `org.freedesktop.NetworkManager`

## Debugging

Set `dbus.log = true` to pass `--log` to `xdg-dbus-proxy` and inspect filter decisions from the proxy wrapper's stderr:

```nix
cloister.sandboxes.myapp.dbus.log = true;
```

Then inspect the wrapper stderr from the launcher invocation or any surrounding service that started it.

Privacy note: this logging can retain sandbox activity metadata in whatever captures the wrapper stderr, including accessed bus names and policy decisions. Leave `dbus.log = false` for normal use, and enable it only while actively debugging.
