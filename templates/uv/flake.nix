{
  description = "Combined Python (uv) and Node.js development environment";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  inputs.crucible = {
    url = "github:kiyors/crucible";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    { nixpkgs, ... }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      forEachSupportedSystem =
        f:
        nixpkgs.lib.genAttrs supportedSystems (
          system:
          f {
            pkgs = import nixpkgs {
              inherit system;
            };
          }
        );
    in
    {
      overlays.default = _final: _prev: { };

      devShells = forEachSupportedSystem (
        { pkgs }:
        let
          python = pkgs.python313;
        in
        {
          default = pkgs.mkShellNoCC {
            UV_PYTHON = "${python}/bin/python";
            PIP_DISABLE_PIP_VERSION_CHECK = "1";

            packages = with pkgs; [
              uv
              python

              # Add whatever else you'd like here.
              # ruff
              # black
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
            '';
          };
        }
      );
    }; # <--- This was likely missing or misaligned
}
