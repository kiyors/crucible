# Node.js for one nixpkgs instance: the toolchain API, and the toolchains the
# flake ships ready-made.
{ pkgs }:
import ./toolchain.nix {
  inherit (pkgs) lib;
  inherit pkgs;
}
