{
  checks,
  hm,
  lib,
  pkgs,
  ...
}:
let
  zshEnvFile = pkgs.writeText "cloister-test-zshenv" ''
    export FROM_ZSHENV=1
  '';

  zshRcFile = pkgs.writeText "cloister-test-zshrc" ''
    export FROM_ZSHRC=1
  '';

  bashEnvFile = pkgs.writeText "cloister-test-bashenv" ''
    export FROM_BASHENV=1
  '';

  bashRcFile = pkgs.writeText "cloister-test-bashrc" ''
    export FROM_BASHRC=1
  '';

  profileFile = pkgs.writeText "cloister-test-profile" ''
    export FROM_PROFILE=1
  '';

  eval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        defaultCommand = [ "nvim" ];
        registry = {
          commands = [
            "nvim"
            "git"
          ];
          aliases.ll = "ls -l";
          functions.greet = ''
            printf '%s\\n' "hello"
          '';
          noWrap = [ "ll" ];
        };
      };
    };
  };

  multiArgDefaultEval = hm {
    cloister = {
      enable = true;
      sandboxes.browser = {
        defaultCommand = [
          "chromium"
          "--ozone-platform=wayland"
        ];
        registry.commands = [ "chromium" ];
      };
    };
  };

  multiShellEval = hm {
    cloister = {
      enable = true;
      sandboxes = {
        one = {
          shell.name = "zsh";
          registry.commands = [ "git" ];
        };
        two = {
          shell.name = "bash";
          registry.functions.hello = "printf hi\\n";
        };
      };
    };
  };

  shellInitCommandEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        shell.name = "zsh";
        registry.interactiveCommands = [ "git" ];
      };
    };
  };

  bashInteractiveCommandEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        shell.name = "bash";
        registry.interactiveCommands = [ "git" ];
      };
    };
  };

  aliasFunctionOverlap = hm {
    cloister = {
      enable = true;
      sandboxes.dev.registry = {
        aliases.same = "ls";
        functions.same = "printf hi\\n";
      };
    };
  };

  aliasCommandOverlap = hm {
    cloister = {
      enable = true;
      sandboxes.dev.registry = {
        aliases.git = "git status";
        commands = [ "git" ];
      };
    };
  };

  functionCommandOverlap = hm {
    cloister = {
      enable = true;
      sandboxes.dev.registry = {
        functions.git = "printf hi\\n";
        commands = [ "git" ];
      };
    };
  };

  commandInteractiveOverlap = hm {
    cloister = {
      enable = true;
      sandboxes.dev.registry = {
        commands = [ "git" ];
        interactiveCommands = [ "git" ];
      };
    };
  };

  invalidSandboxName = hm {
    cloister = {
      enable = true;
      sandboxes."bad;name" = { };
    };
  };

  invalidNames = hm {
    cloister = {
      enable = true;
      sandboxes.dev.registry = {
        aliases."1bad" = "ls";
        functions."bad-name" = "printf hi\\n";
        commands = [ "1cmd" ];
      };
    };
  };

  invalidWrappedAliasValue = hm {
    cloister = {
      enable = true;
      sandboxes.dev.registry.aliases.pipe = "printf hi | cat";
    };
  };

  collisionEval = hm {
    cloister = {
      enable = true;
      sandboxes = {
        one.registry.commands = [ "git" ];
        two.registry.commands = [ "git" ];
      };
    };
  };

  customRcZshEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        shell = {
          name = "zsh";
          customRcPath = {
            zshenv = zshEnvFile;
            zshrc = zshRcFile;
          };
        };
        init.text = ''
          export FROM_INIT=1
        '';
        registry = {
          aliases.afterInit = "printf after-init\\n";
          noWrap = [ "afterInit" ];
        };
      };
    };
  };

  customRcBashEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        shell = {
          name = "bash";
          customRcPath = {
            bashenv = bashEnvFile;
            bashrc = bashRcFile;
            profile = profileFile;
          };
        };
        init.text = ''
          export FROM_INIT=1
        '';
        registry = {
          aliases.afterInit = "printf after-init\\n";
          noWrap = [ "afterInit" ];
        };
      };
    };
  };

  bashNoHostConfigEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        shell = {
          name = "bash";
          hostConfig = false;
          customRcPath = {
            bashenv = bashEnvFile;
            bashrc = bashRcFile;
            profile = profileFile;
          };
        };
        init.text = ''
          export FROM_INIT=1
        '';
      };
    };
  };

  zshNoHostConfigEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        shell = {
          name = "zsh";
          hostConfig = false;
          customRcPath = {
            zshenv = zshEnvFile;
            zshrc = zshRcFile;
          };
        };
        init.text = ''
          export FROM_INIT=1
        '';
      };
    };
  };

  zshBlankNoHostConfigEval = hm {
    cloister = {
      enable = true;
      sandboxes.evince = {
        shell = {
          name = "zsh";
          hostConfig = false;
        };
      };
    };
  };

  outsideZsh = eval.config.programs.zsh.initContent;
  outsideBash = multiShellEval.config.programs.bash.initExtra;
  multiArgOutsideZsh = multiArgDefaultEval.config.programs.zsh.initContent;
  shellInitOutsideZsh = shellInitCommandEval.config.programs.zsh.initContent;
  bashInteractiveCommandOutside = bashInteractiveCommandEval.config.programs.bash.initExtra;
  bashInteractiveCommandConfig =
    bashInteractiveCommandEval.config.cloister._internal.sandboxConfigs.dev;
  bashInteractiveCommandJson = builtins.toJSON bashInteractiveCommandConfig;
  insideRegistry = eval.config.cloister.sandboxes.dev.registry.rendered.inside;
  multiXdgFiles = multiShellEval.config.xdg.configFile;
  customRcZshInit = customRcZshEval.config.cloister.sandboxes.dev.init.rendered;
  customRcBashInit = customRcBashEval.config.cloister.sandboxes.dev.init.rendered;
  customRcZshInside = customRcZshEval.config.cloister.sandboxes.dev.registry.rendered.inside;
  customRcBashInside = customRcBashEval.config.cloister.sandboxes.dev.registry.rendered.inside;
  customRcZshConfig = ''
    ${customRcZshInit}
    ${customRcZshInside}
  '';
  customRcBashConfig = ''
    ${customRcBashInit}
    ${customRcBashInside}
  '';
  bashNoHostConfig = bashNoHostConfigEval.config.cloister._internal.sandboxConfigs.dev;
  zshNoHostConfig = zshNoHostConfigEval.config.cloister._internal.sandboxConfigs.dev;
  zshBlankNoHostConfig = zshBlankNoHostConfigEval.config.cloister._internal.sandboxConfigs.evince;
  bashNoHostConfigArgs = builtins.toJSON bashNoHostConfig.static_bwrap_args;
  bashNoHostConfigDynamic = builtins.toJSON bashNoHostConfig.dynamic_binds;
  zshNoHostConfigArgs = builtins.toJSON zshNoHostConfig.static_bwrap_args;
  zshBlankNoHostConfigArgs = builtins.toJSON zshBlankNoHostConfig.static_bwrap_args;
  zshBlankNoHostConfigDynamic = builtins.toJSON zshBlankNoHostConfig.dynamic_binds;
  zshBeforeInit = builtins.head (lib.splitString "export FROM_INIT=1" customRcZshInit);
  bashBeforeInit = builtins.head (lib.splitString "export FROM_INIT=1" customRcBashInit);
  zshBeforeRegistry = builtins.head (
    lib.splitString "alias afterInit='printf after-init\\n'" customRcZshConfig
  );
  bashBeforeRegistry = builtins.head (
    lib.splitString "alias afterInit='printf after-init\\n'" customRcBashConfig
  );

  hostConfigHooksEval = hm {
    cloister = {
      enable = true;
      sandboxes = {
        dev = {
          shell = {
            name = "zsh";
            hostConfig = true;
          };
          init.text = ''
            export DEV_ONLY=1
          '';
        };
        evince = {
          shell = {
            name = "zsh";
            hostConfig = true;
          };
        };
      };
    };
  };
  hostConfigHookXdgFiles = hostConfigHooksEval.config.xdg.configFile;
  hostConfigHookDev = hostConfigHooksEval.config.cloister._internal.sandboxConfigs.dev;
  hostConfigHookEvince = hostConfigHooksEval.config.cloister._internal.sandboxConfigs.evince;
  hostConfigHookDevArgs = builtins.toJSON hostConfigHookDev.static_bwrap_args;
  hostConfigHookDevDynamic = builtins.toJSON hostConfigHookDev.dynamic_binds;
  hostConfigHookEvinceArgs = builtins.toJSON hostConfigHookEvince.static_bwrap_args;
  hostConfigHookEvinceDynamic = builtins.toJSON hostConfigHookEvince.dynamic_binds;
in
checks.mkCheck "test-cloister-registry" [
  (checks.expectContains "registry emits outside runner helper" "__cloister_run_dev()" outsideZsh)
  (checks.expectContains "registry wraps default command" "alias nvim='__cloister_run_dev -c nvim'"
    outsideZsh
  )
  (checks.expectContains "registry wraps normal command" "alias git='__cloister_run_dev -c git'"
    outsideZsh
  )
  (checks.expectContains "registry can wrap commands via shell init"
    "alias git='__cloister_run_dev -i git'"
    shellInitOutsideZsh
  )
  (checks.expectContains "bash config keeps interactive shell args distinct"
    ''"shell_interactive_args":["-l"]''
    bashInteractiveCommandJson
  )
  (checks.expectContains "bash config exports wrapped command shell args"
    ''"wrapped_command_shell_args":["-l","-i"]''
    bashInteractiveCommandJson
  )
  (checks.expectContains "bash wrapper renders interactive command alias"
    "alias git='__cloister_run_dev -i git'"
    bashInteractiveCommandOutside
  )
  (checks.expectContains "registry preserves multi-arg default command"
    "__cloister_run_browser -c chromium"
    multiArgOutsideZsh
  )
  (checks.expectContains "registry preserves multi-arg default command flag"
    "--ozone-platform=wayland"
    multiArgOutsideZsh
  )
  (checks.expectContains "registry renders outside function" "greet()" outsideZsh)
  (checks.expectContains "outside function requires store-backed init env"
    "\${CLOISTER_SHELL_INIT:?missing cloister shell init}"
    outsideZsh
  )
  (checks.expectNotContains "outside function does not fall back to legacy init path"
    "/home/tester/.config/zsh/cloister-dev.zsh"
    outsideZsh
  )
  (checks.expectNotContains "wrapper init does not probe legacy init files" "_cloister_init"
    outsideZsh
  )
  (checks.expectNotContains "registry honors noWrap outside" "alias ll='__cloister_run_dev -c ls -l'"
    outsideZsh
  )
  (checks.expectContains "registry keeps noWrap inside" "alias ll='ls -l'" insideRegistry)
  (checks.expectContains "bash wrapper content renders for bash sandboxes" "hello()" outsideBash)
  (checks.expectFalse "multi-sandbox zsh init file is not globally installed" (
    multiXdgFiles ? "zsh/cloister-one.zsh"
  ))
  (checks.expectFalse "multi-sandbox bash init file is not globally installed" (
    multiXdgFiles ? "bash/cloister-two.bash"
  ))
  (checks.expectFalse "nonblank zsh hook is not globally installed" (
    hostConfigHookXdgFiles ? "zsh/cloister-dev.zsh"
  ))
  (checks.expectContains "nonblank hostConfig zsh hook exports init env" "CLOISTER_SHELL_INIT"
    hostConfigHookDevArgs
  )
  (checks.expectContains "nonblank hostConfig zsh hook uses store init" "cloister-dev.zsh"
    hostConfigHookDevArgs
  )
  (checks.expectNotContains "nonblank hostConfig zsh hook does not mount into host config"
    "/home/tester/.config/zsh/cloister-dev.zsh"
    hostConfigHookDevDynamic
  )
  (checks.expectNotContains "owning sandbox omits unrelated zsh hook statically" "cloister-evince.zsh"
    hostConfigHookDevArgs
  )
  (checks.expectNotContains "owning sandbox omits unrelated zsh hook dynamically"
    "cloister-evince.zsh"
    hostConfigHookDevDynamic
  )
  (checks.expectNotContains "blank hostConfig zsh hook is not mounted statically"
    "cloister-evince.zsh"
    hostConfigHookEvinceArgs
  )
  (checks.expectNotContains "blank hostConfig zsh hook is not mounted dynamically"
    "cloister-evince.zsh"
    hostConfigHookEvinceDynamic
  )
  (checks.expectAssertionMessage "registry collision fails with message" collisionEval.assertions
    "cross-sandbox name collision"
  )
  (checks.expectAssertionMessage "alias and function overlap is rejected"
    aliasFunctionOverlap.assertions
    "names defined as both alias and function"
  )
  (checks.expectAssertionMessage "alias and command overlap is rejected"
    aliasCommandOverlap.assertions
    "names defined as both alias and command"
  )
  (checks.expectAssertionMessage "function and command overlap is rejected"
    functionCommandOverlap.assertions
    "names defined as both function and command"
  )
  (checks.expectAssertionMessage "command and interactiveCommand overlap is rejected"
    commandInteractiveOverlap.assertions
    "names defined as both command and interactiveCommand"
  )
  (checks.expectAssertionMessage "invalid sandbox names are rejected" invalidSandboxName.assertions
    "sandbox names must match"
  )
  (checks.expectAssertionMessage "invalid alias names are rejected" invalidNames.assertions
    "alias names must match"
  )
  (checks.expectAssertionMessage "outside-wrapped aliases reject shell metacharacters"
    invalidWrappedAliasValue.assertions
    "aliases wrapped outside the sandbox must be argv-safe"
  )
  (checks.expectAssertionMessage "invalid function names are rejected" invalidNames.assertions
    "function names must match"
  )
  (checks.expectAssertionMessage "invalid command names are rejected" invalidNames.assertions
    "command names must match"
  )
  (checks.expectContains "zsh custom zshenv is sourced"
    "source \"$HOME/.config/cl-shell/dev/custom/zshenv\""
    customRcZshInit
  )
  (checks.expectContains "zsh custom zshrc is sourced"
    "source \"$HOME/.config/cl-shell/dev/custom/zshrc\""
    customRcZshInit
  )
  (checks.expectContains "bash custom bashenv is sourced"
    "source \"$HOME/.config/cl-shell/dev/custom/bashenv\""
    customRcBashInit
  )
  (checks.expectContains "bash custom bashrc is sourced"
    "source \"$HOME/.config/cl-shell/dev/custom/bashrc\""
    customRcBashInit
  )
  (checks.expectContains "bash custom profile is sourced"
    "source \"$HOME/.config/cl-shell/dev/custom/profile\""
    customRcBashInit
  )
  (checks.expectContains "zsh custom rc runs before init text" "custom/zshrc" zshBeforeInit)
  (checks.expectContains "bash custom rc runs before init text" "custom/profile" bashBeforeInit)
  (checks.expectContains "zsh registry content runs after init text" "export FROM_INIT=1"
    zshBeforeRegistry
  )
  (checks.expectContains "bash registry content runs after init text" "export FROM_INIT=1"
    bashBeforeRegistry
  )
  (checks.expectContains "bash hostConfig false still binds custom bashenv"
    "/home/tester/.config/cl-shell/dev/custom/bashenv"
    bashNoHostConfigArgs
  )
  (checks.expectContains "bash hostConfig false still binds custom profile"
    "/home/tester/.config/cl-shell/dev/custom/profile"
    bashNoHostConfigArgs
  )
  (checks.expectContains "bash hostConfig false installs minimal bash profile" "$HOME/.bash_profile"
    bashNoHostConfigDynamic
  )
  (checks.expectNotContains "bash hostConfig false omits host bashrc bind" "$HOME/.bashrc"
    bashNoHostConfigArgs
  )
  (checks.expectContains "zsh hostConfig false still binds custom zshenv"
    "/home/tester/.config/cl-shell/dev/custom/zshenv"
    zshNoHostConfigArgs
  )
  (checks.expectContains "zsh hostConfig false sets ZDOTDIR" ''"ZDOTDIR"'' zshNoHostConfigArgs)
  (checks.expectNotContains "zsh hostConfig false omits host zshrc bind" "$HOME/.zshrc"
    zshNoHostConfigArgs
  )
  (checks.expectContains "zsh hostConfig false exports nonblank cloister hook env"
    "CLOISTER_SHELL_INIT"
    zshNoHostConfigArgs
  )
  (checks.expectContains "zsh hostConfig false keeps nonblank cloister hook in store"
    "cloister-dev.zsh"
    zshNoHostConfigArgs
  )
  (checks.expectNotContains "blank zsh hostConfig false omits cloister hook statically"
    "cloister-evince.zsh"
    zshBlankNoHostConfigArgs
  )
  (checks.expectNotContains "blank zsh hostConfig false omits cloister hook dynamically"
    "cloister-evince.zsh"
    zshBlankNoHostConfigDynamic
  )
  (checks.expectNotContains "blank zsh hostConfig false omits zsh config dir statically"
    "/home/tester/.config/zsh"
    zshBlankNoHostConfigArgs
  )
  (checks.expectNotContains "blank zsh hostConfig false omits zsh config dir dynamically"
    "/home/tester/.config/zsh"
    zshBlankNoHostConfigDynamic
  )
]
