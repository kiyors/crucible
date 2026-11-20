# One exact package-manager release from data/node/package-managers.json,
# installed from the tarball npm publishes for it and run by this toolchain's
# Node.js. Native pnpm and bun ship a standalone binary per system; yarn is a
# JavaScript package and needs the toolchain's node to execute it.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  nodejs, # the toolchain's Node.js, passed by toolchain.nix
  name, # pnpm | bun | yarn
  major,
  pin, # the matching entry of data/node/package-managers.json
}:
let
  inherit (stdenv.hostPlatform) system isLinux;
  inherit (pin) version;
  source =
    if pin ? platforms then
      pin.platforms.${system} or (throw "crucible: ${name} ${version} has no build for ${system}.")
    else
      pin;
  src = fetchurl { inherit (source) url hash; };
  # pnpm's exe and bun are standalone binaries; yarn's bin has this name too.
  exe = name;
in
stdenv.mkDerivation {
  pname = name;
  inherit version src;

  # npm tarballs wrap everything in package/; keep that root so the install
  # paths below name it, instead of letting mkDerivation cd into it.
  sourceRoot = ".";

  nativeBuildInputs = [ makeWrapper ] ++ lib.optionals isLinux [ autoPatchelfHook ];
  # The native binaries link glibc (and libstdc++ for pnpm).
  buildInputs = lib.optionals isLinux [ stdenv.cc.cc.lib ];

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
  ''
  + (
    if name == "pnpm" then
      ''
        install -m 0755 package/pnpm $out/bin/pnpm
        wrapProgram $out/bin/pnpm --prefix PATH : ${lib.makeBinPath [ nodejs ]}
      ''
    else if name == "bun" then
      ''
        install -m 0755 package/bin/bun $out/bin/bun
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

  # Stripping would break the signature of the native macOS binaries.
  dontStrip = true;

  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    export HOME=$TMPDIR
    $out/bin/${exe} --version
    runHook postInstallCheck
  '';

  passthru = {
    inherit name major;
  };

  meta = {
    description = "${name} ${version} from npm, run by Node.js ${nodejs.version}";
    homepage = "https://www.npmjs.com/package/${name}";
    license = lib.licenses.mit;
    mainProgram = exe;
    platforms = import ../common/systems.nix;
  };
}
