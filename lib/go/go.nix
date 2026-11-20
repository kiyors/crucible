# One official Go release from go.dev, installed as upstream ships it: the go
# command, gofmt, the compiler tools and the standard library sources, in one
# GOROOT.
{
  lib,
  stdenvNoCC,
  fetchurl,
  buildGoModule,
}:
# { version, platforms.<system> = { url, hash } }: an entry of
# data/go/go.json, or one built from a `hashes` argument.
release:
let
  inherit (stdenvNoCC.hostPlatform) system;
  source =
    release.platforms.${system}
      or (throw "crucible: Go ${release.version} has no build for ${system}.");
in
stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "go";
  inherit (release) version;

  src = fetchurl { inherit (source) url hash; };

  # The release is used exactly as upstream built it. Its tools are static
  # Go binaries, src/ holds testdata ELF and Mach-O files that patchelf or
  # strip must not touch, stripping would break the macOS signatures, and
  # patchShebangs would rewrite scripts inside the standard library sources.
  # So no fixup at all; the setup hook is written by hand.
  dontConfigure = true;
  dontBuild = true;
  dontFixup = true;

  # go.env is the distribution's own defaults file (Go 1.21 and later), read
  # below the environment and `go env -w`. Upstream ships GOTOOLCHAIN=auto,
  # which downloads whatever toolchain a go.mod asks for and runs it from the
  # module cache instead of this one.
  #
  # bin/ holds symlinks: the go command finds GOROOT from its resolved path.
  installPhase = ''
    runHook preInstall
    mkdir -p $out/share $out/bin $out/nix-support
    cp -R . $out/share/go
    chmod u+w $out/share/go/go.env
    substituteInPlace $out/share/go/go.env \
      --replace-fail GOTOOLCHAIN=auto GOTOOLCHAIN=local
    ln -s ../share/go/bin/go ../share/go/bin/gofmt $out/bin/

    # Overwritten, not defaulted: a GOTOOLCHAIN=auto from the login shell
    # would let go switch to a downloaded toolchain, and a GOROOT left over
    # from another install would pair this go with that standard library.
    cat > $out/nix-support/setup-hook <<'EOF'
    export GOTOOLCHAIN=local
    unset GOROOT
    EOF
    runHook postInstall
  '';

  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    export HOME=$TMPDIR
    unset GOTOOLCHAIN GOROOT
    $out/bin/go version | grep -q '^go version go${finalAttrs.version} '
    test "$($out/bin/go env GOTOOLCHAIN)" = local
    test "$($out/bin/go env GOROOT)" = "$out/share/go"
    runHook postInstallCheck
  '';

  passthru = {
    # buildGoModule reads this for its default; Go itself turns cgo off when
    # it finds no C compiler.
    CGO_ENABLED = 1;
    goroot = "${finalAttrs.finalPackage}/share/go";
    buildGoModule = buildGoModule.override { go = finalAttrs.finalPackage; };
  };

  meta = {
    description = "Go ${release.version}, the official build from go.dev";
    homepage = "https://go.dev";
    changelog = "https://go.dev/doc/devel/release#go${lib.versions.majorMinor release.version}";
    license = lib.licenses.bsd3;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    mainProgram = "go";
    platforms = import ../common/systems.nix;
  };
})
