{
  rustPlatform,
  wayland,
  pkg-config,
}:
rustPlatform.buildRustPackage {
  pname = "cloister-sandbox";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ wayland ];
  meta.description = "Compiled sandbox runner for cloister — replaces per-sandbox bash scripts";
}
