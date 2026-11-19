# Everything crucible provides, for one nixpkgs instance. The flake, the
# overlay and non-flake users all go through here.
{ pkgs }:
rec {
  rust = import ./lib/rust { inherit pkgs; };

  # `crucible update | doctor | pin`
  cli = pkgs.callPackage ./cli/package.nix { inherit (rust.stable) rustPlatform; };
}
