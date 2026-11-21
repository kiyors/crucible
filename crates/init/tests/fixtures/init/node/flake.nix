{
  description = "A Node.js project with crucible";

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

      # package.json, .node-version and packageManager stay the single source
      # of truth for nvm/corepack, CI and Nix. Nothing here touches the network
      # at evaluation time: the versions are resolved against crucible's pins.
      nodeFor = system: crucible.lib.${system}.node.fromPackageJson { file = ./package.json; };
    in
    {
      # The Node.js toolchain carries node, npm, vp and the package manager,
      # all running on its own Node.js. Its setup hook keeps vp from
      # downloading a Node.js of its own.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (nodeFor system) ];
        };
      });
    };
}
