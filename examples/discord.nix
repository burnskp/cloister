# Discord sandbox example
#
# A sandboxed Discord client with Wayland, GPU, filtered PipeWire audio
# (speakers + microphone + camera), and D-Bus policies aligned with the Flatpak
# default finish-args.
#
# Usage:
#  cl-discord
#  cl-discord --enable-features=WaylandWindowDecorations
#
{ config, pkgs, ... }:
{
  cloister.sandboxes.discord = {
    shell = {
      hostConfig = false;
    };

    extraPackages = with pkgs; [ discord ];

    # Display & rendering
    gui.enable = true;

    # Audio & screen sharing (PipeWire with filtering)
    audio.pipewire = {
      enable = true;
      filters = {
        audioIn = true; # microphone for voice chat
        videoIn = true; # camera for video calls
      };
      # audioOut is true by default
    };

    # D-Bus policies (mirrors Flatpak defaults)
    dbus = {
      enable = true;
      notifications = true;
      portal = {
        screencast = true;
        camera = true;
        openUri = true;
      };
      rawPolicies.talk = [
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
      state.dirs."${config.xdg.stateHome}" = [
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
