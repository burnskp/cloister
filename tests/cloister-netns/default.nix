{
  checks,
  lib,
  nixos,
  pkgs,
  ...
}:
let
  expectOrdered =
    label: first: second: text:
    let
      parts = lib.splitString first text;
      afterFirst = if builtins.length parts > 1 then lib.concatStringsSep first (lib.tail parts) else "";
    in
    if afterFirst != "" && lib.hasInfix second afterFirst then
      true
    else
      throw "${label}: expected ${builtins.toJSON first} to appear before ${builtins.toJSON second}";

  eval = nixos.netns {
    cloister-netns = {
      networks = {
        dev = {
          type = "localhost";
          allowedPorts = [ 3000 ];
        };
        lan = {
          type = "lan";
          allowedRanges = [ "192.168.1.0/24" ];
        };
      };
    };
  };

  customGroup = nixos.netns {
    cloister-netns = {
      group = "sandboxers";
      networks.vpn.type = "isolated";
    };
  };

  invalidVethPool = nixos.netns {
    cloister-netns = {
      networks.vpn.type = "isolated";
      veth.addressPool = "bad";
    };
  };

  vethPoolExhausted = nixos.netns {
    cloister-netns = {
      veth.addressPool = "172.29.0.0/30";
      networks = {
        a.type = "localhost";
        b.type = "localhost";
      };
    };
  };

  invalidLanRange = nixos.netns {
    cloister-netns = {
      networks.lan = {
        type = "lan";
        allowedRanges = [ "not-a-cidr" ];
      };
    };
  };

  emptyLocalhostPorts = nixos.netns {
    cloister-netns = {
      networks.dev = {
        type = "localhost";
        allowedPorts = [ ];
      };
    };
  };

  ifnameTooLong = nixos.netns {
    cloister-netns = {
      networks.abcdefghijkl.type = "localhost";
    };
  };

  isolatedNoDns = nixos.netns {
    cloister-netns = {
      networks.offline = {
        type = "isolated";
        dns.nameservers = [ "1.1.1.1" ];
      };
    };
  };

  missingWireguardConfig = nixos.netns {
    cloister-netns = {
      networks.missing-wg.type = "wireguard";
    };
  };

  wgInvalidAddress = nixos.netns {
    cloister-netns = {
      networks.vpn = {
        type = "wireguard";
        privateKeyFile = pkgs.writeText "wg-private-key" "private";
        address = [ "bad-address" ];
        peers = [
          {
            publicKey = "public";
            endpoint = "vpn.example:51820";
          }
        ];
      };
    };
  };

  invalidVethPoolPrefix = nixos.netns {
    cloister-netns = {
      networks.vpn.type = "isolated";
      veth.addressPool = "172.29.0.0/31";
    };
  };

  orderedPairsEval = nixos.netns {
    cloister-netns = {
      networks = {
        zeta.type = "localhost";
        alpha.type = "localhost";
        gamma.type = "lan";
        beta.type = "lan";
      };
    };
  };

  dnsFile = pkgs.writeText "cloister-netns-dns" ''
    1.1.1.1, 8.8.8.8
    9.9.9.9
  '';

  wgAddressFile = pkgs.writeText "cloister-netns-wg-address" "10.23.0.2/32\n";
  wgPublicKeyFile = pkgs.writeText "cloister-netns-wg-pub" "public-from-file\n";
  wgEndpointFile = pkgs.writeText "cloister-netns-wg-endpoint" "vpn.example:51820\n";

  fileBackedInputsEval = nixos.netns {
    cloister-netns = {
      networks = {
        vpn = {
          type = "wireguard";
          privateKeyFile = pkgs.writeText "wg-private-key-file-backed" "private\n";
          addressFile = wgAddressFile;
          mtu = 1420;
          peers = [
            {
              publicKeyFile = wgPublicKeyFile;
              endpointFile = wgEndpointFile;
              persistentKeepalive = 21;
            }
          ];
          dns.nameserversFile = dnsFile;
        };
        dev = {
          type = "localhost";
          allowedPorts = [ 4000 ];
        };
      };
    };
  };

  isolatedEval = nixos.netns {
    cloister-netns = {
      networks.offline.type = "isolated";
    };
  };

  wgAutoHostnameEval = nixos.netns {
    cloister-netns = {
      networks.vpn = {
        type = "wireguard";
        privateKeyFile = pkgs.writeText "wg-private-key-auto-host" "private\n";
        address = [ "10.42.0.2/32" ];
        peers = [
          {
            publicKey = "public";
            endpoint = "vpn.example:51820";
          }
        ];
      };
    };
  };

  wgAutoIpEval = nixos.netns {
    cloister-netns = {
      networks.vpn = {
        type = "wireguard";
        privateKeyFile = pkgs.writeText "wg-private-key-auto-ip" "private\n";
        address = [ "10.43.0.2/32" ];
        peers = [
          {
            publicKey = "public";
            endpoint = "203.0.113.10:51820";
          }
        ];
      };
    };
  };

  dnsConflict = nixos.netns {
    cloister-netns = {
      networks.vpn = {
        type = "wireguard";
        privateKeyFile = pkgs.writeText "wg-private-key-dns-conflict" "private\n";
        address = [ "10.0.0.2/32" ];
        peers = [
          {
            publicKey = "public";
            endpoint = "vpn.example:51820";
          }
        ];
        dns = {
          nameservers = [ "1.1.1.1" ];
          nameserversFile = dnsFile;
        };
      };
    };
  };

  wgAddressConflict = nixos.netns {
    cloister-netns = {
      networks.vpn = {
        type = "wireguard";
        privateKeyFile = pkgs.writeText "wg-private-key-address-conflict" "private\n";
        address = [ "10.0.0.2/32" ];
        addressFile = wgAddressFile;
        peers = [
          {
            publicKey = "public";
            endpoint = "vpn.example:51820";
          }
        ];
      };
    };
  };

  peerKeyConflict = nixos.netns {
    cloister-netns = {
      networks.vpn = {
        type = "wireguard";
        privateKeyFile = pkgs.writeText "wg-private-key-peer-conflict" "private\n";
        address = [ "10.0.0.2/32" ];
        peers = [
          {
            publicKey = "public";
            publicKeyFile = wgPublicKeyFile;
            endpoint = "vpn.example:51820";
          }
        ];
      };
    };
  };

  peerEndpointConflict = nixos.netns {
    cloister-netns = {
      networks.vpn = {
        type = "wireguard";
        privateKeyFile = pkgs.writeText "wg-private-key-endpoint-conflict" "private\n";
        address = [ "10.0.0.2/32" ];
        peers = [
          {
            publicKey = "public";
            endpoint = "vpn.example:51820";
            endpointFile = wgEndpointFile;
          }
        ];
      };
    };
  };

  nftRulesFromStart =
    start:
    let
      afterNft = builtins.elemAt (lib.splitString "nft -f " start) 1;
      path = builtins.head (lib.splitString "\n" afterNft);
    in
    builtins.readFile path;

  orderedPairsServices = orderedPairsEval.config.systemd.services;
  orderedAlphaStart =
    builtins.readFile
      orderedPairsServices."cloister-netns-alpha".serviceConfig.ExecStart;
  orderedZetaStart =
    builtins.readFile
      orderedPairsServices."cloister-netns-zeta".serviceConfig.ExecStart;
  orderedBetaStart =
    builtins.readFile
      orderedPairsServices."cloister-netns-beta".serviceConfig.ExecStart;
  orderedGammaStart =
    builtins.readFile
      orderedPairsServices."cloister-netns-gamma".serviceConfig.ExecStart;
  evalServices = eval.config.systemd.services;
  evalDevStart = builtins.readFile evalServices."cloister-netns-dev".serviceConfig.ExecStart;
  evalDevNft = nftRulesFromStart evalDevStart;
  evalLanStart = builtins.readFile evalServices."cloister-netns-lan".serviceConfig.ExecStart;
  fileBackedServices = fileBackedInputsEval.config.systemd.services;
  fileBackedVpnStart =
    builtins.readFile
      fileBackedServices."cloister-netns-vpn".serviceConfig.ExecStart;
  isolatedService = isolatedEval.config.systemd.services."cloister-netns-offline";
  autoHostnameService = wgAutoHostnameEval.config.systemd.services."cloister-netns-vpn";
  autoIpService = wgAutoIpEval.config.systemd.services."cloister-netns-vpn";
  orderedAlphaNft = nftRulesFromStart orderedAlphaStart;
in
checks.mkCheck "test-cloister-netns" [
  (checks.expectTrue "netns wrapper configured" (eval.config.security.wrappers ? "cloister-netns"))
  (checks.expectTrue "localhost service exists" (eval.config.systemd.services ? "cloister-netns-dev"))
  (checks.expectTrue "lan service exists" (eval.config.systemd.services ? "cloister-netns-lan"))
  (checks.expectEq "netns wrapper uses restrictive permissions" "u+rx,g+rx,o-rwx"
    eval.config.security.wrappers.cloister-netns.permissions
  )
  (checks.expectEq "netns wrapper uses default authorization group" "cloister-netns"
    eval.config.security.wrappers.cloister-netns.group
  )
  (checks.expectEq "firewall interface opened for localhost ports" [
    3000
  ] eval.config.networking.firewall.interfaces."veth-dev".allowedTCPPorts)
  (checks.expectEq "firewall interface opens UDP localhost ports too" [
    3000
  ] eval.config.networking.firewall.interfaces."veth-dev".allowedUDPPorts)
  (checks.expectEq "default localhost firewall opens all TCP ports" [
    {
      from = 1;
      to = 65535;
    }
  ] orderedPairsEval.config.networking.firewall.interfaces."veth-alpha".allowedTCPPortRanges)
  (checks.expectEq "lan enables ip forwarding" 1 eval.config.boot.kernel.sysctl."net.ipv4.ip_forward")
  (checks.expectContains "ordered localhost host address allocation is deterministic"
    "ip addr add 172.29.0.1/30 dev veth-alpha"
    orderedAlphaStart
  )
  (checks.expectContains "ordered localhost next slot is deterministic"
    "ip addr add 172.29.0.13/30 dev veth-zeta"
    orderedZetaStart
  )
  (checks.expectContains "ordered lan host address allocation is deterministic"
    "ip addr add 172.29.0.5/30 dev veth-beta"
    orderedBetaStart
  )
  (checks.expectContains "ordered lan next slot is deterministic"
    "ip addr add 172.29.0.9/30 dev veth-gamma"
    orderedGammaStart
  )
  (checks.expectContains "localhost disables host-side IPv6 on the veth"
    "sysctl -w net.ipv6.conf.veth-dev.disable_ipv6=1"
    evalDevStart
  )
  (checks.expectContains "localhost applies a generated nft ruleset" "nft -f /nix/store/"
    evalDevStart
  )
  (checks.expectContains "lan applies a generated nft ruleset" "nft -f /nix/store/" evalLanStart)
  (checks.expectContains "localhost all-port nft rules use a protocol match"
    ''iifname "veth-alpha" meta l4proto tcp dnat to 127.0.0.1''
    orderedAlphaNft
  )
  (checks.expectContains "localhost restricted nft rules keep port match"
    ''iifname "veth-dev" tcp dport { 3000 } dnat to 127.0.0.1''
    evalDevNft
  )
  (checks.expectEq "localhost service skips network-online target in auto mode" [ ] (
    evalServices."cloister-netns-dev".after or [ ]
  ))
  (checks.expectEq "lan service skips network-online target in auto mode" [ ] (
    evalServices."cloister-netns-lan".after or [ ]
  ))
  (checks.expectEq "isolated service skips network-online target" [ ] (isolatedService.after or [ ]))
  (checks.expectEq "wireguard hostname endpoint waits for network-online in auto mode" [
    "network-online.target"
  ] (autoHostnameService.after or [ ]))
  (checks.expectEq "wireguard endpointFile waits for network-online in auto mode" [
    "network-online.target"
  ] (fileBackedServices."cloister-netns-vpn".after or [ ]))
  (checks.expectEq "wireguard literal IP endpoint skips network-online in auto mode" [ ] (
    autoIpService.after or [ ]
  ))
  (checks.expectEq "custom authorization group is honored" "sandboxers"
    customGroup.config.security.wrappers.cloister-netns.group
  )
  (checks.expectTrue "custom authorization group is declared" (
    customGroup.config.users.groups ? "sandboxers"
  ))
  (checks.expectTrue "localhost firewall integration is always enabled" (
    fileBackedInputsEval.config.networking.firewall.interfaces ? "veth-dev"
  ))
  (checks.expectContains "nameserversFile content is rendered into service" ''echo "nameserver $d"''
    fileBackedVpnStart
  )
  (checks.expectContains "wireguard addressFile is consumed in service"
    "WG_ADDR=\"$(tr -d '\\n' < /nix/store/"
    fileBackedVpnStart
  )
  (checks.expectContains "wireguard publicKeyFile is consumed in service"
    "PUBKEY=\"$(tr -d '\\n' < /nix/store/"
    fileBackedVpnStart
  )
  (checks.expectContains "wireguard endpointFile is consumed in service"
    "ENDPOINT=\"$(tr -d '\\n' < /nix/store/"
    fileBackedVpnStart
  )
  (expectOrdered "wireguard resolv.conf is created before endpoint configuration"
    "install -d -m 0711 /etc/netns"
    "wg set wg-vpn private-key"
    fileBackedVpnStart
  )
  (expectOrdered "wireguard endpoint setup happens after namespace DNS exists"
    "/etc/netns/vpn/resolv.conf"
    ''wg set wg-vpn "''${WG_ARGS[@]}"''
    fileBackedVpnStart
  )
  (checks.expectContains "wireguard persistentKeepalive is rendered" "persistent-keepalive 21"
    fileBackedVpnStart
  )
  (checks.expectContains "wireguard mtu is rendered" "link set wg-vpn mtu 1420" fileBackedVpnStart)
  (checks.expectAssertionMessage "invalid veth pool is rejected" invalidVethPool.assertions
    "cloister-netns.veth.addressPool must be valid IPv4 CIDR notation"
  )
  (checks.expectAssertionMessage "veth pool exhaustion is rejected" vethPoolExhausted.assertions
    "veth address pool exhausted"
  )
  (checks.expectAssertionMessage "veth pool prefix must allow /30 allocation"
    invalidVethPoolPrefix.assertions
    "cloister-netns.veth.addressPool prefix must be <= 30"
  )
  (checks.expectAssertionMessage "invalid lan CIDR is rejected" invalidLanRange.assertions
    "allowedRanges contains invalid CIDR notation"
  )
  (checks.expectAssertionMessage "empty localhost port list is rejected"
    emptyLocalhostPorts.assertions
    "allowedPorts must be non-empty when set"
  )
  (checks.expectAssertionMessage "veth interface names must stay short" ifnameTooLong.assertions
    "veth interface name"
  )
  (checks.expectAssertionMessage "isolated networks reject DNS" isolatedNoDns.assertions
    "isolated networks have no connectivity"
  )
  (checks.expectAssertionMessage "wireguard type requires private key"
    missingWireguardConfig.assertions
    "wireguard requires privateKeyFile"
  )
  (checks.expectAssertionMessage "wireguard addresses must use CIDR" wgInvalidAddress.assertions
    "wireguard address entries must be in CIDR notation"
  )
  (checks.expectAssertionMessage "dns nameservers and file are mutually exclusive"
    dnsConflict.assertions
    "nameservers and nameserversFile are mutually exclusive"
  )
  (checks.expectAssertionMessage "wireguard address and addressFile are mutually exclusive"
    wgAddressConflict.assertions
    "address and addressFile are mutually exclusive"
  )
  (checks.expectAssertionMessage "wireguard peer public key sources are exclusive"
    peerKeyConflict.assertions
    "exactly one of publicKey or publicKeyFile"
  )
  (checks.expectAssertionMessage "wireguard peer endpoint sources are exclusive"
    peerEndpointConflict.assertions
    "endpoint and endpointFile are mutually exclusive"
  )
]
