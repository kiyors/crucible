# Everything nix-cargo provides, for one nixpkgs instance. The flake, the
# overlay and non-flake users all go through here.
{ pkgs }:
let
  cargo = pkgs.callPackage ./package.nix { };

  toolchains = import ./lib/toolchain.nix {
    inherit (pkgs) lib;
    inherit pkgs cargo;
  };
in
toolchains
// rec {
  inherit cargo;
  default = toolchains.mkToolchain { };
  nightly = toolchains.mkToolchain {
    channel = "nightly";
    # On a nightly rustc the fork compiles local crates with Cranelift.
    components = toolchains.defaultComponents ++ [ "rustc-codegen-cranelift" ];
  };
  # `nix-cargo update | doctor | pin`
  cli = pkgs.callPackage ./cli/package.nix { inherit (default) rustPlatform; };
}
