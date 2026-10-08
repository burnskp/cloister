# Developer sandbox preset example
#
# Trusted day-to-day development sandbox with network, git, SSH, and desktop
# notifications enabled.
{ pkgs, ... }:
{
  cloister.sandboxes.dev = {
    preset = "developer";

    extraPackages = with pkgs; [
      fd
      git
      nil
      opencode
      ripgrep
    ];

    registry = {
      commands = [
        "git"
        "nvim"
      ];

      interactiveCommands = [ "opencode" ];
    };
  };
}
