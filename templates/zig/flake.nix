{
  description = "A Zig project with crucible";

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

      # .minimum_zig_version in build.zig.zon is the one place the Zig
      # version lives, for zig itself as much as for Nix.
      zigFor = system: crucible.lib.${system}.zig.fromBuildZigZon { file = ./build.zig.zon; };
    in
    {
      # The toolchain is the official ziglang.org release, as it ships.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (zigFor system) ];
        };
      });

      # Package builds run `zig build` with the same Zig. A project with
      # dependencies in build.zig.zon has to provide them offline, for
      # example with `zig build --system`.
      packages = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.stdenvNoCC.mkDerivation {
          pname = "hello-zig";
          version = "0.1.0";
          src = ./.;
          nativeBuildInputs = [ (zigFor system) ];
          # zig's global cache is ~/.cache/zig, which is read-only in a build.
          buildPhase = ''
            export ZIG_GLOBAL_CACHE_DIR=$TMPDIR/zig-cache
            zig build --prefix $out -Doptimize=ReleaseSafe
          '';
          dontInstall = true;
          doInstallCheck = true;
          installCheckPhase = ''
            test "$($out/bin/hello-zig 2>&1)" = "hello from crucible"
          '';
        };
      });
    };
}
