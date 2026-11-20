# The crucible CLI, built with the stock-cargo sibling of crucible's own
# stable toolchain, so every CI run also proves that toolchain packages a real
# crate with dependencies.
{
  lib,
  rustPlatform,
  # false builds `doctor` alone (the `net` feature off), which is all a bare
  # `nix run github:kiyors/crucible` runs, without rustls and ring to compile.
  net ? true,
}:
let
  manifest = lib.importTOML ./Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = manifest.package.name + lib.optionalString (!net) "-doctor";
  inherit (manifest.package) version;
  buildNoDefaultFeatures = !net;

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./src
      ./tests
    ];
  };

  # gpg.rs includes the vendored Node.js release keys from the repo's data/.
  # Copy them into the crate's source tree at build time so the fileset can
  # stay cli-only and cargo still runs from the source root.
  postPatch = ''
    mkdir -p ../data/node
    cp ${../data/node/release-keys.asc} ../data/node/release-keys.asc
  '';
  cargoLock.lockFile = ./Cargo.lock;

  meta = {
    inherit (manifest.package) description;
    homepage = "https://github.com/kiyors/crucible";
    license = lib.licenses.mit;
    mainProgram = "crucible";
  };
}
