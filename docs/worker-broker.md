# Worker Broker

Worker broker lets a parent Cloister sandbox launch pre-approved child sandboxes for the same project without giving the parent general host container authority.

## What It Is For

Use worker broker when a sandboxed tool needs to delegate work to another Cloister sandbox, such as:

- launching a more restricted worker sandbox from a broader development sandbox
- running nested helper commands against the same project checkout
- exposing only specific delegated per-directory mounts like worktrees or caches

The parent sandbox gets an opaque capability token in env, while authoritative launch policy stays on the host in a trusted session record. The parent launcher may still have host-authored broker profile and delegated mount metadata available from its rendered config.

Worker broker also requires `XDG_RUNTIME_DIR` so the parent launch can register its trusted session record on the host.

When `workerBroker.profiles` is non-empty, Cloister adds one launcher per profile to the parent sandbox `PATH`: `clb-<profile>`.
Those launchers are available by plain name inside the parent sandbox, so run them as:

```sh
clb-<profile> <command> [args...]
```

The launcher selects the configured child sandbox and passes the command argv directly. `-c` is rejected for broker launches.

Brokered child launches inherit the launcher's stdin, stdout, and stderr via Unix file descriptor passing, so pipes, redirections, and output capture from programs that spawn `clb-<profile>` behave like normal subprocess stdio. If one of the standard descriptors is closed, the launcher substitutes `/dev/null` for that stream so non-interactive broker launches can still start.

## Trust Model

- parent sandboxes receive an opaque capability token in env
- parent launchers may also have host-authored broker profile and delegated mount metadata from rendered config
- child launches re-load policy from a trusted mounted session record
- child launches must stay within the same project identity
- delegated mounts come only from host-authored profile metadata
- session-record cleanup is attempted on normal parent exit

Crash-leftover session records are not yet pruned automatically.

## Minimal Configuration

```nix
cloister.sandboxes = {
  dev = {
    preset = "developer";
    sandbox.bindWorkingDirectory = true;

    workerBroker.profiles = {
      project = {
        sandbox = "worker";
        workspace.mode = "project-rw";
      };

      overlay = {
        sandbox = "worker";
        workspace.mode = "project-overlay";
        delegatedPerDirMounts.worktrees = {
          mode = "rw";
          path = "/local/worktrees/dev";
        };
        delegatedPerDirMounts.".cache/pre-commit" = {
          mode = "ro";
          path = "/local/ephemeral/dev";
          subPath = ".cache/pre-commit";
        };
      };
    };
  };

  worker = {
    preset = "hardened";
    shell.hostConfig = false;
  };
};
```

## Choosing A Workspace Mode

### `project-rw`

Use `project-rw` when the child sandbox should write directly to the same project tree the parent is using.

### `project-overlay`

Use `project-overlay` when the child sandbox should see the project tree but keep its file writes isolated in an overlay view.

This is useful for nested automation that should inspect and modify files temporarily without writing back to the host checkout.

## Delegated Per-Directory Mounts

`delegatedPerDirMounts` lets a profile opt into specific extra paths under the project root. Each entry is keyed by the sandbox-relative destination path and declares the host base path, optional subpath, and `ro`/`rw` mode.

Example:

```nix
delegatedPerDirMounts.worktrees = {
  mode = "rw";
  path = "/local/worktrees/dev";
};

delegatedPerDirMounts.".cache/pre-commit" = {
  mode = "ro";
  path = "/local/ephemeral/dev";
  subPath = ".cache/pre-commit";
};
```

## Manual Test Flow

One simple manual test is:

1. Start in a project checkout managed by the `dev` sandbox.
1. Enter the parent sandbox normally.
1. From inside that sandbox, run the generated launcher for the profile you want. For example, `clb-overlay <command> [args...]` launches the configured child sandbox for the `overlay` profile.
1. In the child sandbox, verify that:
   - the child is running in the `worker` sandbox
   - the project root matches the same project
   - delegated mounts appear only where configured
   - `project-rw` writes affect the shared project tree, or `project-overlay` writes stay isolated

The runtime coverage for this flow lives in `tests/runtime/worker-broker.nix`.

## Current Limitations

- session-record cleanup is best-effort on normal parent exit
- crash or hard-kill leftovers are not yet pruned automatically
- worker broker requires `XDG_RUNTIME_DIR`
- worker broker requires `sandbox.bindWorkingDirectory = true`
- child launches are limited to the same project identity and configured profiles
