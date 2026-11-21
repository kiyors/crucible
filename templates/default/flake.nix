{
  description = "A Rust project with crucible";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    crucible = {
      url = "github:kiyors/crucible";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, crucible, ... }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];

      # rust-toolchain.toml is the one place the Rust version lives, for rustup
      # and CI as much as for Nix. To pin a numbered channel ("1.99"), add its
      # manifest as an input and pass it here; `crucible pin rust 1.99` prints both.
      rustFor = system: crucible.lib.${system}.rust.fromToolchainFile { file = ./rust-toolchain.toml; };
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

      # Package builds use the stock cargo with the same rustc: the fork keeps
      # its build directory outside the Nix sandbox.
      packages = forAllSystems (system: {
        default = (rustFor system).rustPlatform.buildRustPackage {
          pname = "hello";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
        };
      });
    };
}
