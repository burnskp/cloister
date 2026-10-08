{
  checks,
  hm,
  pkgs,
  ...
}:
let
  managedXdgSource = pkgs.writeText "managed-config-toml" "[app]\nenabled = true\n";
  managedHmConfigSource = pkgs.writeText "managed-home-manager-ini" "value = 1\n";
  managedHomeSource = pkgs.writeText "managed-home-file" "node_modules\n";

  eval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        sandbox = {
          state.projectDirs."/ephemeral" = [ ".cache/nvim" ];
          copies = [
            {
              src = "/host/source/app.conf";
              dest = "/home/tester/.config/app.conf";
              overwrite = true;
            }
          ];
        };
      };
    };
  };

  bindMatrixEval = hm {
    xdg.configFile."app/config.toml".source = managedXdgSource;
    home.file."/home/tester/.config/hm/app.ini".source = managedHmConfigSource;
    home.file.".gitignore".source = managedHomeSource;

    cloister = {
      enable = true;
      sandboxes.dev = {
        shell.hostConfig = false;
        sandbox = {
          readOnly = [
            ".bashrc"
            {
              src = ".zshrc";
              optional = true;
            }
          ];
          readWrite = [
            ".cache/app"
            {
              src = ".local/share/app";
              optional = true;
            }
          ];
          managed = [
            "app/config.toml"
            "hm/app.ini"
            ".gitignore"
          ];
        };
      };
    };
  };

  persistentStateEval = hm {
    xdg.configFile."app/dir/settings.json".source = managedXdgSource;

    cloister = {
      enable = true;
      sandboxes.dev.sandbox = {
        state = {
          dirs."/persist" = [
            ".cache/app"
            ".config/app"
          ];
          files."/persist" = [ ".local/state/app/history" ];
        };
        managed = [ "app/dir/settings.json" ];
      };
    };
  };

  perDirManagedFileEval = hm {
    xdg.configFile."opencode/tui.json".source = managedXdgSource;
    xdg.configFile."opencode/opencode.json".source = managedHmConfigSource;

    cloister = {
      enable = true;
      sandboxes.dev.sandbox = {
        state.projectDirs."/ephemeral" = [ ".config/opencode" ];
        managed = [
          "opencode/tui.json"
          "opencode/opencode.json"
        ];
      };
    };
  };

  explicitManagedFileBindEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox = {
        state.projectDirs."/ephemeral" = [ ".config/opencode" ];
        managed = [
          {
            src = managedXdgSource;
            dest = ".config/opencode/tui.json";
          }
          {
            src = managedHmConfigSource;
            dest = ".config/opencode/opencode.json";
          }
        ];
      };
    };
  };

  symlinkEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox = {
        managed = [
          {
            src = managedXdgSource;
            dest = ".ssh/.config";
          }
        ];
        symlinks = [
          {
            target = ".config";
            link = ".ssh/config";
          }
        ];
      };
    };
  };

  managedPrefixEval = hm {
    xdg.configFile."app/exact".source = managedXdgSource;
    xdg.configFile."app/prefix/alpha.toml".source = managedXdgSource;
    home.file."/home/tester/.config/hm-only/beta.ini".source = managedHmConfigSource;
    home.file."notes/todo.txt".source = managedHomeSource;

    cloister = {
      enable = true;
      sandboxes.dev = {
        shell.hostConfig = false;
        sandbox.managed = [
          "app/exact"
          "app/prefix"
          "hm-only"
          "notes"
        ];
      };
    };
  };

  managedExactWinsEval = hm {
    xdg.configFile."app".source = managedXdgSource;
    xdg.configFile."app/child.toml".source = managedHmConfigSource;

    cloister = {
      enable = true;
      sandboxes.dev = {
        shell.hostConfig = false;
        sandbox.managed = [ "app" ];
      };
    };
  };

  multiPerDirEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox = {
        state.projectDirs = {
          "/var/lib/cloister-per-dir" = [ ".cache/custom" ];
          "/var/lib/cloister-worktrees" = [ ".local/worktrees/project" ];
        };
        copyBase = "/var/lib/cloister-copy";
        copies = [
          {
            src = "/host/source/custom.conf";
            dest = "/home/tester/.config/custom.conf";
          }
        ];
      };
    };
  };

  copyHomeDestEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.copies = [
        {
          src = "/host/source/home.conf";
          dest = "$HOME/.config/home.conf";
        }
      ];
    };
  };

  strictHomePolicyEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox = {
        enforceStrictHomePolicy = false;
        disallowedPaths = [
          "/"
          "/tmp/deny"
        ];
      };
    };
  };

  missingManagedFile = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.managed = [ "missing/path" ];
    };
  };

  invalidPerDir = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox = {
        bindWorkingDirectory = false;
        state.projectDirs."/ephemeral" = [ ".cache/nvim" ];
      };
    };
  };

  emptyPerDirBucketEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox = {
        bindWorkingDirectory = false;
        state.projectDirs."/ephemeral" = [ ];
      };
    };
  };

  duplicatePerDirDest = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.state.projectDirs = {
        "/ephemeral" = [ ".cache/shared" ];
        "/local/worktrees" = [ ".cache/shared" ];
      };
    };
  };

  invalidCopyMode = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.copies = [
        {
          src = "/host/source/app.conf";
          dest = "/home/tester/.config/app.conf";
          mode = "64x";
        }
      ];
    };
  };

  gitEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.git.enable = true;
    };
  };

  duplicateBindDest = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.readOnly = [
        {
          src = "/host/one";
          dest = "/sandbox/shared";
        }
        {
          src = "/host/two";
          dest = "/sandbox/shared";
        }
      ];
    };
  };

  duplicateManagedFile = hm {
    xdg.configFile."app/config.toml".source = managedXdgSource;

    cloister = {
      enable = true;
      sandboxes.dev.sandbox.managed = [
        "app/config.toml"
        "app/config.toml"
      ];
    };
  };

  duplicateManagedFileBindDest = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.managed = [
        {
          src = managedXdgSource;
          dest = ".config/app/config.toml";
        }
        {
          src = managedHmConfigSource;
          dest = ".config/app/config.toml";
        }
      ];
    };
  };

  invalidSymlink = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.symlinks = [
        {
          target = ".config";
          link = "../config";
        }
      ];
    };
  };

  duplicateSymlinkDest = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.symlinks = [
        {
          target = "one";
          link = ".config/shared";
        }
        {
          target = "two";
          link = ".config/shared";
        }
      ];
    };
  };

  symlinkInternalBindCollision = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.symlinks = [
        {
          target = "/tmp/hosts";
          link = "/etc/hosts";
        }
      ];
    };
  };

  symlinkUserBindCollision = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox = {
        readOnly = [
          {
            src = "/host/app.conf";
            dest = ".config/app.conf";
          }
        ];
        symlinks = [
          {
            target = "app.conf.real";
            link = ".config/app.conf";
          }
        ];
      };
    };
  };

  symlinkHiddenByBind = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox = {
        readWrite = [ ".ssh" ];
        symlinks = [
          {
            target = "config.real";
            link = ".ssh/config";
          }
        ];
      };
    };
  };

  pathOverride = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.env.PATH = "/tmp/bin";
    };
  };

  unsafePath = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.readOnly = [ "$(pwd)" ];
    };
  };

  bindTraversal = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.readOnly = [ "../secrets" ];
    };
  };

  stateTraversal = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.state.dirs."/persist" = [ "../shared" ];
    };
  };

  copyBaseTraversal = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.copyBase = "/var/lib/../tmp";
    };
  };

  copySourceTraversal = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.copies = [
        {
          src = "/host/../source/app.conf";
          dest = "/home/tester/.config/app.conf";
        }
      ];
    };
  };

  blockedPassthrough = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        dbus.enable = true;
        sandbox.passthroughEnv = [
          "PATH"
          "DBUS_SESSION_BUS_ADDRESS"
          "bad-name!"
        ];
      };
    };
  };

  sandboxHomeManagerXdgSource = pkgs.writeText "sandbox-hm-xdg" "sandbox = true\n";
  sandboxHomeManagerHomeSource = pkgs.writeText "sandbox-hm-home" "sandbox home file\n";

  sandboxHomeManagerEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.homeManager = {
        enable = true;
        config = {
          home = {
            homeDirectory = "/home/tester";
            packages = [ pkgs.jq ];
            file.".toolrc".source = sandboxHomeManagerHomeSource;
            file."wrong-home-name" = {
              source = sandboxHomeManagerHomeSource;
              target = ".tool-target";
            };
            file."disabled-home" = {
              source = sandboxHomeManagerHomeSource;
              enable = false;
            };
          };
          xdg.configFile."tool/config.toml".source = sandboxHomeManagerXdgSource;
          xdg.configFile."wrong-xdg-name" = {
            source = sandboxHomeManagerXdgSource;
            target = "tool/target.toml";
          };
          xdg.configFile."disabled-xdg" = {
            source = sandboxHomeManagerXdgSource;
            enable = false;
          };
        };
      };
    };
  };

  copyOutsideHome = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.copies = [
        {
          src = "/host/source/app.conf";
          dest = "/etc/app.conf";
        }
      ];
    };
  };

  sandboxConfig = eval.config.cloister._internal.sandboxConfigs.dev;
  copySpec = builtins.head sandboxConfig.copy_files;
  bindMatrix = bindMatrixEval.config.cloister._internal.sandboxConfigs.dev.dynamic_binds;
  staticArgsJson = builtins.toJSON bindMatrixEval.config.cloister._internal.sandboxConfigs.dev.static_bwrap_args;
  persistentState = persistentStateEval.config.cloister._internal.sandboxConfigs.dev;
  persistentStateJson = builtins.toJSON persistentState;
  perDirManagedFileState = perDirManagedFileEval.config.cloister._internal.sandboxConfigs.dev;
  perDirManagedFileStateJson = builtins.toJSON perDirManagedFileState;
  explicitManagedFileBindState =
    explicitManagedFileBindEval.config.cloister._internal.sandboxConfigs.dev;
  explicitManagedFileBindStateJson = builtins.toJSON explicitManagedFileBindState;
  symlinkStaticArgs = builtins.toJSON symlinkEval.config.cloister._internal.sandboxConfigs.dev.static_bwrap_args;
  managedPrefixConfig = managedPrefixEval.config.cloister._internal.sandboxConfigs.dev;
  managedPrefixStaticArgs = builtins.toJSON managedPrefixConfig.static_bwrap_args;
  managedExactWinsConfig = managedExactWinsEval.config.cloister._internal.sandboxConfigs.dev;
  managedExactWinsStaticArgs = builtins.toJSON managedExactWinsConfig.static_bwrap_args;
  multiPerDirConfig = multiPerDirEval.config.cloister._internal.sandboxConfigs.dev;
  multiPerDirCopySpec = builtins.head multiPerDirConfig.copy_files;
  multiPerDirDynamicBinds = builtins.toJSON multiPerDirConfig.dynamic_binds;
  copyHomeDestSpec = builtins.head copyHomeDestEval.config.cloister._internal.sandboxConfigs.dev.copy_files;
  strictHomePolicyConfig = strictHomePolicyEval.config.cloister._internal.sandboxConfigs.dev;
  emptyPerDirBucketConfig = emptyPerDirBucketEval.config.cloister._internal.sandboxConfigs.dev;
  gitConfig = gitEval.config.cloister._internal.sandboxConfigs.dev;
  sandboxHomeManagerConfig = sandboxHomeManagerEval.config.cloister._internal.sandboxConfigs.dev;
  sandboxHomeManagerJson = builtins.toJSON sandboxHomeManagerConfig;
  emptyPerDirBucketFailures = builtins.filter (a: !a.assertion) emptyPerDirBucketEval.assertions;

  getBind = src: builtins.head (builtins.filter (bind: bind.src == src) bindMatrix);

  requiredRoBind = getBind "$HOME/.bashrc";
  optionalRoBind = getBind "$HOME/.zshrc";
  requiredRwBind = getBind "$HOME/.cache/app";
  optionalRwBind = getBind "$HOME/.local/share/app";
in
checks.mkCheck "test-cloister-sandbox-core" [
  (checks.expectEq "per-dir mapping preserved" {
    "/ephemeral" = [ ".cache/nvim" ];
  } sandboxConfig.per_dir)
  (checks.expectContains "per-dir bind uses DIR_HASH" "$DIR_HASH/.cache/nvim" (
    builtins.toJSON sandboxConfig.dynamic_binds
  ))
  (checks.expectEq "copies host dest rendered"
    "/home/tester/.local/state/cloister/cloister/dev/.config/app.conf"
    copySpec.host_dest
  )
  (checks.expectEq "required read-only bind renders as strict ro" "ro" requiredRoBind.mode)
  (checks.expectFalse "required read-only bind is not optional" requiredRoBind.try_bind)
  (checks.expectEq "optional read-only bind renders as ro" "ro" optionalRoBind.mode)
  (checks.expectTrue "optional read-only bind uses try mode" optionalRoBind.try_bind)
  (checks.expectEq "required read-write bind renders as rw" "rw" requiredRwBind.mode)
  (checks.expectFalse "required read-write bind is not optional" requiredRwBind.try_bind)
  (checks.expectEq "optional read-write bind renders as rw" "rw" optionalRwBind.mode)
  (checks.expectTrue "optional read-write bind uses try mode" optionalRwBind.try_bind)
  (checks.expectContains "managed XDG file resolves to config home"
    ''"/home/tester/.config/app/config.toml"''
    staticArgsJson
  )
  (checks.expectContains "managed Home Manager config file resolves to config home"
    ''"/home/tester/.config/hm/app.ini"''
    staticArgsJson
  )
  (checks.expectContains "managed direct home file resolves to home" ''"/home/tester/.gitignore"''
    staticArgsJson
  )
  (checks.expectContains "managedFile prefix resolves xdg children"
    ''"/home/tester/.config/app/prefix/alpha.toml"''
    managedPrefixStaticArgs
  )
  (checks.expectContains "managedFile prefix resolves home-manager config children"
    ''"/home/tester/.config/hm-only/beta.ini"''
    managedPrefixStaticArgs
  )
  (checks.expectContains "managedFile direct home prefix resolves to home"
    ''"/home/tester/notes/todo.txt"''
    managedPrefixStaticArgs
  )
  (checks.expectContains "managedFile exact key resolves direct config path"
    ''"/home/tester/.config/app"''
    managedExactWinsStaticArgs
  )
  (checks.expectNotContains "managedFile exact key wins over prefix expansion"
    ''"/home/tester/.config/app/child.toml"''
    managedExactWinsStaticArgs
  )
  (checks.expectContains "persistent dir bind renders host path" "/persist/cloister/dev/.cache/app"
    persistentStateJson
  )
  (checks.expectContains "persistent file bind renders host path"
    "/persist/cloister/dev/.local/state/app/history"
    persistentStateJson
  )
  (checks.expectContains "managed file overlap creates host mkdir path"
    "/persist/cloister/dev/.config/app"
    (builtins.toJSON persistentState.managed_file_host_mkdirs)
  )
  (checks.expectContains "per-dir managed file overlap creates hashed host mkdir path"
    "/ephemeral/$DIR_HASH/.config/opencode"
    (builtins.toJSON perDirManagedFileState.managed_file_host_mkdirs)
  )
  (checks.expectContains "per-dir managed file bind is rendered for tui.json"
    ''"/home/tester/.config/opencode/tui.json"''
    perDirManagedFileStateJson
  )
  (checks.expectContains "per-dir managed file bind is rendered for opencode.json"
    ''"/home/tester/.config/opencode/opencode.json"''
    perDirManagedFileStateJson
  )
  (checks.expectContains "explicit managed file bind overlap creates hashed host mkdir path"
    "/ephemeral/$DIR_HASH/.config/opencode"
    (builtins.toJSON explicitManagedFileBindState.managed_file_host_mkdirs)
  )
  (checks.expectContains "explicit managed file bind renders tui.json destination"
    ''"/home/tester/.config/opencode/tui.json"''
    explicitManagedFileBindStateJson
  )
  (checks.expectContains "explicit managed file bind renders store source"
    (builtins.unsafeDiscardStringContext (toString managedXdgSource))
    explicitManagedFileBindStateJson
  )
  (checks.expectContains "sandbox home-manager xdg config is bound inside sandbox"
    ''"/home/tester/.config/tool/config.toml"''
    sandboxHomeManagerJson
  )
  (checks.expectContains "sandbox home-manager home.file is bound inside sandbox"
    ''"/home/tester/.toolrc"''
    sandboxHomeManagerJson
  )
  (checks.expectContains "sandbox home-manager xdg target is honored"
    ''"/home/tester/.config/tool/target.toml"''
    sandboxHomeManagerJson
  )
  (checks.expectContains "sandbox home-manager home.file target is honored"
    ''"/home/tester/.tool-target"''
    sandboxHomeManagerJson
  )
  (checks.expectNotContains "sandbox home-manager disabled xdg file is skipped"
    ''"/home/tester/.config/disabled-xdg"''
    sandboxHomeManagerJson
  )
  (checks.expectNotContains "sandbox home-manager disabled home.file is skipped"
    ''"/home/tester/disabled-home"''
    sandboxHomeManagerJson
  )
  (checks.expectNotContains "sandbox home-manager xdg source name is not used when target is set"
    ''"/home/tester/.config/wrong-xdg-name"''
    sandboxHomeManagerJson
  )
  (checks.expectNotContains "sandbox home-manager home source name is not used when target is set"
    ''"/home/tester/wrong-home-name"''
    sandboxHomeManagerJson
  )
  (checks.expectContains "sandbox home-manager package is added to PATH"
    (builtins.unsafeDiscardStringContext "${pkgs.jq}/bin")
    sandboxHomeManagerJson
  )
  (checks.expectContains "sandbox symlink renders target" ''".config"'' symlinkStaticArgs)
  (checks.expectContains "sandbox symlink renders home-relative link" ''"/home/tester/.ssh/config"''
    symlinkStaticArgs
  )
  (checks.expectContains "sandbox symlink creates parent directory" ''"/home/tester/.ssh"''
    symlinkStaticArgs
  )
  (checks.expectEq "multi per-dir mapping is rendered" {
    "/var/lib/cloister-per-dir" = [ ".cache/custom" ];
    "/var/lib/cloister-worktrees" = [ ".local/worktrees/project" ];
  } multiPerDirConfig.per_dir)
  (checks.expectContains "first per-dir base feeds dynamic bind source"
    "/var/lib/cloister-per-dir/$DIR_HASH/.cache/custom"
    multiPerDirDynamicBinds
  )
  (checks.expectContains "second per-dir base feeds dynamic bind source"
    "/var/lib/cloister-worktrees/$DIR_HASH/.local/worktrees/project"
    multiPerDirDynamicBinds
  )
  (checks.expectEq "custom copy base feeds copy host destination"
    "/var/lib/cloister-copy/cloister/dev/.config/custom.conf"
    multiPerDirCopySpec.host_dest
  )
  (checks.expectEq "copies accept HOME-relative destinations"
    "/home/tester/.local/state/cloister/cloister/dev/.config/home.conf"
    copyHomeDestSpec.host_dest
  )
  (checks.expectEq "custom copy base is rendered for runtime validation" "/var/lib/cloister-copy"
    multiPerDirConfig.copy_file_base
  )
  (checks.expectEq "strict home policy toggle is rendered" false
    strictHomePolicyConfig.enforce_strict_home_policy
  )
  (checks.expectEq "custom disallowed paths are rendered" [
    "/"
    "/tmp/deny"
  ] strictHomePolicyConfig.disallowed_paths)
  (checks.expectContains "git enable binds xdg git config" "$HOME/.config/git/config" (
    builtins.toJSON gitConfig.dynamic_binds
  ))
  (checks.expectContains "git enable binds gitconfig" "$HOME/.gitconfig" (
    builtins.toJSON gitConfig.dynamic_binds
  ))
  (checks.expectNotContains "git enable does not bind git config directory" ''"$HOME/.config/git"'' (
    builtins.toJSON gitConfig.dynamic_binds
  ))
  (checks.expectAssertionMessage "per-dir requires bindWorkingDirectory" invalidPerDir.assertions
    "sandbox.bindWorkingDirectory = false is incompatible with sandbox.state.projectDirs"
  )
  (checks.expectEq "empty per-dir buckets are dropped from config" { }
    emptyPerDirBucketConfig.per_dir
  )
  (checks.expectEq "empty per-dir buckets do not require bindWorkingDirectory" [ ]
    emptyPerDirBucketFailures
  )
  (checks.expectAssertionMessage "duplicate per-dir destinations are rejected"
    duplicatePerDirDest.assertions
    "duplicate bind mount destinations"
  )
  (checks.expectAssertionMessage "copies mode validation fails" invalidCopyMode.assertions
    "sandbox.copies contains invalid mode values"
  )
  (checks.expectAssertionMessage "copies destination must stay inside home" copyOutsideHome.assertions
    "all sandbox.copies dest paths must start with $HOME/"
  )
  (checks.expectAssertionMessage "duplicate bind destinations are rejected"
    duplicateBindDest.assertions
    "duplicate bind mount destinations"
  )
  (checks.expectAssertionMessage "duplicate managed files are rejected"
    duplicateManagedFile.assertions
    "duplicate managed file destinations"
  )
  (checks.expectAssertionMessage "duplicate explicit managed file destinations are rejected"
    duplicateManagedFileBindDest.assertions
    "duplicate managed file destinations"
  )
  (checks.expectAssertionMessage "invalid sandbox symlink paths are rejected"
    invalidSymlink.assertions
    "sandbox.symlinks entries must have non-empty targets"
  )
  (checks.expectAssertionMessage "duplicate sandbox symlink destinations are rejected"
    duplicateSymlinkDest.assertions
    "duplicate symlink destinations"
  )
  (checks.expectAssertionMessage "sandbox symlink cannot collide with internal bind destination"
    symlinkInternalBindCollision.assertions
    "sandbox.symlinks link paths must not collide with or be hidden by bind mount destinations"
  )
  (checks.expectAssertionMessage "sandbox symlink cannot collide with user bind destination"
    symlinkUserBindCollision.assertions
    "sandbox.symlinks link paths must not collide with or be hidden by bind mount destinations"
  )
  (checks.expectAssertionMessage "sandbox symlink cannot be hidden by containing bind destination"
    symlinkHiddenByBind.assertions
    "sandbox.symlinks link paths must not collide with or be hidden by bind mount destinations"
  )
  (checks.expectAssertionMessage "PATH override is rejected" pathOverride.assertions
    "computed and cannot be overridden"
  )
  (checks.expectAssertionMessage "unsafe path expansion is rejected" unsafePath.assertions
    "cannot contain unsafe variable expansions ($) or newlines"
  )
  (checks.expectAssertionMessage "readOnly traversal is rejected" bindTraversal.assertions
    "sandbox.readOnly/readWrite entries must use traversal-free paths"
  )
  (checks.expectAssertionMessage "state traversal is rejected" stateTraversal.assertions
    "sandbox.state paths must be home-relative descendants without traversal"
  )
  (checks.expectAssertionMessage "copyBase traversal is rejected" copyBaseTraversal.assertions
    "sandbox.copyBase must be an absolute traversal-free host path"
  )
  (checks.expectAssertionMessage "copy source traversal is rejected" copySourceTraversal.assertions
    "sandbox.copies src paths must be absolute traversal-free host paths"
  )
  (checks.expectAssertionMessage "invalid passthrough env names are rejected"
    blockedPassthrough.assertions
    "contains invalid variable names"
  )
  (checks.expectAssertionMessage "managed passthrough env keys are blocked"
    blockedPassthrough.assertions
    "cannot include computed/managed keys"
  )
  (checks.expectFailure "missing managed entries fail evaluation" missingManagedFile.config.cloister._internal.sandboxConfigs.dev.static_bwrap_args)
]
