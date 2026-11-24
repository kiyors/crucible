{
  description = "Python with CPython and uv pinned from crucible";

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

      # "3" is the newest supported CPython line. The toolchain includes pip
      # and uv; its setup hook points uv at this interpreter and stops it
      # downloading another one.
      pythonFor = system: crucible.lib.${system}.python.toolchain { version = "3"; };
    in
    {
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShellNoCC {
          packages = [ (pythonFor system) ];
        };
      });
    };
}
