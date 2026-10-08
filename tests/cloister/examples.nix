{
  checks,
  hm,
  lib,
  ...
}:
let
  developer = hm {
    cloister.enable = true;
    imports = [ ../../examples/developer.nix ];
  };

  gui = hm {
    cloister.enable = true;
    imports = [ ../../examples/gui.nix ];
  };

  hardened = hm {
    cloister.enable = true;
    imports = [ ../../examples/hardened.nix ];
  };

  untrusted = hm {
    cloister.enable = true;
    imports = [ ../../examples/untrusted.nix ];
  };

  shellCustomRc = hm {
    imports = [ ../../examples/shell-custom-rc.nix ];
  };

  nixdev = hm {
    cloister.enable = true;
    imports = [ ../../examples/nixdev.nix ];
  };

  imageStore = hm {
    cloister.enable = true;
    xdg.configFile."helix/languages.toml".text = "[[language]]\nname = \"nix\"\n";
    imports = [ ../../examples/image-store.nix ];
  };

  evince = hm {
    cloister.enable = true;
    imports = [ ../../examples/evince.nix ];
  };

  discord = hm {
    cloister.enable = true;
    imports = [ ../../examples/discord.nix ];
  };

  chromium = hm {
    cloister.enable = true;
    imports = [ ../../examples/chromium.nix ];
  };

  workerBroker = hm {
    cloister.enable = true;
    imports = [ ../../examples/worker-broker.nix ];
  };

  shellCustomRcConfig = shellCustomRc.config.cloister._internal.sandboxConfigs.dev;
  shellCustomRcStaticArgs = builtins.toJSON shellCustomRcConfig.static_bwrap_args;
  packageNames = packages: map lib.getName packages;
in
checks.mkCheck "test-cloister-examples" [
  (checks.expectEq "developer example sets preset" "developer"
    developer.config.cloister.sandboxes.dev.preset
  )
  (checks.expectEq "gui example sets default command" [
    "evince"
  ] gui.config.cloister.sandboxes.evince.defaultCommand)
  (checks.expectEq "hardened example sets preset" "hardened"
    hardened.config.cloister.sandboxes.hardened.preset
  )
  (checks.expectEq "untrusted example sets hardened preset" "hardened"
    untrusted.config.cloister.sandboxes.untrusted.preset
  )
  (checks.expectContains "shell custom rc example binds dev zshenv" "dev.zshenv"
    shellCustomRcStaticArgs
  )
  (checks.expectContains "shell custom rc example binds dev zshrc" "dev.zshrc"
    shellCustomRcStaticArgs
  )
  (checks.expectEq "shell custom rc example disables host config for work sandbox" false
    shellCustomRc.config.cloister.sandboxes.work.shell.hostConfig
  )
  (checks.expectTrue "nixdev example enables ssh" nixdev.config.cloister.sandboxes.nixdev.ssh.enable)
  (checks.expectEq "image store example switches nix store mode" "image-store"
    imageStore.config.cloister.sandboxes.editor-dev.sandbox.nixStore.mode
  )
  (checks.expectEq "evince example disables network" false
    evince.config.cloister.sandboxes.evince.network.enable
  )
  (checks.expectEq "discord example enables camera portal" true
    discord.config.cloister.sandboxes.discord.dbus.portal.camera
  )
  (checks.expectEq "chromium example wraps chromium" [
    "chromium"
  ] chromium.config.cloister.sandboxes.chromium.registry.commands)
  (checks.expectTrue "worker broker example enables broker" (
    workerBroker.config.cloister.sandboxes.dev.workerBroker.profiles != { }
  ))
  (checks.expectEq "worker broker example enables dbus for dev" true
    workerBroker.config.cloister.sandboxes.dev.dbus.enable
  )
  (checks.expectEq "worker broker example enables notifications for dev" true
    workerBroker.config.cloister.sandboxes.dev.dbus.notifications
  )
  (checks.expectEq "worker broker example maps overlay profile to overlay worker" "worker-overlay"
    workerBroker.config.cloister.sandboxes.dev.workerBroker.profiles.overlay.sandbox
  )
  (checks.expectEq "worker broker example uses overlay workspace mode" "project-overlay"
    workerBroker.config.cloister.sandboxes.dev.workerBroker.profiles.overlay.workspace.mode
  )
  (checks.expectEq "worker broker example maps rw profile to rw worker" "worker-rw"
    workerBroker.config.cloister.sandboxes.dev.workerBroker.profiles.rw.sandbox
  )
  (checks.expectEq "worker broker example uses rw workspace mode" "project-rw"
    workerBroker.config.cloister.sandboxes.dev.workerBroker.profiles.rw.workspace.mode
  )
  (checks.expectContains "worker broker example preserves fd from developer base" "fd" (
    builtins.toJSON (packageNames workerBroker.config.cloister.sandboxes.dev.extraPackages)
  ))
  (checks.expectContains "worker broker example preserves nil from developer base" "nil" (
    builtins.toJSON (packageNames workerBroker.config.cloister.sandboxes.dev.extraPackages)
  ))
  (checks.expectContains "worker broker example shares nodejs on dev" "nodejs" (
    builtins.toJSON (packageNames workerBroker.config.cloister.sandboxes.dev.extraPackages)
  ))
  (checks.expectContains "worker broker example shares nodejs on overlay worker" "nodejs" (
    builtins.toJSON (packageNames workerBroker.config.cloister.sandboxes.worker-overlay.extraPackages)
  ))
  (checks.expectContains "worker broker example shares nodejs on rw worker" "nodejs" (
    builtins.toJSON (packageNames workerBroker.config.cloister.sandboxes.worker-rw.extraPackages)
  ))
  (checks.expectContains "worker broker example shares neovim on dev" "neovim" (
    builtins.toJSON (packageNames workerBroker.config.cloister.sandboxes.dev.extraPackages)
  ))
  (checks.expectContains "worker broker example shares neovim on overlay worker" "neovim" (
    builtins.toJSON (packageNames workerBroker.config.cloister.sandboxes.worker-overlay.extraPackages)
  ))
  (checks.expectContains "worker broker example shares neovim on rw worker" "neovim" (
    builtins.toJSON (packageNames workerBroker.config.cloister.sandboxes.worker-rw.extraPackages)
  ))
  (checks.expectEq "worker broker example enables ssh for rw worker" true
    workerBroker.config.cloister.sandboxes.worker-rw.ssh.enable
  )
  (checks.expectEq "worker broker example disables host config for rw worker" false
    workerBroker.config.cloister.sandboxes.worker-rw.shell.hostConfig
  )
  (checks.expectEq "worker broker example enables git for rw worker" true
    workerBroker.config.cloister.sandboxes.worker-rw.git.enable
  )
  (checks.expectEq "worker broker example disables host config for overlay worker" false
    workerBroker.config.cloister.sandboxes.worker-overlay.shell.hostConfig
  )
  (checks.expectEq "worker broker example disables ssh for overlay worker" false
    workerBroker.config.cloister.sandboxes.worker-overlay.ssh.enable
  )
  (checks.expectEq "worker broker example disables git for overlay worker" false
    workerBroker.config.cloister.sandboxes.worker-overlay.git.enable
  )
  (checks.expectEq "worker broker example hardens overlay worker" "hardened"
    workerBroker.config.cloister.sandboxes.worker-overlay.preset
  )
  (checks.expectEq "worker broker example hardens rw worker" "hardened"
    workerBroker.config.cloister.sandboxes.worker-rw.preset
  )
]
