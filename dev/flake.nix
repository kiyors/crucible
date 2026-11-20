{
  description = "Development environment for crucible";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";

    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # The main flake deliberately has no Home Manager input. Its module still
    # needs a check, so this dev flake owns the only one that evaluates it.
    crucible = {
      url = "path:..";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        inputs.treefmt-nix.flakeModule
        inputs.git-hooks.flakeModule
      ];

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      perSystem =
        {
          config,
          pkgs,
          lib,
          system,
          ...
        }:
        {
          treefmt = {
            # treefmt's own detection asks git for the top level. The default,
            # `.git/config`, only exists in the main checkout: from a linked
            # worktree, where `.git` is a file, it resolved to the main checkout
            # and formatted that tree instead of the one treefmt ran in.
            projectRootFile = null;
            programs = {
              nixfmt.enable = true;
              deadnix.enable = true;
              statix.enable = true;
              typos.enable = true;
              oxfmt.enable = true;
              rustfmt.enable = true;
              taplo.enable = true;
            };
            # treefmt-nix defaults rustfmt to edition 2024, but the CLI workspace
            # is 2021: the two editions sort imports differently, so `nix fmt`
            # and `cargo fmt` would keep undoing each other.
            programs.rustfmt.edition = "2021";
            # Written by `crucible update`; reformatting them would churn every update.
            settings.global.excludes = [
              "data/**/*.json"
              # Armored key data is base64: a "typo fix" silently corrupts it.
              "data/node/release-keys.asc"
              # Exact expected output of the trimmer, and a GPG signature.
              "crates/*/tests/fixtures/*.json"
              "crates/*/tests/fixtures/*.sig"
              "crates/*/tests/fixtures/shasums-*.txt"
            ];
          };

          pre-commit.settings = {
            src = ./..;
            package = pkgs.prek;
            hooks = {
              treefmt = {
                enable = true;
                package = config.treefmt.build.wrapper;
              };
              gitleaks = {
                enable = true;
                name = "gitleaks";
                entry = "${pkgs.gitleaks}/bin/gitleaks git --pre-commit --staged --redact --config ${./..}/.gitleaks.toml";
                language = "system";
                pass_filenames = false;
              };
            };
          };

          checks = {
            # The main flake must keep nixpkgs as its only input, so this is
            # where the Home Manager module's user-visible options are checked.
            home-manager-module =
              let
                hm = inputs.home-manager.lib.homeManagerConfiguration {
                  inherit pkgs;
                  modules = [
                    inputs.crucible.homeManagerModules.default
                    {
                      home.username = "tester";
                      home.homeDirectory = "/home/tester";
                      home.stateVersion = "25.11";
                      home.sessionVariables.VP_HOME = "/home/tester/.local/share/vite-plus";
                      programs.crucible.rust = {
                        enable = true;
                        settings.net.git-fetch-with-cli = true;
                        gc.max-size = "auto";
                      };
                      programs.crucible.node = {
                        enable = true;
                        npmPrefix = ".npm-global";
                        pnpmHome = ".local/share/pnpm";
                      };
                    }
                  ];
                };
              in
              assert hm.config.home.sessionVariables.CARGO_HOME == "/home/tester/.cargo";
              assert hm.config.home.sessionVariables.NPM_CONFIG_PREFIX == "/home/tester/.npm-global";
              assert hm.config.home.sessionVariables.PNPM_HOME == "/home/tester/.local/share/pnpm";
              assert lib.elem "/home/tester/.local/share/pnpm/bin" hm.config.home.sessionPath;
              assert lib.any (
                package: package == inputs.crucible.packages.${system}.rust
              ) hm.config.home.packages;
              assert lib.any (
                package: package == inputs.crucible.packages.${system}.node
              ) hm.config.home.packages;
              assert hm.config.home.file.".cargo/config.toml".source != null;
              assert lib.any (warning: builtins.match ".*VP_HOME.*" warning != null) hm.config.warnings;
              pkgs.runCommand "crucible-home-manager-check" { } "touch $out";
          };

          packages = {
            inherit (pkgs) taplo typos typos-lsp;
          };

          devShells.default = pkgs.mkShellNoCC {
            inherit (config.pre-commit) shellHook;
            packages = [
              config.packages.taplo
              config.packages.typos
              config.packages.typos-lsp
              pkgs.gitleaks
            ];
          };
        };
    };
}
