# uv from its GitHub release, wrapped per Python so it uses that interpreter
# and never downloads one of its own.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  runCommand,
}:
let
  inherit (stdenv.hostPlatform) system isLinux;
  pins = lib.importJSON ../../data/python/uv.json;
  inherit (pins) version;

  source = pins.platforms.${system} or (throw "crucible: uv ${version} has no build for ${system}.");

  unwrapped = stdenv.mkDerivation {
    pname = "uv-unwrapped";
    inherit version;

    src = fetchurl { inherit (source) url hash; };

    nativeBuildInputs = lib.optionals isLinux [ autoPatchelfHook ];
    # The glibc build links libgcc_s besides glibc.
    buildInputs = lib.optionals isLinux [ stdenv.cc.cc.lib ];

    # uvx finds uv next to itself, so both stay in one directory.
    installPhase = ''
      runHook preInstall
      install -Dm755 -t $out/bin uv uvx
      runHook postInstall
    '';

    dontStrip = true;

    doInstallCheck = true;
    installCheckPhase = ''
      runHook preInstallCheck
      $out/bin/uv --version | grep -q '^uv ${version}'
      runHook postInstallCheck
    '';

    meta = {
      description = "uv, the Python package and project manager (without a Python)";
      homepage = "https://docs.astral.sh/uv";
      license = with lib.licenses; [
        asl20
        mit
      ];
      sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
      mainProgram = "uv";
      platforms = import ../common/systems.nix;
    };
  };
in
{
  inherit unwrapped;

  # uv for one Python. UV_PYTHON makes it the interpreter for every project,
  # whatever a .python-version says (a project picks its Python through
  # fromPythonVersion or fromPyprojectToml instead), UV_NO_MANAGED_PYTHON keeps
  # uv off interpreters it installed itself, and UV_PYTHON_DOWNLOADS=never
  # stops it fetching one when the request does not match.
  #
  # Decision (architecture plan 2.7): keep these as `--set-default`, not
  # `--set`. A standalone `nix run .#uv` can then still be pointed at another
  # interpreter by exporting UV_PYTHON for a single command. The tradeoff is
  # that an inherited UV_PYTHON wins outside a dev shell, so "uv uses this
  # Python" is authoritative only inside one, where the toolchain's setup hook
  # overwrites these variables. Forcing it everywhere would remove that
  # deliberate override; we keep the convenience and document the boundary
  # instead of changing behaviour silently.
  withPython =
    python:
    runCommand "uv-${version}"
      {
        nativeBuildInputs = [ makeWrapper ];
        passthru = { inherit unwrapped; };
        meta = unwrapped.meta // {
          description = "uv, the Python package and project manager";
        };
      }
      ''
        for program in uv uvx; do
          makeWrapper ${unwrapped}/bin/$program $out/bin/$program \
            --set-default UV_PYTHON ${python}/bin/python3 \
            --set-default UV_NO_MANAGED_PYTHON 1 \
            --set-default UV_PYTHON_DOWNLOADS never
        done
      '';
}
