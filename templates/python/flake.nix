{
  description = "A Python project with crucible and uv";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
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
      # The toolchain carries python3, pip and uv. Its setup hook keeps uv on
      # this Python and stops it downloading one of its own, so `uv sync` and
      # `uv run` work on the toolchain's Python.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (pythonFor system) ];
        };
      });

      # A script without dependencies needs no Python packaging: it runs on
      # the toolchain's Python alone. Projects with dependencies build them
      # from uv.lock, for example with uv2nix.
      packages = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.writeShellApplication {
          name = "hello-python";
          runtimeInputs = [ (pythonFor system).python ];
          text = ''exec python3 ${./main.py} "$@"'';
        };
      });
    };
}
