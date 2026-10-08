# Security Policy

Cloister is a sandboxing project, so security reports are especially valuable.

## Reporting a vulnerability

- Do not open a public GitHub issue for a suspected vulnerability.
- [Report a vulnerability privately through GitHub](https://github.com/burnskp/cloister/security/advisories/new). Include reproduction details, impact, and any known mitigations.

## What to include

- affected configuration or helper
- exact commands or configuration needed to reproduce
- expected behavior versus observed behavior
- whether the issue can expose host files, credentials, devices, or privileged services

## Response expectations

- Reports will be triaged as quickly as possible.
- If the issue is confirmed, the goal is to ship a fix and publish coordinated release notes after a patch is available.

## Scope

Please report issues involving:

- sandbox escape or unintended host access
- credential or secret exposure through binds, helpers, or generated wrappers
- validator helpers silently passing unsafe configurations
- namespace, D-Bus, Wayland, audio, or seccomp behavior that weakens documented isolation guarantees
