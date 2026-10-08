{ pkgs, lib }:
let
  posix = import ./_posix.nix { inherit lib; };
  configDir = "zsh";
  initExt = "zsh";
in
{
  package = pkgs.zsh;
  bin = "${pkgs.zsh}/bin/zsh";
  interactiveArgs = [ "-i" ];
  wrappedCommandShellArgs = [ "-i" ];
  shellEnv = "/bin/zsh";
  key = "zsh";
  command = "zsh";
  inherit configDir initExt;

  symlinks = [
    {
      target = "${pkgs.zsh}/bin/zsh";
      link = "/bin/zsh";
    }
    {
      target = "/bin/zsh";
      link = "/run/current-system/sw/bin/zsh";
    }
  ];

  configBinds = [
    {
      src = "$HOME/.config/zsh";
      try = true;
    }
    {
      src = "$HOME/.zshrc";
      try = true;
    }
    {
      src = "$HOME/.zshenv";
      try = true;
    }
  ];

  managedConfigKeys = [
    "zsh"
    ".zshrc"
    ".zshenv"
  ];

  inherit (posix)
    renderAlias
    renderFunction
    renderOutsideCommand
    renderOutsideFunction
    renderOutsideRunner
    ;

  renderWrapperInit = posix.mkRenderWrapperInit { inherit configDir initExt; };
}
