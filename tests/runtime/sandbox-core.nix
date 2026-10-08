{ pkgs }:
let
  inherit (pkgs) lib;
  hm = import ../lib/eval-home-manager.nix { inherit lib pkgs; };

  hmUser = "tester";

  eval = hm {
    home.username = hmUser;
    home.homeDirectory = "/home/${hmUser}";
    xdg = {
      configHome = "/home/${hmUser}/.config";
      stateHome = "/home/${hmUser}/.local/state";
      cacheHome = "/home/${hmUser}/.cache";
      dataHome = "/home/${hmUser}/.local/share";
    };
    cloister = {
      enable = true;
      sandboxes = {
        dev = {
          preset = "hardened";
          defaultCommand = [ "sh" ];
          ssh.enable = true;
          sandbox.passthroughEnv = lib.mkAfter [ "TMUX" ];
          registry = {
            aliases.sandboxalias = "printenv CLOISTER";
            commands = [ "printenv" ];
            functions.greet = ''
              printf '%s\n' "$CLOISTER"
            '';
          };
        };

        dns.network.dns.nameservers = [
          "192.0.2.53"
          "2001:db8::53"
        ];
        dnsfile.network.dns.resolvConf = "/run/NetworkManager/resolv.conf";

        large = {
          preset = "hardened";
          extraPackages = [ pkgs.python3 ];
          sandbox.env =
            builtins.listToAttrs (
              builtins.genList (i: {
                name = "LARGE_${toString i}";
                value = lib.concatStrings (builtins.genList (_: "x") 4096);
              }) 40
            )
            // {
              EMPTY_VALUE = "";
            };
        };

        sec = {
          preset = "hardened";
        };
      };
    };
  };
in
pkgs.testers.runNixOSTest (_: {
  name = "cloister-runtime-sandbox-core";

  nodes.machine =
    { pkgs, ... }:
    {
      virtualisation.cores = 2;

      environment.systemPackages = [
        pkgs.jq
        pkgs.openssh
        pkgs.python3
        pkgs.zsh
      ]
      ++ eval.config.home.packages;

      users.users.${hmUser} = {
        isNormalUser = true;
        createHome = true;
        extraGroups = [ ];
      };

      systemd.services.cloister-runtime-sandbox-fixture = {
        description = "Cloister runtime sandbox fixture";
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          ExecStart = pkgs.writeShellScript "cloister-runtime-sandbox-fixture" ''
            set -eu

            install -d -m 0700 -o ${hmUser} -g users /home/${hmUser}
            install -d -m 0700 -o ${hmUser} -g users /home/${hmUser}/.config
            cat > /home/${hmUser}/.zshenv <<'EOF'
            ${eval.config.programs.zsh.initContent}
            EOF

            chown ${hmUser}:users /home/${hmUser}/.zshenv
          '';
        };
      };
    };

  testScript = ''
    ${builtins.readFile ./lib.py}

    start_all()

    machine.wait_for_unit("cloister-runtime-sandbox-fixture.service")

    tester_shell = (
        "${pkgs.util-linux}/bin/runuser -u ${hmUser} -- env "
        + "HOME=/home/${hmUser} USER=${hmUser} LOGNAME=${hmUser} "
        + "ZDOTDIR=/home/${hmUser} XDG_CONFIG_HOME=/home/${hmUser}/.config "
        + "CLOISTER_DIR=/tmp "
        + "${pkgs.zsh}/bin/zsh -ic "
    )

    dns_command = (
        "${pkgs.util-linux}/bin/runuser -u ${hmUser} -- env HOME=/home/${hmUser} "
        + "XDG_CONFIG_HOME=/home/${hmUser}/.config CLOISTER_DIR=/tmp "
        + "/run/current-system/sw/bin/cl-"
    )
    machine.succeed("timeout 30 " + dns_command + "large -c python -c "
                    + "'import os; assert all(os.environ[\"LARGE_\" + str(i)] == \"x\" * 4096 for i in range(40)); "
                    + "assert os.environ[\"EMPTY_VALUE\"] == \"\"'")

    assert_eq(machine, dns_command + "dns -c cat /etc/resolv.conf",
              "nameserver 192.0.2.53\nnameserver 2001:db8::53",
              "explicit resolver contents")
    machine.succeed("mkdir -p /run/NetworkManager")
    machine.succeed("printf 'nameserver 192.0.2.54\\nsearch example.test\\n' > /run/NetworkManager/resolv.conf")
    assert_eq(machine, dns_command + "dnsfile -c cat /etc/resolv.conf",
              "nameserver 192.0.2.54\nsearch example.test", "runtime resolver contents")
    for sandbox in ["dns", "dnsfile"]:
        assert_failure_contains(machine, dns_command + sandbox
                                + " -c sh -c 'echo changed > /etc/resolv.conf'",
                                "Read-only file system", "resolver is read-only")
    machine.succeed("printf 'nameserver 192.0.2.55\\n' > /run/NetworkManager/resolv.conf.new")
    machine.succeed("mv /run/NetworkManager/resolv.conf.new /run/NetworkManager/resolv.conf")
    assert_eq(machine, dns_command + "dnsfile -c cat /etc/resolv.conf",
              "nameserver 192.0.2.55", "relaunch picks up resolver replacement")
    machine.succeed("rm /run/NetworkManager/resolv.conf")
    assert_failure_contains(machine, dns_command + "dnsfile -c true",
                            "/run/NetworkManager/resolv.conf", "missing explicit resolver fails startup")

    assert_contains(machine, tester_shell + "'type sandboxalias'", "alias", "sandboxalias is rendered as alias")
    assert_contains(machine, tester_shell + "'type greet'", "function", "greet is rendered as function")
    assert_contains(machine, tester_shell + "'type printenv'", "alias", "printenv is wrapped as alias")
    assert_eq(machine, tester_shell + "'sandboxalias'", "dev", "sandboxalias prints sandbox name")
    assert_eq(machine, tester_shell + "'greet'", "dev", "greet prints sandbox name")
    assert_eq(machine, tester_shell + "'printenv CLOISTER'", "dev", "CLOISTER env is set")

    machine.succeed(
        "${pkgs.util-linux}/bin/runuser -u ${hmUser} -- env HOME=/home/${hmUser} "
        + "XDG_CONFIG_HOME=/home/${hmUser}/.config CLOISTER_DIR=/tmp "
        + "/run/current-system/sw/bin/cl-sec -c ${pkgs.python3}/bin/python -c "
        + "'import errno, socket\n"
        + "sock = None\n"
        + "try:\n"
        + "    sock = socket.socket(socket.AF_NETLINK, socket.SOCK_RAW, 0)\n"
        + "except OSError as exc:\n"
        + "    raise SystemExit(0 if exc.errno == errno.EAFNOSUPPORT else 1)\n"
        + "else:\n"
        + "    raise SystemExit(1)\n"
        + "finally:\n"
        + "    if sock is not None:\n"
        + "        sock.close()'"
    )

  '';
})
