# Evince (GNOME Document Viewer) sandbox example
#
# A sandboxed PDF viewer with no network access. Opens documents from the
# working directory with GPU-accelerated rendering and portal file
# dialogs.
#
# Usage:
#  cl-evince document.pdf           # open a PDF from the current directory
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
    gui.enable = true;

    # D-Bus / portals
    dbus.enable = true;
    dbus.portal = {
      fileChooser = true;
      openUri = true;
    };

    # App launcher
    gui.desktopEntry = {
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
      mimeTypes = [
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
