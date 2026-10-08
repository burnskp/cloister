{ pkgs }:
let
  inherit (pkgs) lib;
  dbusValidate = pkgs.callPackage ../../helpers/cloister-dbus-validate { };
  hm = import ../lib/eval-home-manager.nix { inherit lib pkgs; };
in
pkgs.testers.runNixOSTest (_: {
  name = "cloister-runtime-gui-dbus-audio";

  nodes.machine =
    { pkgs, ... }:
    let
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
          sandboxes.browser = {
            defaultCommand = [ "sh" ];
            dbus = {
              enable = true;
              portal = {
                fileChooser = true;
                openUri = true;
              };
              rawPolicies.talk = [ "org.example.Service" ];
            };
          };
        };
      };

      proxyWrapper = eval.config.systemd.user.services.cloister-dbus-proxy-browser.Service.ExecStart;
    in
    {
      virtualisation.cores = 2;

      environment.systemPackages = with pkgs; [
        dbus
        jq
        xdg-dbus-proxy
      ];

      users.users.${hmUser} = {
        isNormalUser = true;
        extraGroups = [ ];
      };

      systemd.services.cloister-runtime-dbus-fixture = {
        description = "Cloister runtime D-Bus fixture";
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "simple";
          User = hmUser;
          Environment = [ "HOME=/home/${hmUser}" ];
          ExecStart = pkgs.writeShellScript "cloister-runtime-dbus-fixture" ''
            set -eu

            export XDG_RUNTIME_DIR="/tmp/cloister-runtime-user"
            export CLOISTER_DBUS_PROXY_INSTANCE_ID="browser-runtime-portal"
            rm -rf "$XDG_RUNTIME_DIR"
            install -d -m 700 "$XDG_RUNTIME_DIR"
            install -d -m 700 "$XDG_RUNTIME_DIR/cloister/flatpak-info"
            install -d -m 700 "$XDG_RUNTIME_DIR/cloister/dbus"

            cat > "$XDG_RUNTIME_DIR/cloister/flatpak-info/$CLOISTER_DBUS_PROXY_INSTANCE_ID.ini" <<EOF
            [Application]
            name=dev.cloister.browser

            [Instance]
            instance-id=$CLOISTER_DBUS_PROXY_INSTANCE_ID
            EOF

            cat > "$XDG_RUNTIME_DIR/cloister-session.conf" <<EOF
            <!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
             "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
            <busconfig>
              <type>session</type>
              <listen>unix:path=$XDG_RUNTIME_DIR/bus</listen>
              <policy context="default">
                <allow send_destination="*" eavesdrop="true"/>
                <allow eavesdrop="true"/>
                <allow own="*"/>
              </policy>
            </busconfig>
            EOF

            ${pkgs.dbus}/bin/dbus-daemon --config-file="$XDG_RUNTIME_DIR/cloister-session.conf" --fork

            DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus" \
              ${pkgs.dbus}/bin/dbus-test-tool echo --name=org.example.Service >/tmp/cloister-dbus-echo.log 2>&1 &
            echo $! > "$XDG_RUNTIME_DIR/org.example.Service.pid"

            DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus" \
              ${pkgs.dbus}/bin/dbus-test-tool echo --name=org.example.Secret >/tmp/cloister-dbus-secret.log 2>&1 &
            echo $! > "$XDG_RUNTIME_DIR/org.example.Secret.pid"

            DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus" \
              ${pkgs.dbus}/bin/dbus-test-tool echo --name=org.freedesktop.portal.Desktop >/tmp/cloister-dbus-portal.log 2>&1 &
            echo $! > "$XDG_RUNTIME_DIR/org.freedesktop.portal.Desktop.pid"

            CLOISTER_DBUS_PROXY_SOCKET="$XDG_RUNTIME_DIR/cloister/dbus/browser-runtime-portal" \
              CLOISTER_DBUS_PROXY_INSTANCE_ID="$CLOISTER_DBUS_PROXY_INSTANCE_ID" \
              XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" \
              ${proxyWrapper} >/tmp/cloister-dbus-proxy.log 2>&1 &
            echo $! > "$XDG_RUNTIME_DIR/cloister-dbus-proxy.pid"

            for _ in $(seq 1 100); do
              if [ -S "$XDG_RUNTIME_DIR/cloister/dbus/browser-runtime-portal" ]; then
                wait "$(cat "$XDG_RUNTIME_DIR/cloister-dbus-proxy.pid")"
                exit 0
              fi
              if ! kill -0 "$(cat "$XDG_RUNTIME_DIR/cloister-dbus-proxy.pid")" 2>/dev/null; then
                echo "dbus proxy exited early" >&2
                if [ -f /tmp/cloister-dbus-proxy.log ]; then
                  cat /tmp/cloister-dbus-proxy.log >&2
                fi
                break
              fi
              sleep 0.1
            done

            echo "proxy socket did not appear" >&2
            exit 1
          '';
          ExecStop = pkgs.writeShellScript "cloister-runtime-dbus-fixture-stop" ''
            set -eu

            export XDG_RUNTIME_DIR="/tmp/cloister-runtime-user"
            for pid_file in \
              "$XDG_RUNTIME_DIR/cloister-dbus-proxy.pid" \
              "$XDG_RUNTIME_DIR/org.example.Service.pid" \
              "$XDG_RUNTIME_DIR/org.example.Secret.pid" \
              "$XDG_RUNTIME_DIR/org.freedesktop.portal.Desktop.pid"
            do
              if [ -f "$pid_file" ]; then
                kill "$(cat "$pid_file")" || true
                rm -f "$pid_file"
              fi
            done
            rm -rf "$XDG_RUNTIME_DIR"
          '';
        };
      };
    };

  testScript = ''
    ${builtins.readFile ./lib.py}

    start_all()

    machine.wait_for_unit("cloister-runtime-dbus-fixture.service")
    machine.wait_until_succeeds("test -S /tmp/cloister-runtime-user/bus")
    machine.wait_until_succeeds("test -S /tmp/cloister-runtime-user/cloister/dbus/browser-runtime-portal")

    raw_bus = "unix:path=/tmp/cloister-runtime-user/bus"
    proxy_bus = "unix:path=/tmp/cloister-runtime-user/cloister/dbus/browser-runtime-portal"
    raw_list_cmd = (
        "${pkgs.util-linux}/bin/runuser -u tester -- env DBUS_SESSION_BUS_ADDRESS=" + raw_bus + " "
        + "${dbusValidate}/bin/cloister-dbus-validate --list --quiet"
    )
    proxy_list_cmd = (
        "${pkgs.util-linux}/bin/runuser -u tester -- env DBUS_SESSION_BUS_ADDRESS=" + proxy_bus + " "
        + "${dbusValidate}/bin/cloister-dbus-validate --list --quiet"
    )

    def wait_until_contains(command, needle, label):
        machine.wait_until_succeeds(command)
        assert_contains(machine, command, needle, label)

    wait_until_contains(raw_list_cmd, "org.example.Service", "raw bus exposes allowed service")
    assert_contains(machine, raw_list_cmd, "org.example.Secret", "raw bus exposes denied service before proxy")
    assert_contains(machine, raw_list_cmd, "org.freedesktop.portal.Desktop", "raw bus exposes portal service")

    wait_until_contains(proxy_list_cmd, "org.example.Service", "proxy bus exposes allowed service")
    wait_until_contains(proxy_list_cmd, "org.freedesktop.portal.Desktop", "proxy bus exposes portal service")
    assert_not_contains(machine, proxy_list_cmd, "org.example.Secret", "proxy bus hides denied service")
    machine.succeed(
        "${pkgs.util-linux}/bin/runuser -u tester -- env DBUS_SESSION_BUS_ADDRESS=" + proxy_bus + " "
        + "${pkgs.dbus}/bin/dbus-send --session --dest=org.example.Service / org.freedesktop.DBus.Peer.Ping"
    )
    machine.succeed(
        "${pkgs.util-linux}/bin/runuser -u tester -- env DBUS_SESSION_BUS_ADDRESS=" + proxy_bus + " "
        + "${pkgs.dbus}/bin/dbus-send --session --dest=org.freedesktop.portal.Desktop / org.freedesktop.DBus.Peer.Ping"
    )
  '';
})
