{
  description = "A Go project with crucible";

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

      # go.mod's toolchain or go directive names the oldest Go that will do,
      # as the go command reads it, and the pinned release of that line
      # satisfies it as the pins move to newer patch releases.
      goFor = system: crucible.lib.${system}.go.fromGoMod { file = ./go.mod; };
    in
    {
      # The toolchain is the official go.dev release. Its setup hook sets
      # GOTOOLCHAIN=local, so go never downloads a toolchain of its own.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (goFor system) ];
        };
      });

      # Package builds use nixpkgs' buildGoModule on the same Go. A module
      # without dependencies needs no vendorHash; once it has some, set
      # vendorHash to the hash the first build prints.
      packages = forAllSystems (system: {
        default = (goFor system).buildGoModule {
          pname = "hello-go";
          version = "0.1.0";
          src = ./.;
          vendorHash = null;
          doInstallCheck = true;
          installCheckPhase = ''
            test "$($out/bin/hello-go)" = "hello from crucible"
          '';
          meta.mainProgram = "hello-go";
        };
      });
    };
}
