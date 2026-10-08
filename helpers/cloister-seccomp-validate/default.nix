{
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage {
  pname = "cloister-seccomp-validate";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ./.
      ../cloister-seccomp-filter
    ];
  };
  sourceRoot = "source/cloister-seccomp-validate";
  cargoLock.lockFile = ./Cargo.lock;
  meta = with lib; {
    description = "Runtime seccomp filter enforcement verifier for cloister sandbox";
    homepage = "https://github.com/burnskp/cloister";
    license = licenses.mit;
    mainProgram = "cloister-seccomp-validate";
    platforms = platforms.linux;
  };
}
