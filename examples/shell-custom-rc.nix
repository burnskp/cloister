# Shell rc subset example
#
# Two sandboxes with different zshrc subsets and host config control.
# Companion rc files live under examples/configs/zsh/.
_: {
  cloister = {
    enable = true;

    sandboxes.dev.shell = {
      name = "zsh";
      hostConfig = true;
      customRcPath = {
        zshenv = ./configs/zsh/dev.zshenv;
        zshrc = ./configs/zsh/dev.zshrc;
      };
    };

    sandboxes.work.shell = {
      name = "zsh";
      hostConfig = false;
      customRcPath = {
        zshrc = ./configs/zsh/work.zshrc;
      };
    };
  };
}
