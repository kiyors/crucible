# One Elm release: the compiler executable from the npm tarball its platform
# package publishes (a `package/` root holding `elm`, `package.json` and a
# README). On Linux, autoPatchelf points it at the store's glibc.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
}:
# { version, platforms.<system> = { url, hash }, ... }: an entry of
# data/elm/elm.json.
release:
let
  inherit (stdenv.hostPlatform) system isLinux;
  source =
    release.platforms.${system} or (throw ''
      crucible: Elm ${release.version} has no build for ${system}.
      Use a system in lib/common/systems.nix, or wait for the next scheduled
      crucible update to pin one.
    '');
  # The toolchain API returns this executable directly, so its `.elm`
  # passthru has to point back at it (as toolchain.nix documents).
  drv = stdenv.mkDerivation {
    pname = "elm";
    inherit (release) version;

    src = fetchurl { inherit (source) url hash; };

    # Keep npm's `package/` root so installPhase can name it directly instead
    # of letting mkDerivation cd into a single top directory.
    sourceRoot = ".";

    nativeBuildInputs = lib.optionals isLinux [ autoPatchelfHook ];
    # The Linux binary links libstdc++ and libgcc_s besides glibc.
    buildInputs = lib.optionals isLinux [ stdenv.cc.cc.lib ];

    dontConfigure = true;
    dontBuild = true;

    installPhase = ''
      runHook preInstall
      mkdir -p $out/bin
      install -m 0755 package/elm $out/bin/elm
      runHook postInstall
    '';

    doInstallCheck = true;
    installCheckPhase = ''
      runHook preInstallCheck
      export HOME=$TMPDIR
      test "$($out/bin/elm --version)" = "${release.version}"
      runHook postInstallCheck
    '';

    # The macOS binary is Developer ID signed; stripping it would break that
    # signature, and the Linux one needs no shrink either.
    dontStrip = true;

    passthru = {
      inherit (release) version;
      elm = drv;
    };

    meta = {
      description = "Elm ${release.version}";
      homepage = "https://elm-lang.org";
      license = lib.licenses.bsd3;
      sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
      mainProgram = "elm";
      platforms = import ../common/systems.nix;
    };
  };
in
drv
