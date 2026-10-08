# Sandbox-local Home Manager profiles

Cloister can use Home Manager as a pure generator for sandbox-local files and packages. This lets a sandbox reuse Home Manager modules such as `programs.bat`, `programs.starship`, or `programs.direnv` without activating that configuration on the host.

Cloister consumes only these outputs from the sandbox profile:

- `home.packages` - appended to the sandbox `PATH`
- `xdg.configFile` - mounted read-only below `$HOME/.config` inside the sandbox
- `home.file` - mounted read-only below `$HOME` inside the sandbox

Cloister does **not** run the profile's activation package. The generated files are not linked into the host home directory, and the packages are not installed into the host user profile.

## Module-list usage

When you want Cloister to evaluate a sandbox-local Home Manager module list, pass Home Manager's configuration builder to Cloister once:

```nix
{
  cloister = {
    enable = true;
    homeManager.builder = inputs.home-manager.lib.homeManagerConfiguration;

    sandboxes.dev = {
      preset = "developer";

      homeManager = {
        enable = true;
        modules = [
          {
            programs.bat.enable = true;
            programs.starship.enable = true;
            programs.direnv.enable = true;

            home.packages = with pkgs; [
              fd
              jq
              ripgrep
            ];
          }
        ];
      };
    };
  };
}
```

Cloister supplies these defaults to the nested Home Manager evaluation:

```nix
home.username = config.home.username;
home.homeDirectory = config.home.homeDirectory;
home.stateVersion = config.home.stateVersion;
```

Your sandbox modules can override them if needed.

If your nested modules need custom arguments, use `extraSpecialArgs`:

```nix
cloister.sandboxes.dev.homeManager.extraSpecialArgs = {
  inherit inputs;
};
```

## Pre-evaluated config usage

You can also define a separate Home Manager configuration yourself and hand Cloister its evaluated `config`:

```nix
# flake output sketch
homeConfigurations.cloister-dev = home-manager.lib.homeManagerConfiguration {
  inherit pkgs;
  modules = [
    {
      home.username = "alice";
      home.homeDirectory = "/home/alice";
      home.stateVersion = "25.05";

      programs.bat.enable = true;
      home.packages = with pkgs; [ jq ripgrep ];
    }
  ];
};
```

Then consume it from your real host Home Manager configuration:

```nix
cloister.sandboxes.dev.homeManager = {
  enable = true;
  config = inputs.self.homeConfigurations.cloister-dev.config;
};
```

Do not run `home-manager switch` for the sandbox-only configuration unless you also want it to manage a real host profile.

## Selectively disabling files or packages

The extracted pieces can be toggled independently:

```nix
cloister.sandboxes.dev.homeManager = {
  enable = true;
  includeFiles = true;
  includePackages = false;
  modules = [ ./sandbox-home.nix ];
};
```

## Interaction with existing managed files

Sandbox-local Home Manager files are merged with `sandbox.managed`. Duplicate destinations are rejected during evaluation, just like duplicate `sandbox.managed` entries.

For example, this should be avoided because both sides may produce the same destination:

```nix
{
  xdg.configFile."bat/config".text = "...";

  cloister.sandboxes.dev = {
    sandbox.managed = [ "bat/config" ];
    homeManager = {
      enable = true;
      modules = [ { programs.bat.enable = true; } ];
    };
  };
}
```

Prefer configuring a file in one place.

## Limitations

This feature intentionally supports only files and packages. The following Home Manager outputs are not activated by Cloister:

- activation scripts
- `systemd.user` services, timers, and sockets
- dconf activation
- imperative setup performed by activation hooks
- session variables, unless a program also writes them into a managed file
- secret material that depends on activation-time host writes

Some Home Manager modules are still useful because their output is just files and packages. Others primarily work through activation scripts or user services; those modules may evaluate but will not have their usual runtime effect inside the sandbox.

Generated files are mounted read-only. If an application needs to mutate its config, put that mutable path in `sandbox.state.dirs`, `sandbox.state.files`, `sandbox.state.projectDirs`, or use `sandbox.copies` for an initial writable copy.
