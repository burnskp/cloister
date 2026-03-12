{
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
  meta.description = "PipeWire filter validator for cloister sandboxes";

  nativeBuildInputs = [
    pkg-config
    rustPlatform.bindgenHook
    clang
  ];
  buildInputs = [ pipewire ];

  LIBCLANG_PATH = "${libclang.lib}/lib";
}
