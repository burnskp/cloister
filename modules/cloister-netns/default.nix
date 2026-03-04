{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.cloister-netns;

  effectiveAllowedNamespaces = cfg.allowedNamespaces ++ lib.attrNames cfg.networks;

  cloister-sandbox = pkgs.callPackage ../../helpers/cloister-sandbox { };
  cloister-netns = pkgs.callPackage ../../helpers/cloister-netns {
    allowedNamespaces = effectiveAllowedNamespaces;
    inherit (cfg) enforceExecAllowlist allowedExecPaths;
  };

  # ── Submodules ────────────────────────────────────────────────────────

  peerSubmodule = {
    options = {
      publicKey = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "WireGuard public key of the peer (mutually exclusive with publicKeyFile).";
      };
      publicKeyFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to a file containing the WireGuard public key (mutually exclusive with publicKey).";
      };
      endpoint = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Peer endpoint in host:port format (mutually exclusive with endpointFile).";
      };
      endpointFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to a file containing the peer endpoint (mutually exclusive with endpoint).";
      };
      presharedKeyFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to the preshared key file for this peer.";
      };
      persistentKeepalive = lib.mkOption {
        type = lib.types.nullOr lib.types.ints.unsigned;
        default = null;
        description = "Persistent keepalive interval in seconds.";
      };
    };
  };

  wireguardSubmodule = {
    options = {
      privateKeyFile = lib.mkOption {
        type = lib.types.path;
        description = "Path to the WireGuard private key file.";
      };
      address = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = ''Addresses to assign to the WireGuard interface in CIDR notation (e.g. ["10.0.0.2/32"]). Mutually exclusive with addressFile.'';
      };
      addressFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to a file containing the WireGuard address in CIDR notation. Mutually exclusive with address.";
      };
      peers = lib.mkOption {
        type = lib.types.listOf (lib.types.submodule peerSubmodule);
        description = "WireGuard peer configurations.";
      };
      mtu = lib.mkOption {
        type = lib.types.nullOr lib.types.ints.positive;
        default = null;
        description = "Optional MTU for the WireGuard interface.";
      };
    };
  };

  localhostSubmodule = {
    options = {
      allowedPorts = lib.mkOption {
        type = lib.types.listOf lib.types.port;
        default = [
          8000
          8080
          8443
        ];
        description = "Host ports accessible from the namespace via DNAT.";
      };
      hostAddress = lib.mkOption {
        type = lib.types.str;
        default = "172.30.0.1/24";
        description = "Address assigned to the host end of the veth pair (CIDR).";
      };
      namespaceAddress = lib.mkOption {
        type = lib.types.str;
        default = "172.30.0.2/24";
        description = "Address assigned to the namespace end of the veth pair (CIDR).";
      };
    };
  };

  lanSubmodule = {
    options = {
      allowedRanges = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [
          "10.0.0.0/8"
          "172.16.0.0/12"
          "192.168.0.0/16"
        ];
        description = "CIDR ranges the namespace is allowed to reach (forwarded through nftables).";
      };
      hostAddress = lib.mkOption {
        type = lib.types.str;
        default = "172.29.0.1/24";
        description = "Address assigned to the host end of the veth pair (CIDR).";
      };
      namespaceAddress = lib.mkOption {
        type = lib.types.str;
        default = "172.29.0.2/24";
        description = "Address assigned to the namespace end of the veth pair (CIDR).";
      };
    };
  };

  networkSubmodule = {
    options = {
      wireguard = lib.mkOption {
        type = lib.types.nullOr (lib.types.submodule wireguardSubmodule);
        default = null;
        description = "WireGuard tunnel configuration for this namespace.";
      };
      localhost = lib.mkOption {
        type = lib.types.nullOr (lib.types.submodule localhostSubmodule);
        default = null;
        description = "Localhost-only veth + DNAT configuration for this namespace.";
      };
      lan = lib.mkOption {
        type = lib.types.nullOr (lib.types.submodule lanSubmodule);
        default = null;
        description = "LAN-access veth configuration for this namespace (forward to allowed ranges only).";
      };
      isolated = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Fully isolated namespace with loopback only (no network connectivity).";
      };
      dns = lib.mkOption {
        type = lib.types.submodule {
          options = {
            nameservers = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              description = "DNS nameservers written to /etc/netns/<name>/resolv.conf. Mutually exclusive with nameserversFile.";
            };
            nameserversFile = lib.mkOption {
              type = lib.types.nullOr lib.types.path;
              default = null;
              description = "Path to a file containing DNS servers (comma/space/newline separated). Mutually exclusive with nameservers.";
            };
          };
        };
        default = { };
        description = "DNS configuration for the network namespace.";
      };
    };
  };

  # ── Service generators ────────────────────────────────────────────────

  mkResolvConf =
    name: dnsCfg:
    let
      escapedName = lib.escapeShellArg name;
    in
    if dnsCfg.nameserversFile != null then
      ''
        install -d -m 0711 /etc/netns
        install -d -m 0711 /etc/netns/${escapedName}
        set -f  # disable globbing — file content could contain * or ? characters
        : > /etc/netns/${escapedName}/resolv.conf
        while IFS= read -r line || [[ -n "$line" ]]; do
          # Split on commas/spaces into an array without globbing
          IFS=', ' read -ra entries <<< "$line"
          for d in "''${entries[@]}"; do
            if [[ -n "$d" ]]; then
              echo "nameserver $d" >> /etc/netns/${escapedName}/resolv.conf
            fi
          done
        done < ${dnsCfg.nameserversFile}
        set +f
        chmod 0644 /etc/netns/${escapedName}/resolv.conf
      ''
    else
      lib.optionalString (dnsCfg.nameservers != [ ]) ''
        install -d -m 0711 /etc/netns
        install -d -m 0711 /etc/netns/${escapedName}
        printf '%s\n' ${
          lib.escapeShellArgs (map (ns: "nameserver ${ns}") dnsCfg.nameservers)
        } > /etc/netns/${escapedName}/resolv.conf
        chmod 0644 /etc/netns/${escapedName}/resolv.conf
      '';

  mkNetnsStopScript =
    name: extraCmds:
    let
      escapedName = lib.escapeShellArg name;
    in
    pkgs.writeShellScript "cloister-netns-${name}-stop" ''
      set -euo pipefail

      if [[ ! -e /var/run/netns/${escapedName} ]]; then
        exit 0
      fi

      pids="$(ip netns pids ${escapedName} 2>/dev/null || true)"
      if [[ -n "$pids" ]]; then
        kill -TERM $pids 2>/dev/null || true
        for _ in $(seq 1 20); do
          alive=""
          for p in $pids; do
            if [[ -d "/proc/$p" ]]; then
              alive+=" $p"
            fi
          done
          if [[ -z "$alive" ]]; then
            break
          fi
          sleep 0.1
        done
        if [[ -n "$alive" ]]; then
          kill -KILL $alive 2>/dev/null || true
        fi
      fi

      ${extraCmds}
      ip netns del ${escapedName}
      rm -rf /etc/netns/${escapedName}
    '';

  mkWireguardService =
    name: netCfg:
    let
      wg = netCfg.wireguard;
      ifName = "wg-${name}";
      escapedName = lib.escapeShellArg name;
      escapedIfName = lib.escapeShellArg ifName;
      hasLiteralAddr = wg.address != [ ];
      hasIPv6 = hasLiteralAddr && builtins.any (addr: lib.hasInfix ":" addr) wg.address;

      peerCmds = lib.concatMapStringsSep "\n" (
        peer:
        let
          pubkeySetup =
            if peer.publicKeyFile != null then
              ''PUBKEY="$(tr -d '\n' < ${peer.publicKeyFile})"''
            else
              "PUBKEY=${lib.escapeShellArg peer.publicKey}";

          hasEndpoint = peer.endpoint != null || peer.endpointFile != null;
          endpointSetup =
            if peer.endpointFile != null then
              ''ENDPOINT="$(tr -d '\n' < ${peer.endpointFile})"''
            else if peer.endpoint != null then
              "ENDPOINT=${lib.escapeShellArg peer.endpoint}"
            else
              "";
        in
        ''
          ${pubkeySetup}
          WG_ARGS=(peer "$PUBKEY" allowed-ips 0.0.0.0/0,::/0)
          ${lib.optionalString hasEndpoint ''
            ${endpointSetup}
            WG_ARGS+=(endpoint "$ENDPOINT")
          ''}
          ${lib.optionalString (peer.presharedKeyFile != null) ''
            WG_ARGS+=(preshared-key ${peer.presharedKeyFile})
          ''}
          ${lib.optionalString (peer.persistentKeepalive != null) ''
            WG_ARGS+=(persistent-keepalive ${toString peer.persistentKeepalive})
          ''}
          ip netns exec ${escapedName} wg set ${escapedIfName} "''${WG_ARGS[@]}"
        ''
      ) wg.peers;

      addrCmds =
        if wg.addressFile != null then
          ''
            WG_ADDR="$(tr -d '\n' < ${wg.addressFile})"
            ip -n ${escapedName} addr add "$WG_ADDR" dev ${escapedIfName}
          ''
        else
          lib.concatMapStringsSep "\n" (
            addr: "ip -n ${escapedName} addr add ${addr} dev ${escapedIfName}"
          ) wg.address;

      ipv6RouteCmds =
        if wg.addressFile != null then
          ''
            if [[ "$WG_ADDR" == *:* ]]; then
              ip -n ${escapedName} -6 route add default dev ${escapedIfName}
            fi
          ''
        else
          lib.optionalString hasIPv6 "ip -n ${escapedName} -6 route add default dev ${escapedIfName}";

      mtuCmd = lib.optionalString (
        wg.mtu != null
      ) "ip -n ${escapedName} link set ${escapedIfName} mtu ${toString wg.mtu}";
    in
    {
      description = "Cloister WireGuard namespace: ${name}";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];
      path = [
        pkgs.coreutils
        pkgs.iproute2
        pkgs.wireguard-tools
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = pkgs.writeShellScript "cloister-netns-${name}-start" ''
          set -euo pipefail
          ip netns add ${escapedName}
          ip -n ${escapedName} link set lo up
          ip link add ${escapedIfName} type wireguard
          ip link set ${escapedIfName} netns ${escapedName}
          ip netns exec ${escapedName} wg set ${escapedIfName} private-key ${wg.privateKeyFile}
          ${peerCmds}
          ${addrCmds}
          ${mtuCmd}
          ip -n ${escapedName} link set ${escapedIfName} up
          ip -n ${escapedName} route add default dev ${escapedIfName}
          ${ipv6RouteCmds}
          ${mkResolvConf name netCfg.dns}
        '';
        ExecStop = mkNetnsStopScript name "";
      };
    };

  mkVethService =
    {
      name,
      netCfg,
      typeName,
      hostAddress,
      namespaceAddress,
      nftRules,
      sysctlKey,
    }:
    let
      escapedName = lib.escapeShellArg name;
      vethHost = "veth-${name}";
      vethNs = "veth-${name}-ns";
      escapedVethHost = lib.escapeShellArg vethHost;
      escapedVethNs = lib.escapeShellArg vethNs;
      hostIp = builtins.head (lib.splitString "/" hostAddress);
      nftRulesFile = pkgs.writeText "cloister-netns-${name}-nft" nftRules;
    in
    {
      description = "Cloister ${typeName} namespace: ${name}";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];
      path = [
        pkgs.coreutils
        pkgs.iproute2
        pkgs.nftables
        pkgs.procps
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = pkgs.writeShellScript "cloister-netns-${name}-start" ''
          set -euo pipefail
          ip netns add ${escapedName}
          ip -n ${escapedName} link set lo up
          ip link add ${escapedVethHost} type veth peer name ${escapedVethNs}
          ip link set ${escapedVethNs} netns ${escapedName}
          ip addr add ${hostAddress} dev ${escapedVethHost}
          ip -n ${escapedName} addr add ${namespaceAddress} dev ${escapedVethNs}
          ip link set ${escapedVethHost} up
          ip -n ${escapedName} link set ${escapedVethNs} up
          ip -n ${escapedName} route add default via ${hostIp}
          sysctl -w net.ipv4.conf.${escapedVethHost}.${sysctlKey}=1
          nft -f ${nftRulesFile}
          ${mkResolvConf name netCfg.dns}
        '';
        ExecStop = mkNetnsStopScript name ''
          nft delete table ip cloister-netns-${escapedName} || true
          sysctl -w net.ipv4.conf.${escapedVethHost}.${sysctlKey}=0 || true
        '';
      };
    };

  mkLocalhostService =
    name: netCfg:
    let
      inherit (netCfg) localhost;
      vethHost = "veth-${name}";
      portList = lib.concatMapStringsSep ", " toString localhost.allowedPorts;
    in
    mkVethService {
      inherit name netCfg;
      typeName = "localhost";
      inherit (localhost) hostAddress namespaceAddress;
      sysctlKey = "route_localnet";
      nftRules = ''
        table ip cloister-netns-${name} {
          chain prerouting {
            type nat hook prerouting priority dstnat; policy accept;
            iifname "${vethHost}" tcp dport { ${portList} } dnat to 127.0.0.1
            iifname "${vethHost}" udp dport { ${portList} } dnat to 127.0.0.1
          }
          chain forward {
            type filter hook forward priority filter; policy accept;
            iifname "${vethHost}" ct state established,related accept
            iifname "${vethHost}" tcp dport { ${portList} } accept
            iifname "${vethHost}" udp dport { ${portList} } accept
            iifname "${vethHost}" drop
          }
          chain input {
            type filter hook input priority filter; policy accept;
            iifname "${vethHost}" ct state established,related accept
            iifname "${vethHost}" drop
          }
        }
      '';
    };

  mkLanService =
    name: netCfg:
    let
      inherit (netCfg) lan;
      vethHost = "veth-${name}";
      rangeList = lib.concatStringsSep ", " lan.allowedRanges;
    in
    mkVethService {
      inherit name netCfg;
      typeName = "LAN";
      inherit (lan) hostAddress namespaceAddress;
      sysctlKey = "forwarding";
      nftRules = ''
        table ip cloister-netns-${name} {
          chain forward {
            type filter hook forward priority filter; policy accept;
            iifname "${vethHost}" ip daddr { ${rangeList} } accept
            oifname "${vethHost}" ct state established,related accept
            iifname "${vethHost}" drop
            oifname "${vethHost}" drop
          }
          chain input {
            type filter hook input priority filter; policy accept;
            iifname "${vethHost}" ct state established,related accept
            iifname "${vethHost}" drop
          }
          chain postrouting {
            type nat hook postrouting priority srcnat; policy accept;
            oifname != "${vethHost}" ip saddr ${lan.namespaceAddress} masquerade
          }
        }
      '';
    };

  mkIsolatedService =
    name:
    let
      escapedName = lib.escapeShellArg name;
    in
    {
      description = "Cloister isolated namespace: ${name}";
      wantedBy = [ "multi-user.target" ];
      path = [
        pkgs.coreutils
        pkgs.iproute2
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = pkgs.writeShellScript "cloister-netns-${name}-start" ''
          set -euo pipefail
          ip netns add ${escapedName}
          ip -n ${escapedName} link set lo up
        '';
        ExecStop = mkNetnsStopScript name "";
      };
    };

  # ── Partition networks by type ────────────────────────────────────────

  wireguardNets = lib.filterAttrs (_: net: net.wireguard != null) cfg.networks;
  localhostNets = lib.filterAttrs (_: net: net.localhost != null) cfg.networks;
  lanNets = lib.filterAttrs (_: net: net.lan != null) cfg.networks;
  isolatedNets = lib.filterAttrs (_: net: net.isolated) cfg.networks;

  wireguardServices = lib.mapAttrs' (
    name: net: lib.nameValuePair "cloister-netns-${name}" (mkWireguardService name net)
  ) wireguardNets;

  localhostServices = lib.mapAttrs' (
    name: net: lib.nameValuePair "cloister-netns-${name}" (mkLocalhostService name net)
  ) localhostNets;

  lanServices = lib.mapAttrs' (
    name: net: lib.nameValuePair "cloister-netns-${name}" (mkLanService name net)
  ) lanNets;

  isolatedServices = lib.mapAttrs' (
    name: _: lib.nameValuePair "cloister-netns-${name}" (mkIsolatedService name)
  ) isolatedNets;

  # ── Assertion helpers ─────────────────────────────────────────────────

  networkAssertions = lib.concatLists (
    lib.mapAttrsToList (
      name: net:
      let
        hasWg = net.wireguard != null;
        hasLocalhost = net.localhost != null;
        hasLan = net.lan != null;
        hasIsolated = net.isolated;
        typeCount = lib.count lib.id [
          hasWg
          hasLocalhost
          hasLan
          hasIsolated
        ];
        cidrPattern = "^((25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])\\.){3}(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])/(3[0-2]|[12][0-9]|[0-9])$";
        cidrV6Pattern = "^[0-9a-fA-F:]+/[0-9]{1,3}$";
        isCidr = addr: builtins.match cidrPattern addr != null || builtins.match cidrV6Pattern addr != null;
        ipv4Pattern = "^((25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])\\.){3}(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])$";
      in
      [
        {
          assertion = builtins.match "^[A-Za-z0-9_-]+$" name != null;
          message = "cloister-netns.networks.${name}: name must match ^[A-Za-z0-9_-]+$.";
        }
        {
          assertion = typeCount == 1;
          message = "cloister-netns.networks.${name}: exactly one of wireguard, localhost, lan, or isolated must be set.";
        }
        {
          assertion = !(net.dns.nameservers != [ ] && net.dns.nameserversFile != null);
          message = "cloister-netns.networks.${name}: nameservers and nameserversFile are mutually exclusive.";
        }
      ]
      ++ lib.optionals (net.dns.nameservers != [ ]) (
        let
          invalidNs = builtins.filter (ns: builtins.match ipv4Pattern ns == null) net.dns.nameservers;
        in
        [
          {
            assertion = invalidNs == [ ];
            message = "cloister-netns.networks.${name}: dns.nameservers contains invalid IPv4 addresses: ${lib.concatStringsSep ", " invalidNs}. Expected format: a.b.c.d (e.g. 1.1.1.1).";
          }
        ]
      )
      ++ lib.optionals hasWg (
        [
          {
            assertion = builtins.length net.wireguard.address > 0 || net.wireguard.addressFile != null;
            message = "cloister-netns.networks.${name}: wireguard requires at least one address or addressFile.";
          }
          {
            assertion = !(builtins.length net.wireguard.address > 0 && net.wireguard.addressFile != null);
            message = "cloister-netns.networks.${name}: address and addressFile are mutually exclusive.";
          }
          {
            assertion = builtins.length net.wireguard.peers > 0;
            message = "cloister-netns.networks.${name}: wireguard requires at least one peer.";
          }
          {
            assertion = lib.all (p: p.endpoint != null || p.endpointFile != null) net.wireguard.peers;
            message = "cloister-netns.networks.${name}: all wireguard peers must have an endpoint or endpointFile.";
          }
          {
            assertion = builtins.stringLength "wg-${name}" <= 15;
            message = "cloister-netns.networks.${name}: wireguard interface name 'wg-${name}' exceeds 15 character Linux limit.";
          }
        ]
        ++ (
          let
            invalidAddrs = builtins.filter (addr: !isCidr addr) net.wireguard.address;
          in
          [
            {
              assertion = invalidAddrs == [ ];
              message = "cloister-netns.networks.${name}: wireguard address entries must be in CIDR notation (e.g. '10.0.0.2/32' or 'fd00::1/128'): ${lib.concatStringsSep ", " invalidAddrs}";
            }
          ]
        )
        ++ lib.concatMap (peer: [
          {
            assertion = (peer.publicKey != null) != (peer.publicKeyFile != null);
            message = "cloister-netns.networks.${name}: exactly one of publicKey or publicKeyFile must be set per peer.";
          }
          {
            assertion = !(peer.endpoint != null && peer.endpointFile != null);
            message = "cloister-netns.networks.${name}: endpoint and endpointFile are mutually exclusive.";
          }
        ]) net.wireguard.peers
      )
      ++ lib.optionals (hasLocalhost || hasLan) [
        {
          assertion = builtins.stringLength "veth-${name}-ns" <= 15;
          message = "cloister-netns.networks.${name}: veth interface name 'veth-${name}-ns' exceeds 15 character Linux limit.";
        }
      ]
      ++ lib.optionals hasLocalhost [
        {
          assertion = builtins.match cidrPattern net.localhost.hostAddress != null;
          message = "cloister-netns.networks.${name}: localhost.hostAddress '${net.localhost.hostAddress}' is not valid CIDR notation. Expected format: a.b.c.d/prefix (e.g. 172.30.0.1/24).";
        }
        {
          assertion = builtins.match cidrPattern net.localhost.namespaceAddress != null;
          message = "cloister-netns.networks.${name}: localhost.namespaceAddress '${net.localhost.namespaceAddress}' is not valid CIDR notation. Expected format: a.b.c.d/prefix (e.g. 172.30.0.2/24).";
        }
      ]
      ++ lib.optionals hasLan (
        let
          invalidRanges = builtins.filter (r: builtins.match cidrPattern r == null) net.lan.allowedRanges;
        in
        [
          {
            assertion = builtins.match cidrPattern net.lan.hostAddress != null;
            message = "cloister-netns.networks.${name}: lan.hostAddress '${net.lan.hostAddress}' is not valid CIDR notation. Expected format: a.b.c.d/prefix (e.g. 172.29.0.1/24).";
          }
          {
            assertion = builtins.match cidrPattern net.lan.namespaceAddress != null;
            message = "cloister-netns.networks.${name}: lan.namespaceAddress '${net.lan.namespaceAddress}' is not valid CIDR notation. Expected format: a.b.c.d/prefix (e.g. 172.29.0.2/24).";
          }
          {
            assertion = builtins.length net.lan.allowedRanges > 0;
            message = "cloister-netns.networks.${name}: lan.allowedRanges must be non-empty.";
          }
          {
            assertion = invalidRanges == [ ];
            message = "cloister-netns.networks.${name}: lan.allowedRanges contains invalid CIDR notation: ${lib.concatStringsSep ", " invalidRanges}. Expected format: a.b.c.d/prefix (e.g. 10.0.0.0/8).";
          }
        ]
      )
      ++ lib.optionals hasIsolated [
        {
          assertion = net.dns.nameservers == [ ] && net.dns.nameserversFile == null;
          message = "cloister-netns.networks.${name}: isolated networks have no connectivity; DNS configuration is not applicable.";
        }
      ]
    ) cfg.networks
  );

  allVethAddresses =
    let
      localhostPairs = lib.mapAttrsToList (name: net: {
        inherit name;
        inherit (net.localhost) hostAddress namespaceAddress;
      }) localhostNets;
      lanPairs = lib.mapAttrsToList (name: net: {
        inherit name;
        inherit (net.lan) hostAddress namespaceAddress;
      }) lanNets;
    in
    localhostPairs ++ lanPairs;

  duplicateHostAddresses = lib.filterAttrs (_: v: builtins.length v > 1) (
    builtins.groupBy (x: x.hostAddress) allVethAddresses
  );
  duplicateNamespaceAddresses = lib.filterAttrs (_: v: builtins.length v > 1) (
    builtins.groupBy (x: x.namespaceAddress) allVethAddresses
  );

  expectedNsNotFound = lib.filter (
    ns: !builtins.elem ns effectiveAllowedNamespaces
  ) cfg.expectedNamespaces;
in
{
  options.cloister-netns = {
    enable = lib.mkEnableOption "capability wrapper for cloister-netns (cloister network namespace helper)";

    allowedNamespaces = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = ''
        List of network namespace names that unprivileged users are allowed to enter via the cloister-netns setcap helper.
        For security, this list defaults to empty and MUST be explicitly populated if the helper is enabled.
        The helper will reject any namespace name not exactly matching an entry in this list.
      '';
    };

    networks = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule networkSubmodule);
      default = { };
      description = ''
        Declarative network namespace definitions. Each attribute name becomes a
        namespace and a systemd service (cloister-netns-<name>). Namespace names
        are automatically added to allowedNamespaces.

        Each network must configure exactly one of `wireguard`, `localhost`, `lan`, or `isolated`.
      '';
    };

    expectedNamespaces = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = ''
        Namespace names expected by sandboxes (populated from sandbox configs).
        An assertion verifies each is in networks or allowedNamespaces.
      '';
    };

    enforceExecAllowlist = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Require cloister-netns to only exec binaries listed in allowedExecPaths.";
    };

    allowedExecPaths = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "${cloister-sandbox}/bin/cloister-sandbox" ];
      description = ''
        Executable paths that cloister-netns is allowed to exec.
        By default includes the compiled cloister-sandbox binary so that
        the Rust sandbox runner can be re-exec'd through the netns helper.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = builtins.length effectiveAllowedNamespaces > 0;
        message = ''
          cloister-netns is enabled but no namespaces are configured.
          Either set allowedNamespaces or define networks. For example:
            cloister-netns.allowedNamespaces = [ "vpn" ];
          or:
            cloister-netns.networks.vpn.wireguard = { ... };
        '';
      }
      {
        assertion = expectedNsNotFound == [ ];
        message = "cloister-netns: expectedNamespaces contains names not in networks or allowedNamespaces: ${lib.concatStringsSep ", " expectedNsNotFound}";
      }
    ]
    ++ networkAssertions
    ++ [
      {
        assertion = duplicateHostAddresses == { };
        message = "cloister-netns: duplicate host addresses across namespaces: ${
          lib.concatStringsSep "; " (
            lib.mapAttrsToList (
              addr: entries: "${addr} (used by: ${lib.concatMapStringsSep ", " (e: e.name) entries})"
            ) duplicateHostAddresses
          )
        }";
      }
      {
        assertion = duplicateNamespaceAddresses == { };
        message = "cloister-netns: duplicate namespace addresses across namespaces: ${
          lib.concatStringsSep "; " (
            lib.mapAttrsToList (
              addr: entries: "${addr} (used by: ${lib.concatMapStringsSep ", " (e: e.name) entries})"
            ) duplicateNamespaceAddresses
          )
        }";
      }
    ];

    security.wrappers.cloister-netns = {
      source = "${cloister-netns}/bin/cloister-netns";
      capabilities = "cap_sys_admin+ep";
      owner = "root";
      group = "root";
      setuid = false;
      setgid = false;
      permissions = "u+rx,g+x,o+x";
    };

    boot.kernel.sysctl = lib.mkIf (lanNets != { }) { "net.ipv4.ip_forward" = 1; };

    systemd.services = wireguardServices // localhostServices // lanServices // isolatedServices;
  };
}
