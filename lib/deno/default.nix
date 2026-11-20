# Deno for one nixpkgs instance: the toolchain API.
{ pkgs }:
import ./toolchain.nix {
  inherit (pkgs) lib;
  inherit pkgs;
}
