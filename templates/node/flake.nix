{
  description = "A Node.js project with crucible, Vite+ and pnpm";

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

      # package.json, .node-version and packageManager stay the single source
      # of truth for nvm/corepack, CI and Nix. Nothing here touches the network
      # at evaluation time: the versions are resolved against crucible's pins.
      nodeFor = system: crucible.lib.${system}.node.fromPackageJson { file = ./package.json; };
    in
    {
      # The toolchain carries node, npm, vp and pnpm, all running on its own
      # Node.js. Its setup hook keeps vp from downloading a Node.js of its own.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [ (nodeFor system) ];
        };
      });

      # Package builds use nixpkgs' builder on the same Node.js, with
      # dependencies fetched by their lockfile integrity (no npmDepsHash).
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = (nodeFor system).buildNpmPackage {
            pname = "hello-node";
            version = "0.1.0";
            src = ./.;
            npmDeps = pkgs.importNpmLock { npmRoot = ./.; };
            inherit (pkgs.importNpmLock) npmConfigHook;
            dontNpmBuild = true;
            doInstallCheck = true;
            installCheckPhase = ''
              test "$($out/bin/hello-node)" = "hello from crucible"
            '';
          };
        }
      );
    };
}
