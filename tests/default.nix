# The flake's checks, one set per language.
{ pkgs, crucible }:
let
  inherit (pkgs) lib;
in
import ./rust { inherit pkgs crucible; }
// {
  # Built and unit-tested by the stable toolchain's rustPlatform, with and
  # without the `net` feature.
  crucible = crucible.cli;
  crucible-doctor = crucible.cli-doctor;

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
        $slim > slim
        diff bare slim
        ! $slim pin rust 1.99 2> missing
        grep -q 'github:kiyors/crucible#crucible -- pin rust 1.99' missing
        touch $out
      '';
}
