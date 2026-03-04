# Pure path functions for dangerous path detection.
# No access to config — operates only on strings.
{ lib }:
let
  dangerousPaths = [
    ".ssh"
    ".gnupg"
    ".aws"
    ".azure"
    ".config/gcloud"
    ".config/op"
    ".local/share/keyrings"
    ".docker/config.json"
    ".kube"
    ".npmrc"
    ".pypirc"
    ".netrc"
    ".config/gh/hosts.yml"
    ".config/hub"
    ".terraform.d"
    ".vault-token"
    ".config/helm"
    ".password-store"
    ".config/sops"
    ".1password"
    ".git-credentials"
    ".config/git/credentials"
    ".cargo/credentials.toml"
    ".cargo/credentials"
    ".config/nix/access-tokens"
    ".config/github-copilot"
    ".config/claude"
    ".config/netlify"
    ".config/vercel"
    ".config/doctl"
    ".config/tailscale"
    ".config/restic"
    ".config/age"
    ".config/rclone"
    ".local/share/gnupg"
    ".local/share/kwalletd"
    ".bash_history"
    ".zsh_history"
    ".local/share/fish/fish_history"
  ];

  # NOTE: This function does NOT resolve symlinks. Symlinks cannot be resolved
  # at Nix evaluation time because the Nix evaluator runs in a pure sandbox
  # without access to the host filesystem. The eval-time dangerous-path check
  # is therefore best-effort only. The actual security boundary is provided by
  # the runtime binary (cloister-sandbox validate.rs), which enforces the strict
  # home policy and blocks dot-directories inside $HOME at execution time.
  normalizeDangerousPath =
    path:
    let
      isAbsolute = lib.hasPrefix "/" path;
      parts = lib.splitString "/" path;
      normalizedParts = builtins.foldl' (
        acc: part:
        if part == "" || part == "." then
          acc
        else if part == ".." then
          if acc == [ ] then [ ] else lib.init acc
        else
          acc ++ [ part ]
      ) [ ] parts;
      joined = lib.concatStringsSep "/" normalizedParts;
    in
    if isAbsolute then (if normalizedParts == [ ] then "/" else "/${joined}") else joined;

  normalizedDangerousPaths = map normalizeDangerousPath dangerousPaths;

  pathsOverlap =
    left: right: left == right || lib.hasPrefix "${left}/" right || lib.hasPrefix "${right}/" left;

  isDangerousPath =
    path:
    let
      normalizedPath = normalizeDangerousPath path;
    in
    normalizedPath != "" && builtins.any (dp: pathsOverlap normalizedPath dp) normalizedDangerousPaths;
in
{
  inherit
    dangerousPaths
    normalizeDangerousPath
    normalizedDangerousPaths
    pathsOverlap
    isDangerousPath
    ;
}
