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

      # .python-version is the one place the Python version lives, for uv
      # and pyenv as much as for Nix.
      pythonFor = system: crucible.lib.${system}.python.fromPythonVersion { file = ./.python-version; };
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
