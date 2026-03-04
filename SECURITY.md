# Security Policy

## Scope

This policy covers security vulnerabilities in:

- The Cloister sandbox runtime (`cloister-sandbox`, `cloister-netns`)
- Seccomp filter generation (`cloister-seccomp-filter`)
- Nix module logic that generates sandboxing configuration

Out of scope:

- Bugs in upstream dependencies (bubblewrap, xdg-dbus-proxy, WireGuard, nftables) — report those to the upstream projects directly.
- Issues that require kernel exploits, root access, or physical access to the machine (these are explicitly excluded from Cloister's threat model).

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Instead, use GitHub's private **Security Advisories** feature:

1. Go to the [Security Advisories page](https://github.com/burnskp/cloister/security/advisories).
2. Click **"New draft security advisory"**.
3. Fill in the vulnerability details.

This ensures the report is visible only to repository maintainers until a fix is ready.

## What to Include

- A clear description of the vulnerability and its impact.
- Steps to reproduce or a proof-of-concept.
- Affected component(s) and version(s), if known.
- Any suggested fix or mitigation.

## Response Timeline

- **Acknowledgement:** Within 3 business days of the report.
- **Initial assessment:** Within 7 business days.
- **Fix and disclosure:** We aim to release a fix within 30 days for high-severity issues. Lower-severity findings are addressed in the next regular release.

## Coordinated Disclosure

We follow a coordinated disclosure process:

1. The reporter submits a private advisory.
2. We confirm the vulnerability and develop a fix.
3. We coordinate a disclosure date with the reporter.
4. The fix is released and the advisory is published.

We credit reporters in the advisory unless they prefer to remain anonymous.
