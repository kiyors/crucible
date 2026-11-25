# One Deno release: the deno executable, from the release zip as denoland
# ships it (a single file at the zip's root). On Linux, autoPatchelf points it
# at the store's glibc.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  unzip,
}:
# { version, platforms.<system> = { url, hash }, ... }: an entry of
# data/deno/deno.json.
release:
let
  inherit (stdenv.hostPlatform) system isLinux;
  source =
    release.platforms.${system}
      or (throw "crucible: Deno ${release.version} has no build for ${system}.");
  # The toolchain API returns this executable directly, so its `.deno`
  # passthru has to point back at it (as toolchain.nix documents).
  drv = stdenv.mkDerivation {
    pname = "deno";
    inherit (release) version;

    src = fetchurl { inherit (source) url hash; };

    nativeBuildInputs = [ unzip ] ++ lib.optionals isLinux [ autoPatchelfHook ];
    # deno links libstdc++ and libgcc_s besides glibc; the macOS build only
    # links system libraries.
    buildInputs = lib.optionals isLinux [ stdenv.cc.cc.lib ];

    dontConfigure = true;
    dontBuild = true;
    # The zip holds the executable at its root, so the build unpacks it itself.
    dontUnpack = true;

    installPhase = ''
      runHook preInstall
      mkdir -p $out/bin $out/deno
      unzip -qq $src -d $out/deno
      install -m 0755 $out/deno/deno $out/bin/deno
      runHook postInstall
    '';

    passthru = {
      inherit (release) version;
      deno = drv;
    };

    meta = {
      description = "Deno ${release.version}";
      homepage = "https://deno.land";
      license = lib.licenses.mit;
      sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
      mainProgram = "deno";
      platforms = import ../common/systems.nix;
    };
  };
in
drv
