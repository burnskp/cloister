{ testLib }:
let
  inherit (testLib)
    evalConfig
    mkCheck
    mkAssertionCheck
    lib
    ;

  # Config with aliases, functions, commands for rendering tests
  renderConfig = evalConfig {
    modules = [
      {
        cloister = {
          enable = true;
          sandboxes.test = {
            shell = {
              name = "zsh";
            };
            registry = {
              aliases = {
                ll = "ls -la";
                gs = "git status";
              };
              functions = {
                mkcd = ''
                  mkdir -p "$1" && cd "$1"
                '';
              };
              commands = [
                "nvim"
                "cargo"
              ];
              noWrap = [ "gs" ];
            };
          };
        };
      }
    ];
  };

  inherit (renderConfig.cloister.sandboxes.test.registry) rendered;
in
{
  # ── Inside rendering tests ────────────────────────────────────────────

  alias-rendering-inside = mkCheck "registry-alias-rendering-inside" (
    lib.hasInfix "alias ll='ls -la'" rendered.inside
  );

  function-rendering-inside = mkCheck "registry-function-rendering-inside" (
    lib.hasInfix "mkcd() {" rendered.inside
  );

  # ── Outside rendering tests ───────────────────────────────────────────

  outside-alias-wrapping = mkCheck "registry-outside-alias-wrapping" (
    lib.hasInfix "alias ll='cl-test ls -la'" rendered.outside.zsh
  );

  noWrap-excludes =
    mkCheck "registry-noWrap-excludes"
      # gs is in noWrap, so it should NOT appear as an outside alias
      (!(lib.hasInfix "alias gs=" rendered.outside.zsh));

  # Outside command wrapping: commands become direct aliases
  outside-command-alias = mkCheck "registry-outside-command-alias" (
    lib.hasInfix "alias nvim='cl-test nvim'" rendered.outside.zsh
    && lib.hasInfix "alias cargo='cl-test cargo'" rendered.outside.zsh
  );

  # Outside function wrapping: functions rendered directly (no eval)
  outside-function-rendering = mkCheck "registry-outside-function-rendering" (
    lib.hasInfix "mkcd() {" rendered.outside.zsh && lib.hasInfix "cl-test zsh" rendered.outside.zsh
  );

  # No runtime arrays (eval elimination)
  no-runtime-arrays = mkCheck "registry-no-runtime-arrays" (
    !(lib.hasInfix "dev_shell_commands=" rendered.outside.zsh)
    && !(lib.hasInfix "dev_shell_functions=" rendered.outside.zsh)
    && !(lib.hasInfix "dev_shell_no_wrap=" rendered.outside.zsh)
  );

  # ── Assertion tests (name collisions) ─────────────────────────────────

  assertion-alias-function-overlap = mkAssertionCheck "registry-alias-function-overlap" [
    {
      cloister = {
        enable = true;
        sandboxes.test = {
          registry.aliases.dupe = "echo dupe";
          registry.functions.dupe = "echo dupe";
        };
      };
    }
  ] "both alias and function";

  assertion-alias-command-overlap = mkAssertionCheck "registry-alias-command-overlap" [
    {
      cloister = {
        enable = true;
        sandboxes.test = {
          registry.aliases.dupe = "echo dupe";
          registry.commands = [ "dupe" ];
        };
      };
    }
  ] "both alias and command";

  assertion-function-command-overlap = mkAssertionCheck "registry-function-command-overlap" [
    {
      cloister = {
        enable = true;
        sandboxes.test = {
          registry.functions.dupe = "echo dupe";
          registry.commands = [ "dupe" ];
        };
      };
    }
  ] "both function and command";

  assertion-invalid-sandbox-name = mkAssertionCheck "registry-invalid-sandbox-name" [
    {
      cloister = {
        enable = true;
        sandboxes = {
          "bad;name" = { };
        };
      };
    }
  ] "sandbox names must match";

  # ── Cross-sandbox collision assertion ──────────────────────────────────

  cross-sandbox-collision = mkAssertionCheck "registry-cross-sandbox-collision" [
    {
      cloister = {
        enable = true;
        sandboxes.a.registry.commands = [ "nvim" ];
        sandboxes.b.registry.commands = [ "nvim" ];
      };
    }
  ] "cross-sandbox name collision";

  # ── Pattern validation tests (_patterns.nix) ─────────────────────────

  # Import patterns directly — they're a standalone attrset
}
// (
  let
    patterns = import ../modules/cloister/_patterns.nix;
    inherit (testLib) mkCheck;
    m = builtins.match;
  in
  {
    # sandboxName: valid
    pattern-sandbox-name-valid-simple = mkCheck "pattern-sandbox-name-valid-simple" (
      m patterns.sandboxName "mybox" != null
    );
    pattern-sandbox-name-valid-hyphen = mkCheck "pattern-sandbox-name-valid-hyphen" (
      m patterns.sandboxName "my-box" != null
    );
    pattern-sandbox-name-valid-underscore = mkCheck "pattern-sandbox-name-valid-underscore" (
      m patterns.sandboxName "my_box" != null
    );
    pattern-sandbox-name-valid-digits = mkCheck "pattern-sandbox-name-valid-digits" (
      m patterns.sandboxName "box123" != null
    );

    # sandboxName: invalid
    pattern-sandbox-name-rejects-semicolon = mkCheck "pattern-sandbox-name-rejects-semicolon" (
      m patterns.sandboxName "a;b" == null
    );
    pattern-sandbox-name-rejects-space = mkCheck "pattern-sandbox-name-rejects-space" (
      m patterns.sandboxName "a b" == null
    );
    pattern-sandbox-name-rejects-empty = mkCheck "pattern-sandbox-name-rejects-empty" (
      m patterns.sandboxName "" == null
    );
    pattern-sandbox-name-rejects-dot = mkCheck "pattern-sandbox-name-rejects-dot" (
      m patterns.sandboxName "a.b" == null
    );

    # safeAlias: valid
    pattern-safe-alias-valid-simple = mkCheck "pattern-safe-alias-valid-simple" (
      m patterns.safeAlias "ll" != null
    );
    pattern-safe-alias-valid-dotted = mkCheck "pattern-safe-alias-valid-dotted" (
      m patterns.safeAlias "foo.bar" != null
    );
    pattern-safe-alias-valid-plus = mkCheck "pattern-safe-alias-valid-plus" (
      m patterns.safeAlias "g++" != null
    );

    # safeAlias: invalid
    pattern-safe-alias-rejects-leading-digit = mkCheck "pattern-safe-alias-rejects-leading-digit" (
      m patterns.safeAlias "1abc" == null
    );
    pattern-safe-alias-rejects-semicolon = mkCheck "pattern-safe-alias-rejects-semicolon" (
      m patterns.safeAlias "a;b" == null
    );
    pattern-safe-alias-rejects-empty = mkCheck "pattern-safe-alias-rejects-empty" (
      m patterns.safeAlias "" == null
    );

    # safeFunction: valid
    pattern-safe-function-valid-simple = mkCheck "pattern-safe-function-valid-simple" (
      m patterns.safeFunction "mkcd" != null
    );
    pattern-safe-function-valid-underscore = mkCheck "pattern-safe-function-valid-underscore" (
      m patterns.safeFunction "_helper" != null
    );

    # safeFunction: invalid
    pattern-safe-function-rejects-hyphen = mkCheck "pattern-safe-function-rejects-hyphen" (
      m patterns.safeFunction "my-func" == null
    );
    pattern-safe-function-rejects-dot = mkCheck "pattern-safe-function-rejects-dot" (
      m patterns.safeFunction "a.b" == null
    );
    pattern-safe-function-rejects-empty = mkCheck "pattern-safe-function-rejects-empty" (
      m patterns.safeFunction "" == null
    );

    # safeCommand: valid
    pattern-safe-command-valid-simple = mkCheck "pattern-safe-command-valid-simple" (
      m patterns.safeCommand "nvim" != null
    );
    pattern-safe-command-valid-dotted = mkCheck "pattern-safe-command-valid-dotted" (
      m patterns.safeCommand "node.js" != null
    );

    # safeCommand: invalid
    pattern-safe-command-rejects-leading-digit = mkCheck "pattern-safe-command-rejects-leading-digit" (
      m patterns.safeCommand "1password" == null
    );
    pattern-safe-command-rejects-space = mkCheck "pattern-safe-command-rejects-space" (
      m patterns.safeCommand "a b" == null
    );
  }
)
