# One tsrs release: the native executable from the release's per-platform npm
# package, which needs no Node.js (the `tsrs` package's own bin is a Node.js
# launcher for it). The executable reads the bundled lib.*.d.ts files from
# beside its real path and panics without them, so the package goes in whole
# and bin/ only links to it. On Linux, autoPatchelf points it at the store's
# glibc.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
}:
# { version, platforms.<system> = { url, hash } }: data/tsrs/tsrs.json.
release:
let
  inherit (stdenv.hostPlatform) system isLinux;
  source =
    release.platforms.${system}
      or (throw "crucible: tsrs ${release.version} has no build for ${system}.");
in
stdenv.mkDerivation {
  pname = "tsrs";
  inherit (release) version;

  src = fetchurl { inherit (source) url hash; };

  nativeBuildInputs = [ makeWrapper ] ++ lib.optionals isLinux [ autoPatchelfHook ];
  # The Linux executable links libgcc_s besides glibc; the macOS one only
  # links system libraries.
  buildInputs = lib.optionals isLinux [ stdenv.cc.cc.lib ];

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/lib/tsrs $out/bin
    cp -R . $out/lib/tsrs/
    chmod 755 $out/lib/tsrs/tsrs
    ln -s ../lib/tsrs/tsrs $out/bin/tsrs
    # The npm package's tsrs-lint, which Oxlint runs for its type-aware rules,
    # is `tsrs --lint`.
    makeWrapper $out/lib/tsrs/tsrs $out/bin/tsrs-lint --add-flags --lint
    runHook postInstall
  '';

  passthru = {
    inherit (release) version;
  };

  meta = {
    description = "tsrs ${release.version}, a Rust port of the TypeScript 7 compiler and language server";
    homepage = "https://github.com/oxc-project/tsrs";
    license = lib.licenses.asl20;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    mainProgram = "tsrs";
    platforms = import ../common/systems.nix;
  };
}
