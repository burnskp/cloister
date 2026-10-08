{
  checks,
  hm,
  ...
}:
let
  missingSandbox = hm {
    cloister = {
      enable = true;
      sandboxes.dev.workerBroker.profiles.ephemeral = {
        sandbox = "worker";
        workspace.mode = "project-overlay";
      };
    };
  };

  workerBrokerWithoutBindWorkingDirectory = hm {
    cloister = {
      enable = true;
      sandboxes.dev = {
        sandbox.bindWorkingDirectory = false;
        workerBroker.profiles.ephemeral = {
          sandbox = "worker";
          workspace.mode = "project-overlay";
        };
      };
      sandboxes.worker = { };
    };
  };

  mkDelegatedMountCase =
    mountName: mount:
    hm {
      cloister = {
        enable = true;
        sandboxes.dev = {
          workerBroker.profiles.ephemeral = {
            sandbox = "worker";
            workspace.mode = "project-overlay";
            delegatedPerDirMounts.${mountName} = mount;
          };
        };
        sandboxes.worker = { };
      };
    };

  unsafeDelegatedMountPath = mkDelegatedMountCase "worktrees" {
    mode = "rw";
    path = "$(pwd)";
  };

  relativeDelegatedMountPath = mkDelegatedMountCase "worktrees" {
    mode = "rw";
    path = "relative/base";
  };

  traversingDelegatedMountPath = mkDelegatedMountCase "worktrees" {
    mode = "rw";
    path = "/safe/base/../escape";
  };

  validAbsoluteHomeDelegatedMountPath = mkDelegatedMountCase "worktrees" {
    mode = "rw";
    path = "/home/tester/projects/worktrees";
  };

  traversingDelegatedMountSubPath = mkDelegatedMountCase "worktrees" {
    mode = "rw";
    path = "/safe/base";
    subPath = "../.ssh";
  };

  absoluteDelegatedMountKey = mkDelegatedMountCase "/etc" {
    mode = "ro";
    path = "/safe/base";
  };

  traversingDelegatedMountKey = mkDelegatedMountCase "../escape" {
    mode = "ro";
    path = "/safe/base";
  };

  unsafeDelegatedMountKey = mkDelegatedMountCase "$HOME/escape" {
    mode = "ro";
    path = "/safe/base";
  };

  newlineDelegatedMountKey = mkDelegatedMountCase "line\nescape" {
    mode = "ro";
    path = "/safe/base";
  };

  mkCollisionCase =
    attr:
    hm {
      cloister = {
        enable = true;
        sandboxes.dev = attr // {
          workerBroker.profiles.ephemeral = {
            sandbox = "worker";
            workspace.mode = "project-overlay";
          };
        };
        sandboxes.worker = { };
      };
    };

  aliasCollision = mkCollisionCase { registry.aliases.clb-ephemeral = "echo existing"; };

  functionCollision = mkCollisionCase {
    registry.functions.clb-ephemeral = ''
      echo existing
    '';
  };

  commandCollision = mkCollisionCase { registry.commands = [ "clb-ephemeral" ]; };

  crossSandboxGeneratedLauncherCollision = hm {
    cloister = {
      enable = true;
      sandboxes = {
        dev.workerBroker.profiles.ephemeral = {
          sandbox = "worker-a";
          workspace.mode = "project-overlay";
        };
        ops.workerBroker.profiles.ephemeral = {
          sandbox = "worker-b";
          workspace.mode = "project-overlay";
        };
        worker-a = { };
        worker-b = { };
      };
    };
  };

  unsafeProfileKey = hm {
    cloister = {
      enable = true;
      sandboxes.dev.workerBroker.profiles."bad/name" = {
        sandbox = "worker";
        workspace.mode = "project-overlay";
      };
      sandboxes.worker = { };
    };
  };
in
checks.mkCheck "test-cloister-worker-broker" [
  (checks.expectAssertionMessage "worker broker profiles require an existing sandbox"
    missingSandbox.assertions
    "workerBroker.profiles.ephemeral.sandbox references unknown sandbox 'worker'"
  )
  (checks.expectAssertionMessage "worker broker requires bind working directory"
    workerBrokerWithoutBindWorkingDirectory.assertions
    "workerBroker.profiles requires sandbox.bindWorkingDirectory = true"
  )
  (checks.expectAssertionMessage "worker broker delegated mount paths reject unsafe expansions"
    unsafeDelegatedMountPath.assertions
    "cannot contain unsafe variable expansions ($) or newlines"
  )
  (checks.expectAssertionMessage "worker broker delegated mount paths must be absolute host paths"
    relativeDelegatedMountPath.assertions
    "workerBroker.profiles.ephemeral.delegatedPerDirMounts.worktrees.path must be an absolute traversal-free host path"
  )
  (checks.expectAssertionMessage "worker broker delegated mount paths reject traversal"
    traversingDelegatedMountPath.assertions
    "workerBroker.profiles.ephemeral.delegatedPerDirMounts.worktrees.path must be an absolute traversal-free host path"
  )
  (checks.expectFalse "worker broker delegated mount paths allow valid absolute home paths" (
    builtins.any (assertion: !assertion.assertion) validAbsoluteHomeDelegatedMountPath.assertions
  ))
  (checks.expectAssertionMessage
    "worker broker delegated mount subpaths must stay relative descendants"
    traversingDelegatedMountSubPath.assertions
    "workerBroker.profiles.ephemeral.delegatedPerDirMounts.worktrees.subPath must be a relative descendant path without traversal"
  )
  (checks.expectAssertionMessage "worker broker delegated mount keys must stay sandbox-relative"
    absoluteDelegatedMountKey.assertions
    "workerBroker delegatedPerDirMounts keys must be sandbox-relative descendant paths"
  )
  (checks.expectAssertionMessage "worker broker delegated mount keys must reject traversal"
    traversingDelegatedMountKey.assertions
    "workerBroker delegatedPerDirMounts keys must be sandbox-relative descendant paths"
  )
  (checks.expectAssertionMessage "worker broker delegated mount keys reject variable expansions"
    unsafeDelegatedMountKey.assertions
    "cannot contain unsafe variable expansions ($) or newlines"
  )
  (checks.expectAssertionMessage "worker broker delegated mount keys reject newlines"
    newlineDelegatedMountKey.assertions
    "cannot contain unsafe variable expansions ($) or newlines"
  )
  (checks.expectAssertionMessage "worker broker generated launchers cannot collide with aliases"
    aliasCollision.assertions
    "generated worker broker launcher names collide with registry.aliases: clb-ephemeral"
  )
  (checks.expectAssertionMessage "worker broker generated launchers cannot collide with functions"
    functionCollision.assertions
    "generated worker broker launcher names collide with registry.functions: clb-ephemeral"
  )
  (checks.expectAssertionMessage "worker broker generated launchers cannot collide with commands"
    commandCollision.assertions
    "generated worker broker launcher names collide with registry.commands: clb-ephemeral"
  )
  (checks.expectFalse "generated launcher names stay sandbox-local across sandboxes" (
    builtins.any (assertion: !assertion.assertion) crossSandboxGeneratedLauncherCollision.assertions
  ))
  (checks.expectAssertionMessage
    "worker broker profile keys must produce safe generated launcher names"
    unsafeProfileKey.assertions
    "workerBroker.profiles keys must produce safe generated launcher names: bad/name"
  )
]
