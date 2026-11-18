# Attaching To A Running Sandbox

`cloister-attach` opens a shell, or runs a command, inside a sandbox that is already running. It sees what the app sees: the same files, including private `/tmp` contents and `project-overlay` changes that exist only inside that sandbox, plus the same processes and network.

Use it to troubleshoot a browser or service from the inside, or to edit temporary files in a dev sandbox without restarting the app running there.

## Usage

```sh
cloister-attach [<sandbox|service>] [-- command [args...]]
cloister-attach --pid <pid> [-- command [args...]]
cloister-attach --list
```

| Example | Result |
| ------------------------------------ | ---------------------------------------------------------------- |
| `cloister-attach chromium` | Opens the sandbox's configured shell inside `cl-chromium` |
| `cloister-attach paseo` | Attaches to a service, by service or sandbox name |
| `cloister-attach dev -- cat /tmp/x` | Runs one command and returns its exit code |
| `cloister-attach` | Uses the instance whose project contains the current directory |
| `cloister-attach --list` | Shows running instances with their PID, name, age and project |

If a name matches several instances, the one whose project contains the current directory wins. If that does not settle it, you get a numbered picker. Without a terminal, an ambiguous match is an error that lists the choices for `--pid`.

The command starts in the sandbox's project directory, or in its home directory if it has no project. `CLOISTER_ATTACHED=1` is set so shell prompts can show that a shell is attached.

Pipes and files work as usual, for example `cloister-attach dev -- cat notes | grep todo`. Started as a background job, the command's stdin is `/dev/null`.

## What The Attached Process Gets

The attached process has the same policy as the target, and no more:

- the target's namespaces
- the seccomp filter the instance started with
- no capabilities, and `no_new_privs`
- the environment of the sandbox's main process, with `TERM` and `COLORTERM` taken from your terminal

Tools are limited to what the sandbox already contains. There is no debug mode that adds host tools or relaxes the filter, so tools such as `strace` will not work where the filter denies `ptrace`.

## Lifetime

- Exiting the attached shell does not affect the app.
- When the sandbox exits, the kernel immediately kills everything attached to it, including unsaved work in an attached editor. `cloister-attach` prints `sandbox exited`.
- An attached shell stays in your terminal's cgroup, so limits on a service unit, such as `MemoryMax`, do not apply to it. `systemctl stop` still ends it, because the sandbox's PID namespace goes away.

## After A Rebuild

An instance keeps the policy it started with, so attach uses that instance's seccomp filter rather than the current generation's. If garbage collection has removed that filter, attach refuses and asks you to restart the sandbox. It never falls back to a different filter.

## Requirements

- `XDG_RUNTIME_DIR` must be set, for both the sandbox and `cloister-attach`.
- No extra privileges are needed. Sandboxes using `network.namespace` go through the `cloister-netns` helper, so you need the same group membership that launching them requires.
- A sandbox that binds `$XDG_RUNTIME_DIR`, or anything containing `$XDG_RUNTIME_DIR/cloister/instances`, prints a warning at launch and cannot be attached to. See [Security Model](security.md#attaching-to-sandboxes).
