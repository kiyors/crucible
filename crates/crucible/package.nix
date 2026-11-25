# The crucible CLI, built with the stock-cargo sibling of crucible's own
# stable toolchain, so every CI run also proves that toolchain packages a real
# crate with dependencies.
{
  lib,
  rustPlatform,
  # false is the user build (the `net` feature off): `init`, `add` and
  # `doctor`, without `update` and `pin` and the rustls and ring they compile.
  net ? true,
}:
let
  root = ../..;
  workspace = lib.importTOML (root + "/Cargo.toml");
  manifest = lib.importTOML ./Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = manifest.package.name + lib.optionalString net "-full";
  inherit (workspace.workspace.package) version;
  buildNoDefaultFeatures = !net;

  # The workspace root is the repo root, so the source is cut down to what
  # cargo reads: a README or Nix edit must not rebuild the CLI. gpg.rs
  # includes the vendored Node.js release keys from data/, and init's tests
  # check it still shares its pieces with templates/*/flake.nix.
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/data/node/release-keys.asc")
      # doctor's unit tests include_str! the shared parser corpora.
      (root + "/tests/corpora")
      (root + "/templates/default/flake.nix")
      (root + "/templates/node/flake.nix")
      (root + "/templates/python/flake.nix")
      (root + "/templates/go/flake.nix")
      (root + "/templates/zig/flake.nix")
      (root + "/templates/deno/flake.nix")
      (root + "/templates/elm/flake.nix")
      (lib.fileset.fileFilter (file: !(file.hasExt "nix" || file.hasExt "md")) (root + "/crates"))
      # init's golden flakes are Nix files the filter above would drop.
      (root + "/crates/init/tests/fixtures")
    ];
  };
  cargoLock.lockFile = root + "/Cargo.lock";

  # Only the binary's package: building the whole workspace would pull the
  # net code back into the user build.
  cargoBuildFlags = [
    "--package"
    manifest.package.name
  ];
  # The full build tests every crate. The user build tests just what it
  # compiles, so it still never builds pins or upstream.
  cargoTestFlags =
    if net then
      [ "--workspace" ]
    else
      [
        "--package"
        manifest.package.name
        "--package"
        "doctor"
        "--package"
        "init"
      ];

  meta = {
    inherit (manifest.package) description;
    homepage = "https://github.com/kiyors/crucible";
    license = lib.licenses.mit;
    mainProgram = "crucible";
  };
}
