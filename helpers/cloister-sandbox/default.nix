{
  lib,
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
  meta = with lib; {
    description = "Compiled sandbox runner for cloister - replaces per-sandbox bash scripts";
    homepage = "https://github.com/burnskp/cloister";
    license = licenses.mit;
    mainProgram = "cloister-sandbox";
    platforms = platforms.linux;
  };
}
