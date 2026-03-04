{
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
  meta.description = "Wayland security context protocol validator for cloister sandbox";
}
