{
  description = "Jupyter with the Python pinned by pyproject.toml, Poetry, ipykernel and a .venv";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable"; # unstable Nixpkgs
  inputs.crucible = {
    url = "github:kiyors/crucible";
    inputs.nixpkgs.follows = "nixpkgs";
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
          # Poetry's "^3.11" in pyproject.toml is resolved offline to a pinned
          # CPython; Poetry and ipykernel stay app-level nixpkgs packages.
          python = crucible.lib.${system}.python.fromPyprojectToml {
            file = ./pyproject.toml;
          };
        in
        {
          default = pkgs.mkShellNoCC {
            venvDir = ".venv";
            packages = [
              python
              pkgs.poetry
              # The python3.11 package set pulls a sphinx 9.1.0 that nixpkgs
              # marks broken for 3.11; the default python3Packages set works.
              pkgs.python3Packages.ipykernel
              pkgs.python3Packages.venvShellHook
            ];
          };
        }
      );
    };
}
