{
  description = "Node.js with pnpm pinned from crucible and run by that Node.js";

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
      # crucible pins toolchains for these systems; x86_64-darwin is not one.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;

      # pnpm comes from pnpm's GitHub releases, checked by crucible against
      # GitHub's artifact attestation, and runs on this toolchain's Node.js.
      # Vite+ is off; add `vitePlus = true` when the project uses it.
      nodeFor =
        system:
        crucible.lib.${system}.node.toolchain {
          vitePlus = false;
          packageManager = "pnpm@12";
        };
    in
    {
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShellNoCC {
          packages = [ (nodeFor system) ];
        };
      });
    };
}
