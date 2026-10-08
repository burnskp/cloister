.PHONY: fmt lint clippy rust-test cargo-audit test test-eval test-runtime test-changed check clean

# Flake check names and top-level Cargo manifests are the validation inventory.
SYSTEM ?= x86_64-linux
CHECK_NAMES = $(eval CHECK_NAMES := $(shell nix eval --no-update-lock-file --raw .\#checks.$(SYSTEM) --apply 'checks: builtins.concatStringsSep " " (builtins.attrNames checks)' || echo DISCOVERY_FAILED))$(if $(filter DISCOVERY_FAILED,$(CHECK_NAMES)),$(error Unable to discover flake checks))$(CHECK_NAMES)
TEST_CHECKS ?= $(filter-out test-runtime-%,$(filter test-%,$(CHECK_NAMES)))
RUNTIME_TEST_CHECKS ?= $(filter test-runtime-%,$(CHECK_NAMES))
TEST_TARGETS = $(addprefix .\#checks.$(SYSTEM).,$(TEST_CHECKS))
RUNTIME_TEST_TARGETS = $(addprefix .\#checks.$(SYSTEM).,$(RUNTIME_TEST_CHECKS))
ALL_TEST_TARGETS = $(TEST_TARGETS) $(RUNTIME_TEST_TARGETS)
RUST_HELPER_WORKSPACES ?= $(patsubst %/Cargo.toml,%,$(wildcard helpers/*/Cargo.toml))
RUST_TEST_HELPERS ?= $(RUST_HELPER_WORKSPACES)
RUST_AUDIT_WORKSPACES ?= $(RUST_HELPER_WORKSPACES)
BASE_REF ?= main
TEST_CHANGED_FILES ?=
# Keep test invocations restricted to the official NixOS binary cache.
# Override NIX_TEST_FLAGS if you intentionally want different Nix test settings.
NIX_TEST_FLAGS ?= --no-update-lock-file --option substituters https://cache.nixos.org/ --option extra-substituters ""

# Apply treefmt fixes, then verify the treefmt check is clean
fmt:
	nix run .#formatter.x86_64-linux --
	nix build .#checks.x86_64-linux.treefmt --print-build-logs

# Static analysis (statix + deadnix)
lint:
	nix run nixpkgs#statix -- check .
	nix run nixpkgs#deadnix -- --fail .

# All Rust categories use the same workspace discovery and committed lockfiles.
clippy:
	nix develop --no-update-lock-file -c sh scripts/rust-check.sh clippy $(RUST_HELPER_WORKSPACES)

rust-test:
	nix develop $(NIX_TEST_FLAGS) -c sh scripts/rust-check.sh test $(RUST_TEST_HELPERS)

cargo-audit:
	nix develop --no-update-lock-file -c sh scripts/rust-check.sh audit $(RUST_AUDIT_WORKSPACES)

# Full test suite; Nix can build eval and runtime checks concurrently in one invocation
test:
	@test -n "$(ALL_TEST_TARGETS)" || { echo "No test checks discovered" >&2; exit 1; }
	nix build $(NIX_TEST_FLAGS) --print-build-logs $(ALL_TEST_TARGETS)

# Eval tests only
test-eval:
	@test -n "$(TEST_TARGETS)" || { echo "No eval checks discovered" >&2; exit 1; }
	nix build $(NIX_TEST_FLAGS) --print-build-logs $(TEST_TARGETS)

# Runtime VM tests; kept separate from the fast eval-only loop
test-runtime:
	@test -n "$(RUNTIME_TEST_TARGETS)" || { echo "No runtime checks discovered" >&2; exit 1; }
	nix build $(NIX_TEST_FLAGS) --print-build-logs $(RUNTIME_TEST_TARGETS)

# Run only checks matching changed files; override TEST_CHANGED_FILES for ad hoc selection
test-changed:
	@set -eu; \
	add_check() { \
		case " $$checks " in \
			*" $$1 "*) ;; \
			*) checks="$$checks $$1" ;; \
		esac; \
	}; \
	add_runtime_check() { \
		case " $$runtime_checks " in \
			*" $$1 "*) ;; \
			*) runtime_checks="$$runtime_checks $$1" ;; \
		esac; \
	}; \
	add_helper() { \
		case " $$helpers " in \
			*" $$1 "*) ;; \
			*) helpers="$$helpers $$1" ;; \
		esac; \
	}; \
	add_dbus_checks() { \
		add_check test-cloister-gui-dbus-audio; \
		add_runtime_check test-runtime-gui-dbus-audio; \
	}; \
	if [ -n "$(TEST_CHANGED_FILES)" ]; then \
		changed_files="$(TEST_CHANGED_FILES)"; \
	else \
		if git rev-parse --verify "$(BASE_REF)" >/dev/null 2>&1; then \
			compare_ref="$(BASE_REF)...HEAD"; \
		else \
			compare_ref="HEAD"; \
		fi; \
		changed_files="$$( \
			{ \
				git diff --name-only --diff-filter=ACMR $$compare_ref; \
				git diff --name-only --diff-filter=ACMR; \
				git diff --cached --name-only --diff-filter=ACMR; \
				git ls-files --others --exclude-standard; \
			} | sort -u \
		)"; \
	fi; \
	if [ -z "$$changed_files" ]; then \
		printf 'No changed files detected. Nothing to test.\n'; \
		exit 0; \
	fi; \
	checks=""; \
	runtime_checks=""; \
	helpers=""; \
	for path in $$changed_files; do \
		case "$$path" in \
			Makefile|scripts/*|.github/workflows/*) \
				checks="$(TEST_CHECKS)"; \
				runtime_checks="$(RUNTIME_TEST_CHECKS)"; \
				helpers="$(RUST_TEST_HELPERS)"; \
				;; \
			tests/cloister/gui-dbus-audio.nix|tests/runtime/gui-dbus-audio.nix) \
				add_dbus_checks \
				;; \
			tests/runtime/sandbox-core.nix) \
				add_runtime_check test-runtime-sandbox-core \
				;; \
			tests/runtime/worker-broker.nix) \
				add_runtime_check test-runtime-worker-broker \
				;; \
			tests/runtime/image-store.nix) \
				add_runtime_check test-runtime-image-store \
				;; \
			tests/runtime/netns.nix) \
				add_runtime_check test-runtime-netns \
				;; \
			flake.nix|flake.lock) \
				checks="$(TEST_CHECKS)"; \
				runtime_checks="$(RUNTIME_TEST_CHECKS)"; \
				helpers="$(RUST_TEST_HELPERS)"; \
				;; \
			tests/default.nix) \
				checks="$(TEST_CHECKS)"; \
				runtime_checks="$(RUNTIME_TEST_CHECKS)"; \
				;; \
			tests/runtime/*) \
				runtime_checks="$(RUNTIME_TEST_CHECKS)" \
				;; \
			tests/*) \
				checks="$(TEST_CHECKS)"; \
				;; \
			modules/cloister/_bwrap.nix|pkgs/bubblewrap-subset-pid/*) \
				add_check test-cloister-bwrap \
				;; \
			examples/*) \
				add_check test-cloister-examples \
				;; \
			modules/cloister/_registry.nix|modules/cloister/_wrappers.nix) \
				add_check test-cloister-registry \
				add_runtime_check test-runtime-sandbox-core \
				;; \
			modules/cloister/_options.nix|modules/cloister/default.nix|modules/cloister/_mkShells.nix|modules/cloister/_shells/*|modules/cloister/_resolve.nix|modules/cloister/_patterns.nix) \
				add_check test-cloister-registry; \
				add_check test-cloister-presets; \
				add_check test-cloister-sandbox-core; \
				add_dbus_checks; \
				add_runtime_check test-runtime-sandbox-core; \
				add_check test-cloister-rendered-config; \
				add_check test-cloister-worker-broker; \
				add_runtime_check test-runtime-worker-broker; \
				add_check test-cloister-image-store; \
				add_runtime_check test-runtime-image-store \
				;; \
			modules/cloister/_assertions.nix) \
				add_check test-cloister-sandbox-core; \
				add_check test-cloister-worker-broker; \
				add_runtime_check test-runtime-worker-broker; \
				add_dbus_checks \
				;; \
			modules/cloister/_sandbox.nix) \
				add_check test-cloister-sandbox-core; \
				add_dbus_checks; \
				add_runtime_check test-runtime-sandbox-core; \
				add_check test-cloister-rendered-config; \
				add_check test-cloister-worker-broker; \
				add_runtime_check test-runtime-worker-broker; \
				add_check test-cloister-image-store; \
				add_runtime_check test-runtime-image-store \
				;; \
			modules/cloister/package-config.nix) \
				add_check test-cloister-sandbox-core; \
				add_runtime_check test-runtime-sandbox-core; \
				add_check test-cloister-rendered-config; \
				add_check test-cloister-worker-broker; \
				add_runtime_check test-runtime-worker-broker; \
				add_dbus_checks; \
				add_check test-cloister-image-store; \
				add_runtime_check test-runtime-image-store \
				;; \
			modules/cloister-image-store/*) \
				add_check test-cloister-image-store; \
				add_runtime_check test-runtime-image-store \
				;; \
			modules/cloister-netns/*) \
				add_check test-cloister-netns; \
				add_runtime_check test-runtime-netns \
				;; \
			helpers/cloister-sandbox|helpers/cloister-sandbox/*) \
				add_helper helpers/cloister-sandbox; \
				add_check test-cloister-rendered-config; \
				add_runtime_check test-runtime-sandbox-core \
				;; \
			helpers/*) \
				helper="$${path#helpers/}"; \
				helper="helpers/$${helper%%/*}"; \
				if [ -f "$$helper/Cargo.toml" ]; then add_helper "$$helper"; fi \
				;; \
			README.md) \
				add_check test-cloister-standalone-example \
				;; \
			CLAUDE.md|AGENTS.md|CONTRIBUTING.md|SECURITY.md|docs/*) \
				: \
				;; \
			*) \
				checks="$(TEST_CHECKS)"; \
				;; \
		esac; \
	done; \
	checks="$$(printf '%s\n' $$checks | xargs)"; \
	runtime_checks="$$(printf '%s\n' $$runtime_checks | xargs)"; \
	helpers="$$(printf '%s\n' $$helpers | xargs)"; \
	if [ -z "$$checks" ] && [ -z "$$runtime_checks" ] && [ -z "$$helpers" ]; then \
		printf 'No eval, runtime, or Rust checks mapped for changed files: %s\n' "$$changed_files"; \
		exit 0; \
	fi; \
	if [ -n "$$checks" ]; then \
		printf 'Running eval checks: %s\n' "$$checks"; \
		$(MAKE) test-eval TEST_CHECKS="$$checks"; \
	fi; \
	if [ -n "$$runtime_checks" ]; then \
		printf 'Running runtime checks: %s\n' "$$runtime_checks"; \
		$(MAKE) test-runtime RUNTIME_TEST_CHECKS="$$runtime_checks"; \
	fi; \
	if [ -n "$$helpers" ]; then \
		printf 'Running Rust helper tests: %s\n' "$$helpers"; \
		$(MAKE) rust-test RUST_TEST_HELPERS="$$helpers"; \
	fi

# Attempt every category even when an earlier one fails. Never apply fixes here.
check:
	@status=0; \
	nix flake check --no-update-lock-file --keep-going --print-build-logs || status=1; \
	$(MAKE) clippy || status=1; \
	$(MAKE) rust-test || status=1; \
	$(MAKE) cargo-audit || status=1; \
	exit $$status

# Remove local Nix output links and Rust build artifacts without a toolchain.
# Leave Git metadata, separate worktrees, and the direnv environment alone.
clean:
	find . \( -name .git -o -name .worktrees -o -name .direnv \) -prune -o \
		-type d -name target -prune -exec rm -rf -- {} + -o \
		-type l \( -name result -o -name 'result-*' \) -exec rm -f -- {} +
