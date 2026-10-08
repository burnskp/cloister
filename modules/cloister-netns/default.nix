{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.cloister-netns;

  effectiveAllowedNamespaces = lib.attrNames cfg.networks;

  isIpv4Address = value: builtins.match "^([0-9]{1,3}\.){3}[0-9]{1,3}$" value != null;

  isBareIpv6Address =
    value: lib.hasInfix ":" value && builtins.match "^[0-9A-Fa-f:.]+$" value != null;

  splitWireguardEndpoint =
    endpoint:
    let
      bracketedParts = lib.splitString "]:" endpoint;
      genericParts = lib.splitString ":" endpoint;
    in
    if lib.hasPrefix "[" endpoint && builtins.length bracketedParts == 2 then
      {
        host = lib.removePrefix "[" (builtins.elemAt bracketedParts 0);
        port = builtins.elemAt bracketedParts 1;
      }
    else if builtins.length genericParts == 2 then
      {
        host = builtins.elemAt genericParts 0;
        port = builtins.elemAt genericParts 1;
      }
    else
      null;

  endpointNeedsNetworkOnline =
    peer:
    if peer.endpointFile != null then
      true
    else if peer.endpoint == null then
      false
    else
      let
        parsed = splitWireguardEndpoint peer.endpoint;
      in
      if parsed == null then
        true
      else
        let
          inherit (parsed) host;
        in
        !(isIpv4Address host || isBareIpv6Address host);

  wireguardNeedsNetworkOnline = netCfg: builtins.any endpointNeedsNetworkOnline netCfg.peers;

  cloister-netns = pkgs.callPackage ../../helpers/cloister-netns {
    allowedNamespaces = effectiveAllowedNamespaces;
    requiredGroup = cfg.group;
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

  allPortRange = {
    from = 1;
    to = 65535;
  };

  defaultLanAllowedRanges = [
    "10.0.0.0/8"
    "172.16.0.0/12"
    "192.168.0.0/16"
  ];

  networkSubmodule = {
    options = {
      type = lib.mkOption {
        type = lib.types.enum [
          "wireguard"
          "localhost"
          "lan"
          "isolated"
        ];
        description = "Network namespace type.";
      };
      privateKeyFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to the WireGuard private key file. Required when type is `wireguard`.";
      };
      address = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = ''Addresses to assign to a WireGuard interface in CIDR notation (e.g. ["10.0.0.2/32"]). Mutually exclusive with addressFile.'';
      };
      addressFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to a file containing a WireGuard address in CIDR notation. Mutually exclusive with address.";
      };
      peers = lib.mkOption {
        type = lib.types.listOf (lib.types.submodule peerSubmodule);
        default = [ ];
        description = "WireGuard peer configurations.";
      };
      mtu = lib.mkOption {
        type = lib.types.nullOr lib.types.ints.positive;
        default = null;
        description = "Optional WireGuard interface MTU.";
      };
      allowedPorts = lib.mkOption {
        type = lib.types.nullOr (lib.types.listOf lib.types.port);
        default = null;
        description = "Host ports accessible from a localhost namespace via DNAT. Null allows all ports when type is `localhost`.";
      };
      allowedRanges = lib.mkOption {
        type = lib.types.nullOr (lib.types.listOf lib.types.str);
        default = null;
        description = "CIDR ranges a LAN namespace is allowed to reach. Defaults to RFC1918 ranges when type is `lan`.";
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

  # ── Address allocation helpers ────────────────────────────────────────

  cidrPattern = "^((25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])\\.){3}(25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9][0-9]|[0-9])/(3[0-2]|[12][0-9]|[0-9])$";
  intMod = x: y: x - (builtins.div x y) * y;
  pow2 = n: if n == 0 then 1 else 2 * pow2 (n - 1);

  ipToInt = octets: lib.foldl' (acc: oct: acc * 256 + oct) 0 octets;

  intToIp =
    ipInt:
    let
      o1 = builtins.div ipInt 16777216;
      r1 = intMod ipInt 16777216;
      o2 = builtins.div r1 65536;
      r2 = intMod r1 65536;
      o3 = builtins.div r2 256;
      o4 = intMod r2 256;
    in
    "${toString o1}.${toString o2}.${toString o3}.${toString o4}";

  parsePool =
    cidr:
    if builtins.match cidrPattern cidr == null then
      null
    else
      let
        parts = lib.splitString "/" cidr;
        ip = builtins.elemAt parts 0;
        prefix = builtins.fromJSON (builtins.elemAt parts 1);
        octets = map builtins.fromJSON (lib.splitString "." ip);
        subnetSize = pow2 (32 - prefix);
        ipInt = ipToInt octets;
        networkInt = ipInt - intMod ipInt subnetSize;
      in
      {
        inherit prefix networkInt;
        slotCount = builtins.div subnetSize 4;
      };

  pairForIndex =
    pool: idx:
    if pool == null || idx >= pool.slotCount then
      {
        hostAddress = "0.0.0.1/30";
        namespaceAddress = "0.0.0.2/30";
      }
    else
      let
        blockBase = pool.networkInt + idx * 4;
      in
      {
        hostAddress = "${intToIp (blockBase + 1)}/30";
        namespaceAddress = "${intToIp (blockBase + 2)}/30";
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
        done < ${lib.escapeShellArg dnsCfg.nameserversFile}
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

  mkHostsFile =
    name: hostInternalIp:
    let
      escapedName = lib.escapeShellArg name;
    in
    ''
      install -d -m 0711 /etc/netns
      install -d -m 0711 /etc/netns/${escapedName}
      if [[ -f /etc/hosts ]]; then
        cp /etc/hosts /etc/netns/${escapedName}/hosts
      else
        : > /etc/netns/${escapedName}/hosts
      fi
      ${lib.optionalString (hostInternalIp != null) ''
        echo "${hostInternalIp} host.internal" >> /etc/netns/${escapedName}/hosts
      ''}
      chmod 0644 /etc/netns/${escapedName}/hosts
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
      wg = netCfg;
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
            WG_ARGS+=(preshared-key ${lib.escapeShellArg peer.presharedKeyFile})
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
            WG_ADDR="$(tr -d '\n' < ${lib.escapeShellArg wg.addressFile})"
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
      stopScript = mkNetnsStopScript name "";
      needsNetworkOnline = wireguardNeedsNetworkOnline netCfg;
    in
    {
      description = "Cloister WireGuard namespace: ${name}";
      after = lib.optional needsNetworkOnline "network-online.target";
      wants = lib.optional needsNetworkOnline "network-online.target";
      wantedBy = [ "multi-user.target" ];
      path = [
        pkgs.coreutils
        pkgs.iproute2
        pkgs.wireguard-tools
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStartPre = "-${stopScript}";
        ExecStart = pkgs.writeShellScript "cloister-netns-${name}-start" ''
          set -euo pipefail
          ip netns add ${escapedName}
          ${mkResolvConf name netCfg.dns}
          ${mkHostsFile name null}
          ip -n ${escapedName} link set lo up
          ip link add ${escapedIfName} type wireguard
          ip link set ${escapedIfName} netns ${escapedName}
          ip netns exec ${escapedName} wg set ${escapedIfName} private-key ${lib.escapeShellArg wg.privateKeyFile}
          ${peerCmds}
          ${addrCmds}
          ${mtuCmd}
          ip -n ${escapedName} link set ${escapedIfName} up
          ip -n ${escapedName} route add default dev ${escapedIfName}
          ${ipv6RouteCmds}
        '';
        ExecStop = stopScript;
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
      hostInternalIp ? null,
    }:
    let
      escapedName = lib.escapeShellArg name;
      vethHost = "veth-${name}";
      vethNs = "veth-${name}-ns";
      escapedVethHost = lib.escapeShellArg vethHost;
      escapedVethNs = lib.escapeShellArg vethNs;
      hostIp = builtins.head (lib.splitString "/" hostAddress);
      nftRulesFile = pkgs.writeText "cloister-netns-${name}-nft" nftRules;
      stopScript = mkNetnsStopScript name ''
        nft delete table ip cloister-netns-${escapedName} || true
        nft delete table ip6 cloister-netns-${escapedName} || true
        sysctl -w net.ipv4.conf.${escapedVethHost}.${sysctlKey}=0 || true
        sysctl -w net.ipv6.conf.${escapedVethHost}.disable_ipv6=0 || true
      '';
    in
    {
      description = "Cloister ${typeName} namespace: ${name}";
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
        ExecStartPre = "-${stopScript}";
        ExecStart = pkgs.writeShellScript "cloister-netns-${name}-start" ''
          set -euo pipefail
          ip netns add ${escapedName}
          ip -n ${escapedName} link set lo up
          ip link add ${escapedVethHost} type veth peer name ${escapedVethNs}
          ip link set ${escapedVethNs} netns ${escapedName}
          sysctl -w net.ipv6.conf.${escapedVethHost}.disable_ipv6=1
          ip netns exec ${escapedName} sysctl -w net.ipv6.conf.${escapedVethNs}.disable_ipv6=1
          ip addr add ${hostAddress} dev ${escapedVethHost}
          ip -n ${escapedName} addr add ${namespaceAddress} dev ${escapedVethNs}
          ip link set ${escapedVethHost} up
          ip -n ${escapedName} link set ${escapedVethNs} up
          ip -n ${escapedName} route add default via ${hostIp}
          sysctl -w net.ipv4.conf.${escapedVethHost}.${sysctlKey}=1
          nft -f ${nftRulesFile}
          ${mkResolvConf name netCfg.dns}
          ${mkHostsFile name hostInternalIp}
        '';
        ExecStop = stopScript;
      };
    };

  mkLocalhostService =
    name: netCfg:
    let
      inherit (netCfg) allowedPorts hostAddress namespaceAddress;
      vethHost = "veth-${name}";
      allowsAllPorts = allowedPorts == null;
      portList = lib.optionalString (!allowsAllPorts) (
        lib.concatMapStringsSep ", " toString allowedPorts
      );
      protoMatch =
        proto: if allowsAllPorts then "meta l4proto ${proto}" else "${proto} dport { ${portList} }";
    in
    mkVethService {
      inherit
        name
        netCfg
        hostAddress
        namespaceAddress
        ;
      typeName = "localhost";
      sysctlKey = "route_localnet";
      hostInternalIp = builtins.head (lib.splitString "/" hostAddress);
      nftRules = ''
        table ip cloister-netns-${name} {
          chain prerouting {
            type nat hook prerouting priority dstnat; policy accept;
            iifname "${vethHost}" ${protoMatch "tcp"} dnat to 127.0.0.1
           iifname "${vethHost}" ${protoMatch "udp"} dnat to 127.0.0.1
         }
         chain forward {
           type filter hook forward priority filter; policy accept;
           iifname "${vethHost}" ct state established,related accept
           iifname "${vethHost}" ${protoMatch "tcp"} accept
           iifname "${vethHost}" ${protoMatch "udp"} accept
           iifname "${vethHost}" drop
         }
         chain input {
           type filter hook input priority filter; policy accept;
           iifname "${vethHost}" ct state established,related accept
           iifname "${vethHost}" ${protoMatch "tcp"} accept
           iifname "${vethHost}" ${protoMatch "udp"} accept
            iifname "${vethHost}" drop
          }
        }
        table ip6 cloister-netns-${name} {
          chain forward {
            type filter hook forward priority filter; policy accept;
            iifname "${vethHost}" drop
            oifname "${vethHost}" drop
          }
          chain input {
            type filter hook input priority filter; policy accept;
            iifname "${vethHost}" drop
          }
        }
      '';
    };

  mkLanService =
    name: netCfg:
    let
      inherit (netCfg) hostAddress namespaceAddress;
      allowedRanges =
        if netCfg.allowedRanges == null then defaultLanAllowedRanges else netCfg.allowedRanges;
      vethHost = "veth-${name}";
      rangeList = lib.concatStringsSep ", " allowedRanges;
    in
    mkVethService {
      inherit
        name
        netCfg
        hostAddress
        namespaceAddress
        ;
      typeName = "LAN";
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
            oifname != "${vethHost}" ip saddr ${namespaceAddress} masquerade
          }
        }
        table ip6 cloister-netns-${name} {
          chain forward {
            type filter hook forward priority filter; policy accept;
            iifname "${vethHost}" drop
            oifname "${vethHost}" drop
          }
          chain input {
            type filter hook input priority filter; policy accept;
            iifname "${vethHost}" drop
          }
        }
      '';
    };

  mkIsolatedService =
    name:
    let
      escapedName = lib.escapeShellArg name;
      stopScript = mkNetnsStopScript name "";
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
        ExecStartPre = "-${stopScript}";
        ExecStart = pkgs.writeShellScript "cloister-netns-${name}-start" ''
          set -euo pipefail
          ip netns add ${escapedName}
          ip -n ${escapedName} link set lo up
          ${mkHostsFile name null}
        '';
        ExecStop = stopScript;
      };
    };

  # ── Partition networks by type ────────────────────────────────────────

  wireguardNets = lib.filterAttrs (
    _: net: net.type == "wireguard" && net.privateKeyFile != null
  ) cfg.networks;
  localhostNets = lib.filterAttrs (_: net: net.type == "localhost") cfg.networks;
  lanNets = lib.filterAttrs (_: net: net.type == "lan") cfg.networks;
  isolatedNets = lib.filterAttrs (_: net: net.type == "isolated") cfg.networks;

  vethNames = lib.sort builtins.lessThan (lib.attrNames (localhostNets // lanNets));

  vethPool = parsePool cfg.veth.addressPool;

  effectiveVethAddressPairs =
    if vethPool == null then
      { }
    else
      builtins.listToAttrs (
        lib.imap0 (idx: name: {
          inherit name;
          value = pairForIndex vethPool idx;
        }) vethNames
      );

  wireguardServices = lib.mapAttrs' (
    name: net: lib.nameValuePair "cloister-netns-${name}" (mkWireguardService name net)
  ) wireguardNets;

  localhostServices = lib.mapAttrs' (
    name: net:
    lib.nameValuePair "cloister-netns-${name}" (
      mkLocalhostService name (net // effectiveVethAddressPairs.${name})
    )
  ) localhostNets;

  lanServices = lib.mapAttrs' (
    name: net:
    lib.nameValuePair "cloister-netns-${name}" (
      mkLanService name (net // effectiveVethAddressPairs.${name})
    )
  ) lanNets;

  isolatedServices = lib.mapAttrs' (
    name: _: lib.nameValuePair "cloister-netns-${name}" (mkIsolatedService name)
  ) isolatedNets;

  localhostFirewallInterfaces = lib.mapAttrs' (
    name: net:
    lib.nameValuePair "veth-${name}" (
      if net.allowedPorts == null then
        {
          allowedTCPPortRanges = lib.mkDefault [ allPortRange ];
          allowedUDPPortRanges = lib.mkDefault [ allPortRange ];
        }
      else
        {
          allowedTCPPorts = lib.mkDefault net.allowedPorts;
          allowedUDPPorts = lib.mkDefault net.allowedPorts;
        }
    )
  ) localhostNets;

  # ── Assertion helpers ─────────────────────────────────────────────────

  networkAssertions = lib.concatLists (
    lib.mapAttrsToList (
      name: net:
      let
        hasWg = net.type == "wireguard";
        hasLocalhost = net.type == "localhost";
        hasLan = net.type == "lan";
        hasIsolated = net.type == "isolated";
        hasWireguardOptions =
          net.privateKeyFile != null
          || net.address != [ ]
          || net.addressFile != null
          || net.peers != [ ]
          || net.mtu != null;
        effectiveAllowedRanges =
          if net.allowedRanges == null then defaultLanAllowedRanges else net.allowedRanges;
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
            assertion = net.privateKeyFile != null;
            message = "cloister-netns.networks.${name}: wireguard requires privateKeyFile.";
          }
          {
            assertion = builtins.length net.address > 0 || net.addressFile != null;
            message = "cloister-netns.networks.${name}: wireguard requires at least one address or addressFile.";
          }
          {
            assertion = !(builtins.length net.address > 0 && net.addressFile != null);
            message = "cloister-netns.networks.${name}: address and addressFile are mutually exclusive.";
          }
          {
            assertion = builtins.length net.peers > 0;
            message = "cloister-netns.networks.${name}: wireguard requires at least one peer.";
          }
          {
            assertion = lib.all (p: p.endpoint != null || p.endpointFile != null) net.peers;
            message = "cloister-netns.networks.${name}: all wireguard peers must have an endpoint or endpointFile.";
          }
          {
            assertion = builtins.stringLength "wg-${name}" <= 15;
            message = "cloister-netns.networks.${name}: wireguard interface name 'wg-${name}' exceeds 15 character Linux limit.";
          }
        ]
        ++ (
          let
            invalidAddrs = builtins.filter (addr: !isCidr addr) net.address;
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
        ]) net.peers
      )
      ++ lib.optionals (hasLocalhost || hasLan) [
        {
          assertion = builtins.stringLength "veth-${name}-ns" <= 15;
          message = "cloister-netns.networks.${name}: veth interface name 'veth-${name}-ns' exceeds 15 character Linux limit.";
        }
      ]
      ++ lib.optionals hasLocalhost [
        {
          assertion = net.allowedPorts == null || builtins.length net.allowedPorts > 0;
          message = "cloister-netns.networks.${name}: allowedPorts must be non-empty when set; omit it or set it to null to allow all ports.";
        }
      ]
      ++ lib.optionals hasLan (
        let
          invalidRanges = builtins.filter (r: builtins.match cidrPattern r == null) effectiveAllowedRanges;
        in
        [
          {
            assertion = builtins.length effectiveAllowedRanges > 0;
            message = "cloister-netns.networks.${name}: allowedRanges must be non-empty.";
          }
          {
            assertion = invalidRanges == [ ];
            message = "cloister-netns.networks.${name}: allowedRanges contains invalid CIDR notation: ${lib.concatStringsSep ", " invalidRanges}. Expected format: a.b.c.d/prefix (e.g. 10.0.0.0/8).";
          }
        ]
      )
      ++ lib.optionals (!hasWg) [
        {
          assertion = !hasWireguardOptions;
          message = "cloister-netns.networks.${name}: WireGuard options are only valid when type is `wireguard`.";
        }
      ]
      ++ lib.optionals (!hasLocalhost) [
        {
          assertion = net.allowedPorts == null;
          message = "cloister-netns.networks.${name}: allowedPorts is only valid when type is `localhost`.";
        }
      ]
      ++ lib.optionals (!hasLan) [
        {
          assertion = net.allowedRanges == null;
          message = "cloister-netns.networks.${name}: allowedRanges is only valid when type is `lan`.";
        }
      ]
      ++ lib.optionals hasIsolated [
        {
          assertion = net.dns.nameservers == [ ] && net.dns.nameserversFile == null;
          message = "cloister-netns.networks.${name}: isolated networks have no connectivity; DNS configuration is not applicable.";
        }
      ]
    ) cfg.networks
  );

  allVethAddresses =
    if vethPool == null then
      [ ]
    else
      lib.mapAttrsToList (name: _: {
        inherit name;
        inherit (effectiveVethAddressPairs.${name}) hostAddress namespaceAddress;
      }) (localhostNets // lanNets);

  duplicateHostAddresses = lib.filterAttrs (_: v: builtins.length v > 1) (
    builtins.groupBy (x: x.hostAddress) allVethAddresses
  );
  duplicateNamespaceAddresses = lib.filterAttrs (_: v: builtins.length v > 1) (
    builtins.groupBy (x: x.namespaceAddress) allVethAddresses
  );

  enabled = cfg.networks != { };
in
{
  options.cloister-netns = {
    networks = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule networkSubmodule);
      default = { };
      description = ''
        Declarative network namespace definitions. Each attribute name becomes a
        namespace and a systemd service (cloister-netns-<name>). Namespace names
        are automatically added to the helper allowlist.

        Each network must set `type` to one of `wireguard`, `localhost`, `lan`, or `isolated`.
      '';
    };

    veth.addressPool = lib.mkOption {
      type = lib.types.str;
      default = "172.29.0.0/16";
      description = "CIDR pool for auto-assigning veth addresses for localhost and LAN namespaces (/30 blocks by sorted namespace index).";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "cloister-netns";
      description = ''
        Unix group allowed to execute the `cloister-netns` capability wrapper.
        Add trusted users to this group with `users.users.<name>.extraGroups`.
      '';
    };

  };

  config = lib.mkIf enabled {
    assertions = [
      {
        assertion = vethPool != null;
        message = "cloister-netns.veth.addressPool must be valid IPv4 CIDR notation (e.g. 172.29.0.0/16).";
      }
      {
        assertion = vethPool == null || vethPool.prefix <= 30;
        message = "cloister-netns.veth.addressPool prefix must be <= 30 to allocate /30 pairs.";
      }
      {
        assertion = vethPool == null || builtins.length vethNames <= vethPool.slotCount;
        message = "cloister-netns: veth address pool exhausted for configured namespaces.";
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

    users.groups = lib.mkIf (cfg.group != "root") (lib.setAttrByPath [ cfg.group ] { });

    security.wrappers.cloister-netns = {
      source = "${cloister-netns}/bin/cloister-netns";
      capabilities = "cap_sys_admin+ep";
      owner = "root";
      inherit (cfg) group;
      setuid = false;
      setgid = false;
      permissions = "u+rx,g+rx,o-rwx";
    };

    boot.kernel.sysctl = lib.mkIf (lanNets != { }) { "net.ipv4.ip_forward" = 1; };

    networking.firewall.interfaces = localhostFirewallInterfaces;

    systemd.services = wireguardServices // localhostServices // lanServices // isolatedServices;
  };
}
