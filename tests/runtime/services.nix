{ pkgs }:
let
  inherit (pkgs) lib;
  hm = import ../lib/eval-home-manager.nix { inherit lib pkgs; };

  hmUser = "tester";
  home = "/home/${hmUser}";

  probeScript = ''
    set -eu
    {
      pwd
      printenv SVC_VAR
      if [ -w ${home}/svc-state ]; then echo state-rw; fi
      if [ -e ${home}/secret ]; then echo secret-visible; else echo secret-hidden; fi
    } > ${home}/svc/probe.tmp
    mv ${home}/svc/probe.tmp ${home}/svc/probe
    touch ${home}/svc-state/written
    exec sleep infinity
  '';

  eval = hm {
    home.username = hmUser;
    home.homeDirectory = home;
    xdg = {
      configHome = "${home}/.config";
      stateHome = "${home}/.local/state";
      cacheHome = "${home}/.cache";
      dataHome = "${home}/.local/share";
    };
    cloister = {
      enable = true;
      sandboxes.svc.preset = "hardened";
      services.probe = {
        sandbox = "svc";
        command = [
          "sh"
          "-c"
          probeScript
        ];
        workingDirectory = "svc";
        readWrite = [ "svc-state" ];
        env.SVC_VAR = "from-service";
      };
    };
  };

  unit = eval.config.systemd.user.services.cloister-service-probe;
in
pkgs.testers.runNixOSTest (_: {
  name = "cloister-runtime-services";

  nodes.machine = _: {
    # One vCPU plus the CPU hogs below keep the launcher's setup window wide
    # enough for stop requests to land in it.
    virtualisation.cores = 1;

    environment.systemPackages = eval.config.home.packages;

    users.users.${hmUser} = {
      isNormalUser = true;
      createHome = true;
      linger = true;
    };

    # Install the generated Home Manager unit as a NixOS user unit so the test
    # exercises the real user manager (XDG_RUNTIME_DIR, cgroup teardown).
    systemd.user.services.cloister-service-probe = {
      unitConfig = unit.Unit;
      serviceConfig = unit.Service;
      wantedBy = unit.Install.WantedBy;
    };

    systemd.services.cloister-runtime-services-fixture = {
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = pkgs.writeShellScript "cloister-runtime-services-fixture" ''
          set -eu
          install -d -m 0700 -o ${hmUser} -g users ${home}
          echo top-secret > ${home}/secret
          chown ${hmUser}:users ${home}/secret
        '';
      };
    };
  };

  testScript = ''
    ${builtins.readFile ./lib.py}

    start_all()

    machine.wait_for_unit("cloister-runtime-services-fixture.service")
    machine.wait_for_unit("user@1000.service")

    def user_systemctl(args):
        return (
            "${pkgs.util-linux}/bin/runuser -u ${hmUser} -- env "
            + "XDG_RUNTIME_DIR=/run/user/1000 systemctl --user " + args
        )

    import time

    # The unit is already started by default.target (linger). Stop it and
    # drop its probe so the checks below only see the fresh instance.
    machine.succeed(user_systemctl("stop cloister-service-probe.service"))
    machine.succeed("rm -f ${home}/svc/probe")
    machine.succeed(user_systemctl("start cloister-service-probe.service"))
    machine.wait_for_unit("cloister-service-probe.service", "${hmUser}")
    machine.wait_for_file("${home}/svc/probe")

    assert_eq(machine, "cat ${home}/svc/probe",
              "${home}/svc\nfrom-service\nstate-rw\nsecret-hidden",
              "service sees its working dir, env and binds, but not unbound home files")
    machine.succeed("test -f ${home}/svc-state/written")
    assert_eq(machine, "stat -c %a ${home}/svc", "700", "launcher creates working dir 0700")
    assert_eq(machine, "stat -c %a ${home}/svc-state", "700", "launcher creates rw bind 0700")

    def timed_stop(label):
        started = time.monotonic()
        machine.succeed(user_systemctl("stop cloister-service-probe.service"))
        elapsed = time.monotonic() - started
        # The launcher's own SIGKILL escalation fires after 10s; anything
        # slower means the stop request was lost and systemd had to step in.
        assert elapsed < 9, f"{label}: stop took {elapsed:.1f}s"
        machine.wait_until_fails("pgrep -u ${hmUser} -f 'sleep infinity'")
        machine.fail(user_systemctl("is-failed cloister-service-probe.service"))

    machine.succeed("pgrep -u ${hmUser} -f 'sleep infinity'")
    timed_stop("stop running service")

    # Stop while the launcher is still setting up, on a starved CPU. SIGTERM
    # used to be dropped when it arrived before the sandbox child existed, and
    # the SIGKILL escalation targeted a process group that did not exist.
    machine.succeed("for i in 1 2 3 4 5 6; do (nice -n -5 yes > /dev/null &) ; done")
    for attempt in range(10):
        # Clear StartLimitBurst so rapid restarts here are not rate limited.
        machine.succeed(user_systemctl("reset-failed cloister-service-probe.service"))
        machine.succeed(user_systemctl("start cloister-service-probe.service"))
        timed_stop(f"stop during startup #{attempt}")
  '';
})
