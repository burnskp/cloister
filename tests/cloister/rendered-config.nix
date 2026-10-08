{
  checks,
  hm,
  pkgs,
  ...
}:
let
  eval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        defaultCommand = [
          "nvim"
          "--clean"
        ];
        shell.name = "bash";
        network.namespace = "vpn";
        dbus.enable = true;
        sandbox.env = {
          DOLLAR_VAR = "$HOME/literal";
          PROMPT_VAR = "$" + "{PS1@P}";
        };
        sandbox = {
          anonymize = {
            enable = true;
            username = "guest";
          };
          readOnly = [
            {
              src = ".gitconfig";
              optional = true;
            }
          ];
        };
      };
    };
  };

  dnsEval = hm {
    cloister = {
      enable = true;
      sandboxes = {
        explicit.network.dns.nameservers = [
          "1.1.1.1"
          "2606:4700:4700::1111"
        ];
        file.network.dns.resolvConf = "/run/NetworkManager/resolv.conf";
        namespace.network = {
          namespace = "vpn";
          dns.resolvConf = "/run/NetworkManager/resolv.conf";
        };
        isolated.network = {
          enable = false;
          dns.nameservers = [ "1.1.1.1" ];
        };
      };
    };
  };
  dnsConfigs = dnsEval.config.cloister._internal.sandboxConfigs;
  dnsArgs = name: builtins.toJSON dnsConfigs.${name}.static_bwrap_args;
  generatedResolver = builtins.head (
    builtins.filter (
      arg: builtins.match "/nix/store/.*-cloister-explicit-resolv.conf" arg != null
    ) dnsConfigs.explicit.static_bwrap_args
  );
  invalidDnsEval =
    dns:
    hm {
      cloister = {
        enable = true;
        sandboxes.dev.network.dns = dns;
      };
    };

  hostConfigOnEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.shell = {
        name = "zsh";
        hostConfig = true;
      };
    };
  };

  localeOverrideEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.env.LOCALE_ARCHIVE = "/custom/locale-archive";
    };
  };

  plainEval = hm {
    cloister = {
      enable = true;
      sandboxes.plain = { };
    };
  };

  workdirDisabledEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.bindWorkingDirectory = false;
    };
  };

  featuresEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        ssh = {
          enable = true;
          allowFingerprints = [ "SHA256:test-fingerprint" ];
          filterTimeoutSeconds = 7;
        };
        git.enable = true;
        sandbox.devices = [
          "/dev/input/js0"
          "/dev/kvm"
        ];
      };
    };
  };

  noNetworkEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.network.enable = false;
    };
  };

  workerBrokerNoNetworkEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        network.enable = false;
        workerBroker.profiles.ephemeral = {
          sandbox = "worker";
          workspace.mode = "project-overlay";
        };
      };
      sandboxes.worker.preset = "hardened";
    };
  };

  seccompDisabledEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.seccomp.enable = false;
    };
  };

  chromiumSeccompEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.sandbox.seccomp.allowChromiumSandbox = true;
    };
  };

  imageStoreEval = hm {
    _module.args.osConfig = {
      cloister.imageStore = {
        base = "/var/lib/cloister/images";
        mountBase = "/run/cloister/images";
      };
    };

    cloister = {
      enable = true;
      sandboxes.dev.sandbox.nixStore.mode = "image-store";
    };
  };

  imageStoreUncompressedEval = hm {
    _module.args.osConfig = {
      cloister.imageStore = {
        base = "/var/lib/cloister/images";
        mountBase = "/run/cloister/images";
        compression.enable = false;
      };
    };

    cloister = {
      enable = true;
      sandboxes.dev.sandbox.nixStore.mode = "image-store";
    };
  };

  namespaceWithNetworkDisabledEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.network = {
        enable = false;
        namespace = "vpn";
      };
    };
  };

  defaultBashEval = hm {
    cloister = {
      enable = true;
      defaultShell = "bash";
      sandboxes.dev = { };
    };
  };

  packagesEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev.extraPackages = [
        pkgs.hello
        pkgs.jq
      ];
    };
  };

  workerBrokerEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        workerBroker.profiles = {
          ephemeral = {
            sandbox = "worker";
            workspace.mode = "project-overlay";
            delegatedPerDirMounts.".cache/pre-commit" = {
              mode = "ro";
              path = "/local/ephemeral/dev";
              subPath = ".cache/pre-commit";
            };
          };
          project = {
            sandbox = "worker";
            workspace.mode = "project-rw";
            delegatedPerDirMounts = {
              "worktrees" = {
                mode = "rw";
                path = "/local/worktrees/dev";
              };
              ".cache/pre-commit" = {
                mode = "ro";
                path = "/local/ephemeral/dev";
                subPath = ".cache/pre-commit";
              };
            };
          };
        };
      };
      sandboxes.worker.preset = "hardened";
    };
  };

  sameSandboxWorkerBrokerEval = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        workerBroker.profiles.local = {
          sandbox = "dev";
          workspace.mode = "project-overlay";
        };
      };
    };
  };

  sandboxConfig = eval.config.cloister._internal.sandboxConfigs.dev;
  plainConfig = plainEval.config.cloister._internal.sandboxConfigs.plain;
  hostConfigOn = hostConfigOnEval.config.cloister._internal.sandboxConfigs.dev;
  localeOverride = localeOverrideEval.config.cloister._internal.sandboxConfigs.dev;
  workdirDisabled = workdirDisabledEval.config.cloister._internal.sandboxConfigs.dev;
  featuresConfig = featuresEval.config.cloister._internal.sandboxConfigs.dev;
  noNetworkConfig = noNetworkEval.config.cloister._internal.sandboxConfigs.dev;
  workerBrokerNoNetworkConfig =
    workerBrokerNoNetworkEval.config.cloister._internal.sandboxConfigs.dev;
  seccompDisabled = seccompDisabledEval.config.cloister._internal.sandboxConfigs.dev;
  chromiumSeccomp = chromiumSeccompEval.config.cloister._internal.sandboxConfigs.dev;
  imageStoreConfig = imageStoreEval.config.cloister._internal.sandboxConfigs.dev;
  imageStoreInternal = imageStoreEval.config.cloister._internal.sandboxInternals.dev;
  defaultBashConfig = defaultBashEval.config.cloister._internal.sandboxConfigs.dev;
  packagesConfig = packagesEval.config.cloister._internal.sandboxConfigs.dev;
  packagesStaticArgs = builtins.toJSON packagesConfig.static_bwrap_args;
  workerBrokerConfig = workerBrokerEval.config.cloister._internal.sandboxConfigs.dev;
  workerBrokerChildConfig = workerBrokerEval.config.cloister._internal.sandboxConfigs.worker;
  workerBrokerInternal = workerBrokerEval.config.cloister._internal.sandboxInternals.dev;
  sameSandboxWorkerBrokerInternal =
    sameSandboxWorkerBrokerEval.config.cloister._internal.sandboxInternals.dev;
  inherit (workerBrokerInternal) workerBrokerLauncherPackage;
  workerBrokerParentPackage = builtins.elemAt workerBrokerEval.config.home.packages 0;
  workerBrokerChildPackage = builtins.elemAt workerBrokerEval.config.home.packages 1;
  workerBrokerParentWrapperText = builtins.readFile "${workerBrokerParentPackage}/bin/cl-dev";
  workerBrokerInstalledLauncherText = builtins.readFile "${workerBrokerLauncherPackage}/bin/clb-ephemeral";
  sameSandboxWorkerBrokerLauncherText =
    sameSandboxWorkerBrokerInternal.workerBrokerLauncherTexts.clb-local;

  plainStaticArgs = builtins.toJSON plainConfig.static_bwrap_args;
  plainDynamicBinds = builtins.toJSON plainConfig.dynamic_binds;
  hostConfigOnDynamicBinds = builtins.toJSON hostConfigOn.dynamic_binds;
  sandboxConfigJson = builtins.toJSON sandboxConfig;
  localeOverrideStaticArgs = builtins.toJSON localeOverride.static_bwrap_args;
in
checks.mkCheck "test-cloister-rendered-config" [
  (checks.expectFalse "default DNS has no override" plainConfig.network_dns_override)
  (checks.expectContains "default DNS uses host resolver"
    ''"--ro-bind-try","/etc/resolv.conf","/etc/resolv.conf"''
    plainStaticArgs
  )
  (checks.expectEq "explicit DNS renders IPv4 and IPv6 in order" (toString (
    pkgs.writeText "cloister-explicit-resolv.conf" "nameserver 1.1.1.1\nnameserver 2606:4700:4700::1111\n"
  )) generatedResolver)
  (checks.expectTrue "nameservers enable DNS override" dnsConfigs.explicit.network_dns_override)
  (checks.expectTrue "resolver file enables DNS override" dnsConfigs.file.network_dns_override)
  (checks.expectContains "resolver file is a required read-only bind"
    ''"--ro-bind","/run/NetworkManager/resolv.conf","/etc/resolv.conf"''
    (dnsArgs "file")
  )
  (checks.expectNotContains "explicit DNS replaces the default resolver source"
    ''"--ro-bind-try","/etc/resolv.conf"''
    (dnsArgs "explicit")
  )
  (checks.expectContains "namespace uses explicit resolver"
    ''"--ro-bind","/run/NetworkManager/resolv.conf","/etc/resolv.conf"''
    (dnsArgs "namespace")
  )
  (checks.expectNotContains "explicit resolver replaces namespace resolver"
    "/etc/netns/vpn/resolv.conf"
    (dnsArgs "namespace")
  )
  (checks.expectFalse "DNS does not enable networking" dnsConfigs.isolated.network_enable)
  (checks.expectAssertionMessage "DNS sources conflict"
    (invalidDnsEval {
      nameservers = [ "1.1.1.1" ];
      resolvConf = "/run/NetworkManager/resolv.conf";
    }).assertions
    "network.dns.nameservers and network.dns.resolvConf are mutually exclusive"
  )
  (checks.expectTrue "invalid DNS entries are rejected" (
    builtins.all
      (
        server:
        checks.expectAssertionMessage "invalid nameserver"
          (invalidDnsEval { nameservers = [ server ]; }).assertions
          "entries must be nonempty and contain no whitespace"
      )
      [
        ""
        "1.1.1.1 8.8.8.8"
        "1.1.1.1\nsearch example.org"
        "1.1.1.1\t"
      ]
  ))
  (checks.expectFailure "resolver path must be absolute"
    (invalidDnsEval { resolvConf = "relative/resolv.conf"; })
    .config.cloister.sandboxes.dev.network.dns.resolvConf
  )
  (checks.expectEq "sandbox name propagates into config" "dev" sandboxConfig.name)
  (checks.expectEq "anonymized sandbox home" "/home/guest" sandboxConfig.sandbox_home)
  (checks.expectContains "anonymized locale defaults to C.UTF-8" ''"LANG","C.UTF-8"''
    sandboxConfigJson
  )
  (checks.expectContains "anonymized lc_all defaults to C.UTF-8" ''"LC_ALL","C.UTF-8"''
    sandboxConfigJson
  )
  (checks.expectContains "anonymized timezone defaults to UTC" ''"TZ","UTC"'' sandboxConfigJson)
  (checks.expectNotContains "anonymized sandbox does not passthrough host locale vars"
    ''"passthrough_env":["LANG"''
    sandboxConfigJson
  )
  (checks.expectEq "default command renders in config" [
    "nvim"
    "--clean"
  ] sandboxConfig.default_command)
  (checks.expectContains "literal dollar env value is preserved" ''"$HOME/literal"''
    sandboxConfigJson
  )
  (checks.expectContains "literal prompt env value is preserved" (
    "\"$" + "{PS1@P}\""
  ) sandboxConfigJson)
  (checks.expectContains "anonymized sandbox uses patched bubblewrap" "bubblewrap-subset-pid"
    sandboxConfig.bwrap_path
  )
  (checks.expectNotContains "non-anonymized sandbox uses regular bubblewrap" "bubblewrap-subset-pid"
    plainConfig.bwrap_path
  )
  (checks.expectEq "network namespace helper path" "/run/wrappers/bin/cloister-netns"
    sandboxConfig.netns_helper_path
  )
  (checks.expectEq "dbus proxy socket name" "cloister/dbus/dev" sandboxConfig.dbus_proxy_socket_name)
  (checks.expectEq "bash shell selected" "bash" sandboxConfig.shell_name)
  (checks.expectEq "defaultShell propagates to sandboxes" "bash" defaultBashConfig.shell_name)
  (checks.expectTrue "build revision is populated" (sandboxConfig.build_revision != null))
  (checks.expectTrue "plain sandbox defaults to host store" (plainConfig.store_mode == "host"))
  (checks.expectTrue "bind working directory defaults true" plainConfig.bind_working_directory)
  (checks.expectContains "bind working directory true adds sandbox dir" "$SANDBOX_DIR" (
    builtins.toJSON plainConfig.dynamic_binds
  ))
  (checks.expectContains "dynamic bind keeps HOME reference" "$HOME/.gitconfig" (
    builtins.toJSON sandboxConfig.dynamic_binds
  ))
  (checks.expectFalse "host config defaults disabled" plainConfig.shell_host_config)
  (checks.expectNotContains "default host config omits host zshrc" ''"$HOME/.zshrc"''
    plainDynamicBinds
  )
  (checks.expectTrue "host config can be enabled" hostConfigOn.shell_host_config)
  (checks.expectContains "enabled host config keeps shell binds" ''"$HOME/.zshrc"''
    hostConfigOnDynamicBinds
  )
  (checks.expectEq "working directory binding can be disabled" false
    workdirDisabled.bind_working_directory
  )
  (checks.expectNotContains "disabled working directory omits sandbox dir" "$SANDBOX_DIR" (
    builtins.toJSON workdirDisabled.dynamic_binds
  ))
  (checks.expectContains "locale archive defaults to glibc locales" "locale-archive" plainStaticArgs)
  (checks.expectContains "locale archive override is preserved" "/custom/locale-archive"
    localeOverrideStaticArgs
  )
  (checks.expectEq "shared network defaults on" true plainConfig.network_enable)
  (checks.expectEq "shared network can be disabled" false noNetworkConfig.network_enable)
  (checks.expectContains "worker broker with disabled network uses nested-sandbox seccomp variant"
    "-chromium"
    workerBrokerNoNetworkConfig.seccomp_filter_path
  )
  (checks.expectFalse
    "worker broker with disabled network does not reuse plain no-network seccomp filter"
    (workerBrokerNoNetworkConfig.seccomp_filter_path == noNetworkConfig.seccomp_filter_path)
  )
  (checks.expectEq "network namespace implies shared networking" true
    namespaceWithNetworkDisabledEval.config.cloister._internal.sandboxConfigs.dev.network_enable
  )
  (checks.expectContains "netns resolv.conf bind renders" "/etc/netns/vpn/resolv.conf" (
    builtins.toJSON sandboxConfig.static_bwrap_args
  ))
  (checks.expectContains "anonymized sandbox keeps netns hosts source available"
    "/etc/netns/vpn/hosts"
    (builtins.toJSON sandboxConfig.static_bwrap_args)
  )
  (checks.expectContains "anonymized sandbox uses utc localtime" "/share/zoneinfo/UTC" (
    builtins.toJSON sandboxConfig.static_bwrap_args
  ))
  (checks.expectEq "ssh integration toggle renders" true featuresConfig.ssh_enable)
  (checks.expectEq "ssh fingerprints render" [
    "SHA256:test-fingerprint"
  ] featuresConfig.ssh_allow_fingerprints)
  (checks.expectEq "ssh timeout renders" 7 featuresConfig.ssh_filter_timeout_seconds)
  (checks.expectEq "git integration toggle renders" true featuresConfig.git_enable)
  (checks.expectEq "device binds render" [
    "/dev/input/js0"
    "/dev/kvm"
  ] featuresConfig.dev_binds)
  (checks.expectContains "extraPackages include hello in PATH" "hello" packagesStaticArgs)
  (checks.expectContains "extraPackages are included in PATH" "jq" packagesStaticArgs)
  (checks.expectTrue "worker broker profiles render" (
    workerBrokerConfig.worker_broker.profiles != { }
  ))
  (checks.expectEq "worker broker ephemeral sandbox renders" "worker"
    workerBrokerConfig.worker_broker.profiles.ephemeral.sandbox
  )
  (checks.expectEq "worker broker project sandbox renders" "worker"
    workerBrokerConfig.worker_broker.profiles.project.sandbox
  )
  (checks.expectEq "worker broker project mode renders" "project-rw"
    workerBrokerConfig.worker_broker.profiles.project.workspace.mode
  )
  (checks.expectEq "worker broker delegated mount access renders" "rw"
    workerBrokerConfig.worker_broker.profiles.project.delegated_per_dir_mounts.worktrees.mode
  )
  (checks.expectEq "worker broker generated launcher profile renders" "ephemeral"
    workerBrokerConfig.worker_broker.generated_launchers.clb-ephemeral.profile
  )
  (checks.expectEq "worker broker generated launcher sandbox renders" "worker"
    workerBrokerConfig.worker_broker.generated_launchers.clb-ephemeral.sandbox
  )
  (checks.expectEq "worker broker generated launcher metadata is keyed by launcher name" [
    "clb-ephemeral"
    "clb-project"
  ] (builtins.attrNames workerBrokerConfig.worker_broker.generated_launchers))
  (checks.expectEq "worker broker parent sandbox renders generated launchers" [
    "clb-ephemeral"
    "clb-project"
  ] (builtins.attrNames workerBrokerConfig.worker_broker.generated_launchers))
  (checks.expectEq "worker broker child sandbox does not render generated launchers" { }
    workerBrokerChildConfig.worker_broker.generated_launchers
  )
  (checks.expectEq "worker broker package exposes launcher scripts by name" [
    "clb-ephemeral"
    "clb-project"
  ] (builtins.attrNames workerBrokerInternal.workerBrokerLauncherTexts))
  (checks.expectTrue "worker broker launcher package installs generated launcher" (
    builtins.pathExists "${workerBrokerLauncherPackage}/bin/clb-ephemeral"
  ))
  (checks.expectFalse "worker broker parent package does not install generated launcher" (
    builtins.pathExists "${workerBrokerParentPackage}/bin/clb-ephemeral"
  ))
  (checks.expectFalse "worker broker child package does not install parent launcher" (
    builtins.pathExists "${workerBrokerChildPackage}/bin/clb-ephemeral"
  ))
  (checks.expectContains "worker broker launcher is a real shell script" "#!"
    workerBrokerInstalledLauncherText
  )
  (checks.expectContains "worker broker launcher requires exact usage guard"
    ''echo "clb-ephemeral: expected a command to run" >&2''
    workerBrokerInstalledLauncherText
  )
  (checks.expectContains "worker broker launcher exits with shell usage code" "exit 2"
    workerBrokerInstalledLauncherText
  )
  (checks.expectContains "worker broker launcher requires a command argv" ''if [ "$#" -eq 0 ]; then''
    workerBrokerInstalledLauncherText
  )
  (checks.expectContains "worker broker launcher invokes cloister-sandbox directly"
    ''bin/cloister-sandbox \''
    workerBrokerInstalledLauncherText
  )
  (checks.expectContains "worker broker wrapper exports runtime config path env"
    "CLOISTER_CONFIG_PATH"
    workerBrokerParentWrapperText
  )
  (checks.expectContains "worker broker launcher passes target config path" "--config /nix/store/"
    workerBrokerInstalledLauncherText
  )
  (checks.expectContains "worker broker launcher passes broker launch profile"
    "--broker-launch-profile ephemeral"
    workerBrokerInstalledLauncherText
  )
  (checks.expectContains "worker broker launcher passes broker launch sandbox"
    "--broker-launch-sandbox worker"
    workerBrokerInstalledLauncherText
  )
  (checks.expectContains "same-sandbox worker broker launcher reads config path from runtime env"
    "CLOISTER_CONFIG_PATH:?CLOISTER_CONFIG_PATH must be set"
    sameSandboxWorkerBrokerLauncherText
  )
  (checks.expectNotContains "same-sandbox worker broker launcher does not bake config store path"
    "--config /nix/store/"
    sameSandboxWorkerBrokerLauncherText
  )
  (checks.expectContains "same-sandbox worker broker launcher still targets current sandbox"
    "--broker-launch-sandbox dev"
    sameSandboxWorkerBrokerLauncherText
  )
  (checks.expectContains "worker broker launcher forwards raw argv after separator" ''-- "$@"''
    workerBrokerInstalledLauncherText
  )
  (checks.expectNotContains "worker broker launcher does not use obsolete child profile env"
    "CLOISTER_BROKER_CHILD_PROFILE"
    workerBrokerInstalledLauncherText
  )
  (checks.expectNotContains "worker broker launcher does not force shell command mode" "-c "
    workerBrokerInstalledLauncherText
  )
  (checks.expectEq "worker broker delegated worktrees path renders" "/local/worktrees/dev"
    workerBrokerConfig.worker_broker.profiles.project.delegated_per_dir_mounts.worktrees.path
  )
  (checks.expectEq "worker broker delegated worktrees subPath defaults null" null
    workerBrokerConfig.worker_broker.profiles.project.delegated_per_dir_mounts.worktrees.sub_path
  )
  (checks.expectEq "worker broker delegated pre-commit path renders" "/local/ephemeral/dev"
    workerBrokerConfig.worker_broker.profiles.project.delegated_per_dir_mounts.".cache/pre-commit".path
  )
  (checks.expectEq "worker broker delegated pre-commit subPath renders" ".cache/pre-commit"
    workerBrokerConfig.worker_broker.profiles.project.delegated_per_dir_mounts.".cache/pre-commit".sub_path
  )
  (checks.expectNotContains "worker broker rendered config does not imply env session authority"
    ''"parent_session"''
    (builtins.toJSON workerBrokerConfig.worker_broker)
  )
  (checks.expectEq "seccomp can be disabled" false seccompDisabled.seccomp_enable)
  (checks.expectContains "chromium seccomp uses chromium-named filter" "-chromium"
    chromiumSeccomp.seccomp_filter_path
  )
  (checks.expectEq "image-store mode renders" "image-store" imageStoreConfig.store_mode)
  (checks.expectContains "image-store path renders store id" ".squashfs"
    imageStoreConfig.store_image_path
  )
  (checks.expectContains "image-store mount path renders store id" "/run/cloister/images/"
    imageStoreConfig.store_mount_path
  )
  (checks.expectEq "image-store defaults to zstd level 10 with 1M blocks"
    "-comp zstd -Xcompression-level 10 -b 1M"
    imageStoreInternal.squashfsCompressionArgs
  )
  (checks.expectEq "image-store compression can be disabled" "-no-compression"
    imageStoreUncompressedEval.config.cloister._internal.sandboxInternals.dev.squashfsCompressionArgs
  )
]
