# One exact package-manager release from data/node/package-managers.json,
# installed from the archive upstream ships for it and run by this toolchain's
# Node.js. pnpm and bun ship native binaries per system from their GitHub
# releases; yarn is a JavaScript package and needs the toolchain's node to
# execute it.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  nodejs, # the toolchain's Node.js, passed by toolchain.nix
  unzip,
  name, # pnpm | bun | yarn
  major,
  pin, # the matching entry of data/node/package-managers.json
}:
let
  inherit (stdenv.hostPlatform) system isLinux;
  inherit (pin) version;
  source =
    if pin ? platforms then
      pin.platforms.${system} or (throw ''
        crucible: ${name} ${version} has no build for ${system}.
        Use a system in lib/common/systems.nix, or wait for the next scheduled
        crucible update to pin one. If you maintain the pins yourself, use
        `#crucible-full` and run `crucible update node`.
      '')
    else
      pin;
  src = fetchurl { inherit (source) url hash; };
  # pnpm 10's asset is one raw executable; 11 and 12 ship a tarball with the
  # binary and its `dist/` tree (node-gyp and the rest of the JavaScript side)
  # beside it. bun's zip is handled by its name below.
  layout = pin.layout or "tarball";
  pnpmBinary = layout == "binary";
  # pnpm's exe; yarn's bin has this name too.
  exe = name;
in
stdenv.mkDerivation {
  pname = name;
  inherit version src;

  # Keep the fetched archive's root so the install paths below name it
  # directly, instead of letting mkDerivation cd into a single top directory.
  sourceRoot = ".";

  nativeBuildInputs = [
    makeWrapper
    unzip
  ]
  # pnpm 10's asset is a statically linked (macOS: Mach-O) executable: the
  # Linux builds are `pkg` snapshots that patchelf corrupts, so they must
  # run exactly as downloaded. The pinned tarballs still need patching.
  ++ lib.optionals (isLinux && !pnpmBinary) [ autoPatchelfHook ];
  # The tarballs' native binaries link glibc (and libstdc++ for pnpm);
  # pnpm 10's static one does not, but the input is harmless there.
  buildInputs = lib.optionals isLinux [ stdenv.cc.cc.lib ];

  dontConfigure = true;
  dontBuild = true;
  # bun's zip and pnpm 10's raw binary are installed straight from $src;
  # pnpm 11/12's tarballs and yarn's npm tarball are auto-unpacked.
  dontUnpack = name == "bun" || (name == "pnpm" && pnpmBinary);

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
  ''
  + (
    if name == "pnpm" then
      # The real binary lives in libexec so, for 11 and 12, its `dist/` can sit
      # beside it; pnpm finds that tree from its own path, not from bin/.
      ''
        mkdir -p $out/libexec/pnpm
      ''
      + (
        if pnpmBinary then
          ''
            install -m 0755 $src $out/libexec/pnpm/pnpm
          ''
        else
          ''
            install -m 0755 pnpm $out/libexec/pnpm/pnpm
            cp -R dist $out/libexec/pnpm/dist
          ''
      )
      + ''
        makeWrapper $out/libexec/pnpm/pnpm $out/bin/pnpm \
          --prefix PATH : ${lib.makeBinPath [ nodejs ]}
        # The npm package installs pn (pnpm) and pnx/pnpx (pnpm dlx) beside
        # pnpm, and nixpkgs does the same. GitHub's release assets hold only
        # the binary, so make those aliases here too: Node.js 25 and later
        # ship no corepack to create them, and a dev shell or Home Manager
        # user has no npm global install to bring them in.
        makeWrapper $out/libexec/pnpm/pnpm $out/bin/pn \
          --prefix PATH : ${lib.makeBinPath [ nodejs ]}
        makeWrapper $out/libexec/pnpm/pnpm $out/bin/pnx \
          --add-flags dlx \
          --prefix PATH : ${lib.makeBinPath [ nodejs ]}
        makeWrapper $out/libexec/pnpm/pnpm $out/bin/pnpx \
          --add-flags dlx \
          --prefix PATH : ${lib.makeBinPath [ nodejs ]}
      ''
    else if name == "bun" then
      # bun is not an npm package: GitHub's release ships a zip per platform,
      # holding the executable as bun-<platform>/bun.
      ''
        mkdir -p $out/bun
        unzip -qq $src -d $out/bun
        install -m 0755 $out/bun/bun-*/bun $out/bin/bun
        wrapProgram $out/bin/bun --prefix PATH : ${lib.makeBinPath [ nodejs ]}
      ''
    else
      # Both yarn majors ship bin/yarn.js; classic needs the rest of package/
      # beside it, Berry's entry is self-contained. Either way the entry is
      # JavaScript and has to run on this toolchain's node.
      ''
        mkdir -p $out/lib/node_modules/yarn
        cp -R package/. $out/lib/node_modules/yarn/
        chmod -R u+w $out/lib/node_modules/yarn
        makeWrapper ${nodejs}/bin/node $out/bin/yarn \
          --add-flags "$out/lib/node_modules/yarn/bin/yarn.js" \
          --prefix PATH : ${lib.makeBinPath [ nodejs ]}
      ''
  )
  + ''
    runHook postInstall
  '';

  # Stripping would break the signature of the native macOS binaries, and
  # RPATH shrinking cannot touch a static pnpm 10 executable.
  dontStrip = true;
  dontPatchELF = pnpmBinary;
  dontShrinkRpath = pnpmBinary;

  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    export HOME=$TMPDIR
    $out/bin/${exe} --version
  ''
  + lib.optionalString (name == "pnpm") ''
    # pnx/pnpx add `dlx`, whose --version is not the pnpm version on 10.
    test "$($out/bin/pn --version)" = "$($out/bin/pnpm --version)"
    $out/bin/pnx --help | grep -q 'Usage: pnpm dlx'
    $out/bin/pnpx --help | grep -q 'Usage: pnpm dlx'
  ''
  + lib.optionalString (name == "pnpm" && !pnpmBinary) ''
    test -x $out/libexec/pnpm/dist/node-gyp-bin/node-gyp
  ''
  + ''
    runHook postInstallCheck
  '';

  passthru = {
    inherit name major;
  };

  meta = {
    description = "${name} ${version} from upstream, run by Node.js ${nodejs.version}";
    homepage =
      if name == "pnpm" then
        "https://github.com/pnpm/pnpm"
      else if name == "bun" then
        "https://github.com/oven-sh/bun"
      else
        "https://yarnpkg.com";
    license = lib.licenses.mit;
    mainProgram = exe;
    platforms = import ../common/systems.nix;
  };
}
