{
  lib,
  symlinkJoin,
  callPackage,
}:
let
  cloister-wayland-validate = callPackage ../../helpers/cloister-wayland-validate { };
  cloister-dbus-validate = callPackage ../../helpers/cloister-dbus-validate { };
  cloister-seccomp-validate = callPackage ../../helpers/cloister-seccomp-validate { };
  cloister-pipewire-validate = callPackage ../../helpers/cloister-pipewire-validate { };
in
symlinkJoin {
  name = "cloister-diagnostics";
  paths = [
    cloister-wayland-validate
    cloister-dbus-validate
    cloister-seccomp-validate
    cloister-pipewire-validate
  ];

  meta = with lib; {
    description = "Diagnostic validator helpers for cloister sandboxes";
    homepage = "https://github.com/burnskp/cloister";
    license = licenses.mit;
    platforms = platforms.linux;
  };
}
