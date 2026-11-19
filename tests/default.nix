# The flake's checks, one set per language.
{ pkgs, crucible }:
import ./rust { inherit pkgs crucible; }
// {
  # Built and unit-tested by the stable toolchain's rustPlatform.
  crucible = crucible.cli;

  # A bare `crucible` (what `nix run github:kiyors/crucible` starts) is
  # `crucible doctor`, run here with the fork first on PATH so it passes.
  crucible-default-doctor =
    pkgs.runCommand "crucible-check-default-doctor"
      {
        nativeBuildInputs = [
          crucible.cli
          crucible.rust.stable
        ];
      }
      ''
        export HOME=$TMPDIR
        crucible > bare
        crucible doctor > named
        diff bare named
        grep -q 'huacnlee/cargo' bare
        touch $out
      '';
}
