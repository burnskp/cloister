{
  checks,
  hm,
  ...
}:
let
  eval = hm {
    cloister = {
      enable = true;
      sandboxes = {
        hard.preset = "hardened";
        hardAudio = {
          preset = "hardened";
          audio.pipewire.enable = true;
        };
        dev.preset = "developer";
        gui = {
          preset = "gui";
          gui.enable = false;
        };
        chromium.preset = "chromium";
      };
    };
  };

  overrideEval = hm {
    cloister = {
      enable = true;
      sandboxes = {
        hard = {
          preset = "hardened";
          network.enable = true;
        };
        dev = {
          preset = "developer";
          git.enable = false;
          dbus.notifications = false;
        };
        gui = {
          preset = "gui";
          dbus.enable = false;
          dbus.notifications = false;
          gui.enable = false;
        };
        chromium = {
          preset = "chromium";
          audio.pipewire.pulseOnly = false;
          sandbox.seccomp.allowChromiumSandbox = false;
        };
      };
    };
  };

  plainEval = hm {
    cloister = {
      enable = true;
      sandboxes.plain = { };
    };
  };

  inherit (eval.config.cloister) sandboxes;
  overrideSandboxes = overrideEval.config.cloister.sandboxes;
  plainSandbox = plainEval.config.cloister.sandboxes.plain;
in
checks.mkCheck "test-cloister-presets" [
  (checks.expectFalse "hardened disables network" sandboxes.hard.network.enable)
  (checks.expectFalse "hardened disables ssh" sandboxes.hard.ssh.enable)
  (checks.expectFalse "hardened disables dbus" sandboxes.hard.dbus.enable)
  (checks.expectTrue "hardened pipewire opt-in enables filtered audio" sandboxes.hardAudio.audio.pipewire.enable)
  (checks.expectTrue "developer enables git" sandboxes.dev.git.enable)
  (checks.expectTrue "developer enables notifications default" sandboxes.dev.dbus.notifications)
  (checks.expectTrue "developer enables ssh" sandboxes.dev.ssh.enable)
  (checks.expectFalse "gui override beats preset default" sandboxes.gui.gui.enable)
  (checks.expectTrue "gui preset enables dbus" sandboxes.gui.dbus.enable)
  (checks.expectTrue "gui preset enables notifications" sandboxes.gui.dbus.notifications)
  (checks.expectTrue "chromium enables pulseOnly" sandboxes.chromium.audio.pipewire.pulseOnly)
  (checks.expectTrue "chromium enables seccomp chromium mode" sandboxes.chromium.sandbox.seccomp.allowChromiumSandbox)
  (checks.expectTrue "chromium enables filtered pipewire" sandboxes.chromium.audio.pipewire.enable)
  (checks.expectTrue "hardened override can re-enable network" overrideSandboxes.hard.network.enable)
  (checks.expectFalse "developer override can disable git" overrideSandboxes.dev.git.enable)
  (checks.expectFalse "developer override can disable notifications" overrideSandboxes.dev.dbus.notifications)
  (checks.expectFalse "gui override can disable dbus" overrideSandboxes.gui.dbus.enable)
  (checks.expectFalse "gui override can disable gui" overrideSandboxes.gui.gui.enable)
  (checks.expectFalse "chromium override can disable pulseOnly" overrideSandboxes.chromium.audio.pipewire.pulseOnly)
  (checks.expectFalse "chromium override can disable chromium seccomp mode" overrideSandboxes.chromium.sandbox.seccomp.allowChromiumSandbox)
  (checks.expectFalse "plain sandbox keeps dbus disabled" plainSandbox.dbus.enable)
]
