{ pkgs, lib }:
let
  posix = import ./_posix.nix { inherit lib; };
  configDir = "bash";
  initExt = "bash";
in
{
  package = pkgs.bashInteractive;
  bin = "${pkgs.bashInteractive}/bin/bash";
  interactiveArgs = [ "-l" ];
  wrappedCommandShellArgs = [
    "-l"
    "-i"
  ];
  shellEnv = "/bin/bash";
  key = "bash";
  command = "bash";
  inherit configDir initExt;

  symlinks = [
    # No shell-specific symlinks needed — the base sandbox always includes
    # bash (/bin/bash, /bin/sh) since it's required for the entry script
    # regardless of the chosen shell.
  ];

  configBinds = [
    {
      src = "$HOME/.bashrc";
      try = true;
    }
    {
      src = "$HOME/.bash_profile";
      try = true;
    }
    {
      src = "$HOME/.config/bash";
      try = true;
    }
  ];

  managedConfigKeys = [
    ".bashrc"
    ".bash_profile"
    "bash"
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
