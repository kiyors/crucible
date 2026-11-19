# Rust for one nixpkgs instance: the fork's cargo, the toolchain API, and the
# two toolchains the flake ships ready-made.
{ pkgs }:
let
  cargo = pkgs.callPackage ./cargo.nix { };

  toolchains = import ./toolchain.nix {
    inherit (pkgs) lib;
    inherit pkgs cargo;
  };
in
toolchains
// {
  inherit cargo;
  stable = toolchains.mkToolchain { };
  nightly = toolchains.mkToolchain {
    channel = "nightly";
    # On a nightly rustc the fork compiles local crates with Cranelift.
    components = toolchains.defaultComponents ++ [ "rustc-codegen-cranelift" ];
  };
}
