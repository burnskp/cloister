#!/bin/sh
# Shared by local validation and CI. Continue through all selected workspaces.
set -u
category=$1
shift
status=0
for dir do
    if ! test -f "$dir/Cargo.lock" || ! git ls-files --error-unmatch "$dir/Cargo.lock" >/dev/null 2>&1; then
        printf 'Missing committed lockfile: %s/Cargo.lock\n' "$dir" >&2
        status=1
        continue
    fi
    printf 'Running %s in %s\n' "$category" "$dir"
    case "$category" in
        clippy) (cd "$dir" && cargo clippy --workspace --locked -- -D warnings -W clippy::cast_possible_truncation) || status=1 ;;
        test) (cd "$dir" && cargo test --workspace --locked) || status=1 ;;
        audit)
            # See docs/security.md: bundled XML parsed by wayrs-scanner at build time.
            case "$dir" in
                helpers/cloister-sandbox|helpers/cloister-wayland-validate)
                    cargo-audit audit -f "$dir/Cargo.lock" --deny unsound --ignore RUSTSEC-2026-0194 --ignore RUSTSEC-2026-0195 || status=1 ;;
                *) cargo-audit audit -f "$dir/Cargo.lock" --deny unsound || status=1 ;;
            esac ;;
        *) printf 'Unknown Rust validation category: %s\n' "$category" >&2; exit 1 ;;
    esac
done
exit "$status"
