{
  lib,
  rustPlatform,
  wayland,
  pkg-config,
}:
rustPlatform.buildRustPackage {
  pname = "cloister-wayland-validate";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ wayland ];
  meta = with lib; {
    description = "Wayland security context protocol validator for cloister sandbox";
    homepage = "https://github.com/burnskp/cloister";
    license = licenses.mit;
    mainProgram = "cloister-wayland-validate";
    platforms = platforms.linux;
  };
}
