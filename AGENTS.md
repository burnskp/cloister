# AGENTS.md

This agent runs in a sandbox and does not have access to the NixOS host environment. This means you cannot query systemd services, read `/etc/nixos`, inspect running processes, or access files outside the project directory. If you need information from the host system (e.g., current service status, hardware details, system logs), ask the user to run the relevant commands and provide the output.

When the user references `~` in a path, do not assume it maps to `/root`. For Bash commands, use `~` or `$HOME` directly and let the shell expand it. For non-shell tools (Read, Glob, etc.), resolve `$HOME` once with a quick `echo $HOME` and reuse that value for the rest of the session.

## Validation Steps

After you complete a set of changes to this repo that would modify any nix configuration files, run the following commands and fix any warnings or errors. Do not ask for permission first; run the suite and report results:

```
treefmt
deadnix
statix check
nix flake check
```

Before running `nix flake check`, make sure any new files are staged with git. The check runs against the git worktree snapshot, so unstaged new paths will be missing and the evaluation can fail.

After you complete a set of changes to this repo that would modify any Rust source files, run the following commands and fix any warnings or errors. Do not ask for permission first; run the suite and report results:

```
nix develop -c make test-rust
nix develop -c make clippy
```
