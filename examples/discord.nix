# Discord sandbox example
#
# A sandboxed Discord client with Wayland/X11, audio, GPU, portal-based screen
# sharing (via PipeWire), and D-Bus policies aligned with the Flatpak default
# finish-args.
#
# Usage:
#  cl-discord
#  cl-discord discord
#
{ pkgs, ... }:
{
  cloister.sandboxes.discord = {
    shell = {
      hostConfig = false;
    };

    extraPackages = with pkgs; [ discord ];

    # Display & rendering
    gui.wayland.enable = true;

    # Audio
    audio.pulseaudio.enable = true;
    audio.pipewire.enable = true; # portal-based screen sharing

    # D-Bus policies (mirrors Flatpak defaults)
    dbus = {
      enable = true;
      portal = true;
      policies.talk = [
        "org.freedesktop.ScreenSaver"
        "org.kde.StatusNotifierWatcher"
        "com.canonical.AppMenu.Registrar"
        "com.canonical.indicator.application"
        "com.canonical.Unity"
      ];
    };

    # Network
    network.enable = true;

    # Persistence
    sandbox = {
      bindWorkingDirectory = false;
      extraBinds.perDir = [
        ".config/discord"
        ".cache/discord"
      ];
      seccomp.allowChromiumSandbox = true;
    };

    # Security
    ssh.enable = false;
    git.enable = false;

    # Wrapped commands
    defaultCommand = [ "discord" ];
    registry.commands = [ "discord" ];
  };
}
