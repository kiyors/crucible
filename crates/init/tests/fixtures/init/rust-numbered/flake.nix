{
  description = "A Rust project with crucible";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crucible = {
      url = "github:kiyors/crucible";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Must match `channel` in rust-toolchain.
    rust-manifest = {
      url = "https://static.rust-lang.org/dist/channel-rust-1.100.toml";
      flake = false;
    };
  };

  outputs =
    {
      nixpkgs,
      crucible,
      rust-manifest,
      ...
    }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];

      # rust-toolchain is the one place the Rust version lives, for rustup
      # and CI as much as for Nix.
      rustFor =
        system:
        crucible.lib.${system}.rust.fromToolchainFile {
          file = ./rust-toolchain;
          manifest = rust-manifest;
        };
    in
    {
      # The toolchain's cargo is huacnlee/cargo: every project and worktree
      # shares one build directory under ~/.cargo/targets. Its setup hook sets
      # RUST_SRC_PATH, and rust-analyzer is part of it.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (rustFor system) ];
        };
      });
    };
}
