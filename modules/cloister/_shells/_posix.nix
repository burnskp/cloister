{ lib }:
let
  renderWrapperInit =
    { outsideRendered, ... }:
    ''
      if [[ -n "''${CLOISTER:-}" && "''${CLOISTER}" =~ ^[A-Za-z0-9_-]+$ ]]; then
        if [[ -n "''${CLOISTER_SHELL_INIT:-}" ]]; then
          source "''${CLOISTER_SHELL_INIT}"
        fi
      else
        ${outsideRendered}
      fi
    '';
in
{
  renderAlias = name: value: "alias ${name}=${lib.escapeShellArg value}";

  renderFunction = name: body: ''
    ${name}() {
    ${body}
    }
  '';

  renderOutsideFunction =
    {
      name,
      sandbox,
      command,
      ...
    }:
    ''
      ${name}() {
        __cloister_run_${sandbox} -c ${command} -lc "source \"\''${CLOISTER_SHELL_INIT:?missing cloister shell init}\"; ${name} \"\$@\"" -- "$@"
      }
    '';

  renderOutsideCommand =
    {
      name,
      sandbox,
      wrappedCommand,
    }:
    "alias ${name}=${lib.escapeShellArg "__cloister_run_${sandbox} -i ${wrappedCommand}"}";

  renderOutsideRunner = sandbox: ''
    __cloister_run_${sandbox}() {
      _cloister_args=()
      while [[ "$#" -gt 0 ]]; do
        case "$1" in
          --shell)
            _cloister_args+=("$1")
            shift
            ;;
          -c)
            _cloister_args+=("$1")
            shift
            if [[ "$#" -gt 0 ]]; then
              _cloister_args+=("$1")
              shift
            fi
            break
            ;;
          --)
            _cloister_args+=("$1")
            shift
            break
            ;;
          *)
            break
            ;;
        esac
      done

      cl-${sandbox} "''${_cloister_args[@]}" "$@"
      }
  '';

  inherit renderWrapperInit;

  mkRenderWrapperInit = _: { outsideRendered, ... }: renderWrapperInit { inherit outsideRendered; };
}
