{ pkgs, lib }:
{
  zsh = import ./_shells/zsh.nix { inherit pkgs lib; };
  bash = import ./_shells/bash.nix { inherit pkgs lib; };
}
