{
  description = "Bun pinned from crucible, without Node.js";

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

      # Bun is one of the package managers the Node toolchain can carry.
      # `packageManager` is the bun derivation on its own, without Node.js;
      # its hash comes from oven-sh/bun's SHASUMS256.txt.
      bunFor =
        system:
        (crucible.lib.${system}.node.toolchain {
          vitePlus = false;
          packageManager = "bun@1";
        }).packageManager;
    in
    {
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShellNoCC {
          packages = [ (bunFor system) ];
        };
      });
    };
}
