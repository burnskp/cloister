{ testLib }:
let
  inherit (testLib) evalConfig mkCheck lib;

  # Config with init text and aliases for wrapper content tests
  zshConfig = evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            shell = {
              name = "zsh";
            };
            init.text = "# custom init line";
            registry.aliases.ll = "ls -la";
          };
        };
      }
    ];
  };

  bashConfig = evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            shell = {
              name = "bash";
            };
            init.text = "# bash init line";
            registry.aliases.ll = "ls -la";
          };
        };
      }
    ];
  };

  disabledConfig = evalConfig { modules = [ { cloister.enable = false; } ]; };

  # Helper: evaluate a desktop entry config with Wayland enabled.
  # `sandboxName` is the attrset key; `overrides` are recursively merged
  # into the sandbox config (add gui.desktopEntry fields here).
  mkDesktopEntryConfig =
    sandboxName: overrides:
    evalConfig {
      modules = [
        {
          cloister = {
            enable = true;
            sandboxes.${sandboxName} = lib.recursiveUpdate {
              gui.wayland = {
                enable = true;
                securityContext.enable = false;
              };
            } overrides;
          };
        }
      ];
    };

  # Multi-sandbox config for wrapper tests
  multiConfig = evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes = {
            a = {
              shell = {
                name = "zsh";
              };
              registry.commands = [ "nvim" ];
            };
            b = {
              shell = {
                name = "zsh";
              };
              registry.commands = [ "cargo" ];
            };
          };
        };
      }
    ];
  };
in
{
  # ── Zsh wrapper tests ─────────────────────────────────────────────────

  zsh-init-contains-init-text = mkCheck "wrappers-zsh-init-text" (
    lib.hasInfix "# custom init line" zshConfig.xdg.configFile."zsh/cloister-test.zsh".text
  );

  zsh-init-contains-registry = mkCheck "wrappers-zsh-init-registry" (
    lib.hasInfix "alias ll='ls -la'" zshConfig.xdg.configFile."zsh/cloister-test.zsh".text
  );

  zsh-wrapper-init-regex = mkCheck "wrappers-zsh-init-regex" (
    lib.hasInfix "^[A-Za-z0-9_-]+$" zshConfig.programs.zsh.initContent
  );

  # ── Bash wrapper tests ────────────────────────────────────────────────

  bash-init-file-exists = mkCheck "wrappers-bash-init-exists" (
    lib.hasInfix "# bash init line" bashConfig.xdg.configFile."bash/cloister-test.bash".text
  );

  bash-init-contains-registry = mkCheck "wrappers-bash-init-registry" (
    lib.hasInfix "alias ll='ls -la'" bashConfig.xdg.configFile."bash/cloister-test.bash".text
  );

  # ── Disabled test ─────────────────────────────────────────────────────

  disabled-no-packages = mkCheck "wrappers-disabled-no-packages" (
    let
      hasCloister = builtins.any (
        p:
        let
          pname = p.pname or p.name or "";
        in
        lib.hasPrefix "cl-" pname
      ) disabledConfig.home.packages;
    in
    !hasCloister
  );

  # ── Multi-sandbox wrapper tests ───────────────────────────────────────

  multi-sandbox-both-scripts = mkCheck "wrappers-multi-sandbox-both-scripts" (
    let
      names = map (p: p.pname or p.name or "") multiConfig.home.packages;
    in
    builtins.elem "cl-a" names && builtins.elem "cl-b" names
  );

  multi-sandbox-init-files = mkCheck "wrappers-multi-sandbox-init-files" (
    multiConfig.xdg.configFile ? "zsh/cloister-a.zsh"
    && multiConfig.xdg.configFile ? "zsh/cloister-b.zsh"
  );

  # ── Desktop entry tests ──────────────────────────────────────────────

  desktop-entry-exists = mkCheck "wrappers-desktop-entry-exists" (
    let
      config = mkDesktopEntryConfig "firefox" {
        defaultCommand = [ "firefox" ];
        gui.desktopEntry = {
          enable = true;
          name = "Firefox";
          icon = "firefox";
        };
      };
    in
    config.xdg.desktopEntries ? "cl-firefox"
  );

  desktop-entry-name = mkCheck "wrappers-desktop-entry-name" (
    let
      config = mkDesktopEntryConfig "firefox" {
        defaultCommand = [ "firefox" ];
        gui.desktopEntry = {
          enable = true;
          name = "Firefox";
        };
      };
    in
    config.xdg.desktopEntries."cl-firefox".name == "Firefox"
  );

  desktop-entry-exec = mkCheck "wrappers-desktop-entry-exec" (
    let
      config = mkDesktopEntryConfig "firefox" {
        defaultCommand = [ "firefox" ];
        gui.desktopEntry = {
          enable = true;
          name = "Firefox";
          execArgs = "%u";
        };
      };
      execLine = config.xdg.desktopEntries."cl-firefox".exec;
    in
    lib.hasPrefix "/nix/store/" execLine
    && lib.hasInfix "/bin/cl-firefox" execLine
    && lib.hasSuffix "%u" execLine
  );

  desktop-entry-icon = mkCheck "wrappers-desktop-entry-icon" (
    let
      config = mkDesktopEntryConfig "firefox" {
        defaultCommand = [ "firefox" ];
        gui.desktopEntry = {
          enable = true;
          name = "Firefox";
          icon = "firefox";
        };
      };
    in
    config.xdg.desktopEntries."cl-firefox".icon == "firefox"
  );

  desktop-entry-default-name = mkCheck "wrappers-desktop-entry-default-name" (
    let
      config = mkDesktopEntryConfig "myapp" { gui.desktopEntry.enable = true; };
      execLine = config.xdg.desktopEntries."cl-myapp".exec;
    in
    config.xdg.desktopEntries."cl-myapp".name == "cl-myapp"
    && lib.hasPrefix "/nix/store/" execLine
    && lib.hasSuffix "/bin/cl-myapp" execLine
  );

  desktop-entry-disabled = mkCheck "wrappers-desktop-entry-disabled" (
    let
      config = mkDesktopEntryConfig "firefox" { gui.desktopEntry.enable = false; };
    in
    !(config.xdg.desktopEntries ? "cl-firefox")
  );
}
