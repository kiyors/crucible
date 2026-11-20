# One official Node.js release from nodejs.org, installed as upstream ships it:
# node, the npm it bundles, and the headers node-gyp builds addons against.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  python3,
}:
# { version, platforms.<system> = { url, hash }, ... }: an entry of
# data/node/node.json, or one built from a release's SHASUMS256.txt.
release:
let
  inherit (stdenv.hostPlatform) system isLinux;
  source =
    release.platforms.${system}
      or (throw "crucible: Node.js ${release.version} has no build for ${system}.");
in
stdenv.mkDerivation {
  pname = "nodejs";
  inherit (release) version;

  src = fetchurl { inherit (source) url hash; };

  nativeBuildInputs = lib.optionals isLinux [ autoPatchelfHook ];

  # node links libstdc++ and libgcc_s besides glibc; the macOS build only
  # links system libraries.
  buildInputs = lib.optionals isLinux [ stdenv.cc.cc.lib ];

  # patchelf 0.19 and later sort the program headers they rewrite, after which
  # the official node binary crashes at start. Older ones neither sort nor
  # know the flag, so it is passed only where it exists.
  #
  # npm, npx and corepack start with `#!/usr/bin/env node`; patchShebangs only
  # looks for interpreters among the inputs, so point it at this node, which
  # they ship with.
  preFixup = ''
    addToSearchPath HOST_PATH $out/bin
  ''
  + lib.optionalString isLinux ''
    if patchelf --help 2>&1 | grep -q -- --no-sort; then
      patchelfFlagsArray+=(--no-sort)
    fi
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -R bin include lib share $out/
    runHook postInstall
  '';

  # Stripping would invalidate the macOS binary's signature, which arm64
  # refuses to run, and gains little on an already release-built node.
  dontStrip = true;

  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    test "$($out/bin/node --version)" = "v${release.version}"
    HOME=$TMPDIR $out/bin/npm --version
    runHook postInstallCheck
  '';

  passthru = {
    # buildNpmPackage puts the interpreter node-gyp needs next to node.
    python = python3;
    lts = release.lts or null;
    # The ABI native addons are built for (process.versions.modules).
    modules = release.modules or null;
  };

  meta = {
    description = "Node.js ${release.version}, the official build from nodejs.org";
    homepage = "https://nodejs.org";
    license = lib.licenses.mit;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    mainProgram = "node";
    platforms = import ../common/systems.nix;
  };
}
