# Go for one nixpkgs instance: the toolchain API, and the toolchain the flake
# ships ready-made.
{ pkgs }:
import ./toolchain.nix {
  inherit (pkgs) lib;
  inherit pkgs;
}
