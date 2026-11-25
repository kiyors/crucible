# Everything crucible provides, for one nixpkgs instance. The flake, the
# overlay and non-flake users all go through here.
{ pkgs }:
rec {
  rust = import ./lib/rust { inherit pkgs; };
  node = import ./lib/node { inherit pkgs; };
  deno = import ./lib/deno { inherit pkgs; };
  elm = import ./lib/elm { inherit pkgs; };
  python = import ./lib/python { inherit pkgs; };
  go = import ./lib/go { inherit pkgs; };
  zig = import ./lib/zig { inherit pkgs; };
  tsrs = import ./lib/tsrs { inherit pkgs; };

  # `crucible init | add | doctor`, what users install and `nix run` starts.
  # It leaves out `update` and `pin` and the TLS stack they need.
  cli = cli-full.override { net = false; };
  # The maintainer build: the above plus `update` and `pin`, for CI.
  cli-full = pkgs.callPackage ./crates/crucible/package.nix { inherit (rust.stable) rustPlatform; };
}
