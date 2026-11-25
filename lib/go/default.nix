# Go for one nixpkgs instance: the toolchain API, and the toolchain the flake
# ships ready-made. It lives here, next to this comment, rather than in
# toolchain.nix, so every language keeps its ready-made toolchains in one place
# and the `stable` attribute name stays the same for callers.
{ pkgs }:
let
  toolchains = import ./toolchain.nix {
    inherit (pkgs) lib;
    inherit pkgs;
  };
in
toolchains
// {
  stable = toolchains.toolchain { };
}
