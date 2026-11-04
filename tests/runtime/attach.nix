{ pkgs }:
let
  inherit (pkgs) lib;
  hm = import ../lib/eval-home-manager.nix { inherit lib pkgs; };

  hmUser = "tester";
  home = "/home/${hmUser}";

  # Records what the target itself looks like, so the attached process is
  # compared against the real sandbox rather than against expected constants.
  probeScript = ''
    set -u
    echo marker > /tmp/marker
    for ns in user mnt pid ipc uts cgroup net; do
      readlink /proc/self/ns/$ns
    done > /tmp/target-ns
    grep -E '^(NoNewPrivs|Seccomp|Seccomp_filters):' /proc/self/status > /tmp/target-status
    printenv PATH > /tmp/target-path
    unshare -U true 2> /dev/null
    echo "$?" > /tmp/target-unshare
    touch "$PWD/ready"
    exec sleep infinity
  '';

  mkService = sandbox: {
    inherit sandbox;
    command = [
      "sh"
      "-c"
      probeScript
    ];
    workingDirectory = "proj-${sandbox}";
    autoStart = false;
  };

  packages = [
    pkgs.util-linux
    pkgs.gnugrep
  ];

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
      sandboxes = {
        # Private network namespace, seccomp on.
        priv = {
          preset = "hardened";
          extraPackages = packages;
          network.enable = false;
        };
        # Shares the caller's network namespace.
        hostnet = {
          preset = "hardened";
          extraPackages = packages;
          network.enable = true;
        };
        # Named network namespace, reached through cloister-netns.
        vpn = {
          preset = "hardened";
          extraPackages = packages;
          network.namespace = "dev";
        };
        # A browser-shaped target: no project directory, Chromium seccomp mode.
        browser = {
          preset = "chromium";
          extraPackages = packages;
          sandbox.bindWorkingDirectory = false;
          # The VM has no session bus or PipeWire; neither relates to attaching.
          dbus = {
            enable = false;
            notifications = false;
          };
          audio.pipewire = {
            enable = false;
            pulseOnly = false;
          };
        };
        # Remapped identity.
        anon = {
          preset = "hardened";
          extraPackages = packages;
          sandbox.anonymize.enable = true;
        };
      };
      services = {
        svc-priv = mkService "priv";
        svc-hostnet = mkService "hostnet";
        svc-vpn = mkService "vpn";
        svc-anon = mkService "anon";
      };
    };
  };

  services = [
    "svc-priv"
    "svc-hostnet"
    "svc-vpn"
    "svc-anon"
  ];
  unitOf = name: eval.config.systemd.user.services."cloister-service-${name}";
in
pkgs.testers.runNixOSTest (_: {
  name = "cloister-runtime-attach";

  nodes.machine = _: {
    virtualisation.cores = 2;
    virtualisation.memorySize = 2048;

    imports = [ ../../modules/cloister-netns ];

    cloister-netns.networks.dev = {
      type = "localhost";
      allowedPorts = [ 4000 ];
    };

    environment.systemPackages = eval.config.home.packages ++ [
      pkgs.util-linux
      pkgs.python3
    ];

    users.users.${hmUser} = {
      isNormalUser = true;
      createHome = true;
      linger = true;
      extraGroups = [ "cloister-netns" ];
    };

    systemd.user.services = lib.listToAttrs (
      map (name: {
        name = "cloister-service-${name}";
        value = {
          unitConfig = (unitOf name).Unit;
          serviceConfig = (unitOf name).Service;
        };
      }) services
    );
  };

  testScript = ''
    ${builtins.readFile ./lib.py}

    import shlex

    start_all()
    machine.wait_for_unit("user@1000.service")
    machine.wait_for_unit("cloister-netns-dev.service")

    def as_user(script):
        return (
            "${pkgs.util-linux}/bin/runuser -u ${hmUser} -- env "
            + "XDG_RUNTIME_DIR=/run/user/1000 HOST_SECRET=leak "
            + "${pkgs.runtimeShell} -c " + shlex.quote(script)
        )

    def attach(target, command):
        return as_user(f"cd ${home} && timeout 20 cloister-attach {target} -- sh -c {shlex.quote(command)}")

    def in_sandbox(target, command):
        return machine.succeed(attach(target, command)).strip()

    for sandbox in ["priv", "hostnet", "vpn", "anon"]:
        machine.succeed(as_user(f"systemctl --user start cloister-service-svc-{sandbox}.service"))
        machine.wait_for_file(f"${home}/proj-{sandbox}/ready", timeout=60)

    registry = "/run/user/1000/cloister/instances"
    assert_eq(machine, f"stat -c %a {registry}", "700", "registry is private")

    # --- Selection ---
    listing = machine.succeed(as_user("cloister-attach --list"))
    for name in ["svc-priv", "svc-hostnet", "svc-vpn", "svc-anon"]:
        assert name in listing, listing
    # Ambiguous without a TTY: an error, never a prompt.
    machine.fail(as_user("cd / && cloister-attach -- true < /dev/null"))
    # The current directory picks the instance.
    assert_contains(machine, as_user("cd ${home}/proj-priv && cloister-attach -- cat /tmp/marker"),
                    "marker", "current directory selects the instance")
    # A sandbox name works as well as a service name.
    assert_contains(machine, as_user("cd / && cloister-attach priv -- cat /tmp/marker"),
                    "marker", "sandbox name selects the instance")

    # --- Each network shape joins the target's namespaces and keeps its policy ---
    ns_loop = 'for n in user mnt pid ipc uts cgroup net; do readlink /proc/self/ns/$n; done'
    for service in ["svc-priv", "svc-hostnet", "svc-vpn", "svc-anon"]:
        assert in_sandbox(service, "cat /tmp/marker") == "marker", service
        assert in_sandbox(service, ns_loop) == in_sandbox(service, "cat /tmp/target-ns"), service
        attached = in_sandbox(service, "grep -E '^(NoNewPrivs|Seccomp|Seccomp_filters):' /proc/self/status")
        assert attached == in_sandbox(service, "cat /tmp/target-status"), f"{service}: {attached}"
        assert "Seccomp:\t2" in attached, attached
        assert in_sandbox(service, "grep ^CapEff /proc/self/status").endswith("0000000000000000")
        assert in_sandbox(service, "grep ^CapBnd /proc/self/status").endswith("0000000000000000")
        assert in_sandbox(service, "grep ^CapPrm /proc/self/status").endswith("0000000000000000")
        assert in_sandbox(service, "printenv PATH") == in_sandbox(service, "cat /tmp/target-path")
        # The filter really is active, not merely reported.
        assert in_sandbox(service, "cat /tmp/target-unshare") != "0", service
        machine.fail(attach(service, "unshare -U true"))

    # --- Environment and working directory ---
    assert in_sandbox("svc-priv", "printenv CLOISTER_ATTACHED") == "1"
    machine.fail(attach("svc-priv", "printenv HOST_SECRET"))
    assert in_sandbox("svc-priv", "pwd") == "${home}/proj-priv"

    # Host-only: the command is not available inside a sandbox.
    machine.fail(attach("svc-priv", "command -v cloister-attach"))

    # --- Host descriptors stay out ---
    fds = machine.succeed(as_user(
        "cd / && exec 9< /etc/hostname; cloister-attach svc-priv -- ls /proc/self/fd < /dev/null"
    )).split()
    assert "9" not in fds, f"host fd leaked into sandbox: {fds}"

    # Pipes still pass through, in both directions.
    assert_contains(machine, as_user("echo piped | cloister-attach svc-priv -- cat"), "piped", "stdin pipe")

    # A host TTY is replaced by the relay PTY.
    machine.succeed(as_user(
        "cd / && script -qfec \"tty > /tmp/host-tty; cloister-attach svc-priv -- "
        "sh -c 'readlink /proc/self/fd/0 > /tmp/attached-tty'\" /dev/null"
    ))
    machine.succeed(as_user("cd / && cloister-attach svc-priv -- cat /tmp/attached-tty > /tmp/attached-tty-host"))
    host_tty = machine.succeed("cat /tmp/host-tty").strip()
    inner_tty = machine.succeed("cat /tmp/attached-tty-host").strip()
    assert host_tty.startswith("/dev/pts/"), host_tty
    assert inner_tty.startswith("/dev/pts/"), inner_tty
    assert host_tty != inner_tty, f"host TTY {host_tty} reached the sandbox"

    # No controlling terminal is inherited, even with no TTY on stdio.
    machine.fail(as_user(
        "cd / && script -qfec \"cloister-attach svc-priv -- sh -c ': < /dev/tty' < /dev/null > /tmp/o 2>&1\" /dev/null"
    ))

    # The default action is an interactive shell.
    output = machine.succeed(as_user(
        "cd / && printf 'echo attached-$CLOISTER_ATTACHED\\nexit\\n' | script -qfec 'cloister-attach svc-priv' /dev/null"
    ))
    assert "attached-1" in output, output

    # Exit codes come back, and leaving never touches the app.
    machine.succeed(attach("svc-priv", "exit 0"))
    code = machine.execute(attach("svc-priv", "exit 7"))[0]
    assert code == 7, code
    machine.succeed("pgrep -u ${hmUser} -f 'sleep infinity'")
    assert_contains(machine, as_user("systemctl --user is-active cloister-service-svc-priv.service"),
                    "active", "app survives the attached shell")

    # --- Refusals ---
    def edit_record(sandbox, change):
        path = machine.succeed(f"grep -l '\"sandbox\":\"{sandbox}\"' {registry}/*.json").strip()
        original = machine.succeed(f"cat {path}")
        record = json.loads(original)
        change(record)
        machine.succeed(f"cat > {path} <<'RECORD'\n{json.dumps(record)}\nRECORD")
        return path, original

    # The pinned filter is gone: refuse rather than run with a weaker policy.
    path, original = edit_record("priv", lambda r: r.update(seccomp_filter_path="/nonexistent/filter"))
    assert_failure_contains(machine, attach("svc-priv", "true"), "restart", "missing filter is refused")
    machine.succeed(f"cat > {path} <<'RECORD'\n{original}\nRECORD")

    # A record pointing at a host process must not run anything on the host.
    machine.succeed(as_user("nohup sleep 300 > /dev/null 2>&1 & echo $! > /tmp/host-pid"))
    host_pid = int(machine.succeed("cat /tmp/host-pid"))
    start_time = int(machine.succeed(f"sed 's/.*) //' /proc/{host_pid}/stat | cut -d' ' -f20"))
    def to_host(record):
        record.update(sandbox_pid1=host_pid, sandbox_pid1_start_time=start_time,
                      main_pid=host_pid, main_pid_start_time=start_time)
    path, original = edit_record("priv", to_host)
    assert_failure_contains(machine, attach("svc-priv", "touch /tmp/escaped"), "namespace", "host target is refused")
    machine.fail("test -e /tmp/escaped")
    machine.succeed(f"cat > {path} <<'RECORD'\n{original}\nRECORD")

    # --- Lifetime ---
    # The launcher's record names the live bwrap, and goes away with it.
    machine.succeed(as_user(
        "cd / && (script -qfec 'cloister-attach svc-hostnet -- sleep 300' /tmp/attach-out > /dev/null 2>&1 &) ; true"
    ))
    machine.wait_until_succeeds("pgrep -u ${hmUser} -f 'sleep 300' | wc -l | grep -v '^0$'")
    machine.succeed(as_user("systemctl --user stop cloister-service-svc-hostnet.service"))
    machine.wait_until_succeeds("grep -q 'sandbox exited' /tmp/attach-out", timeout=20)
    machine.wait_until_fails("pgrep -u ${hmUser} -f 'cloister-attach svc-hostnet'", timeout=20)
    machine.fail(f"grep -l '\"sandbox\":\"hostnet\"' {registry}/*.json")

    # SIGKILLed launcher: the stale record is pruned on the next read.
    stale = machine.succeed(f"grep -l '\"sandbox\":\"anon\"' {registry}/*.json").strip()
    machine.succeed(as_user("systemctl --user kill -s SIGKILL cloister-service-svc-anon.service"))
    # Stopping also cancels any restart, whose fresh instance would be listed.
    machine.succeed(as_user("systemctl --user stop cloister-service-svc-anon.service"))
    listing = machine.succeed(as_user("cloister-attach --list"))
    assert "svc-anon" not in listing, listing
    machine.fail(f"test -e {stale}")

    # --- A sandbox with no project, like a browser ---
    browser_probe = (
        "echo marker > /tmp/marker; "
        "grep -E '^(NoNewPrivs|Seccomp|Seccomp_filters):' /proc/self/status > /tmp/target-status; "
        "exec sleep 600"
    )
    machine.succeed(as_user(
        "cd / && (WAYLAND_DISPLAY= nohup cl-browser -c sh -c " + shlex.quote(browser_probe)
        + " > /tmp/browser.log 2>&1 &) ; true"
    ))
    try:
        machine.wait_until_succeeds(as_user("cloister-attach browser -- cat /tmp/marker"), timeout=30)
    except Exception:
        print("browser launcher log: " + machine.execute("cat /tmp/browser.log")[1])
        raise
    assert in_sandbox("browser", "pwd") == in_sandbox("browser", "printenv HOME"), "starts in HOME"
    # This launcher ran with HOST_SECRET set, so its bwrap PID 1 still carries it.
    # The attached environment must come from the sanitized PID 2.
    machine.fail(attach("browser", "printenv HOST_SECRET"))
    status = in_sandbox("browser", "grep -E '^(NoNewPrivs|Seccomp|Seccomp_filters):' /proc/self/status")
    assert status == in_sandbox("browser", "cat /tmp/target-status"), status
    assert "Seccomp:\t2" in status, status
    browser_row = [l for l in machine.succeed(as_user("cloister-attach --list")).splitlines() if " browser " in l]
    assert browser_row and browser_row[0].rstrip().endswith("-"), browser_row
    # The default action works without a project too.
    output = machine.succeed(as_user(
        "cd / && printf 'echo attached-$CLOISTER_ATTACHED\\nexit\\n' | script -qfec 'cloister-attach browser' /dev/null"
    ))
    assert "attached-1" in output, output

    # --- Terminal details ---
    # Externally delivered SIGINT restores the host terminal after raw mode.
    terminal_cleanup = """
    import fcntl
    import os
    import pty
    import signal
    import subprocess
    import termios
    import time

    master, slave = pty.openpty()
    saved = termios.tcgetattr(slave)
    def foreground_terminal():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    proc = subprocess.Popen(
        ["cloister-attach", "svc-priv", "--", "sleep", "300"],
        stdin=slave, stdout=slave, stderr=slave,
        preexec_fn=foreground_terminal,
    )
    try:
        deadline = time.monotonic() + 10
        while termios.tcgetattr(slave)[3] & (termios.ECHO | termios.ICANON):
            assert proc.poll() is None, "attach exited before entering raw mode"
            assert time.monotonic() < deadline, "attach never entered raw mode"
            time.sleep(0.01)
        proc.send_signal(signal.SIGINT)
        proc.wait(timeout=10)
        assert termios.tcgetattr(slave) == saved, "SIGINT left host terminal in raw mode"
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        os.close(slave)
        os.close(master)
    """
    import textwrap
    machine.succeed(as_user("cd / && python3 -c " + shlex.quote(textwrap.dedent(terminal_cleanup))))

    # The relay PTY starts at the caller's window size.
    output = machine.succeed(as_user(
        "cd / && script -qfec 'stty rows 45 cols 123; cloister-attach svc-priv -- stty size' /dev/null"
    ))
    assert "45 123" in output, output

    # A later resize reaches the attached process.
    resize = (
        "stty rows 45 cols 123; (sleep 0.5; stty rows 50 cols 130 < /dev/tty) & "
        "cloister-attach svc-priv -- sh -c 'sleep 1.5; stty size'"
    )
    output = machine.succeed(as_user("cd / && script -qfec " + shlex.quote(resize) + " /dev/null"))
    assert "50 130" in output, output

    # With several matches and a terminal, the picker chooses.
    output = machine.succeed(as_user(
        "cd / && printf '1\\n' | script -qfec 'cloister-attach -- cat /tmp/marker' /dev/null"
    ))
    assert "Attach to which instance?" in output and "marker" in output, output

    # --- Interactive targets register too ---
    machine.succeed(as_user(
        "mkdir -p ${home}/inter && cd ${home}/inter && "
        "(sleep 300 | script -qfc cl-priv /dev/null > /dev/null 2>&1 &) ; true"
    ))
    machine.wait_until_succeeds(as_user("cd ${home}/inter && cloister-attach -- true < /dev/null"), timeout=30)
    assert_eq(machine, as_user("cd ${home}/inter && cloister-attach -- pwd"), "${home}/inter", "interactive target")
  '';
})
