# Testing

`make check` is the complete local and CI acceptance command. It attempts full `nix flake check --keep-going`, Clippy, Rust workspace tests, and dependency auditing, returning failure if any category fails. It checks formatting without applying fixes; use `make fmt` separately. VM checks require usable `/dev/kvm`.

`flake.nix` exposes every published package as `package-<name>` alongside tests and treefmt. Test subsets are discovered from `test-*` check names (runtime checks use `test-runtime-*`). Rust workspaces are discovered once per top-level `helpers/*/Cargo.toml`; Clippy and tests use `--workspace --locked`. Missing committed lockfiles fail validation. New checks, packages, and helpers require no workflow edits.

For iteration, use the smallest matching command:

- `make test` runs both eval and runtime checks in one `nix build` so Nix can schedule them together.
- `make test-eval` is the fast eval-only subset.
- `make test-runtime` is the VM-backed subset for behaviors eval tests cannot prove.

## Concrete Checklist

- [x] Create shared eval helpers for Home Manager-style and NixOS-style module tests.
- [x] Add section-scoped flake checks so changed code can run a targeted subset.
- [x] Add an examples eval check so published example configs stay importable.
- [x] Add first-pass coverage for `registry`, `presets`, `sandbox-core`, `gui-dbus-audio`, `rendered-config`, `image-store`, and `netns`.
- [x] Expand negative tests from fail-or-pass checks into message-specific assertion coverage where useful.
- [x] Add more fixture coverage for `managedFile`, shell custom rc, image-store edge cases, and additional netns address-allocation cases.
- [x] Use one GitHub Actions job running the shared `make check` command.
- [x] Add a separate runtime/VM test tier for high-value integration behavior.

## Test Commands

- `make test`
- `make test-eval`
- `make test-runtime`
- `make rust-test`
- `make test-changed`
- `make test-changed BASE_REF=origin/main`
- `make test-changed TEST_CHANGED_FILES="modules/cloister/_registry.nix modules/cloister-netns/default.nix"`
- `make test-changed TEST_CHANGED_FILES="helpers/cloister-netns/src/main.rs"`
- `make test-eval TEST_CHECKS="test-cloister-registry test-cloister-netns"`
- `nix build .#checks.x86_64-linux.test-cloister-registry`
- `nix build .#checks.x86_64-linux.test-cloister-bwrap`
- `nix build .#checks.x86_64-linux.test-cloister-examples`
- `nix build .#checks.x86_64-linux.test-cloister-presets`
- `nix build .#checks.x86_64-linux.test-cloister-sandbox-core`
- `nix build .#checks.x86_64-linux.test-cloister-gui-dbus-audio`
- `nix build .#checks.x86_64-linux.test-cloister-rendered-config`
- `nix build .#checks.x86_64-linux.test-cloister-image-store`
- `nix build .#checks.x86_64-linux.test-cloister-netns`
- `nix build .#checks.x86_64-linux.test-runtime-gui-dbus-audio`
- `nix build .#checks.x86_64-linux.test-runtime-image-store`
- `nix build .#checks.x86_64-linux.test-runtime-netns`
- `nix build .#checks.x86_64-linux.tests-runtime`

## Change Mapping

Shared validation infrastructure (`Makefile`, `scripts/*`, CI, and flake files) selects all test tiers and helpers. Sandbox runner and lockfile changes select its Rust tests, rendered-config eval, and sandbox-core VM check. Other helper lockfiles select their workspace tests; run `make cargo-audit` for dependency changes.

- `modules/cloister/_bwrap.nix`, `pkgs/bubblewrap-subset-pid/*` -> `test-cloister-bwrap`
- `examples/*` -> `test-cloister-examples`
- `modules/cloister/_registry.nix`, `modules/cloister/_wrappers.nix` -> `test-cloister-registry`
- `modules/cloister/_options.nix`, `modules/cloister/default.nix`, `modules/cloister/_mkShells.nix`, `modules/cloister/_shells/*`, `modules/cloister/_resolve.nix`, `modules/cloister/_patterns.nix` -> `test-cloister-registry`, `test-cloister-presets`, `test-cloister-sandbox-core`, `test-cloister-gui-dbus-audio`, `test-runtime-gui-dbus-audio`, `test-cloister-rendered-config`, `test-cloister-image-store`, `test-runtime-image-store`
- `modules/cloister/_sandbox.nix`, `modules/cloister/_assertions.nix` -> `test-cloister-sandbox-core`, `test-cloister-gui-dbus-audio`, `test-runtime-gui-dbus-audio`, `test-cloister-rendered-config`, `test-cloister-image-store`, `test-runtime-image-store`
- GUI, portal, and D-Bus-related changes -> `test-cloister-gui-dbus-audio`, `test-runtime-gui-dbus-audio`
- `modules/cloister/package-config.nix` -> `test-cloister-sandbox-core`, `test-cloister-rendered-config`, `test-cloister-gui-dbus-audio`, `test-runtime-gui-dbus-audio`, `test-cloister-image-store`, `test-runtime-image-store`
- `modules/cloister-image-store/default.nix` -> `test-cloister-image-store`
- `modules/cloister-netns/default.nix` -> `test-cloister-netns`
- `tests/cloister/gui-dbus-audio.nix`, `tests/runtime/gui-dbus-audio.nix` -> `test-cloister-gui-dbus-audio`, `test-runtime-gui-dbus-audio`
- `tests/runtime/image-store.nix` -> `test-runtime-image-store`
- `tests/runtime/netns.nix` -> `test-runtime-netns`
- `helpers/cloister-*` -> `make rust-test` for the changed helper crates

## Runtime Test Assertions

- Use Python assertions in `testScript` for content checks instead of shell pipelines like `machine.succeed("cmd | grep ...")`.
- Prefer the shared helpers in `tests/runtime/lib.py`: `assert_contains(...)`, `assert_not_contains(...)`, `assert_eq(...)`, and `assert_json_eq(...)`.
- Keep `machine.fail(...)` for cases where non-zero exit status is the behavior under test.

Instead of:

```python
machine.succeed("cloister-dbus-validate --list --quiet | grep -F org.example.Service")
```

Prefer:

```python
assert_contains(
    machine,
    "cloister-dbus-validate --list --quiet",
    "org.example.Service",
    "proxy bus exposes allowed service",
)
```

## Concurrency

- `make test` calls a single `nix build` with all eval and runtime checks, so Nix can schedule the full test suite concurrently instead of stepping through `nix flake check` serially.
- `make test-eval` keeps the eval-only subset available for fast module iteration.
- `make test-runtime` keeps the VM-backed runtime checks available as a focused subset.
- `make rust-test` runs `cargo test --workspace --locked` for each Rust helper inside `nix develop`, so crates with system-library dependencies still work locally.
- `make test-changed` maps changed files to eval checks, runtime checks, and changed Rust helpers, then runs only the needed subsets.
- `make test-changed` uses `BASE_REF` for the committed diff and also includes staged, unstaged, and untracked files.
- Override `TEST_CHANGED_FILES` when you want to drive the mapping manually without relying on git diff state.
- Override `TEST_CHECKS` to run just an eval subset while keeping the same concurrent `nix build` flow.
- Override `RUNTIME_TEST_CHECKS` to run just a runtime subset.
- GitHub Actions runs `make check`; targeted commands remain available locally.
- Override `RUST_HELPER_WORKSPACES`, `RUST_TEST_HELPERS`, or `RUST_AUDIT_WORKSPACES` to select Rust subsets. Audit exceptions are documented in [Security Model](security.md#dependency-audit-exceptions).
