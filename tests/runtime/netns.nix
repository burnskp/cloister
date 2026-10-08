{ pkgs }:
let
  webRoot = pkgs.writeTextDir "index.html" "hello from host\n";
in
pkgs.testers.runNixOSTest (_: {
  name = "cloister-runtime-netns";

  nodes.machine =
    { pkgs, ... }:
    {
      virtualisation.cores = 2;

      imports = [ ../../modules/cloister-netns ];

      users.users.tester = {
        isNormalUser = true;
        extraGroups = [ "cloister-netns" ];
      };

      users.users.outsider = {
        isNormalUser = true;
        extraGroups = [ ];
      };

      cloister-netns.networks = {
        dev = {
          type = "localhost";
          allowedPorts = [ 4000 ];
        };
        lanonly = {
          type = "lan";
          allowedRanges = [ "192.168.0.0/16" ];
        };
      };

      systemd.services.localhost-http = {
        description = "HTTP server for cloister-netns runtime test";
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "simple";
          ExecStart = "${pkgs.python3}/bin/python -m http.server 4000 --bind 127.0.0.1 --directory ${webRoot}";
          Restart = "always";
        };
      };
    };

  testScript = ''
    ${builtins.readFile ./lib.py}

    start_all()

    machine.wait_for_unit("cloister-netns-dev.service")
    machine.wait_for_unit("cloister-netns-lanonly.service")
    machine.wait_for_unit("localhost-http.service")
    machine.wait_until_succeeds("ip netns list | grep -F dev")
    machine.wait_until_succeeds("ip netns list | grep -F lanonly")

    machine.succeed(
        "${pkgs.util-linux}/bin/runuser -u tester -- ${pkgs.runtimeShell} -lc "
        + "'grep -F host.internal /etc/netns/dev/hosts'"
    )
    assert_contains(
        machine,
        "ip netns exec dev ${pkgs.iproute2}/bin/ip route",
        "default via 172.29.0.1",
        "dev netns has expected default route",
    )
    assert_contains(
        machine,
        "ip netns exec dev ${pkgs.curl}/bin/curl -fsS http://172.29.0.1:4000/",
        "hello from host",
        "dev netns can reach host service",
    )
    machine.fail(
        "${pkgs.util-linux}/bin/runuser -u tester -- ${pkgs.runtimeShell} -lc "
        + "'/run/wrappers/bin/cloister-netns --netns dev -- ${pkgs.runtimeShell} -lc true'"
    )
    machine.fail(
        "${pkgs.util-linux}/bin/runuser -u outsider -- ${pkgs.runtimeShell} -lc "
        + "'/run/wrappers/bin/cloister-netns --netns dev -- ${pkgs.iproute2}/bin/ip route'"
    )
    machine.fail(
        "/run/wrappers/bin/cloister-netns --netns dev -- ${pkgs.iproute2}/bin/ip route"
    )
    machine.fail("ip -6 addr show dev veth-dev | grep -F \"inet6 \"")
    machine.fail("ip -6 addr show dev veth-lanonly | grep -F \"inet6 \"")
    machine.fail(
        "ip netns exec dev ${pkgs.iproute2}/bin/ip -6 addr show dev veth-dev-ns | grep -F \"inet6 \""
    )
    machine.fail(
        "ip netns exec lanonly ${pkgs.iproute2}/bin/ip -6 addr show dev veth-lanonly-ns | grep -F \"inet6 \""
    )
  '';
})
