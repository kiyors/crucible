{
  description = "A Rust and Node.js project with crucible";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
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

      # package.json, .node-version and packageManager stay the single source
      # of truth for nvm/corepack, CI and Nix. Nothing here touches the network
      # at evaluation time: the versions are resolved against crucible's pins.
      nodeFor = system: crucible.lib.${system}.node.fromPackageJson { file = ./package.json; };
    in
    {
      # The toolchain's cargo is huacnlee/cargo: every project and worktree
      # shares one build directory under ~/.cargo/targets. Its setup hook sets
      # RUST_SRC_PATH, and rust-analyzer is part of it.
      #
      # The Node.js toolchain carries node, npm, vp and the package manager,
      # all running on its own Node.js. Its setup hook keeps vp from
      # downloading a Node.js of its own.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [
            (rustFor system)
            (nodeFor system)
          ];
        };
      });
    };
}
