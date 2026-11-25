{
  description = "Language toolchains for Nix, straight from upstream releases: Rust with huacnlee/cargo, Node.js with Vite+, Deno, Python with uv, Go, Zig and tsrs.";

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
            cli-full
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
          crucible-full = cli-full;
        }
      );

      # `nix run github:kiyors/crucible` is the user CLI: `init`, `add` and
      # `doctor`. The maintainer commands `update` and `pin` are in
      # `crucible-full`, which CI runs.
      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe crucible.${system}.cli;
          meta.description = "the crucible CLI: `init`, `add` and `doctor`";
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

      templates = import ./templates.nix;
    };
}
