# nix-cargo

Rust toolchains for Nix, built from the official rust-lang.org channel manifests, with
[huacnlee/cargo](https://github.com/huacnlee/cargo) as their cargo: a Cargo fork whose projects
and git worktrees share one build directory that it keeps under a size limit.

- **No intermediaries.** Components come straight from `static.rust-lang.org`, unpacked into one
  prefix the way rustup installs them, so every official binary finds its libraries where it
  expects to. The only flake input is nixpkgs.
- **Pinned and updated daily.** `data/{stable,beta,nightly}.json` hold the trimmed channel
  manifests and `sources.json` the fork's release; a scheduled workflow refreshes both.
- **Nightly is always complete.** The pinned nightly is the newest one that ships rustc, clippy,
  rustfmt, rust-src, llvm-tools, rust-analyzer and Cranelift on every supported host.
- **The fork cannot update itself** inside the read-only store (`CARGO_FORK_AUTO_UPDATE=0`).

Systems: `x86_64-linux`, `aarch64-linux`, `aarch64-darwin`, the ones both nixpkgs and the fork's
releases support.

## Outputs

| Output                           | What it is                                                    |
| -------------------------------- | ------------------------------------------------------------- |
| `packages.<system>.cargo`        | the patched cargo alone                                       |
| `packages.<system>.default`      | stable + patched cargo                                        |
| `packages.<system>.nightly`      | complete nightly with Cranelift + patched cargo               |
| `lib.<system>.mkToolchain`       | a toolchain from a channel, components and targets            |
| `lib.<system>.fromToolchainFile` | a toolchain from a project's `rust-toolchain.toml`            |
| `overlays.default`               | `pkgs.nix-cargo` (everything above) and `pkgs.huacnlee-cargo` |
| `homeManagerModules.default`     | `programs.nix-cargo`                                          |

Every toolchain also carries:

- `.stock`: the same components with the channel's own cargo
- `.rustPlatform`: `makeRustPlatform` over `.stock`, for `buildRustPackage`
- `.rust-analyzer`: the channel's rust-analyzer, built for its rustc (`null` if it has none)
- `.rustSrcPath`: the value for `RUST_SRC_PATH`

> **Dev shells, not package builds.** The fork always builds in `./target` and
> `~/.cargo/targets` and ignores `CARGO_TARGET_DIR`, which does not fit the Nix sandbox. Build
> packages with `toolchain.rustPlatform`: the channel's stock cargo, with the same rustc.

## mkToolchain

```nix
nix-cargo.lib.${system}.mkToolchain {
  channel ? "stable";   # "stable" | "beta" | "nightly", pinned in data/;
                        # or any rustup channel ("1.99", "1.99.0", "nightly-2026-11-01")
                        # with `manifest` or `sha256`
  components ? [ "rustc" "rust-std" "rust-src" "clippy" "rustfmt" "llvm-tools" ];
  targets ? [ ];        # e.g. [ "wasm32-unknown-unknown" "aarch64-unknown-linux-musl" ]
  manifest ? null;      # channel-rust-<channel>.toml as a file
  sha256 ? null;        # or that file's hash, fetched at evaluation time
}
```

Components take rustup's names (`clippy`, `rust-analyzer`, `rustc-codegen-cranelift`, ...).
`"cargo"` is ignored: the fork's cargo is always the one added.

## fromToolchainFile

Reads `channel`, `profile`, `components` and `targets` from `rust-toolchain.toml` (or the
one-line `rust-toolchain`), so the file stays the single source of truth for rustup, CI and Nix.

The neatest way to pin a numbered channel is to lock its manifest as a flake input. `nix flake
update` then moves you to the newest patch release of the series, with no hash to paste:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix-cargo = {
      url = "github:kiyors/nix-cargo";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Must match `channel` in rust-toolchain.toml.
    rust-manifest = {
      url = "https://static.rust-lang.org/dist/channel-rust-1.99.toml";
      flake = false;
    };
  };

  outputs =
    { nixpkgs, nix-cargo, rust-manifest, ... }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      rustFor =
        system:
        nix-cargo.lib.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          manifest = rust-manifest;
        };
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          rust = rustFor system;
        in
        {
          default = pkgs.mkShell {
            packages = [ rust rust.rust-analyzer ];
            RUST_SRC_PATH = rust.rustSrcPath;
          };
        }
      );

      packages = forAllSystems (system: {
        default = (rustFor system).rustPlatform.buildRustPackage {
          pname = "my-app";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
        };
      });
    };
}
```

`channel = "stable"`, `"beta"` and `"nightly"` need neither `manifest` nor `sha256`.

## devShell

```nix
devShells.default = pkgs.mkShell {
  packages = [
    (nix-cargo.lib.${system}.mkToolchain {
      targets = [ "wasm32-unknown-unknown" ];
    })
  ];
};
```

## Overlay

```nix
pkgs = import nixpkgs {
  inherit system;
  overlays = [ nix-cargo.overlays.default ];
};

# pkgs.huacnlee-cargo, pkgs.nix-cargo.default, pkgs.nix-cargo.nightly,
# pkgs.nix-cargo.mkToolchain { ... }, pkgs.nix-cargo.fromToolchainFile { ... }
```

## Home Manager

```nix
{ inputs, pkgs, ... }:
{
  imports = [ inputs.nix-cargo.homeManagerModules.default ];

  programs.nix-cargo = {
    enable = true;
    # package = inputs.nix-cargo.lib.${pkgs.stdenv.hostPlatform.system}.mkToolchain { ... };
    extraPackages = [ pkgs.cargo-edit pkgs.bacon ];
    gc = {
      max-size = "auto";
      shared-max-size = "50 GiB";
    };
    settings.net.git-fetch-with-cli = true;
  };
}
```

The module installs the toolchain and its rust-analyzer, sets `CARGO_HOME` and `RUST_SRC_PATH`,
and writes `~/.cargo/config.toml` from `settings` and `gc` (the fork's `[build.gc]`). It warns
about settings that work against the fork: `CARGO_INCREMENTAL=0` (turns off the incremental
caches it shares between worktrees), `CARGO_TARGET_DIR` (ignored) and `RUSTC_WRAPPER` caches such
as sccache or kache (they store every dependency a second time).

## How a toolchain is built

1. `lib/toolchain.nix` resolves the channel to a manifest: a pinned `data/*.json`, or a
   `channel-rust-*.toml` you pass in.
2. `lib/components.nix` fetches each component's tarball by the manifest's hash and unpacks them all
   into one prefix. On Linux it patches the host binaries with `autoPatchelf` and routes rustc's
   bundled `gcc-ld/ld.lld` through the nixpkgs ld wrapper, so binaries it links get the store's
   dynamic linker and library paths.
3. The fork's cargo is joined on top. `.stock` joins the channel's own cargo onto the same
   components instead, so the two share one copy of them.

## Updating

```sh
./update.nu                          # everything
./update.nu cargo                    # latest huacnlee/cargo release
./update.nu cargo --version 0.103.0-1
./update.nu rust                     # stable, beta and the newest complete nightly
```

The fork's hashes come from each release's `SHA256SUMS` and the Rust ones from the channel
manifests, so nothing is downloaded. The `update` workflow runs this and `nix flake update`
daily and opens a pull request.

Pull requests opened with the default `GITHUB_TOKEN` do not trigger other workflows. To have
`check` run on them automatically, give `create-pull-request` a token from a GitHub App or a
fine-grained PAT.

## Development

```sh
direnv allow          # or: nix develop ./dev
nix flake check -L    # builds a crate with every toolchain
cd dev && nix fmt
```
