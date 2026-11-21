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
            tsrs
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
          tsrs = tsrs.toolchain { };
          crucible = cli;
          crucible-doctor = cli-doctor;
        }
      );

      # `nix run github:kiyors/crucible` is the full CLI: `update` and `pin`
      # are in. The doctor-only `crucible-doctor` build stays for CI, where it
      # is the one build without the network commands.
      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe crucible.${system}.cli;
          meta.description = "the full CLI, with `update` and `pin`";
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
        tsrs = {
          inherit (crucible.${system}.tsrs) toolchain;
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
        description = "A Nix-flake-based Node.js development environment";
      };

      templates.python = {
        path = ./templates/python;
        description = "A Nix-flake-based Python development environment";
      };

      templates.go = {
        path = ./templates/go;
        description = "A Nix-flake-based Go development environment";
      };

      templates.zig = {
        path = ./templates/zig;
        description = "A Nix-flake-based Zig development environment";
      };

      templates.deno = {
        path = ./templates/deno;
        description = "A Nix-flake-based Deno development environment";
      };

      templates.bun = {
        path = ./templates/bun;
        description = "A Nix-flake-based Bun development environment";
      };

      templates.c-cpp = {
        path = ./templates/c-cpp;
        description = "A Nix-flake-based C/C++ development environment";
      };

      templates.elixir = {
        path = ./templates/elixir;
        description = "A Nix-flake-based Elixir development environment";
      };

      templates.elm = {
        path = ./templates/elm;
        description = "A Nix-flake-based Elm development environment";
      };

      templates.empty = {
        path = ./templates/empty;
        description = "An empty flake template that you can adapt to your own environment";
      };

      templates.hash = {
        path = ./templates/hashi;
        description = "A Nix-flake-based development environment for Terraform, Packer, and Nomad";
      };

      templates.java = {
        path = ./templates/java;
        description = "A Nix-flake-based Java development environment";
      };

      templates.jupyter = {
        path = ./templates/jupyter;
        description = "A Nix-flake-based Jupyter development environment";
      };

      templates.kotlin = {
        path = ./templates/kotlin;
        description = "A Nix-flake-based Kotlin development environment";
      };

      templates.laravel = {
        path = ./templates/laravel;
        description = "A Nix-flake-based PHP/laravel development environment";
      };

      templates.nim = {
        path = ./templates/nim;
        description = "A Nix-flake-based Nim development environment";
      };

      templates.nix = {
        path = ./templates/nix;
        description = "A Nix-flake-based Nix development environment";
      };

      templates.odin = {
        path = ./templates/odin;
        description = "A Nix-flake-based Odin development environment";
      };

      templates.php = {
        path = ./templates/php;
        description = "A Nix-flake-based PHP development environment";
      };

      templates.rust = {
        path = ./templates/rust;
        description = "A Nix-flake-based Rust development environment";
      };

      templates.scala = {
        path = ./templates/scala;
        description = "A Nix-flake-based Scala development environment";
      };

      templates.shell = {
        path = ./templates/shell;
        description = "A Nix-flake-based Shell development environment";
      };

      templates.swift = {
        path = ./templates/swift;
        description = "A Nix-flake-based Swift development environment";
      };

      templates.uv = {
        path = ./templates/uv;
        description = "Combined Python (uv) and Node.js development environment";
      };

      # Aliases for c-cpp, like the nix-templates repo has
      templates.c = {
        path = ./templates/c-cpp;
        description = "A Nix-flake-based C/C++ development environment";
      };

      templates.cpp = {
        path = ./templates/c-cpp;
        description = "A Nix-flake-based C/C++ development environment";
      };
    };
}
