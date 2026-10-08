# Network Namespaces for Sandboxes

## Purpose

`cloister.sandboxes.<name>.network.namespace` lets a sandbox join a specific Linux network namespace before launching `bwrap`. This routes all sandbox traffic through that namespace while still using `--share-net` (which inherits from the *current* namespace at launch time).

Typical use cases:

- Force all sandbox traffic through a VPN namespace
- Allow access only to localhost dev ports
- Allow access only to specific LAN CIDR ranges
- Fully airgap a sandbox with loopback-only networking

## Setup

Enable namespace selection per sandbox in home-manager:

```nix
cloister.sandboxes.dev.network.namespace = "vpn";
```

Install the `cloister-netns` NixOS module on the host system and define at least one namespace declaratively:

```nix
{
  imports = [ cloister.nixosModules.cloister-netns ];
  cloister-netns.networks.vpn.type = "isolated";
}
```

Each `cloister-netns.networks.<name>` entry is included in the helper allowlist.

This setup has two pieces:

1. **Home-manager option** (`sandboxes.<name>.network.namespace`): tells the sandbox wrapper which namespace to join
1. **NixOS module** (`cloister-netns`): installs the `cloister-netns` helper with `CAP_SYS_ADMIN` via `security.wrappers`

The helper is group-gated by default. Only users in the `cloister-netns` Unix group can execute it. Add trusted users explicitly, for example:

```nix
{
  users.users.alice.extraGroups = [ "cloister-netns" ];
}
```

To use a different authorization group, set `cloister-netns.group = "your-group";` and add trusted users to that group instead.

## Declarative network namespaces

The `cloister-netns` NixOS module can declaratively create and manage network namespaces. Four types are supported: **wireguard** (full VPN tunnel), **localhost** (veth + DNAT to host ports), **lan** (veth + forwarding to allowed CIDR ranges), and **isolated** (loopback only).

Each entry in `cloister-netns.networks` becomes a systemd oneshot service (`cloister-netns-<name>`) and is automatically included in the helper allowlist.

| Type | Connectivity | Use case |
|------|-------------|----------|
| `wireguard` | Full internet via VPN tunnel | Route all traffic through a VPN provider |
| `localhost` | Host localhost ports via `host.internal` (DNAT) | Access local dev servers without internet |
| `lan` | LAN ranges only (configurable CIDRs) | Reach local network services, no internet |
| `isolated` | Loopback only | Fully airgapped sandbox |

### WireGuard namespace with inline values

```nix
cloister-netns.networks.vpn = {
  type = "wireguard";
  privateKeyFile = "/run/secrets/wg-private-key";
  address = [ "10.0.0.2/32" ];
  peers = [
    {
      publicKey = "abc123...";
      endpoint = "vpn.example.com:51820";
      presharedKeyFile = "/run/secrets/wg-preshared-key";
      persistentKeepalive = 25;
    }
  ];
  dns.nameservers = [ "1.1.1.1" "8.8.8.8" ];
};
```

If you use a hostname in `endpoint`, Cloister writes `/etc/netns/<name>/resolv.conf` before configuring the WireGuard peer so endpoint resolution follows the namespace DNS bootstrap settings.

### Localhost namespace

veth pair with DNAT to host ports:

```nix
cloister-netns.networks.devports = {
  type = "localhost";
  # Omit allowedPorts, or leave it null, to allow all host localhost ports.
  # Set a list to restrict access to selected ports.
  allowedPorts = [ 8000 8080 8443 ];
};
```

Inside the sandbox, access these ports as `host.internal:<port>` (for example, `curl host.internal:8080`). `127.0.0.1` remains namespace-local loopback and is not redirected to host services.
If `/etc/netns/<name>/hosts` is missing, Cloister falls back to binding host `/etc/hosts` so basic hostname resolution still works. If `/etc/netns/<name>/resolv.conf` is missing, Cloister likewise falls back to host `/etc/resolv.conf` so DNS still works. Per-sandbox `network.dns.nameservers` or `network.dns.resolvConf` overrides this resolver selection and disables resolver fallback; see [DNS servers](configuration.md#dns-servers).

For localhost namespaces, Cloister always does two things for the selected port policy:

- Adds host firewall interface openings on `veth-<name>` (`networking.firewall.interfaces`).
- Adds matching accepts in the module's localhost nft `input` chain.

When `allowedPorts` is unset or `null`, all host localhost ports are reachable. Set `allowedPorts` to a list to restrict access to selected ports.

IPv6 is disabled on the managed veth pair for `localhost` namespaces, and matching `ip6` nftables drop rules are installed as a deny-by-default backstop.

veth addresses for `localhost` and `lan` namespaces are auto-assigned from `cloister-netns.veth.addressPool` using sorted namespace index. Each namespace gets a `/30` block; host side uses `.1`, namespace side uses `.2`.

### LAN namespace

veth pair with forwarding to allowed CIDR ranges:

```nix
cloister-netns.networks.lanonly = {
  type = "lan";
  allowedRanges = [ "10.0.0.0/8" "192.168.0.0/16" ];
  dns.nameservers = [ "10.0.0.1" ];
};
```

> **Firewall:** LAN namespaces are firewalled. The namespace can only reach configured `allowedRanges` and cannot access host services directly. An `input` chain on the host drops unsolicited traffic from the namespace, and a `forward` chain restricts outbound destinations to allowed CIDRs. IP forwarding (`net.ipv4.ip_forward`) is enabled declaratively via `boot.kernel.sysctl` when any LAN namespace is configured. IPv6 is disabled on the managed veth pair, and `ip6` nftables drop rules are installed so the default posture stays deny-by-default even if IPv6 is enabled later.

## Effective packet rules summary

- `localhost` networks:
- `prerouting` DNATs `veth-<name>` TCP/UDP traffic to `127.0.0.1`, either for all ports when `allowedPorts = null` or for the configured ports.
- `forward` allows the selected TCP/UDP ports from `veth-<name>`, then drops the rest.
- `input` always allows established/related traffic and new TCP/UDP traffic for the selected ports from `veth-<name>`, then drops other traffic from `veth-<name>`.
- IPv6 traffic from or to `veth-<name>` is dropped.
- `lan` networks:
- `forward` allows only destinations in `allowedRanges` from `veth-<name>`.
- Return traffic to `veth-<name>` is allowed (established/related).
- All other namespace-originated forwarding traffic is dropped.
- `input` from `veth-<name>` is dropped except established/related packets.
- IPv6 traffic from or to `veth-<name>` is dropped.

### Isolated namespace

Loopback only, no external connectivity:

```nix
cloister-netns.networks.airgap = {
  type = "isolated";
};
```

## File-based options for secrets management

Every WireGuard option that might contain sensitive or deployment-specific data has a `*File` counterpart. These read values from files at service start time rather than baking them into the Nix store. This is designed for integration with [sops-nix](https://github.com/Mic92/sops-nix), [agenix](https://github.com/ryantm/agenix), or similar secrets managers.

| Inline option | File alternative | Scope |
|---------------|-----------------|-------|
| `publicKey` | `publicKeyFile` | per peer |
| `endpoint` | `endpointFile` | per peer |
| `address` | `addressFile` | per interface |
| `dns.nameservers` | `dns.nameserversFile` | per network |

Each pair is **mutually exclusive**. Setting both triggers an assertion failure.

File-based options expect:

- `publicKeyFile`, `endpointFile`, `addressFile`: a file containing a single value (trailing newline is stripped)
- `nameserversFile`: a file containing DNS servers separated by commas, spaces, or newlines

### WireGuard namespace with sops-nix

```nix
cloister-netns.networks.vpn = {
  type = "wireguard";
  privateKeyFile = config.sops.secrets."wg/private-key".path;
  addressFile = config.sops.secrets."wg/address".path;
  peers = [
    {
      publicKeyFile = config.sops.secrets."wg/peer-public-key".path;
      endpointFile = config.sops.secrets."wg/peer-endpoint".path;
      presharedKeyFile = config.sops.secrets."wg/preshared-key".path;
      persistentKeepalive = 25;
    }
  ];
  dns.nameserversFile = config.sops.secrets."wg/dns".path;
};
```

> **Note:** `privateKeyFile` and `presharedKeyFile` have always been file-based (WireGuard requires this). Only `publicKey`, `endpoint`, `address`, and `dns.nameservers` gained file alternatives.

## NixOS-level options (`cloister-netns.*`)

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `cloister-netns.group` | str | `"cloister-netns"` | Unix group allowed to execute the helper |
| `cloister-netns.networks` | attrsOf submodule | `{}` | Declarative namespace definitions included in the helper allowlist; non-empty networks install the helper and services |
| `cloister-netns.veth.addressPool` | str | `"172.29.0.0/16"` | CIDR pool used for localhost and LAN veth auto-assignment |

### Per-network options (`cloister-netns.networks.<name>.*`)

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `type` | enum | *(required)* | Namespace type: `wireguard`, `localhost`, `lan`, or `isolated` |
| `dns` | submodule | `{}` | DNS configuration for non-isolated namespaces |

### WireGuard network options (`cloister-netns.networks.<name>.*`)

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `privateKeyFile` | nullOr path | `null` | Path to WireGuard private key file; required when `type = "wireguard"` |
| `address` | listOf str | `[]` | Interface addresses in CIDR notation (mutually exclusive with `addressFile`) |
| `addressFile` | nullOr path | `null` | File containing a single CIDR address (mutually exclusive with `address`) |
| `peers` | listOf submodule | `[]` | Peer configurations; must be non-empty when `type = "wireguard"` |
| `mtu` | nullOr positive int | `null` | Optional interface MTU |

### Per-peer options (`cloister-netns.networks.<name>.peers.*`)

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `publicKey` | nullOr str | `null` | Peer public key (mutually exclusive with `publicKeyFile`) |
| `publicKeyFile` | nullOr path | `null` | File containing peer public key (mutually exclusive with `publicKey`) |
| `endpoint` | nullOr str | `null` | Peer endpoint as `host:port` (mutually exclusive with `endpointFile`) |
| `endpointFile` | nullOr path | `null` | File containing peer endpoint (mutually exclusive with `endpoint`) |
| `presharedKeyFile` | nullOr path | `null` | Path to preshared key file |
| `persistentKeepalive` | nullOr unsigned int | `null` | Keepalive interval in seconds |

### Per-network DNS options (`cloister-netns.networks.<name>.dns.*`)

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `nameservers` | listOf str | `[]` | DNS servers for the namespace (mutually exclusive with `nameserversFile`) |
| `nameserversFile` | nullOr path | `null` | File containing DNS servers, comma/space/newline separated (mutually exclusive with `nameservers`) |

### Localhost network options (`cloister-netns.networks.<name>.*`)

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `allowedPorts` | nullOr listOf port | `null` | Host ports accessible via DNAT; `null` allows all ports for localhost namespaces |

### LAN network options (`cloister-netns.networks.<name>.*`)

| Option | Type | Default | Purpose |
|--------|------|---------|---------|
| `allowedRanges` | nullOr listOf str | `null` | CIDR ranges the namespace can reach; `null` uses RFC1918 ranges for LAN namespaces |

### Isolated network type

Set `type = "isolated";` to enable a loopback-only namespace. DNS configuration is not applicable (assertion error if set).
