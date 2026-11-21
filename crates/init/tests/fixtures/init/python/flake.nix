{
  description = "A Python project with crucible";

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

      # pyproject.toml's requires-python and .python-version stay the single
      # source of truth for uv, CI and Nix, resolved against crucible's pins
      # without the network.
      pythonFor = system: crucible.lib.${system}.python.fromPyprojectToml { file = ./pyproject.toml; };
    in
    {
      # The Python toolchain carries python3, pip and uv. Its setup hook
      # keeps uv on this Python and stops it downloading one of its own.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (pythonFor system) ];
        };
      });
    };
}
