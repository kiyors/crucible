{
  description = "Language toolchains for Nix, straight from upstream releases: Rust with huacnlee/cargo, Node.js with Vite+, Python with uv, Go.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    {
      self,
      nixpkgs,
    }:
    let
      forAllSystems = nixpkgs.lib.genAttrs (import ./lib/common/systems.nix);

      crucible = forAllSystems (system: import ./. { pkgs = nixpkgs.legacyPackages.${system}; });
    in
    {
      packages = forAllSystems (
        system:
        let
          inherit (crucible.${system})
            rust
            node
            python
            go
            zig
            cli
            cli-doctor
            ;
        in
        {
          default = rust.stable;
          rust = rust.stable;
          rust-nightly = rust.nightly;
          inherit (rust) cargo;
          node = node.lts;
          node-current = node.current;
          vite-plus = node.lts.vite-plus;
          python = python.latest;
          inherit (python.latest) uv;
          go = go.stable;
          zig = zig.toolchain { };
          crucible = cli;
          crucible-doctor = cli-doctor;
        }
      );

      # `nix run github:kiyors/crucible` runs `doctor`, from a build without
      # the network commands: most people run it once, on a cold cache.
      # `update` and `pin` stay one `#crucible` away.
      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe crucible.${system}.cli-doctor;
          meta.description = "Diagnose a toolchain setup for huacnlee/cargo";
        };
      });

      lib = forAllSystems (system: {
        rust = {
          inherit (crucible.${system}.rust) toolchain fromToolchainFile defaultComponents;
        };
        node = {
          inherit (crucible.${system}.node) toolchain fromPackageJson;
        };
        # Deno 2 is the line with a pin today, so `toolchain { }` and
        # `toolchain { version = "2"; }` give the same release.
        deno = {
          inherit (crucible.${system}.deno) toolchain;
        };
        python = {
          inherit (crucible.${system}.python) toolchain fromPythonVersion fromPyprojectToml;
        };
        go = {
          inherit (crucible.${system}.go) toolchain fromGoMod;
        };
        zig = {
          inherit (crucible.${system}.zig) toolchain fromBuildZigZon;
        };
      });

      checks = forAllSystems (
        system:
        import ./tests {
          pkgs = nixpkgs.legacyPackages.${system};
          crucible = crucible.${system};
        }
      );

      overlays.default = import ./overlay.nix;

      homeManagerModules.default = import ./modules/home-manager.nix { inherit self; };

      templates.default = {
        path = ./templates/default;
        description = "A Rust project whose toolchain comes from rust-toolchain.toml";
      };

      templates.node = {
        path = ./templates/node;
        description = "A Node.js project with Vite+ and pnpm, pinned by package.json";
      };

      templates.python = {
        path = ./templates/python;
        description = "A Python project with uv, pinned by pyproject.toml and .python-version";
      };

      templates.go = {
        path = ./templates/go;
        description = "A Go module whose toolchain comes from go.mod";
      };

      templates.zig = {
        path = ./templates/zig;
        description = "A Zig project whose toolchain comes from build.zig.zon";
      };

      templates.deno = {
        path = ./templates/deno;
        description = "A Deno project on the newest pinned Deno release";
      };
    };
}
