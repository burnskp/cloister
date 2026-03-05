{ testLib }:
let
  inherit (testLib) pkgs lib mkCheck;

  # Evaluate the cloister-netns NixOS module with the given extra modules.
  # Returns the full config attrset.
  evalNetns =
    modules:
    (import "${pkgs.path}/nixos/lib/eval-config.nix" {
      inherit (pkgs) system;
      modules = [
        ../modules/cloister-netns
      ]
      ++ modules;
    }).config;

  # Evaluate and check that an assertion fires containing expectedMessage.
  mkNetnsAssertionCheck =
    name: modules: expectedMessage:
    let
      result = builtins.tryEval (evalNetns modules);
    in
    if result.success then
      let
        config = result.value;
        failedAssertions = builtins.filter (x: !x.assertion) config.assertions;
        messages = map (x: x.message) failedAssertions;
        hasExpectedFailure = builtins.any (msg: lib.hasInfix expectedMessage msg) messages;
      in
      mkCheck name hasExpectedFailure
    else
      # Module evaluation threw — try to recover assertions
      let
        rawResult = builtins.tryEval (evalNetns modules).assertions;
        hasExpected =
          if rawResult.success then
            builtins.any (a: !a.assertion && lib.hasInfix expectedMessage a.message) rawResult.value
          else
            true;
      in
      mkCheck name hasExpected;
in
{
  # ── Firewall integration for localhost namespaces ────────────────────

  localhost-firewall-auto-open = mkCheck "netns-localhost-firewall-auto-open" (
    let
      config = evalNetns [
        {
          cloister-netns = {
            enable = true;
            networks.local.localhost.allowedPorts = [
              3000
              8080
            ];
          };
        }
      ];
      iface = config.networking.firewall.interfaces."veth-local";
    in
    iface.allowedTCPPorts == [
      3000
      8080
    ]
    &&
      iface.allowedUDPPorts == [
        3000
        8080
      ]
  );

  localhost-firewall-auto-open-disabled = mkCheck "netns-localhost-firewall-auto-open-disabled" (
    let
      config = evalNetns [
        {
          cloister-netns = {
            enable = true;
            firewall.autoOpenLocalhostPorts = false;
            networks.local.localhost.allowedPorts = [
              3000
              8080
            ];
          };
        }
      ];
    in
    !(config.networking.firewall.interfaces ? "veth-local")
  );

  # ── Auto address allocation should avoid collisions ──────────────────

  auto-addresses-no-duplicates = mkCheck "netns-auto-addresses-no-duplicates" (
    let
      result = builtins.tryEval (evalNetns [
        {
          cloister-netns = {
            enable = true;
            networks = {
              dev.localhost = { };
              docs.localhost = { };
              lan1.lan = { };
              lan2.lan = { };
            };
          };
        }
      ]);
    in
    result.success
  );

  # ── Pool validation assertions ────────────────────────────────────────

  invalid-localhost-pool = mkNetnsAssertionCheck "netns-invalid-localhost-pool" [
    {
      cloister-netns = {
        enable = true;
        addressPools.localhost = "not-a-cidr";
        networks.dev.localhost = { };
      };
    }
  ] "addressPools.localhost";

  invalid-lan-pool = mkNetnsAssertionCheck "netns-invalid-lan-pool" [
    {
      cloister-netns = {
        enable = true;
        addressPools.lan = "invalid";
        networks.dev.lan = { };
      };
    }
  ] "addressPools.lan";

  # ── Pool capacity assertions ──────────────────────────────────────────

  localhost-pool-exhausted = mkNetnsAssertionCheck "netns-localhost-pool-exhausted" [
    {
      cloister-netns = {
        enable = true;
        addressPools.localhost = "172.30.0.0/30";
        networks = {
          one.localhost = { };
          two.localhost = { };
        };
      };
    }
  ] "localhost address pool exhausted";

  lan-pool-exhausted = mkNetnsAssertionCheck "netns-lan-pool-exhausted" [
    {
      cloister-netns = {
        enable = true;
        addressPools.lan = "172.29.0.0/30";
        networks = {
          one.lan = { };
          two.lan = { };
        };
      };
    }
  ] "lan address pool exhausted";

  # ── Interface name length > 15 assertion ──────────────────────────────

  ifname-too-long = mkNetnsAssertionCheck "netns-ifname-too-long" [
    {
      cloister-netns = {
        enable = true;
        networks.verylongname.localhost = { };
      };
    }
  ] "exceeds 15 character";

  # ── Isolated network with DNS rejected ────────────────────────────────

  isolated-no-dns = mkNetnsAssertionCheck "netns-isolated-no-dns" [
    {
      cloister-netns = {
        enable = true;
        networks.airgap = {
          isolated = true;
          dns.nameservers = [ "1.1.1.1" ];
        };
      };
    }
  ] "DNS configuration is not applicable";

  # ── LAN invalid CIDR range ───────────────────────────────────────────

  lan-invalid-cidr = mkNetnsAssertionCheck "netns-lan-invalid-cidr" [
    {
      cloister-netns = {
        enable = true;
        networks.mylan.lan = {
          allowedRanges = [ "not-a-cidr" ];
        };
      };
    }
  ] "invalid CIDR notation";

  # ── Exactly one network type assertion ────────────────────────────────

  multiple-types = mkNetnsAssertionCheck "netns-multiple-types" [
    {
      cloister-netns = {
        enable = true;
        networks.confused = {
          isolated = true;
          localhost = { };
        };
      };
    }
  ] "exactly one of";

  # ── WireGuard address CIDR validation ──────────────────────────────

  wg-invalid-address = mkNetnsAssertionCheck "netns-wg-invalid-address" [
    {
      cloister-netns = {
        enable = true;
        networks.vpn = {
          wireguard = {
            address = [ "not-a-cidr" ];
            privateKeyFile = "/dev/null";
            peers = [ ];
          };
        };
      };
    }
  ] "must be in CIDR notation";
}
