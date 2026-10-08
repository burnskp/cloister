{
  lib,
  rustPlatform,
  pkg-config,
  pipewire,
  clang,
  libclang,
}:
rustPlatform.buildRustPackage {
  pname = "cloister-pipewire-validate";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  nativeBuildInputs = [
    pkg-config
    rustPlatform.bindgenHook
    clang
  ];
  buildInputs = [ pipewire ];

  LIBCLANG_PATH = "${libclang.lib}/lib";

  meta = with lib; {
    description = "PipeWire filter validator for cloister sandboxes";
    homepage = "https://github.com/burnskp/cloister";
    license = licenses.mit;
    mainProgram = "cloister-pipewire-validate";
    platforms = platforms.linux;
  };
}
