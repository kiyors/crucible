# The flake's checks, one set per language.
{ pkgs, crucible }:
import ./rust { inherit pkgs crucible; }
// {
  # Built and unit-tested by the stable toolchain's rustPlatform.
  crucible = crucible.cli;
}
