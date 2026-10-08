# Chromium sandbox example
#
# A sandboxed Chromium browser with GPU acceleration, audio,
# xdg-desktop-portal integration, and a generated .desktop entry for your
# app launcher.
#
# Usage:
#  cl-chromium
#  cl-chromium https://example.com
#
# Add this to your home-manager config alongside the cloister module import:
#
#  imports = [ cloister.homeManagerModules.default ];
#  cloister.enable = true;
#
# Then merge the sandboxes definition below into your cloister config.
{
  config,
  pkgs,
  ...
}:
{
  cloister.sandboxes.chromium = {
    preset = "chromium";

    extraPackages = with pkgs; [ chromium ];

    # Audio (PipeWire with filtering - only expose speakers by default)
    audio.pipewire = {
      dbus.enable = false;
      # audioOut is true by default; audioIn/videoIn/control/routing are false
    };

    # D-Bus / portals - matches Flatpak's Chromium policy
    dbus = {
      enable = true;
      notifications = true;
      portal = {
        fileChooser = true;
        openUri = true;
      };
    };

    # App launcher integration
    gui.desktopEntry = {
      name = "Chromium (Sandboxed)";
      execArgs = "%U";
      icon = "chromium";
      genericName = "Web Browser";
      comment = "Sandboxed Chromium via cloister";
      categories = [
        "Network"
        "WebBrowser"
      ];
      mimeTypes = [
        "text/html"
        "application/xhtml+xml"
        "x-scheme-handler/http"
        "x-scheme-handler/https"
      ];
      startupNotify = true;
    };

    # Persistence
    sandbox = {
      # Browser sandboxes should use one stable profile so repeated launches
      # can hand URLs to the existing Chromium instance.
      #
      # Chromium-family browsers also use a singleton socket under TMPDIR on
      # Linux, so give them a shared temp dir instead of the sandbox's private
      # /tmp when you want "open in existing window/tab" behavior.
      bindWorkingDirectory = false;
      state.dirs."${config.xdg.stateHome}" = [
        ".config/chromium"
        ".cache/chromium"
        ".cache/chromium-tmp"
        "Downloads"
      ];

      env = {
        TMPDIR = "${config.xdg.cacheHome}/chromium-tmp";
      };

    };

    # Wrapped commands
    # Typing `chromium` in your host shell routes through the sandbox
    defaultCommand = [
      "chromium"
      "--ozone-platform=wayland"
    ];
    registry.commands = [ "chromium" ];
  };
}
