{
  description = "A Zig project with crucible";

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

      # .minimum_zig_version in build.zig.zon is the one place the Zig
      # version lives, for zig itself as much as for Nix.
      zigFor = system: crucible.lib.${system}.zig.fromBuildZigZon { file = ./build.zig.zon; };
    in
    {
      # The Zig toolchain is the official ziglang.org release, as it ships.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (zigFor system) ];
        };
      });
    };
}
