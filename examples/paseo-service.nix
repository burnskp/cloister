# Paseo daemon as a sandboxed service
#
# Runs `paseo daemon run` as the systemd user unit cloister-service-paseo.
# Agents that paseo spawns run in the same sandbox, so it carries the dev
# tools and agent CLIs. Repos and worktrees live under ~/cloister/paseo;
# paseo's own state is kept in ~/.local/state/cloister/paseo and appears as
# ~/.paseo inside the sandbox.
#
# Keep the service running after logout with: loginctl enable-linger "$USER"
{ config, pkgs, ... }:
{
  cloister.sandboxes.agents = {
    preset = "developer";

    # Add the agent CLIs paseo should drive (e.g. claude-code, codex).
    extraPackages = with pkgs; [
      paseo
      fd
      ripgrep
    ];

    # Agent CLI login state, shared with interactive `cl-agents` sessions.
    sandbox.state.dirs."${config.xdg.stateHome}" = [
      ".claude"
      ".codex"
    ];
  };

  cloister.services.paseo = {
    sandbox = "agents";
    command = [
      "paseo"
      "daemon"
      "run"
    ];

    # Project root: holds repos/ and worktrees/.
    workingDirectory = "cloister/paseo";

    readWrite = [
      {
        src = ".local/state/cloister/paseo";
        dest = ".paseo";
      }
    ];

    env.PASEO_LISTEN = "127.0.0.1:6767";

    # To isolate the daemon in a cloister-netns namespace instead of sharing
    # host networking (NixOS only; reach it on the namespace's veth address):
    # network.namespace = "agents";
  };
}
