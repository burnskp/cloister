# Diagnostics

## Validating the Wayland security context

When `gui.enable = true`, the sandbox uses the `wp-security-context-v1` protocol to tell the compositor that the client is sandboxed. The compositor then filters which protocol globals are advertised - privileged extensions (screencopy, virtual keyboard injection, etc.) should be hidden.

The `cloister-wayland-validate` helper connects to the Wayland display, enumerates all advertised globals, and checks them against a list of 19 known privileged protocols.

Validator helpers are provided by the separate `cloister-diagnostics` package and are not installed in sandboxes by default. Add that package explicitly where you want to run the tools. For example, install it on the host:

```nix
environment.systemPackages = [ cloister.packages.${pkgs.system}.cloister-diagnostics ];
```

or add it to a sandbox that needs in-sandbox checks:

```nix
cloister.sandboxes.dev.extraPackages = [ cloister.packages.${pkgs.system}.cloister-diagnostics ];
```

```sh
cloister-wayland-validate
```

The tool reports PASS (exit 0) if all privileged protocols are blocked, or FAIL (exit 1) if any are exposed:

```
── Advertised globals (17 total) ───────────────
  wl_compositor                            v6    core
  wl_shm                                  v2    core
  ...

── Privileged protocols ────────────────────────
  ✓ zwlr_screencopy_manager_v1              blocked
  ✓ zwlr_layer_shell_v1                     blocked
  ...

── Core protocols ──────────────────────────────
  ✓ wl_compositor                           present (v6)
  ✓ wl_shm                                  present (v2)
  ...

RESULT: PASS - all privileged protocols blocked
```

Running outside a sandbox (on the raw compositor socket) shows which privileged protocols your compositor exposes by default - useful for understanding what the security context is actually filtering.

## Validating the D-Bus proxy

The `cloister-dbus-validate` helper connects to the sandbox D-Bus proxy, lists visible names, and verifies that denylisted high-risk services are not exposed.

```sh
cloister-dbus-validate
```

By default it validates against the built-in denylist and prints discovered names. Use `--quiet` to suppress the name list, `--show-all` to force it on, `--activatable` to include activatable bus names, and `--deny` to override the denylist:

```sh
cloister-dbus-validate \
  --deny org.freedesktop.secrets,org.freedesktop.login1 \
  --activatable \
  --show-all
```

Use `--list` to inspect visible names without validating, or `--json` for machine-readable output.

## Validating the seccomp filter

The `cloister-seccomp-validate` helper verifies that the seccomp BPF filter is working correctly by attempting blocked syscalls and confirming they return `ENOSYS`.

```sh
# Run inside a sandbox
cl-dev cloister-seccomp-validate

# Run outside the sandbox on the host (for comparison)
cloister-seccomp-validate
```

The tool tests each syscall that should be blocked by the filter (kernel module loading, mount/unmount, namespace creation, `io_uring`, `ptrace`, restricted socket families, etc.) and reports whether the filter correctly denied it:

- **PASS** - syscall was blocked with `ENOSYS` as expected
- **FAIL** - syscall was not blocked (filter may be misconfigured)
- **SKIPPED** - syscall is not available on this architecture

Use `--allow-chromium-sandbox` to test with the Chromium-compatible filter variant (which permits `clone`, `unshare`, and `chroot` for Chromium/Electron internal sandboxing):

```sh
cl-chromium cloister-seccomp-validate --allow-chromium-sandbox
```

Use `--json` for machine-readable output:

```sh
cl-dev cloister-seccomp-validate --json
```

## Validating audio filtering

The fourth validator helper, `cloister-pipewire-validate`, checks that only the expected PipeWire objects are visible inside the sandbox. See `docs/audio.md` for usage and interpretation details.
