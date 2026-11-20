# Everything crucible provides, for one nixpkgs instance. The flake, the
# overlay and non-flake users all go through here.
{ pkgs }:
rec {
  rust = import ./lib/rust { inherit pkgs; };
  node = import ./lib/node { inherit pkgs; };
  deno = import ./lib/deno { inherit pkgs; };

  # `crucible update | doctor | pin`
  cli = pkgs.callPackage ./crates/crucible/package.nix { inherit (rust.stable) rustPlatform; };
  # `crucible doctor` alone, quicker to build for `nix run` on a cold cache.
  cli-doctor = cli.override { net = false; };
}
