{ rustPlatform }:
rustPlatform.buildRustPackage {
  pname = "cloister-dbus-validate";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  meta.description = "D-Bus proxy validator for cloister sandboxes";
}
