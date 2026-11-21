{
  description = "A Rust, Node.js, Deno, Python, Go and Zig project with crucible";

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

      # rust-toolchain.toml is the one place the Rust version lives, for rustup
      # and CI as much as for Nix. To pin a numbered channel ("1.99"), add its
      # manifest as an input and pass it here; `crucible pin rust 1.99` prints both.
      rustFor = system: crucible.lib.${system}.rust.fromToolchainFile { file = ./rust-toolchain.toml; };

      # package.json, .node-version and packageManager stay the single source
      # of truth for nvm/corepack, CI and Nix. Nothing here touches the network
      # at evaluation time: the versions are resolved against crucible's pins.
      nodeFor = system: crucible.lib.${system}.node.fromPackageJson { file = ./package.json; };

      # deno.jsonc has no field for the Deno version, so this is the newest
      # pinned release; `version = "2";` keeps the shell on one major line.
      denoFor = system: crucible.lib.${system}.deno.toolchain { };

      # pyproject.toml's requires-python and .python-version stay the single
      # source of truth for uv, CI and Nix, resolved against crucible's pins
      # without the network.
      pythonFor = system: crucible.lib.${system}.python.fromPyprojectToml { file = ./pyproject.toml; };

      # go.mod's toolchain or go directive names the oldest Go that will do,
      # as the go command reads it, and the pinned release of that line
      # satisfies it as the pins move to newer patch releases.
      goFor = system: crucible.lib.${system}.go.fromGoMod { file = ./go.mod; };

      # build.zig.zon has no .minimum_zig_version, so this is the newest
      # pinned release. Once it has one, fromBuildZigZon resolves it.
      zigFor = system: crucible.lib.${system}.zig.toolchain { };
    in
    {
      # The toolchain's cargo is huacnlee/cargo: every project and worktree
      # shares one build directory under ~/.cargo/targets. Its setup hook sets
      # RUST_SRC_PATH, and rust-analyzer is part of it.
      #
      # The Node.js toolchain carries node, npm, vp and the package manager,
      # all running on its own Node.js. Its setup hook keeps vp from
      # downloading a Node.js of its own.
      #
      # The Deno toolchain is the official deno executable, from the release
      # zip Deno itself publishes.
      #
      # The Python toolchain carries python3, pip and uv. Its setup hook
      # keeps uv on this Python and stops it downloading one of its own.
      #
      # The Go toolchain is the official go.dev release. Its setup hook sets
      # GOTOOLCHAIN=local, so go never downloads a toolchain of its own.
      #
      # The Zig toolchain is the official ziglang.org release, as it ships.
      devShells = forAllSystems (system: {
        default = nixpkgs.legacyPackages.${system}.mkShell {
          packages = [
            (rustFor system)
            (nodeFor system)
            (denoFor system)
            (pythonFor system)
            (goFor system)
            (zigFor system)
          ];
        };
      });
    };
}
