{
  description = "Rust toolchain pinned from crucible, with native build inputs";

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
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          # The sysroot includes rustc, cargo, rustfmt, clippy, rust-src and
          # rust-analyzer. `fromToolchainFile` reads a rust-toolchain.toml when
          # the project pins a numbered channel instead.
          rust = crucible.lib.${system}.rust.toolchain { };
        in
        {
          default = pkgs.mkShell {
            packages = [
              rust
              pkgs.openssl
              pkgs.pkg-config
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
          };
        }
      );
    };
}
