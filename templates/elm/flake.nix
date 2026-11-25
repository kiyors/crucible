{
  description = "Elm pinned from crucible, with elm2nix";

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
      # npm publishes no darwin x64 @elm_binaries package, so follow the three
      # systems crucible pins instead of nixpkgs' wider set.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: nixpkgs.legacyPackages.${system};
    in
    {
      devShells = forAllSystems (system: {
        default = (pkgsFor system).mkShellNoCC {
          packages = [
            (crucible.lib.${system}.elm.fromElmJson { file = ./elm.json; })
            (pkgsFor system).elm2nix
          ];
        };
      });
    };
}
