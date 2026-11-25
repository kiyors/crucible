{
  lib,
  stdenv,
  autoPatchelfHook,
  fetchurl,
  makeWrapper,
  zlib,
}:
let
  sourcesData = lib.importJSON ../../data/rust/cargo.json;
  inherit (sourcesData) version;
  sources = sourcesData.platforms;

  source =
    sources.${stdenv.hostPlatform.system} or (throw ''
      crucible: huacnlee/cargo has no build for ${stdenv.hostPlatform.system}.
      Use a system in lib/common/systems.nix (or a newer crucible that pins one);
      a maintainer can add one with `crucible update cargo`.
    '');

  cargoBinary = fetchurl {
    inherit (source) url hash;
  };
in
stdenv.mkDerivation {
  pname = "huacnlee-cargo";
  inherit version;

  dontUnpack = true;

  nativeBuildInputs = [
    makeWrapper
  ]
  ++ lib.optionals stdenv.hostPlatform.isLinux [
    autoPatchelfHook
  ];

  # The Linux binary links libz and libgcc_s; the macOS one only links system
  # libraries and frameworks.
  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
    zlib
    stdenv.cc.cc.lib
  ];

  installPhase = ''
    runHook preInstall

    mkdir -p $out/bin

    # The tarball holds the bare `cargo` binary, with no directory around it.
    tar xzf ${cargoBinary} -C $out/bin
    chmod 755 $out/bin/cargo

    # Once a day the fork checks for a newer release in the background and
    # renames it over its own binary, which cannot work inside /nix/store.
    # Updates come through data/rust/cargo.json instead.
    wrapProgram $out/bin/cargo \
      --set CARGO_FORK_AUTO_UPDATE 0

    runHook postInstall
  '';

  doInstallCheck = true;

  installCheckPhase = ''
    runHook preInstallCheck
    output=$(HOME=$(mktemp -d) $out/bin/cargo -V 2>&1)
    echo "$output"
    echo "$output" | grep -q "huacnlee/cargo ${version}"
    runHook postInstallCheck
  '';

  dontStrip = true;

  meta = with lib; {
    inherit version;
    description = "Cargo fork with a shared, size-bounded build directory for worktrees and agents";
    homepage = "https://github.com/huacnlee/cargo";
    license = with licenses; [
      mit
      asl20
    ];
    sourceProvenance = with lib.sourceTypes; [ binaryNativeCode ];
    mainProgram = "cargo";
    platforms = import ../common/systems.nix;
  };
}
