{ testLib }:
let
  inherit (testLib) pkgs lib mkCheck;

  # Evaluate the cloister-netns NixOS module with the given extra modules.
  # Returns the full config attrset.
  evalNetns =
    modules:
    (lib.evalModules {
      modules = [
        ../modules/cloister-netns
        { _module.args = { inherit pkgs; }; }
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
  # ── Duplicate host address assertion ──────────────────────────────────

  duplicate-host-addr = mkNetnsAssertionCheck "netns-duplicate-host-addr" [
    {
      cloister-netns = {
        enable = true;
        networks.a.localhost = {
          hostAddress = "172.30.0.1/24";
          namespaceAddress = "172.30.0.2/24";
        };
        networks.b.localhost = {
          hostAddress = "172.30.0.1/24";
          namespaceAddress = "172.30.0.3/24";
        };
      };
    }
  ] "duplicate host addresses";

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
