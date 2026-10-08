{
  lib,
  rustPlatform,
  callPackage,
  allowedNamespaces ? [ ],
  requiredGroup ? "",
}:
let
  allowlist = builtins.concatStringsSep "\n" allowedNamespaces;
  cloister-sandbox = callPackage ../cloister-sandbox { };
in
rustPlatform.buildRustPackage {
  pname = "cloister-netns";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;

  CLOISTER_NETNS_ALLOWLIST = allowlist;
  CLOISTER_NETNS_ENFORCE_EXEC = "1";
  CLOISTER_NETNS_ALLOWED_EXEC_PATHS = "${cloister-sandbox}/bin/cloister-sandbox";
  CLOISTER_NETNS_REQUIRED_GROUP = requiredGroup;

  meta = with lib; {
    description = "Network namespace helper for cloister sandbox";
    homepage = "https://github.com/burnskp/cloister";
    license = licenses.mit;
    mainProgram = "cloister-netns";
    platforms = platforms.linux;
  };
}
