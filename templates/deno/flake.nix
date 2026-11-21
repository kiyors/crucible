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
      # The toolchain is the official deno executable, from the release zip
      # Deno itself publishes.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (denoFor system) ];
        };
      });

      # A script without dependencies runs on the toolchain's deno as it is.
      # Dependencies from jsr: or npm: are fetched by deno at run time, so a
      # sandboxed build would need them vendored first (`deno install`).
      packages = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.writeShellApplication {
          name = "hello-deno";
          runtimeInputs = [ (denoFor system) ];
          text = ''exec deno run ${./main.ts} "$@"'';
        };
      });
    };
}
