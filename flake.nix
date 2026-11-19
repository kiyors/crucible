{
  description = "Rust toolchains from static.rust-lang.org, with huacnlee/cargo as their cargo.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    {
      self,
      nixpkgs,
    }:
    let
      forAllSystems = nixpkgs.lib.genAttrs (import ./lib/systems.nix);

      nixCargo = forAllSystems (system: import ./. { pkgs = nixpkgs.legacyPackages.${system}; });
    in
    {
      packages = forAllSystems (system: {
        inherit (nixCargo.${system}) cargo default nightly;
      });

      lib = forAllSystems (system: {
        inherit (nixCargo.${system}) mkToolchain fromToolchainFile defaultComponents;
      });

      checks = forAllSystems (
        system:
        import ./tests {
          pkgs = nixpkgs.legacyPackages.${system};
          nixCargo = nixCargo.${system};
        }
      );

      overlays.default = import ./overlay.nix;

      homeManagerModules.default = import ./modules/home-manager.nix { inherit self; };
    };
}
