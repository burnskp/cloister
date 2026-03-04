{
  rustPlatform,
  lib,
  libseccomp,
  pkg-config,
}:
rustPlatform.buildRustPackage {
  pname = "cloister-seccomp-filter";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ libseccomp ];
  env.LIBSECCOMP_LIB_PATH = "${lib.getLib libseccomp}/lib";
  meta.description = "Seccomp BPF filter generator for cloister sandbox";
}
