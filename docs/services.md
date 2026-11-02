# Services

`cloister.services.<name>` runs a long-lived command, such as a daemon or a server, inside an existing Cloister sandbox. A systemd user unit supervises it.

## What It Is For

Use a service when a tool should keep running in the background with the same policy as one of your interactive sandboxes. For example:

- an agent orchestrator such as [paseo](https://github.com/getpaseo/paseo) that manages repos and worktrees in a dedicated directory
- a language server, build watcher, or local web app that should outlive your terminal

A service does not define its own sandbox. It points at a `cloister.sandboxes` entry and can only add to it, so a service is always "the same sandbox as `cl-<sandbox>`, plus these paths". If a service needs a different policy, define a separate sandbox for it.

## Minimal Configuration

```nix
cloister = {
  enable = true;

  sandboxes.agents = {
    preset = "developer";
    extraPackages = [ pkgs.paseo ];
  };

  services.paseo = {
    sandbox = "agents";
    command = [ "paseo" "daemon" "run" ];
    workingDirectory = "cloister/paseo";
    readWrite = [
      {
        src = ".local/state/cloister/paseo";
        dest = ".paseo";
      }
    ];
    env.PASEO_LISTEN = "127.0.0.1:6767";
  };
};
```

This produces:

- `cloister-service-paseo.service`: a systemd user unit, started with your session
- `cloister-service-paseo`: the launcher the unit runs. It is also on `PATH`, so you can run the service in the foreground while debugging

See `examples/paseo-service.nix` for a fuller example.

## Options

| Option | Default | Meaning |
| ------------------- | ---------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `sandbox` | (required) | Sandbox whose packages, presets, state, and policy the service uses. |
| `command` | (required) | Argv executed inside the sandbox. It runs directly, not through a shell. |
| `workingDirectory` | (required) | Host directory bound read-write as the project root. Absolute or home-relative. |
| `readWrite` | `[ ]` | Extra read-write binds appended to the sandbox's `sandbox.readWrite`. |
| `readOnly` | `[ ]` | Extra read-only binds appended to the sandbox's `sandbox.readOnly`. |
| `env` | `{ }` | Extra environment variables merged over the sandbox's `sandbox.env`. |
| `network.namespace` | `null` | Join a `cloister-netns` namespace instead of using the sandbox's network setting. |
| `autoStart` | `true` | Add `WantedBy=default.target`. |
| `extraServiceConfig` | `{ }` | Unit sections (`Unit`, `Service`, `Install`) recursively merged over the generated unit. |

Bind entries use the same syntax as `sandbox.readWrite` and `sandbox.readOnly`. A string entry is home-relative. An absolute path needs the attrset form `{ src = "/abs/path"; }`. Nix store paths are rejected.

## How It Runs

- The launcher `cd`s into `workingDirectory` and pins `CLOISTER_DIR` to it. The service therefore gets the same mounts and per-project state as running `cl-<sandbox>` from that directory, and an enclosing git work tree never widens the project root.
- Before the sandbox starts, the launcher creates the working directory and any missing non-optional `readWrite` sources, with mode `0700`. Existing paths are never chmod-ed.
- The unit uses `Restart=on-failure` with a 5 second backoff, and sends output to the journal: `journalctl --user -u cloister-service-<name>`.
- On stop, systemd sends SIGTERM to the launcher, which forwards it to the sandboxed command and SIGKILLs the sandbox if it has not exited after 10 seconds. A stop that arrives while the launcher is still setting up cancels the launch. As a backstop, `KillMode=mixed` with `TimeoutStopSec=20` lets systemd kill whatever is left. A clean stop exits with status 143, which the unit treats as success.
- The launcher starts the D-Bus proxy and PipeWire bridge per launch, so they work as they do for interactive launches. Integrations that read your login session's environment (for example `SSH_AUTH_SOCK`) only work if the systemd user manager has that variable. Use `systemctl --user import-environment` or `extraServiceConfig.Service.Environment` to provide it.

### Surviving Logout

User units stop when your last session ends unless lingering is enabled:

```sh
loginctl enable-linger "$USER"
```

Cloister does not change this for you.

### systemd Hardening

Cloister deliberately sets **no** systemd sandboxing directives on service units, because bubblewrap is the isolation boundary. Several common directives also break Cloister:

- `NoNewPrivileges=yes` stops the `cap_sys_admin` `cloister-netns` wrapper from joining a network namespace.
- `PrivateUsers=yes` interferes with the user namespace bubblewrap creates.

You can still add directives through `extraServiceConfig.Service`, but you then own any breakage.

## Networking

A service gets its network from the referenced sandbox:

- **Host networking** (`network.enable = true`, the default for the `developer` preset): the daemon listens on the host's interfaces. Bind it to `127.0.0.1` unless you mean to expose it.
- **Network namespace** (`network.namespace = "<name>"`, NixOS with the `cloister-netns` module): the daemon is isolated in that namespace and listens on the namespace's veth address, which the host can reach. Cloister does not forward inbound traffic into a namespace. For access from other machines, put a reverse proxy, or something like Tailscale serve, on the host in front of the veth address.

`network.namespace` on a service overrides only that service. Interactive `cl-<sandbox>` launches keep the sandbox's own network setting.

## Debugging

- `systemctl --user status cloister-service-<name>` and `journalctl --user -u cloister-service-<name>`
- Run `cloister-service-<name>` in a terminal to start the service in the foreground with the exact same config. Stop the unit first if the service holds a port or lock.
- Run `cl-<sandbox>` from `workingDirectory` for an interactive shell with the sandbox's policy. Service-only additions (extra binds, env, namespace) are not applied there.

Attaching a shell to the running service's namespaces is not supported.

## Validation

Evaluation fails when:

- `sandbox` names a sandbox that does not exist
- the service name does not match `^[A-Za-z0-9_-]+$`
- `command` is empty
- `workingDirectory` is `/`, your home directory, contains `$` or a newline, or is a Nix store path
- an `env` key is not a valid variable name
- the derived sandbox fails any normal sandbox assertion (for example unsafe bind paths, duplicate destinations, or env keys that override computed variables). These messages are prefixed with `cloister.services.<name> (derived from sandbox <sandbox>)`.
