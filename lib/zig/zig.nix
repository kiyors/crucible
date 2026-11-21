# One official Zig release from ziglang.org, installed as upstream ships it:
# the zig executable and its standard library (lib/) side by side, so the
# standard library is found next to the executable.
{
  lib,
  stdenv,
  fetchurl,
}:
# { version, platforms.<system> = { url, hash }, ... }: an entry of
# data/zig/zig.json.
release:
let
  inherit (stdenv.hostPlatform) system;
  source =
    release.platforms.${system}
      or (throw "crucible: Zig ${release.version} has no build for ${system}.");
in
stdenv.mkDerivation {
  pname = "zig";
  inherit (release) version;

  src = fetchurl { inherit (source) url hash; };

  # The tarball extracts as zig-<platform>-<version>/, with the executable
  # and lib/ in it. Installing it under bin/ puts `zig` on PATH of shells
  # that include the toolchain, the way the node package does for node.
  installPhase = ''
    mkdir -p $out/bin
    tar -xf $src --strip-components=1 -C $out/bin
  '';

  # Stripping would invalidate the macOS binary's signature, which arm64
  # refuses to run, and gains little on an already release-built zig.
  dontStrip = true;

  doInstallCheck = true;
  installCheckPhase = ''
    test "$($out/bin/zig version)" = "${release.version}"
  '';

  meta = {
    description = "Zig ${release.version}";
    homepage = "https://ziglang.org";
    license = lib.licenses.mit;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    mainProgram = "zig";
    platforms = import ../common/systems.nix;
  };
}
