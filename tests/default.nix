# The flake's checks, one set per language.
{ pkgs, crucible }:
let
  inherit (pkgs) lib;
in
import ./rust { inherit pkgs crucible; }
// import ./node { inherit pkgs crucible; }
// import ./init.nix { inherit pkgs crucible; }
// import ./deno { inherit pkgs crucible; }
// import ./python { inherit pkgs crucible; }
// {
  # Built and unit-tested by the stable toolchain's rustPlatform, with and
  # without the `net` feature.
  crucible = crucible.cli;
  crucible-doctor = crucible.cli-doctor;

  # The Node template must resolve from its own files, with pnpm in the
  # toolchain (not an extra package the shell has to remember to add).
  node-template =
    let
      toolchain = crucible.node.fromPackageJson {
        file = ../templates/node/package.json;
      };
    in
    assert toolchain.packageManager.pname == "pnpm";
    pkgs.runCommand "crucible-check-node-template" { } "touch $out";

  # A bare `crucible` is `crucible doctor`, run here with the fork first on
  # PATH so it passes. The doctor-only build is what `nix run
  # github:kiyors/crucible` starts, so it must say the same thing and point
  # `pin` and `update` at the full CLI.
  crucible-default-doctor =
    pkgs.runCommand "crucible-check-default-doctor"
      {
        nativeBuildInputs = [ crucible.rust.stable ];
      }
      ''
        export HOME=$TMPDIR
        full=${lib.getExe crucible.cli}
        slim=${lib.getExe crucible.cli-doctor}
        $full > bare
        $full doctor > named
        diff bare named
        grep -q 'huacnlee/cargo' bare
        # The doctor-only build must still carry the Node.js checks.
        grep -q 'VP_HOME' bare
        $slim > slim
        diff bare slim
        ! $slim pin rust 1.99 2> missing
        grep -q 'github:kiyors/crucible#crucible -- pin rust 1.99' missing
        touch $out
      '';
}
