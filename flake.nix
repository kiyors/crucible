{
  description = "Language toolchains for Nix, straight from upstream releases: Rust with huacnlee/cargo.";

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
          inherit (crucible.${system}) rust cli cli-doctor;
        in
        {
          default = rust.stable;
          rust = rust.stable;
          rust-nightly = rust.nightly;
          inherit (rust) cargo;
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
    };
}
