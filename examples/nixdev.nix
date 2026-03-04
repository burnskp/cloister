# Nix development sandbox example
#
# A sandbox for working on NixOS/home-manager configurations. Includes
# editors, formatters, LSP, and the Nix toolchain with persistent caches.
#
# Usage:
#  cd ~/nixos-config && cl-nixdev        # interactive shell at your config repo
#  cl-nixdev nvim flake.nix              # edit a file directly
#  nvim flake.nix                        # same, via host-side command wrapping
#
{ pkgs, ... }:
{
  cloister.sandboxes.nixdev = {
    shell = {
      name = "zsh";
    };

    extraPackages = with pkgs; [
      # Editors
      neovim

      # Nix toolchain
      nixfmt-rfc-style
      nil # Nix LSP
      nix-diff
      nix-tree
      nvd # NixOS version diff
      nix-output-monitor # nom — pretty build output

      # Useful utilities
      ripgrep
      fd
      jq
      tree
      delta # git diff pager
    ];

    # Network (needed for flake fetches and nix build)
    network.enable = true;

    # SSH (needed for private flake inputs)
    ssh.enable = true;

    # Git config (needed for commits, signing, etc.)
    git.enable = true;

    # Notifications
    dbus = {
      enable = true;
      policies.talk = [ "org.freedesktop.Notifications" ];
    };

    sandbox = {
      # Persistence
      extraBinds = {
        # Nix store and daemon socket — required for nix commands to work
        required.rw = [ ".local/state/nix" ];

        # Editor state — optional, survives across sessions if present
        optional.rw = [
          ".local/share/nvim"
          ".local/state/nvim"
        ];

        # Per-directory state — each config repo gets its own nix eval cache
        perDir = [ ".cache/nix" ];
      };

      # Home-manager managed config files
      # Uncomment entries that match your xdg.configFile / home.file setup
      # extraBinds.managedFile = [
      #  "nvim"           # neovim config tree
      #  "starship.toml"  # prompt config
      #  "bat"            # bat theme/config
      # ];

      # Environment
      env = {
        EDITOR = "nvim";
        VISUAL = "nvim";
        MANPAGER = "nvim +Man!";
      };
    };

    # Registry
    registry = {
      commands = [
        "nvim"
        "nix-tree"
        "nix-diff"
      ];

      aliases = {
        nb = "nix build --no-link --print-out-paths";
        nfc = "nix flake check";
        nfu = "nix flake update";
        nfmt = "nixfmt .";
      };

      functions = {
        # Build and diff against current system generation
        ndiff = ''
          local new
          new=$(nix build --no-link --print-out-paths "$@" 2>&1) || { echo "$new" >&2; return 1; }
          nvd diff /run/current-system "$new"
        '';
      };
    };
  };
}
