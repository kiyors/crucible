{
  description = "A Elm project with crucible";

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
      forAllSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];

      # elm.json's elm-version names the compiler the project uses, resolved
      # against crucible's pins without the network.
      elmFor = system: crucible.lib.${system}.elm.fromElmJson { file = ./elm.json; };
    in
    {
      # The Elm toolchain is the official compiler from npm's signed
      # @elm_binaries packages.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (elmFor system) ];
        };
      });
    };
}
