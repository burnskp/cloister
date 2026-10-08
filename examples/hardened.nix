# Hardened sandbox preset example
#
# Low-trust CLI sandbox for AI agents, third-party CLIs, and unknown scripts.
# Networking and host-facing integrations stay off unless you opt in.
_: {
  cloister.sandboxes.hardened = {
    preset = "hardened";

    extraPackages = [ ];

    registry.commands = [
      "bash"
      "sh"
    ];
  };
}
