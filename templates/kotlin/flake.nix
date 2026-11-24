{
  description = "Kotlin on Java 25 from nixpkgs, with Gradle";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable"; # unstable Nixpkgs
  inputs.crucible = {
    url = "github:kiyors/crucible";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    inputs:

    let
      javaVersion = 25; # Change this value to update the whole stack

      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forEachSupportedSystem =
        f:
        inputs.nixpkgs.lib.genAttrs supportedSystems (
          system:
          f {
            pkgs = import inputs.nixpkgs {
              inherit system;
              overlays = [ inputs.self.overlays.default ];
            };
          }
        );
    in
    {
      overlays.default =
        _final: prev:
        let
          jdk = prev."jdk${toString javaVersion}";
        in
        {
          gradle = prev.gradle.override { java = jdk; };
          kotlin = prev.kotlin.override { jre = jdk; };
        };

      devShells = forEachSupportedSystem (
        { pkgs }:
        {
          default = pkgs.mkShellNoCC {
            packages = with pkgs; [
              gcc
              gradle
              kotlin
              ncurses
              patchelf
              zlib
            ];
          };
        }
      );
    };
}
