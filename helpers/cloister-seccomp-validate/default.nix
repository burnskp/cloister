{ rustPlatform }:
rustPlatform.buildRustPackage {
  pname = "cloister-seccomp-validate";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  meta.description = "Runtime seccomp filter enforcement verifier for cloister sandbox";
}
