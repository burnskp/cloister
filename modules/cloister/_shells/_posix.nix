{ lib }:
let
  renderWrapperInit =
    {
      configHome,
      configDir,
      initExt,
      outsideRendered,
    }:
    ''
      if [[ -n "''${CLOISTER:-}" && "''${CLOISTER}" =~ ^[A-Za-z0-9_-]+$ ]]; then
        _cloister_init="${configHome}/${configDir}/cloister-''${CLOISTER}.${initExt}"
        if [[ -f "$_cloister_init" ]]; then
          source "$_cloister_init"
        fi
        unset _cloister_init
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
      initPath,
      command,
    }:
    ''
      ${name}() {
        cl-${sandbox} ${command} -lc "source \"${initPath}\"; ${name} \"\$@\"" -- "$@"
      }
    '';

  inherit renderWrapperInit;

  mkRenderWrapperInit =
    { configDir, initExt }:
    { configHome, outsideRendered }:
    renderWrapperInit {
      inherit
        configHome
        configDir
        initExt
        outsideRendered
        ;
    };
}
