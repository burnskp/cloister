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
  meta = with lib; {
    description = "Seccomp BPF filter generator for cloister sandbox";
    homepage = "https://github.com/burnskp/cloister";
    license = licenses.mit;
    mainProgram = "cloister-seccomp-filter";
    platforms = platforms.linux;
  };
}
