{ config, pkgs, ... }:
{
  cloister.sandboxes.editor-dev = {
    shell.name = "zsh";

    extraPackages = with pkgs; [
      helix
      git
      ripgrep
      fd
      jq
      nil
      lua-language-server
    ];

    git.enable = true;

    sandbox = {
      nixStore.mode = "image-store";

      state.dirs."${config.xdg.stateHome}" = [
        ".local/share/helix"
        ".cache/helix"
      ];

      managed = [ "helix/languages.toml" ];

      env = {
        EDITOR = "hx";
        VISUAL = "hx";
      };
    };

    registry.commands = [ "hx" ];
  };
}
