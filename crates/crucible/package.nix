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
  root = ../..;
  workspace = lib.importTOML (root + "/Cargo.toml");
  manifest = lib.importTOML ./Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = manifest.package.name + lib.optionalString (!net) "-doctor";
  inherit (workspace.workspace.package) version;
  buildNoDefaultFeatures = !net;

  # The workspace root is the repo root, so the source is cut down to what
  # cargo reads: a README or Nix edit must not rebuild the CLI. gpg.rs
  # includes the vendored Node.js release keys from data/.
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/data/node/release-keys.asc")
      (lib.fileset.fileFilter (file: !(file.hasExt "nix" || file.hasExt "md")) (root + "/crates"))
    ];
  };
  cargoLock.lockFile = root + "/Cargo.lock";

  # Only the binary's package: building the whole workspace would pull the
  # net code back into the doctor-only build.
  cargoBuildFlags = [
    "--package"
    manifest.package.name
  ];
  cargoTestFlags = [
    "--package"
    manifest.package.name
  ];

  meta = {
    inherit (manifest.package) description;
    homepage = "https://github.com/kiyors/crucible";
    license = lib.licenses.mit;
    mainProgram = "crucible";
  };
}
