# crucible

Language toolchains for Nix, built from each language's own upstream releases rather than from
source or through another overlay, and pinned from the files projects already have.

- **Rust**: toolchains from the official rust-lang.org channel manifests, with
  [huacnlee/cargo](https://github.com/huacnlee/cargo) as their cargo: a Cargo fork whose projects
  and git worktrees share one build directory that it keeps under a size limit.

- **No intermediaries.** Components come straight from `static.rust-lang.org`, unpacked into one
  prefix the way rustup installs them, so every official binary finds its libraries where it
  expects to. The only flake input is nixpkgs.
- **Pinned and updated daily.** `data/rust/{stable,beta,nightly}.json` hold the trimmed channel
  manifests and `data/rust/cargo.json` the fork's release; a scheduled workflow refreshes them
  with the repo's own CLI.
- **Nightly is always complete.** The pinned nightly is the newest one that ships rustc, clippy,
  rustfmt, rust-src, llvm-tools, rust-analyzer and Cranelift on every supported host.
- **The fork cannot update itself** inside the read-only store (`CARGO_FORK_AUTO_UPDATE=0`).

Systems: `x86_64-linux`, `aarch64-linux`, `aarch64-darwin`, the ones both nixpkgs and the fork's
releases support.

## Outputs

| Output                                | What it is                                          |
| ------------------------------------- | --------------------------------------------------- |
| `packages.<system>.rust` (`default`)  | stable + patched cargo                              |
| `packages.<system>.rust-nightly`      | complete nightly with Cranelift + patched cargo     |
| `packages.<system>.cargo`             | the patched cargo alone                             |
| `packages.<system>.crucible`          | the CLI (`update`, `doctor`, `pin`)                 |
| `lib.<system>.rust.toolchain`         | a toolchain from a channel, components and targets  |
| `lib.<system>.rust.fromToolchainFile` | a toolchain from a project's `rust-toolchain.toml`  |
| `overlays.default`                    | `pkgs.crucible`: `.rust` (everything above), `.cli` |
| `homeManagerModules.default`          | `programs.crucible.rust`                            |

Every toolchain also carries:

- `.stock`: the same components with the channel's own cargo
- `.rustPlatform`: `makeRustPlatform` over `.stock`, for `buildRustPackage`
- `.rust-analyzer`: the channel's rust-analyzer, built for its rustc (`null` if it has none or
  it was excluded)
- `.rustSrcPath`: the value for `RUST_SRC_PATH`; the toolchain's setup hook also exports it in
  any `mkShell` or build that includes the toolchain, replacing any value inherited from the
  login shell

> **Cross-compiling.** A toolchain runs on the machine that builds with it, so take it from the
> native package set and add the target: `toolchain { targets = [ "aarch64-unknown-linux-musl" ]; }`.
> Asking a `pkgsCross.*` package set for one fails with that hint.

> **`rustPlatform` limits.** `.rustPlatform.buildRustPackage` works; its `rustLibSrc`,
> `rustcSrc` and `rustVendorSrc` helpers expect a rustc built from source and are not available.

> **Dev shells, not package builds.** The fork always builds in `./target` and
> `~/.cargo/targets` and ignores `CARGO_TARGET_DIR`, which does not fit the Nix sandbox. Build
> packages with `toolchain.rustPlatform`: the channel's stock cargo, with the same rustc.

## Rust

### toolchain

```nix
crucible.lib.${system}.rust.toolchain {
  channel ? "stable";   # "stable" | "beta" | "nightly", pinned in data/rust/;
                        # or any rustup channel ("1.99", "1.99.0", "nightly-2026-11-01")
                        # with `manifest` or `sha256`
  components ? [ "rustc" "rust-std" "rust-src" "clippy" "rustfmt" "llvm-tools" ];
  targets ? [ ];        # e.g. [ "wasm32-unknown-unknown" "aarch64-unknown-linux-musl" ]
  exclude ? [ ];        # components to leave out, see "Leaving things out"
  manifest ? null;      # channel-rust-<channel>.toml as a file
  sha256 ? null;        # or that file's hash, fetched at evaluation time
}
```

Components take rustup's names (`clippy`, `rust-analyzer`, `rustc-codegen-cranelift`, ...).
`components` replaces the default list rather than adding to it. `"cargo"` is ignored: the fork's
cargo is always the one added. rust-analyzer is added whenever the channel ships it, without
being listed.

### fromToolchainFile

Reads `channel`, `profile`, `components` and `targets` from `rust-toolchain.toml` (or the
one-line `rust-toolchain`), so the file stays the single source of truth for rustup, CI and Nix.
It also takes `exclude`, for components the file's profile brings in that you do not want.

The neatest way to pin a numbered channel is to lock its manifest as a flake input. `nix flake
update` then moves you to the newest patch release of the series, with no hash to paste:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crucible = {
      url = "github:kiyors/crucible";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # Must match `channel` in rust-toolchain.toml.
    rust-manifest = {
      url = "https://static.rust-lang.org/dist/channel-rust-1.99.toml";
      flake = false;
    };
  };

  outputs =
    { nixpkgs, crucible, rust-manifest, ... }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      rustFor =
        system:
        crucible.lib.${system}.rust.fromToolchainFile {
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
          # rust-analyzer is in the toolchain when the channel ships it, and
          # its setup hook exports RUST_SRC_PATH.
          default = pkgs.mkShell { packages = [ rust ]; };
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

### Leaving things out

Every component is optional except `rustc` and `rust-std`. There are three ways to drop one:

- **`components`** replaces the defaults, so list only what you want:

  ```nix
  crucible.lib.${system}.rust.toolchain {
    components = [ "rustc" "rust-std" "rust-src" "rustfmt" ];  # no clippy, no llvm-tools
  }
  ```

- **`profile = "minimal"`** in `rust-toolchain.toml` starts from rustc, rust-std and cargo only,
  as with rustup; add the rest under `components`.

- **`exclude`** removes components after the defaults or the profile are worked out. It is the
  only way to drop part of a profile (rustup has no syntax for that), and the only way to drop
  rust-analyzer, which is otherwise in every toolchain whose channel ships it (about 75 MB):

  ```nix
  crucible.lib.${system}.rust.fromToolchainFile {
    file = ./rust-toolchain.toml;           # profile = "default"
    exclude = [ "rust-docs" "rust-analyzer" ];
  }
  ```

  Names match either spelling (`clippy` also drops `clippy-preview`). A name that is not in the
  toolchain is ignored, so one `exclude` list can serve several channels.

Some things cannot be left out, and say so at evaluation time instead of building a toolchain
that cannot compile:

| Asking for                      | Result                                                 |
| ------------------------------- | ------------------------------------------------------ |
| no `rustc` or no `rust-std`     | an error naming the missing component                  |
| `exclude = [ "cargo" ]`         | an error: use `.stock` for the channel's own cargo     |
| no `rust-src`                   | works; `.rustSrcPath` is `null` and no setup hook runs |
| `exclude = [ "rust-analyzer" ]` | works; `.rust-analyzer` is `null`                      |

The fork's cargo is always part of the toolchain itself. For the channel's own cargo use
`.stock`, which has the same components; package builds use it through `.rustPlatform`.

### devShell

```nix
devShells.default = pkgs.mkShell {
  packages = [
    (crucible.lib.${system}.rust.toolchain {
      targets = [ "wasm32-unknown-unknown" ];
    })
  ];
};
```

### Home Manager

```nix
{ inputs, pkgs, ... }:
{
  imports = [ inputs.crucible.homeManagerModules.default ];

  programs.crucible.rust = {
    enable = true;
    # package = inputs.crucible.lib.${pkgs.stdenv.hostPlatform.system}.rust.toolchain { ... };
    extraPackages = [ pkgs.cargo-edit pkgs.bacon ];
    gc = {
      max-size = "auto";
      shared-max-size = "50 GiB";
    };
    settings.net.git-fetch-with-cli = true;
  };
}
```

Setting `gc` or `settings` makes Home Manager own `~/.cargo/config.toml`: move anything already in
that file into `settings` first, or activation stops on the existing file.

The module installs the toolchain (with its rust-analyzer), sets `CARGO_HOME`, and writes
`~/.cargo/config.toml` from `settings` and `gc` (the fork's `[build.gc]`). It warns about
settings that work against the fork: `CARGO_INCREMENTAL=0` (turns off the incremental caches it
shares between worktrees), `CARGO_TARGET_DIR` (ignored) and `RUSTC_WRAPPER` caches such as
sccache or kache (they store every dependency a second time).

It does not set `RUST_SRC_PATH`. rust-analyzer finds the standard library in the sysroot of the
`rustc` a project uses, and a global value would point it at this release's sources in projects
on another one (rustup, or a dev shell pinned elsewhere). Shells opened before a switch keep the
old value until you start a new login shell; `crucible doctor` points this out.

### How a toolchain is built

1. `lib/rust/toolchain.nix` resolves the channel to a manifest: a pinned `data/rust/*.json`, or a
   `channel-rust-*.toml` you pass in.
2. `lib/rust/components.nix` turns each component tarball (rustc, rust-std for one target, clippy,
   ...) into its own derivation, fetched by the manifest's hash. On Linux it patches host binaries
   with `autoPatchelf` and routes rustc's bundled `gcc-ld/ld.lld` through the nixpkgs ld wrapper,
   so binaries it links get the store's dynamic linker and library paths.
3. A sysroot joins the components with symlinks. Only `bin/*` and `librustc_driver` are real
   copies: rustc takes its sysroot from where the driver is loaded, and the tools find their
   libraries next to themselves.
4. The fork's cargo is joined on top. `.stock` joins the channel's own cargo onto the same
   sysroot instead.

Components are keyed by their tarball alone, so every toolchain that uses the same rustc shares
one copy of it: a second toolchain on the same release (another target, another component set,
a project's `rust-toolchain.toml`) adds about 150 MB instead of 700 MB, and nothing is unpacked
twice.

## Template

```sh
nix flake init -t github:kiyors/crucible
```

A `flake.nix` with a dev shell and a package, a `rust-toolchain.toml` and an `.envrc`.

## Overlay

```nix
pkgs = import nixpkgs {
  inherit system;
  overlays = [ crucible.overlays.default ];
};

# pkgs.crucible.rust.{stable,nightly,cargo}, pkgs.crucible.cli,
# pkgs.crucible.rust.toolchain { ... }, pkgs.crucible.rust.fromToolchainFile { ... }
```

## CLI

`crucible` is a small Rust program in `cli/`, built with this flake's own toolchain:

```sh
nix run github:kiyors/crucible                   # doctor: is cargo the fork, and is anything working against it?
nix run github:kiyors/crucible -- pin rust 1.99  # the rust-manifest input and sha256 for a channel
```

`doctor` checks that the first `cargo` on PATH is the fork, and warns about other cargos and
rustup proxies on PATH, `CARGO_INCREMENTAL=0`, `RUSTC_WRAPPER` caches, `CARGO_TARGET_DIR`, a
`RUST_SRC_PATH` from another release, and a `rust-toolchain.toml` that disagrees with `rustc`. It
exits non-zero when cargo is not the fork. It is what `crucible` runs with no command.

Two hints cover problems that live outside the toolchain:

- **A shell older than the last Home Manager switch.** When one of those variables is set but the
  current `hm-session-vars.sh` no longer sets it, the shell (or the tmux or zellij server that
  started it) may still hold what an earlier generation set, such as a `RUSTC_WRAPPER` for a cache
  you since removed. A new login shell clears it. The hint is skipped when Home Manager's files
  are not there.
- **GCC as `cc` on macOS.** Crates such as `aws-lc-sys`, and build scripts that compile
  Objective-C, need Apple clang. Put `/usr/bin` earlier on PATH or set `CC=/usr/bin/cc`.

## Updating

```sh
nix run .#crucible -- update                          # everything
nix run .#crucible -- update cargo                    # latest huacnlee/cargo release
nix run .#crucible -- update cargo --version 0.103.0-1
nix run .#crucible -- update rust                     # stable, beta, newest complete nightly
```

The fork's hashes come from each release's `SHA256SUMS` and the Rust ones from the channel
manifests, so nothing is downloaded. The nightly search fetches candidate dates in parallel. The
`update` workflow runs this and `nix flake update` daily and opens a pull request.

Pull requests opened with the default `GITHUB_TOKEN` do not start other workflows, so `check`
waits for a manual run. To have it run on its own, create a GitHub App (or a fine-grained PAT with
`contents` and `pull-requests` write), store it as a secret and pass it to `create-pull-request`
as `token`.

## Development

```sh
direnv allow          # or: nix develop ./dev
nix flake check -L    # builds a crate with every toolchain
cd dev && nix fmt
cd cli && cargo test  # in a shell with the default toolchain
```
