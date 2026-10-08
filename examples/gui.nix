# GUI sandbox preset example
#
# Wayland desktop application sandbox with notifications enabled but no SSH or
# git access by default.
{ pkgs, ... }:
{
  cloister.sandboxes.evince = {
    preset = "gui";

    extraPackages = with pkgs; [ evince ];

    defaultCommand = [ "evince" ];

    gui.desktopEntry = {
      name = "Evince (Sandboxed)";
      icon = "org.gnome.Evince";
      categories = [ "Office" ];
      mimeTypes = [ "application/pdf" ];
    };

    registry.commands = [ "evince" ];
  };
}
