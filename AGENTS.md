# Validation Notes

Use the smallest validation step that matches the files you changed instead of defaulting to the full suite.

## Fastest checks

- `make fmt`

  - Use for formatting, docs, and simple Nix hygiene fixes.
  - Applies `nixfmt`, `mdformat`, `toml-sort`, `rustfmt`, `statix`, and `deadnix` fixes through the treefmt wrapper.

- `make test-changed`

  - Use for Nix module and flake changes while iterating.
  - Maps changed files to the relevant eval checks, runtime checks, and Rust helper tests.

- `make test-eval`

  - Use for the fast eval-only Nix module test subset.

- `nix develop -c make clippy`

  - Use for Rust helper changes when you need lint coverage.

## Complete validation

`make check` is the shared local and CI acceptance command. It attempts every category (full flake checks including package builds and formatting, Clippy, Rust tests, and dependency auditing) without rewriting source files. Use it before review or merging cross-cutting changes. `make fmt` applies fixes separately.

## Nix validation

Run `nix flake check --print-build-logs` when you need the full flake-wide validation pass, especially before merging cross-cutting changes.

## What to run by change type

- `docs/**`, `README.md`, `CLAUDE.md`, simple formatting-only edits

  - Run `make fmt`.

- `modules/**`, `flake.nix`, `pkgs/**`

  - Run `make test-changed`.
  - Use `make test-eval` when you want the full eval-only module pass.
  - Use `nix flake check --print-build-logs` before merging cross-cutting changes.

- `helpers/**/src/**`

  - Run `nix develop -c make clippy`.
  - Add `make test-changed` if the Rust helper is wired into module behavior or packaging.
  - Add `nix flake check --print-build-logs` when the change also affects broader flake wiring.

- `Cargo.toml` or `Cargo.lock`

  - Run `nix develop -c make clippy`.
  - Add `cargo-audit` when dependencies changed.

## Writing tests

- Keep tests scoped to the module or package they verify so `make test-changed` can select the right subset during development.
- Prefer extending an existing eval or runtime test when coverage belongs with that area instead of adding broad cross-cutting tests that force unrelated checks to run.
- Keep fixtures and test inputs minimal; avoid unnecessary evaluations, large generated data, or duplicated setups that slow down the red -> change -> green loop.
- Split unrelated behaviors into separate focused tests only when they still map cleanly to the changed files and do not widen the `make test-changed` surface area.
- Reserve full-suite coverage for behavior that truly spans multiple test areas; otherwise optimize for fast targeted iteration first.

## Notes

- `make test-changed` is the default Nix validation command while iterating.
- `make test` runs the full Nix test suite (eval plus runtime checks) in one `nix build`.
- `make test-eval` runs only the eval checks.
- `nix flake check` still includes the `treefmt` check, while `make fmt` now applies fixes first and then verifies the treefmt check is clean.
- Prefer targeted commands while iterating; run the broader suite before merging cross-cutting changes.
