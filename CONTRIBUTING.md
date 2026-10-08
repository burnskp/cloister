# Contributing

Thanks for improving Cloister.

## Before you open a pull request

- Confirm the change matches the repository scope: sandboxing, docs, examples, helper tools, or tests.
- Keep changes focused so `make test-changed` can select the smallest relevant validation set.
- Update docs and examples when behavior or defaults change.

## Local development

1. Enter the dev shell with `nix develop`.
1. Make your change.
1. Run the smallest matching validation command:
   - docs and formatting: `make fmt`
   - Nix module changes: `make test-changed`
   - Rust helper changes: `nix develop -c make clippy` and `nix develop -c make rust-test`
1. Run `make check` before asking for review. This is the complete, non-mutating local and CI acceptance command: all flake checks and published packages, Clippy, workspace Rust tests, and dependency auditing. Every category is attempted even if another fails. VM tests require usable `/dev/kvm`.

`make fmt` applies fixes separately. Rust workspaces are discovered from `helpers/*/Cargo.toml` and must have committed lockfiles. See [Testing](docs/testing.md) for subset overrides.

## Pull request guidelines

- Explain the user-visible reason for the change.
- Link the issue or motivation when possible.
- Include any follow-up work that should stay out of the current PR.
- Add or extend focused tests near the behavior you changed.

## Coding notes

- Prefer targeted tests over broad new suites.
- Do not check in secrets, private credentials, or local machine paths.
- Keep examples importable and runnable as published documentation.

## Questions and support

- Use GitHub issues for bugs, usability problems, and feature requests.
- For security-sensitive problems, follow `SECURITY.md` instead of opening a public issue.

## Repository creation checklist

- Preserve `.gitignore` exclusions for `.pi/`, Rust targets, Nix results, and other local artifacts; review the files and history intended for publication.
- Enable GitHub private vulnerability reporting under repository Settings → Security so the [private report link](https://github.com/burnskp/cloister/security/advisories/new) works.
- Run `make check` and confirm validation leaves tracked files unchanged.
- Publish or recreate the GitHub repository only as a separate, explicitly requested action.
