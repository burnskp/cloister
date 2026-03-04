# Evince (GNOME Document Viewer) sandbox example
#
# A sandboxed PDF viewer with no network access. Opens documents read-only
# from the working directory with GPU-accelerated rendering, portal file
# dialogs, and printing support.
#
# Usage:
#  cl-evince evince document.pdf    # open a PDF from the current directory
#  evince document.pdf              # same, via host-side command wrapping
#
{ pkgs, ... }:
{
  cloister.sandboxes.evince = {
    shell = {
      hostConfig = false;
    };

    extraPackages = with pkgs; [ evince ];

    # Display
    gui.wayland.enable = true;

    # D-Bus / portals
    dbus.enable = true;
    dbus.portal = true;

    # Printing
    printing.enable = true;

    # App launcher
    gui.desktopEntry = {
      enable = true;
      name = "Document Viewer (Sandboxed)";
      execArgs = "%U";
      icon = "org.gnome.Evince";
      genericName = "Document Viewer";
      comment = "Sandboxed PDF and document viewer";
      categories = [
        "GNOME"
        "GTK"
        "Office"
        "Viewer"
      ];
      mimeType = [
        "application/pdf"
        "application/postscript"
        "image/vnd.djvu"
        "application/x-dvi"
      ];
    };

    # No network
    network.enable = false;

    # No SSH or git
    ssh.enable = false;
    git.enable = false;

    # Wrapped commands
    defaultCommand = [ "evince" ];
    registry.commands = [ "evince" ];
  };
}
