{
  description = "A Deno project with crucible";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
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

      # deno.json has no field for the Deno version, so this is the newest
      # pinned release; `version = "2";` keeps the shell on one major line.
      denoFor = system: crucible.lib.${system}.deno.toolchain { };
    in
    {
      # The Deno toolchain is the official deno executable, from the release
      # zip Deno itself publishes.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (denoFor system) ];
        };
      });
    };
}
