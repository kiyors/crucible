# The nix-cargo CLI, built with the stock-cargo sibling of nix-cargo's own
# stable toolchain, so every CI run also proves that toolchain packages a real
# crate with dependencies.
{
  lib,
  rustPlatform,
}:
let
  manifest = lib.importTOML ./Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = manifest.package.name;
  inherit (manifest.package) version;

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./src
      ./tests
    ];
  };
  cargoLock.lockFile = ./Cargo.lock;

  meta = {
    inherit (manifest.package) description;
    homepage = "https://github.com/kiyors/nix-cargo";
    license = lib.licenses.mit;
    mainProgram = "nix-cargo";
  };
}
