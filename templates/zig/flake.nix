{
  description = "Zig with the newest stable release pinned from crucible";

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

      # The newest stable release by default; a build.zig.zon's minimum
      # version can be read with crucible.lib.${system}.zig.fromBuildZigZon.
      zigFor = system: crucible.lib.${system}.zig.toolchain { };
    in
    {
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShellNoCC {
          packages = [ (zigFor system) ];
        };
      });
    };
}
