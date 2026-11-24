{
  description = "Python (CPython + uv) and Node.js, both pinned from crucible, with a uv venv";

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
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          # The Python toolchain carries uv and points it at its own
          # interpreter, so `uv venv` cannot fetch another Python.
          python = crucible.lib.${system}.python.toolchain { };
          node = crucible.lib.${system}.node.toolchain { vitePlus = false; };
        in
        {
          default = pkgs.mkShellNoCC {
            PIP_DISABLE_PIP_VERSION_CHECK = "1";
            packages = [
              python
              node
            ];

            shellHook = ''
              echo "Loading Hybrid Dev Environment"
              export PATH="$PWD/node_modules/.bin:$PATH"

              if [ ! -d ".venv" ]; then
                echo "Creating uv virtual environment..."
                uv venv
              fi

              source .venv/bin/activate
              echo "Versions:"
              echo "  Python: $(python --version)"
              echo "  uv:     $(uv --version)"
              echo "  Node:   $(node --version)"
            '';
          };
        }
      );
    };
}
