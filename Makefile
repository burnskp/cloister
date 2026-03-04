.PHONY: test test-rust test-nix fmt lint clippy check clean

# Run all tests
test: test-rust test-nix

# Rust unit + integration tests for helper binaries
test-rust:
	cd helpers/cloister-dbus-validate && cargo test
	cd helpers/cloister-netns && cargo test
	cd helpers/cloister-sandbox && cargo test
	cd helpers/cloister-wayland-validate && cargo test
	cd helpers/cloister-seccomp-filter && cargo test
	cd helpers/cloister-seccomp-validate && cargo test

# Nix module evaluation tests (bwrap, sandbox, registry, wrappers)
test-nix:
	bash -o pipefail -c 'nix flake check --print-build-logs 2> >(grep -v "unknown flake output '\''homeManagerModules'\''" >&2)'

# Formatting check (treefmt via flake check)
fmt:
	nix build .#checks.x86_64-linux.treefmt --print-build-logs

# Static analysis (statix + deadnix)
lint:
	nix run nixpkgs#statix -- check .
	nix run nixpkgs#deadnix -- --fail .

# Clippy with security-relevant lints
clippy:
	cd helpers/cloister-dbus-validate && cargo clippy -- -D warnings -W clippy::cast_possible_truncation
	cd helpers/cloister-netns && cargo clippy -- -D warnings -W clippy::cast_possible_truncation
	cd helpers/cloister-sandbox && cargo clippy -- -D warnings -W clippy::cast_possible_truncation
	cd helpers/cloister-wayland-validate && cargo clippy -- -D warnings -W clippy::cast_possible_truncation
	cd helpers/cloister-seccomp-filter && cargo clippy -- -D warnings -W clippy::cast_possible_truncation
	cd helpers/cloister-seccomp-validate && cargo clippy -- -D warnings -W clippy::cast_possible_truncation

# All CI checks (mirrors .github/workflows/ci.yml)
check: test fmt lint clippy

# Remove Rust build artifacts
clean:
	cd helpers/cloister-dbus-validate && cargo clean
	cd helpers/cloister-netns && cargo clean
	cd helpers/cloister-sandbox && cargo clean
	cd helpers/cloister-wayland-validate && cargo clean
	cd helpers/cloister-seccomp-filter && cargo clean
	cd helpers/cloister-seccomp-validate && cargo clean
