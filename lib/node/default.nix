# Node.js for one nixpkgs instance: the toolchain API, and the toolchains the
# flake ships ready-made. They live here, next to this comment, rather than in
# toolchain.nix, so every language keeps its ready-made toolchains in one place
# and the `lts`/`current` attribute names stay the same for callers.
{ pkgs }:
let
  toolchains = import ./toolchain.nix {
    inherit (pkgs) lib;
    inherit pkgs;
  };
in
toolchains
// {
  lts = toolchains.toolchain { };
  current = toolchains.toolchain { version = "current"; };
}
