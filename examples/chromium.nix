# Chromium sandbox example
#
# A sandboxed Chromium browser with GPU acceleration, audio, FIDO2/WebAuthn,
# xdg-desktop-portal integration, and a generated .desktop entry for your
# app launcher.
#
# Usage:
#  cl-chromium                    # launch Chromium interactively
#  cl-chromium chromium %U        # explicit command (args auto-set in .desktop file)
#
# Add this to your home-manager config alongside the cloister module import:
#
#  imports = [ cloister.homeManagerModules.default ];
#  cloister.enable = true;
#
# Then merge the sandboxes definition below into your cloister config.
{ pkgs, ... }:
{
  cloister.sandboxes.chromium = {
    shell = {
      name = "bash";
    };

    extraPackages = with pkgs; [ chromium ];

    # Display & rendering
    gui.wayland = {
      enable = true;
      # securityContext.enable = true;  # default — recommended
    };
    # gui.gpu.enable is auto-set to true when wayland is enabled
    # gui.gpu.shm = true;  # default — /dev/shm needed by Chromium's multi-process IPC

    # Audio (PipeWire with filtering — only expose speakers)
    audio.pipewire = {
      enable = true;
      filters.enable = true;
      # audioOut is true by default; audioIn/videoIn/control/routing are false
    };

    # FIDO2 / WebAuthn
    fido2.enable = true;

    # D-Bus / portals
    # Matches Flatpak's Chromium policy: talk (not see) for services
    # Chromium interacts with, wildcard call/broadcast for portals,
    # and MPRIS ownership for media controls.
    # Flatpak also grants org.freedesktop.secrets and org.kde.kwalletd{5,6}
    # — intentionally omitted here to avoid exposing credential stores.
    dbus = {
      enable = true;
      portal = true;
      policies = {
        talk = [
          "org.freedesktop.Notifications"
          "org.freedesktop.ScreenSaver"
          "org.freedesktop.FileManager1"
          "org.gnome.SessionManager"
          "com.canonical.AppMenu.Registrar"
        ];
        own = [ "org.mpris.MediaPlayer2.chromium.*" ];
        call."org.freedesktop.portal.*" = [ "*" ];
        broadcast."org.freedesktop.portal.*" = [ "*@/org/freedesktop/portal/*" ];
      };
    };

    # App launcher integration
    gui.desktopEntry = {
      enable = true;
      name = "Chromium (Sandboxed)";
      execArgs = "%U";
      icon = "chromium";
      genericName = "Web Browser";
      comment = "Sandboxed Chromium via cloister";
      categories = [
        "Network"
        "WebBrowser"
      ];
      mimeType = [
        "text/html"
        "application/xhtml+xml"
        "x-scheme-handler/http"
        "x-scheme-handler/https"
      ];
      startupNotify = true;
    };

    # Network
    network.enable = true;

    # Persistence
    sandbox = {
      # Chromium profile survives across sessions, isolated per-project
      extraBinds.perDir = [
        ".config/chromium"
        ".cache/chromium"
      ];

      # Allow Chromium's internal sandbox (seccomp-bpf + chroot for renderer
      # process isolation). Cloister's outer bwrap provides namespace isolation;
      # Chromium's inner sandbox further restricts individual renderer
      # processes via seccomp-bpf — a layered defense model.
      seccomp.allowChromiumSandbox = true;

      # Chromium flags
      env = {
        CHROMIUM_FLAGS = "--enable-features=UseOzonePlatform --ozone-platform=wayland";
      };
    };

    # Security
    ssh.enable = false;
    git.enable = false; # browsers don't need git config

    # Wrapped commands
    # Typing `chromium` in your host shell routes through the sandbox
    defaultCommand = [ "chromium" ];
    registry.commands = [ "chromium" ];
  };
}
