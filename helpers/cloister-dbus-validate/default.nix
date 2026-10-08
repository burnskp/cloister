{
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage {
  pname = "cloister-dbus-validate";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  meta = with lib; {
    description = "D-Bus proxy validator for cloister sandboxes";
    homepage = "https://github.com/burnskp/cloister";
    license = licenses.mit;
    mainProgram = "cloister-dbus-validate";
    platforms = platforms.linux;
  };
}
