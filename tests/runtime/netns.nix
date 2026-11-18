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
          allowedHostPorts = [ 4000 ];
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

      # Unlisted port, bound on all addresses (covers loopback and the
      # host-side veth address of the lanonly namespace).
      systemd.services.unlisted-http = {
        description = "Unlisted HTTP server for cloister-netns runtime test";
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "simple";
          ExecStart = "${pkgs.python3}/bin/python -m http.server 4001 --bind 0.0.0.0 --directory ${webRoot}";
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
    # lan + allowedHostPorts: listed host port reachable via host.internal.
    machine.wait_for_unit("unlisted-http.service")
    machine.wait_for_open_port(4001)
    # nscd on the VM host bypasses the netns hosts file under `ip netns exec`;
    # sandboxes bind-mount it directly, so pin the name with --resolve here.
    machine.succeed("grep -F '172.29.0.5 host.internal' /etc/netns/lanonly/hosts")
    assert_contains(
        machine,
        "ip netns exec lanonly ${pkgs.curl}/bin/curl -fsS --max-time 5 --resolve host.internal:4000:172.29.0.5 http://host.internal:4000/",
        "hello from host",
        "lanonly netns can reach listed host port via host.internal",
    )
    machine.fail(
        "ip netns exec lanonly ${pkgs.curl}/bin/curl -fsS --max-time 3 --resolve host.internal:4001:172.29.0.5 http://host.internal:4001/"
    )
    machine.fail(
        "ip netns exec lanonly ${pkgs.curl}/bin/curl -fsS --max-time 3 http://172.29.0.5:4001/"
    )
    machine.succeed("${pkgs.curl}/bin/curl -fsS --max-time 3 http://127.0.0.1:4001/ >/dev/null")

    # Forwarding to allowedRanges still works: fake LAN peer in its own netns.
    machine.succeed(
        "ip netns add peer"
        + " && ip link add vpeer type veth peer name vpeer-ns"
        + " && ip link set vpeer-ns netns peer"
        + " && ip addr add 192.168.77.1/24 dev vpeer"
        + " && ip link set vpeer up"
        + " && ip -n peer addr add 192.168.77.2/24 dev vpeer-ns"
        + " && ip -n peer link set vpeer-ns up"
        + " && ip -n peer link set lo up"
        + " && ip -n peer route add default via 192.168.77.1"
    )
    machine.succeed(
        "systemd-run --unit=peer-http ip netns exec peer "
        + "${pkgs.python3}/bin/python -m http.server 5000 --bind 192.168.77.2 --directory ${webRoot}"
    )
    machine.wait_until_succeeds(
        "ip netns exec peer ${pkgs.curl}/bin/curl -fsS --max-time 3 http://192.168.77.2:5000/"
    )
    assert_contains(
        machine,
        "ip netns exec lanonly ${pkgs.curl}/bin/curl -fsS --max-time 5 http://192.168.77.2:5000/",
        "hello from host",
        "lanonly netns can still forward to allowedRanges",
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
