# Minimal low-trust sandbox example
#
# A sandbox with optional host-facing integrations explicitly disabled.
# Useful for running lower-trust CLI tools without network, GUI, portal,
# agent, media, printer, camera, or Git config access.
#
# Usage:
#   cd ~/some-project && cl-untrusted
#   cl-untrusted some-command arg1 arg2
#
_: {
  cloister.sandboxes.untrusted = {
    network.enable = false;

    dbus.enable = false;

    ssh.enable = false;
    gui.wayland.enable = false;
    gui.x11.enable = false;
    audio.pulseaudio.enable = false;
    audio.pipewire.enable = false;
    fido2.enable = false;
    video.enable = false;
    printing.enable = false;
    git.enable = false;
  };
}
