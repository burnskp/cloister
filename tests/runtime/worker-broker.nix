{ pkgs }:
let
  inherit (pkgs) lib;
  hm = import ../lib/eval-home-manager.nix { inherit lib pkgs; };

  hmUser = "tester";
  homeDir = "/home/${hmUser}";
  runtimeDir = "/tmp/cloister-worker-broker-runtime";
  projectDir = "${homeDir}/project";
  nestedProjectDir = "${projectDir}/other-project";
  devPackage = builtins.elemAt eval.config.home.packages 0;
  workerPackage = builtins.elemAt eval.config.home.packages 1;

  fixture = pkgs.writeShellScriptBin "cloister-worker-broker-fixture" ''
    set -eu

    case "$1" in
      child-project-rw-write)
        printf '%s\n' 'project-rw' > project-rw.txt
        ;;
      child-stdio)
        read -r line
        printf 'stdout:%s\n' "$line"
        printf 'stderr:%s\n' "$line" >&2
        ;;
      child-stdin-eof)
        if read -r line; then
          printf 'unexpected-stdin:%s\n' "$line"
          exit 1
        fi
        printf '%s\n' stdin-eof
        ;;
      child-check-stdout)
        test -w /proc/self/fd/1
        printf '%s\n' stdout-fd-open > broker-closed-stdout.txt
        ;;
      child-large-argv)
        shift
        total=0
        for arg in "$@"; do
          total=$((total + ''${#arg}))
        done
        printf '%s\n' "$total"
        ;;
      child-same-sandbox-write)
        printf '%s\n' 'same-sandbox' > same-sandbox.txt
        ;;
      child-overlay-write)
        printf '%s\n' 'overlay-child' > overlay-only.txt
        printf '%s\n' 'overlay-mutated' > host.txt
        ;;
      parent-local)
        clb-local "$0" child-same-sandbox-write
        ;;
      parent-project-rw)
        clb-project "$0" child-project-rw-write
        ;;
      parent-stdio-capture)
        printf '%s\n' broker-input | clb-project "$0" child-stdio > broker-stdout.txt 2> broker-stderr.txt
        ;;
      parent-closed-stdin)
        clb-project "$0" child-stdin-eof <&- > broker-closed-stdin.txt
        ;;
      parent-closed-stdout)
        clb-project "$0" child-check-stdout >&-
        ;;
      parent-large-argv)
        long_arg=$(${pkgs.python3}/bin/python -c 'print("x" * 1000)')
        set -- "$0" child-large-argv
        i=0
        while [ "$i" -lt 512 ]; do
          set -- "$@" "$long_arg"
          i=$((i + 1))
        done
        clb-project "$@" > broker-large-argv.txt
        ;;
      parent-overlay)
        clb-ephemeral "$0" child-overlay-write
        ;;
      parent-same-project)
        cd ${lib.escapeShellArg nestedProjectDir}
        clb-project "$0" child-project-rw-write
        ;;
      child-netns-route)
        exec ${pkgs.iproute2}/bin/ip route
        ;;
      child-netns-host-internal)
        exec ${pkgs.curl}/bin/curl -fsS http://host.internal:4001/passwd
        ;;
      parent-netns-route)
        clb-project "$0" child-netns-route
        ;;
      parent-netns-host-internal)
        clb-project "$0" child-netns-host-internal
        ;;
      parent-missing-command)
        clb-project
        ;;
      parent-c-rejected)
        clb-project -c "$0" child-project-rw-write
        ;;
      *)
        printf 'unknown fixture command: %s\n' "$1" >&2
        exit 2
        ;;
    esac
  '';

  eval = hm {
    home.username = hmUser;
    home.homeDirectory = homeDir;
    xdg = {
      configHome = "${homeDir}/.config";
      stateHome = "${homeDir}/.local/state";
      cacheHome = "${homeDir}/.cache";
      dataHome = "${homeDir}/.local/share";
    };
    cloister = {
      enable = true;
      sandboxes.dev = {
        preset = "hardened";
        shell.name = "bash";
        shell.hostConfig = false;
        network.enable = true;
        network.namespace = "dev";
        workerBroker = {
          profiles = {
            local = {
              sandbox = "dev";
              workspace.mode = "project-rw";
            };
            ephemeral = {
              sandbox = "worker";
              workspace.mode = "project-overlay";
            };
            project = {
              sandbox = "worker";
              workspace.mode = "project-rw";
            };
          };
        };
      };
      sandboxes.worker = {
        preset = "hardened";
        shell.name = "bash";
        shell.hostConfig = false;
        network.enable = true;
        network.namespace = "dev";
      };
    };
  };
in
pkgs.testers.runNixOSTest (_: {
  name = "cloister-runtime-worker-broker";

  nodes.machine =
    { pkgs, ... }:
    {
      imports = [ ../../modules/cloister-netns ];

      virtualisation.cores = 2;

      environment.systemPackages = [
        fixture
        pkgs.bash
      ]
      ++ eval.config.home.packages;

      users.users.${hmUser} = {
        isNormalUser = true;
        createHome = true;
        extraGroups = [ "cloister-netns" ];
      };

      cloister-netns.networks.dev = {
        type = "localhost";
        allowedPorts = [ 4001 ];
      };

      systemd.services.localhost-http = {
        description = "HTTP server for worker broker netns runtime test";
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "simple";
          ExecStart = "${pkgs.python3}/bin/python -m http.server 4001 --bind 127.0.0.1 --directory /etc";
          Restart = "always";
        };
      };

      systemd.services.cloister-runtime-worker-broker-fixture = {
        description = "Cloister runtime worker broker fixture";
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          ExecStart = pkgs.writeShellScript "cloister-runtime-worker-broker-fixture" ''
            set -eu

            install -d -m 0700 -o ${hmUser} -g users ${homeDir}
            install -d -m 0700 -o ${hmUser} -g users ${homeDir}/.config
            install -d -m 0700 -o ${hmUser} -g users ${homeDir}/.local/state
            install -d -m 0700 -o ${hmUser} -g users ${homeDir}/.cache
            install -d -m 0700 -o ${hmUser} -g users ${homeDir}/.local/share
            install -d -m 0700 -o ${hmUser} -g users ${runtimeDir}
            install -d -m 0700 -o ${hmUser} -g users ${projectDir}
            install -d -m 0700 -o ${hmUser} -g users ${nestedProjectDir}

            printf '%s\n' 'host-original' > ${projectDir}/host.txt
            chown ${hmUser}:users ${projectDir}/host.txt
          '';
        };
      };
    };

  testScript = ''
    ${builtins.readFile ./lib.py}

    start_all()

    machine.wait_for_unit("cloister-netns-dev.service")
    machine.wait_for_unit("localhost-http.service")
    machine.wait_for_unit("cloister-runtime-worker-broker-fixture.service")

    tester_shell = (
        "${pkgs.util-linux}/bin/runuser -u ${hmUser} -- env "
        + "HOME=${homeDir} USER=${hmUser} LOGNAME=${hmUser} "
        + "XDG_CONFIG_HOME=${homeDir}/.config "
        + "XDG_STATE_HOME=${homeDir}/.local/state "
        + "XDG_CACHE_HOME=${homeDir}/.cache "
        + "XDG_DATA_HOME=${homeDir}/.local/share "
        + "XDG_RUNTIME_DIR=${runtimeDir} "
        + "${pkgs.bash}/bin/bash -lc "
    )

    machine.succeed(
        tester_shell
        + "'cd ${projectDir} && rm -f project-rw.txt && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-project-rw'"
    )
    assert_eq(
        machine,
        "cat ${projectDir}/project-rw.txt",
        "project-rw",
        "project-rw worker writes visible file content",
    )
    machine.fail(
        "${pkgs.bash}/bin/bash -lc 'set -- ${runtimeDir}/cloister/broker/sessions/*.json; test -e \"$1\"'"
    )

    machine.succeed(
        tester_shell
        + "'cd ${projectDir} && rm -f same-sandbox.txt && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-local'"
    )
    assert_eq(
        machine,
        "cat ${projectDir}/same-sandbox.txt",
        "same-sandbox",
        "same-sandbox broker launcher writes visible file content",
    )

    machine.succeed(
        tester_shell
        + "'cd ${projectDir} && rm -f broker-stdout.txt broker-stderr.txt && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-stdio-capture'"
    )
    assert_eq(
        machine,
        "cat ${projectDir}/broker-stdout.txt",
        "stdout:broker-input",
        "worker broker child stdout is captured by parent sandbox program",
    )
    assert_eq(
        machine,
        "cat ${projectDir}/broker-stderr.txt",
        "stderr:broker-input",
        "worker broker child stderr is captured by parent sandbox program",
    )

    machine.succeed(
        tester_shell
        + "'cd ${projectDir} && rm -f broker-closed-stdin.txt && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-closed-stdin'"
    )
    assert_eq(
        machine,
        "cat ${projectDir}/broker-closed-stdin.txt",
        "stdin-eof",
        "worker broker launch tolerates closed stdin for non-interactive commands",
    )

    machine.succeed(
        tester_shell
        + "'cd ${projectDir} && rm -f broker-closed-stdout.txt && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-closed-stdout'"
    )
    assert_eq(
        machine,
        "cat ${projectDir}/broker-closed-stdout.txt",
        "stdout-fd-open",
        "worker broker launch tolerates closed stdout for non-interactive commands",
    )

    machine.succeed(
        tester_shell
        + "'cd ${projectDir} && rm -f broker-large-argv.txt && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-large-argv'"
    )
    assert_eq(
        machine,
        "cat ${projectDir}/broker-large-argv.txt",
        "512000",
        "worker broker launch handles request payloads larger than one fd-passing sendmsg",
    )

    machine.succeed(
        tester_shell
        + "'cd ${projectDir} && printf \"%s\\n\" host-original > host.txt && rm -f overlay-only.txt && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-overlay'"
    )
    assert_eq(
        machine,
        "cat ${projectDir}/host.txt",
        "host-original",
        "overlay worker does not mutate host file",
    )
    machine.fail("test -e ${projectDir}/overlay-only.txt")

    assert_contains(
        machine,
        tester_shell
        + "'cd ${projectDir} && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-netns-route'",
        "default via 172.29.0.1 dev veth-dev-ns",
        "worker broker launches worker through host netns helper",
    )
    assert_contains(
        machine,
        tester_shell
        + "'cd ${projectDir} && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-netns-host-internal'",
        "root:x:",
        "worker broker child can reach host.internal service from netns",
    )

    assert_failure_contains(
        machine,
        tester_shell
        + "'cd ${projectDir} && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-missing-command'",
        "clb-project: expected a command to run",
        "generated worker broker launcher rejects missing command argv",
    )

    assert_failure_contains(
        machine,
        tester_shell
        + "'cd ${projectDir} && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-c-rejected'",
        "broker launcher does not support -c; pass the command argv directly",
        "generated worker broker launcher rejects -c shorthand",
    )

    assert_failure_contains(
        machine,
        tester_shell
        + "'cd ${projectDir} && env CLOISTER_BROKER_CHILD_PROFILE=project CLOISTER_BROKER_PARENT_CAPABILITY='\"'\"'{\"token\":\"forged-token\"}'\"'\"' ${workerPackage}/bin/cl-worker -c ${fixture}/bin/cloister-worker-broker-fixture child-project-rw-write'",
        "trusted broker session record mount is unavailable",
        "forged minimal broker capability is rejected",
    )
    forged_payload_output = assert_failure(
        machine,
        tester_shell
        + "'cd ${projectDir} && env CLOISTER_BROKER_CHILD_PROFILE=project CLOISTER_BROKER_PARENT_CAPABILITY='\"'\"'{\"token\":\"forged-token\",\"project_root\":\"${projectDir}\",\"dir_hash\":\"forged\",\"profiles\":{}}'\"'\"' ${workerPackage}/bin/cl-worker -c ${fixture}/bin/cloister-worker-broker-fixture child-project-rw-write'",
        "forged legacy broker payload is rejected",
    )
    if "parse CLOISTER_BROKER_PARENT_CAPABILITY" not in forged_payload_output and "unknown field" not in forged_payload_output:
        raise AssertionError(
            _format_failure(
                "forged legacy broker payload is rejected",
                "forged CLOISTER_BROKER_PARENT_CAPABILITY payload",
                "output containing 'parse CLOISTER_BROKER_PARENT_CAPABILITY' or 'unknown field'",
                forged_payload_output,
            )
        )

    assert_failure_contains(
        machine,
        tester_shell
        + "'cd ${projectDir} && ${devPackage}/bin/cl-dev -c ${fixture}/bin/cloister-worker-broker-fixture parent-same-project'",
        "broker project identity does not match",
        "broker rejects mismatched nested project roots",
    )
  '';
})
