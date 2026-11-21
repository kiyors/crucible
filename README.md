# crucible

Language toolchains for Nix, built from each language's own upstream releases rather than from
source or through another overlay, and pinned from the files projects already have.

- **Rust**: toolchains from the official rust-lang.org channel manifests, with
  [huacnlee/cargo](https://github.com/huacnlee/cargo) as their cargo: a Cargo fork whose projects
  and git worktrees share one build directory that it keeps under a size limit.
- **Node.js**: the official nodejs.org builds, with [Vite+](https://viteplus.dev) (`vp`) next to
  them, set up to use that Node.js instead of downloading its own.
- **Python**: CPython from [python-build-standalone](https://github.com/astral-sh/python-build-standalone),
  the relocatable builds uv installs, with [uv](https://docs.astral.sh/uv) next to them, set up
  to use that Python and never download one.
- **Deno**: the official denoland/deno release builds.
- **Go**: the official go.dev builds, set up so `go` never downloads another toolchain.
- **Zig**: the official ziglang.org releases, pinned from ziglang.org's own `index.json`.

- **No intermediaries.** Every toolchain is built from its project's own release archives, not
  from source. Rust's components come straight from `static.rust-lang.org`, joined into one
  prefix the way rustup installs them, so every official binary finds its libraries where it
  expects to. The only flake input is nixpkgs, and the pinned releases evaluate offline.
- **Pinned and updated about every 10 hours.** `data/` holds every pin, with the hash of every
  archive:
  - `data/rust/{stable,beta,nightly}.json`: the trimmed channel manifests, and
    `data/rust/cargo.json` the fork's release
  - `data/node/node.json`: the newest release of every supported Node.js line,
    `data/node/vite-plus.json` the Vite+ release with its dependency tree, and
    `data/node/package-managers.json` the newest pnpm, yarn and bun of every supported major
  - `data/deno/deno.json`: the newest Deno release
  - `data/python/python.json`: the newest CPython of every supported line, regular and
    freethreaded, and `data/python/uv.json` the uv release
  - `data/go/go.json`: the newest release of the two supported Go lines
  - `data/zig/zig.json`: the newest tagged release of every minor Zig line

  The scheduled `update` workflow refreshes them with the latest released CLI.

- **Nightly is always complete.** The pinned nightly is the newest one that ships rustc, clippy,
  rustfmt, rust-src, llvm-tools, rust-analyzer and Cranelift on every supported host.
- **The fork cannot update itself** inside the read-only store (`CARGO_FORK_AUTO_UPDATE=0`).

Systems: `x86_64-linux`, `aarch64-linux`, `aarch64-darwin`, the ones both nixpkgs and the fork's
releases support.

## Outputs

| Output                                  | What it is                                                       |
| --------------------------------------- | ---------------------------------------------------------------- |
| `packages.<system>.rust` (`default`)    | stable + patched cargo                                           |
| `packages.<system>.rust-nightly`        | complete nightly with Cranelift + patched cargo                  |
| `packages.<system>.cargo`               | the patched cargo alone                                          |
| `packages.<system>.crucible`            | the CLI (`update`, `doctor`, `pin`, `init`)                      |
| `packages.<system>.crucible-doctor`     | the CLI with `doctor` and `init` (`apps.default`)                |
| `lib.<system>.rust.toolchain`           | a toolchain from a channel, components and targets               |
| `lib.<system>.rust.fromToolchainFile`   | a toolchain from a project's `rust-toolchain.toml`               |
| `packages.<system>.node`                | Node.js LTS + npm + Vite+                                        |
| `packages.<system>.node-current`        | the newest Node.js line + npm + Vite+                            |
| `packages.<system>.vite-plus`           | `vp` alone, on Node.js LTS                                       |
| `lib.<system>.node.toolchain`           | a toolchain from a Node.js version or range                      |
| `lib.<system>.node.fromPackageJson`     | a toolchain from a project's `package.json`                      |
| `lib.<system>.deno.toolchain`           | a Deno release from denoland/deno's GitHub releases              |
| `packages.<system>.python`              | the newest CPython + pip + uv                                    |
| `packages.<system>.uv`                  | `uv` alone, on that CPython                                      |
| `lib.<system>.python.toolchain`         | a toolchain from a Python version or specifier                   |
| `lib.<system>.python.fromPythonVersion` | a toolchain from a project's `.python-version`                   |
| `lib.<system>.python.fromPyprojectToml` | a toolchain from a project's `pyproject.toml`                    |
| `packages.<system>.go`                  | the newest Go line                                               |
| `lib.<system>.go.toolchain`             | a toolchain from a Go line or release                            |
| `lib.<system>.go.fromGoMod`             | a toolchain from a project's `go.mod`                            |
| `packages.<system>.zig`                 | Zig (the newest pinned release)                                  |
| `lib.<system>.zig.toolchain`            | a toolchain from a Zig line or release                           |
| `lib.<system>.zig.fromBuildZigZon`      | a toolchain from a project's `build.zig.zon`                     |
| `overlays.default`                      | `pkgs.crucible`: `.rust`, `.node`, `.python`, `.go`, `.zig`, ... |
| `homeManagerModules.default`            | `programs.crucible.{rust,node,python,go,zig}`                    |
| `templates.{default,node,python,...}`   | starter projects for every language, see [Templates](#templates) |

Every Rust toolchain also carries:

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

`channel = "stable"`, `"beta"` and `"nightly"` need neither `manifest` nor `sha256`. For a
numbered channel, crucible checks the manifest's version against that series, so a stale
`rust-manifest` input fails at evaluation instead of building a different release.

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

## Node.js

### toolchain

```nix
crucible.lib.${system}.node.toolchain {
  version ? "lts";        # "lts", "current", "lts/<codename>", a line ("24"), an npm range
                          # ("^22.12.0 || >=24") or an exact release, pinned in data/node/;
                          # any other exact release ("24.11.0") with `shasums` or `sha256`
  vitePlus ? true;        # include vp, set up for this Node.js
  packageManager ? null;  # "pnpm@12" (the pinned 12.x), a range ("pnpm@^10"), an exact
                          # pinned release ("pnpm@12.21.0"), or a package built from
                          # another release; all run by this Node.js. "npm@..." needs nothing
                          # "bun@1" is pinned from oven-sh/bun's GitHub release; it needs no nixpkgs.
  shasums ? null;         # the release's SHASUMS256.txt as a file (a `flake = false` input)
  sha256 ? null;          # or that file's hash, fetched at evaluation time
}
```

A range resolves offline to the newest pinned release it allows. The pins hold the newest release
of each line the Node.js release schedule still supports (22, 24 and 26 today), so `"~22.11"`
matches nothing and says so: for that, pin the exact release.

`nix run github:kiyors/crucible#crucible -- pin node 24.11.0` prints the `SHASUMS256.txt` input
and `sha256` for a release, the counterpart of `pin rust`; it verifies the file's GPG signature
first.

Every toolchain carries:

- `.node`: the Node.js alone (node, npm, npx, and corepack before Node.js 25)
- `.vite-plus`: vp for this Node.js, or `null`
- `.packageManager`: the package manager it includes, or `null`
- `.buildNpmPackage`: nixpkgs' `buildNpmPackage` on this Node.js

### fromPackageJson

```nix
crucible.lib.${system}.node.fromPackageJson {
  file = ./package.json;
  vitePlus ? true;
  packageManager ? null; # override package.json's spec, e.g. a package for an exact release
  shasums ? null;
  sha256 ? null;
}
```

Reads the Node.js version from `devEngines.runtime` (the entry named `node`), then the project's
`.node-version` or `.nvmrc` (in that order), then `engines.node`, then falls back to the pinned
LTS. The version-manager files are explicit pins, so they beat `engines.node`'s advisory range.
The package manager comes from `packageManager`, then `devEngines.packageManager`, the order
corepack uses. An explicit `packageManager` argument overrides both.

Package managers run on the toolchain's Node.js. The newest release of each supported major is
pinned in `data/node/package-managers.json`: pnpm and yarn from the npm registry's own
integrity hashes, bun from oven-sh/bun's `SHASUMS256.txt`, one zip per platform. So
`packageManager = "pnpm@12"` selects the pinned 12.x (12.21.0 today) and
`packageManager = "pnpm@12.21.0"` is exactly that release. `packageManager = "bun@1"` resolves
to the pinned bun: every supported major has one, so it never depends on nixpkgs' one bun. A
range is used only when the pinned release, or nixpkgs' build of that major (`pnpm_12`,
`yarn-berry_4`, ...), satisfies it. An exact release that neither has is an evaluation error
rather than a silent substitution: pass a package built from that release as `packageManager`
(or `fromPackageJson`'s `packageManager` argument), or use the major. npm ships with Node.js and
needs no separate pin.

### devShell

```nix
devShells.default = pkgs.mkShell {
  packages = [
    (crucible.lib.${system}.node.fromPackageJson { file = ./package.json; })
  ];
};
```

`vp` normally manages Node.js itself: it downloads the version a project's `.node-version` or
`engines` asks for into its own data directory and runs that instead of the Node.js on PATH.
crucible's vp does not:

- It runs with `VP_ENV_FORBIDDEN=1`, so it never downloads or runs a managed Node.js or package
  manager. The toolchain's setup hook also sets it in every shell that includes the toolchain,
  replacing a value inherited from the login shell, and unsets `VP_HOME`.
- It keeps its state in `~/.local/share/crucible/vite-plus` (its own `VP_HOME`, apart from any
  standalone vp install), where its first run writes a `config.json` that puts system tools first,
  as `vp env off` would. Vite+ 1.7 reads Node.js from PATH only with that config.
- The store copy is marked as set up, so vp's first run installs no shims and edits no shell
  profiles.

Projects still install `vite-plus` as a dev dependency, as with any vp; vp uses that local copy
when it is there and its own otherwise.

### Package builds

`.buildNpmPackage` is nixpkgs' builder on the toolchain's Node.js. With `importNpmLock`, the
dependencies are fetched by their `package-lock.json` integrity hashes, so there is no
`npmDepsHash` to update:

```nix
(crucible.lib.${system}.node.toolchain { }).buildNpmPackage {
  pname = "my-app";
  version = "0.1.0";
  src = ./.;
  npmDeps = pkgs.importNpmLock { npmRoot = ./.; };
  npmConfigHook = pkgs.importNpmLock.npmConfigHook;
}
```

pnpm projects can use nixpkgs' `fetchPnpmDeps` and `pnpmConfigHook` with the toolchain's
`.packageManager` as `pnpm`.

### Home Manager

```nix
programs.crucible.node = {
  enable = true;
  # package = inputs.crucible.lib.${pkgs.stdenv.hostPlatform.system}.node.toolchain { version = "24"; };
  extraPackages = [ pkgs.typescript ];
  npmPrefix = ".npm-global";  # npm install -g goes here instead of the read-only store
  pnpmHome = ".local/share/pnpm";  # PNPM_HOME, with $PNPM_HOME/bin (pnpm 12's global bins) on PATH
};
```

### How a toolchain is built

1. `lib/node/toolchain.nix` resolves the version against `data/node/node.json`, or reads the
   release and its hashes from a `SHASUMS256.txt` you pass in.
2. `lib/node/node.nix` unpacks the official tarball as it is. On Linux, `autoPatchelf` points
   node at the store's glibc and libstdc++, with `--no-sort` wherever patchelf has it (patchelf
   0.19 and later sort program headers, which breaks the official node). npm's scripts get this
   node in their shebangs.
3. `lib/node/vite-plus.nix` installs the `vp` binary from its GitHub release and lays out the
   JavaScript packages it runs (vite-plus, Vite, Vitest, Oxlint, ...) from the release's own
   pnpm lockfile, one fetch per package by its registry integrity hash, the way pnpm's isolated
   linker arranges `node_modules`. That copy is shared by every toolchain; each one wraps it for
   its Node.js.
4. A `symlinkJoin` puts node, vp and the package manager together, with the setup hook.

## Deno

```nix
crucible.lib.${system}.deno.toolchain {
  version ? "latest";  # the newest pinned release; "2", or an exact pinned release
}
```

The toolchain carries `.deno`, the executable from the denoland/deno release, and `.version`.
The release is pinned in `data/deno/deno.json` from denoland/deno's GitHub releases, and
`crucible update deno` refreshes it; today it is the newest Deno 2, 2.9.8. A major or release
that is not pinned is an evaluation error rather than a silent substitution.

There is no `fromDenoJson`: `deno.json` (and `deno.jsonc`) pins a project's dependencies and
tasks, but has no field for the version of Deno itself, so there is nothing in it to resolve.
`crucible init` and the Deno template call `toolchain { }`; pass `version = "2"` to stay on one
major line when a newer one is pinned.

## Python

### toolchain

```nix
crucible.lib.${system}.python.toolchain {
  version ? "3";        # "3" (the newest pinned line), a line ("3.14"), a PEP 440 specifier
                        # (">=3.12,<3.15", "~=3.13.2") or an exact release, pinned in
                        # data/python/; any other release with `sha256sums`. A trailing
                        # "t" ("3.14t", "3.14.10t") asks for the freethreaded build
  uv ? true;            # include uv, set up for this Python
  sha256sums ? null;    # a python-build-standalone release's SHA256SUMS as a file
}
```

A version or specifier resolves offline to the newest pinned release it allows. The pins hold the
newest release of each line python.org still fixes (3.11 to 3.15 today, the `bugfix` and
`security` lines of its release cycle), all from one python-build-standalone build, so
`"==3.12.4"` matches nothing and says so.

The freethreaded builds (no GIL) are pinned as lines of their own next to the regular ones (3.13t
to 3.15t today), at the same versions: `version = "3.14t"` picks the freethreaded interpreter,
the same one uv installs as `3.14t`. A version without the `t` never gets a freethreaded build,
and one with it never gets a regular build. PyPy and GraalPy are not pinned, and a project asking
for one fails at evaluation time.

For a release that is not pinned, `crucible pin python <x.y.z>` prints the `SHA256SUMS` flake
input of the build that carries it, for every system, with the file's hash:

```
$ nix run github:kiyors/crucible#crucible -- pin python 3.14.0
python: 3.14.0
shasums: https://github.com/astral-sh/python-build-standalone/releases/download/20251120/SHA256SUMS
sha256: sha256-3Rc2o1sonRG5Cta3CxEs0EilA/xVdKBRFc0U7VBdPt0=

pbs-shasums = {
  url = "https://github.com/astral-sh/python-build-standalone/releases/download/20251120/SHA256SUMS";
  flake = false;
};

toolchain {
  version = "3.14.0";
  sha256sums = pbs-shasums;
};
```

Every build names the CPython releases it carries in its archive names, so the file is enough; a
`flake = false` input keeps it locked:

```nix
inputs.pbs-sums = {
  url = "https://github.com/astral-sh/python-build-standalone/releases/download/20251014/SHA256SUMS";
  flake = false;
};
# ...
crucible.lib.${system}.python.toolchain { version = "3.14.0"; sha256sums = pbs-sums; }
```

With `sha256sums` given, a `version` ending in `t` (`3.14t`) picks the freethreaded build the sums
have for every system.

Every toolchain carries:

- `.python`: the CPython alone (python3, pip, headers, the standard library)
- `.uv`: uv for this Python, or `null`
- `.pythonVersion` (`"3.14"`) and `.sitePackages` (`"lib/python3.14/site-packages"`, or
  `"lib/python3.14t/site-packages"` on a freethreaded build)

### fromPythonVersion and fromPyprojectToml

```nix
crucible.lib.${system}.python.fromPythonVersion { file = ./.python-version; uv ? true; sha256sums ? null; }
crucible.lib.${system}.python.fromPyprojectToml { file = ./pyproject.toml; uv ? true; sha256sums ? null; }
```

`fromPythonVersion` reads the first entry of a `.python-version` as uv and pyenv write it:
`3.14`, `3.14.2`, `cpython@3.14`, `cpython-3.14`, `3.14t` for the freethreaded build, or a
specifier. `fromPyprojectToml` reads the project the way uv does: the `.python-version` next to
`pyproject.toml`, then `project.requires-python`, then the newest pinned line. A
`.python-version` that `requires-python` does not allow is an error, as it is for uv.

### devShell

```nix
devShells.default = pkgs.mkShell {
  packages = [
    (crucible.lib.${system}.python.fromPyprojectToml { file = ./pyproject.toml; })
  ];
};
```

uv normally finds or downloads the Python a project asks for and keeps its own copies under
`~/.local/share/uv/python`. crucible's uv does not:

- It runs with `UV_PYTHON` set to the toolchain's `python3`, so every `uv run`, `uv sync` and
  `uv venv` uses that interpreter. This beats a project's `.python-version`, which is why the
  project's toolchain should come from `fromPythonVersion` or `fromPyprojectToml`; uv still
  refuses an interpreter that `requires-python` does not allow.
- `UV_NO_MANAGED_PYTHON=1` keeps it off Pythons it installed itself, and
  `UV_PYTHON_DOWNLOADS=never` stops it downloading one when a request (`uv python find 3.11`,
  `uv run -p 3.11`) does not match.
- The wrapper sets these as defaults, so a single command can still point elsewhere. The
  toolchain's setup hook sets them in every shell that includes the toolchain, replacing values
  inherited from the login shell, and unsets `UV_MANAGED_PYTHON` and `UV_PYTHON_PREFERENCE`.

The CPython is marked externally managed (PEP 668): `pip install` and `uv pip install --system`
refuse the read-only store and point at a virtual environment instead. Its sysconfig names its own
prefix and the shell's `cc`, `c++` and `ar`, so C extensions build in a dev shell with a C
compiler, as they would against a python.org install.

### Home Manager

```nix
programs.crucible.python = {
  enable = true;
  # package = inputs.crucible.lib.${pkgs.stdenv.hostPlatform.system}.python.toolchain { version = "3.13"; };
  extraPackages = [ pkgs.ruff ];
};
```

The module installs the toolchain and puts `~/.local/bin`, where `uv tool install` puts commands,
on PATH. It warns when `PYTHONHOME` is set (python3 would look for its standard library there),
or a `UV_PYTHON*` or `UV_MANAGED_PYTHON` session variable overrides what crucible's uv sets.

### How a toolchain is built

1. `lib/python/toolchain.nix` resolves the version against `data/python/python.json`, or reads
   the releases and their hashes from a `SHA256SUMS` you pass in. A `version` ending in `t` looks
   only at the freethreaded releases, any other only at the regular ones.
2. `lib/python/python.nix` unpacks the `install_only` archive as it is. On Linux, `autoPatchelf`
   points python at the store's glibc, libgcc_s and libcrypt.so.1. sysconfig's build-machine
   paths (`/install`, clang, the build's llvm-ar) are rewritten to this prefix and the shell's
   tools, as uv does when it installs these builds, and an `EXTERNALLY-MANAGED` file is added.
   The freethreaded builds get the same treatment under `lib/python3.14t`; they have
   `bin/python3.14t` as well as `bin/python3`.
3. `lib/python/uv.nix` installs the `uv` and `uvx` binaries from uv's GitHub release and wraps
   them for the toolchain's `python3`.
4. A `symlinkJoin` puts python and uv together, with the setup hook.

## Go

### toolchain

```nix
crucible.lib.${system}.go.toolchain {
  version ? "stable";  # "stable", a line ("1.27") or an exact release ("1.27.4", "go1.27.4"),
                       # pinned in data/go/; any other exact release with `hashes`
  hashes ? null;       # { x86_64-linux = "sha256-..."; aarch64-darwin = "sha256-..."; ... }
}
```

The pins hold the newest release of each line Go still supports, which is the two newest (1.26
and 1.27 today). A line resolves to its pinned release. An exact release has to be the pinned
one, or come with the archive hashes of the systems you build on:
`nix run github:kiyors/crucible#crucible -- pin go 1.25.3` prints the `toolchain` call with all
of them, taken from go.dev's release index.

Every toolchain carries:

- `.goroot`: its GOROOT
- `.buildGoModule`: nixpkgs' `buildGoModule` on this Go

### fromGoMod

```nix
crucible.lib.${system}.go.fromGoMod {
  file = ./go.mod;  # or ./go.work
  hashes ? null;    # for the exact release the file names, when it is not pinned
}
```

Reads the `toolchain` directive, then the `go` directive, the order the go command uses. Both
name the oldest release that will do, so `go 1.26.2` or `toolchain go1.26.2` gets the oldest
pinned release that is as new: the pinned 1.26 release (1.26.11 today), and keeps working as the
update moves the pin. A line that is no longer pinned (`go 1.21`) moves on to the oldest
pinned release, as a newer `go` builds the module anyway. A file asking for a newer release than
every pin is an evaluation error that says to update crucible or pass `hashes`.

### devShell

```nix
devShells.default = pkgs.mkShell {
  packages = [
    (crucible.lib.${system}.go.fromGoMod { file = ./go.mod; })
    pkgs.gopls
  ];
};
```

Since Go 1.21, `go` switches to whatever toolchain a module's `go.mod` asks for, downloading it
into the module cache (`GOTOOLCHAIN=auto`). crucible's go does not:

- Its own `go.env` sets `GOTOOLCHAIN=local`, so a module that needs a newer Go stops with
  `go.mod requires go >= ... (running go ...; GOTOOLCHAIN=local)` instead of running something
  that is not in the store. This holds wherever the toolchain is installed, Home Manager
  included.
- The toolchain's setup hook also exports `GOTOOLCHAIN=local` in every shell or build that
  includes it, replacing a value inherited from the login shell or written by `go env -w`, and
  unsets `GOROOT`, which would otherwise pair this `go` with another installation's standard
  library.

cgo uses the shell's C compiler, as with any `go`. On Linux, programs it links get the store's
dynamic linker from the nixpkgs compiler wrapper, so they run on NixOS. Without a C compiler on
PATH, Go turns cgo off by itself.

**gopls is not part of the toolchain.** rust-analyzer is in Rust toolchains because the channel
ships it, prebuilt for its rustc. Go releases ship no gopls, so it would have to be built from
source, and it needs no matching build: it runs the `go` on PATH, and supports the current and
previous Go releases. Use nixpkgs' `pkgs.gopls`, which is built with the newest Go.

### Package builds

`.buildGoModule` is nixpkgs' builder on the toolchain's Go, with the usual `vendorHash`:

```nix
(crucible.lib.${system}.go.fromGoMod { file = ./go.mod; }).buildGoModule {
  pname = "my-app";
  version = "0.1.0";
  src = ./.;
  vendorHash = null; # or the hash of the vendored dependencies
}
```

### Home Manager

```nix
programs.crucible.go = {
  enable = true;
  # package = inputs.crucible.lib.${pkgs.stdenv.hostPlatform.system}.go.toolchain { version = "1.26"; };
  extraPackages = [ pkgs.gopls pkgs.golangci-lint ];
  goPath = "go";  # the default: GOPATH=~/go, with ~/go/bin (`go install`) on PATH
};
```

It warns when `GOROOT` is set, and when `GOTOOLCHAIN` is set to anything but `local`.

### How a toolchain is built

1. `lib/go/toolchain.nix` resolves the version against `data/go/go.json`, or builds the release
   from the `hashes` you pass in.
2. `lib/go/go.nix` unpacks the official archive into `share/go` (the GOROOT) as it is, with no
   fixup: its tools are static, stripping would break the macOS signatures, and the standard
   library sources hold test binaries that must not be patched. The only change is
   `GOTOOLCHAIN=local` in `go.env`. `bin/go` and `bin/gofmt` are symlinks, from which `go` finds
   its GOROOT.

## Zig

### toolchain

```nix
crucible.lib.${system}.zig.toolchain {
  version ? "stable";  # the newest pinned release;
                       # or a line ("0.15" -> the newest patch of that line),
                       # or an exact pinned release ("0.15.2")
}
```

A line or release that is not pinned is an error rather than a download: `crucible update zig`
pins the newest patch of every minor line in ziglang.org's `index.json`.

Every toolchain carries:

- `.zig`: the zig executable alone
- `.version`: the release it was built from

### fromBuildZigZon

```nix
crucible.lib.${system}.zig.fromBuildZigZon { file = ./build.zig.zon; }
```

Reads `.minimum_zig_version` from the project's `build.zig.zon` (the string form Zig writes
since 0.13, or the tuple form before it) and returns the toolchain for it. Like Zig itself, it
reads the version as the oldest release that will do: `"0.15.0"` gets the pinned 0.15 release, and
keeps working as the update moves the pin to each new patch. It never moves to another line,
since Zig changes its language between 0.x lines, so a minimum newer than its line's pin, or on
a line that is not pinned, is an evaluation error.

### devShell

```nix
devShells.default = pkgs.mkShell {
  packages = [ (crucible.lib.${system}.zig.toolchain { }) ];
};
```

### Home Manager

```nix
programs.crucible.zig = {
  enable = true;
  # package = inputs.crucible.lib.${pkgs.stdenv.hostPlatform.system}.zig.toolchain { version = "0.15"; };
  extraPackages = [ pkgs.zls ];
};
```

### How a toolchain is built

1. `lib/zig/toolchain.nix` resolves the version against `data/zig/zig.json`.
2. `lib/zig/zig.nix` unpacks the official tarball as it is: the executable and `lib/` sit side
   by side under `bin/`, so the standard library is found next to the executable.
3. A `symlinkJoin` puts the toolchain together.

### Master

ziglang.org also lists `master`, a rolling snapshot whose version changes daily. `crucible
update zig` does not pin it: a daily pin would rewrite `data/zig/zig.json` every day while
resolving to a different compiler each time, so only tagged releases are pinned.

## Templates

```sh
nix flake init -t github:kiyors/crucible            # Rust
nix flake init -t github:kiyors/crucible#node       # Node.js + Vite+ + pnpm
nix flake init -t github:kiyors/crucible#python     # Python + uv
nix flake init -t github:kiyors/crucible#go         # Go
nix flake init -t github:kiyors/crucible#zig        # Zig
nix flake init -t github:kiyors/crucible#deno       # Deno
```

The Rust one has a `flake.nix` with a dev shell and a package, a `rust-toolchain.toml` and an
`.envrc`. The Node one resolves its toolchain from `package.json`, `.node-version` and
`packageManager` (including pnpm), and builds a package through `buildNpmPackage` with
`importNpmLock`. Each of the others has a dev shell and a hello-world package:

- **python**: `fromPyprojectToml` reads `.python-version` and `requires-python`; the package
  runs `main.py` on the toolchain's Python (a project with dependencies builds them from
  `uv.lock`, for example with uv2nix).
- **go**: `fromGoMod` reads `go.mod`; the package is the toolchain's `.buildGoModule` with no
  `vendorHash`, since the module has no dependencies.
- **zig**: `fromBuildZigZon` reads `.minimum_zig_version`; the package runs `zig build`.
- **deno**: `deno.toolchain { }`, because `deno.json` names no Deno version (see [Deno](#deno));
  the package runs `main.ts` with `deno run`.

For a project that already exists, `crucible init` writes just the `flake.nix`:

```sh
nix run github:kiyors/crucible -- init          # in the project's directory
nix run github:kiyors/crucible -- init --force  # replace an existing flake.nix
```

It looks in the current directory for each language's project files and writes one dev shell,
for every supported system, with a toolchain for every language it finds:

| Found                                   | The dev shell's toolchain                                                      |
| --------------------------------------- | ------------------------------------------------------------------------------ |
| `rust-toolchain.toml`, `rust-toolchain` | `rust.fromToolchainFile`                                                       |
| `package.json`                          | `node.fromPackageJson` (reads `.node-version` or `.nvmrc` too)                 |
| `deno.json`, `deno.jsonc`               | `deno.toolchain { }`, the newest pinned release                                |
| `pyproject.toml`                        | `python.fromPyprojectToml` (reads `.python-version` too)                       |
| `.python-version` alone                 | `python.fromPythonVersion`                                                     |
| `go.mod`                                | `go.fromGoMod`                                                                 |
| `build.zig.zon`                         | `zig.fromBuildZigZon`, or `zig.toolchain { }` without a `.minimum_zig_version` |

A Rust channel crucible does not pin, such as `"1.99"` or `"nightly-2026-11-01"`, gets the
`rust-manifest` input described under [fromToolchainFile](#fromtoolchainfile). A `go.mod`
without a `go` directive is an error, because `fromGoMod` needs one. init refuses to replace an
existing `flake.nix` without `--force`, needs no network, and prints what each toolchain is
pinned by and what to run next: `git add` for the files the flake reads when the project is a
git repository (a flake sees only tracked files), `nix develop`, and `use flake` for direnv when
`direnv` is on PATH. A `.nvmrc` or `.node-version` without a `package.json` is left out with a
note, because `fromPackageJson` needs the `package.json`.

## Overlay

```nix
pkgs = import nixpkgs {
  inherit system;
  overlays = [ crucible.overlays.default ];
};

# pkgs.crucible.cli, pkgs.crucible.cli-doctor
# pkgs.crucible.rust.{stable,nightly,cargo}, .toolchain { ... }, .fromToolchainFile { ... }
# pkgs.crucible.node.{lts,current}, .toolchain { ... }, .fromPackageJson { ... }
# pkgs.crucible.deno.toolchain { ... }
# pkgs.crucible.python.latest, .toolchain { ... }, .fromPythonVersion { ... },
#   .fromPyprojectToml { ... }
# pkgs.crucible.go.stable, .toolchain { ... }, .fromGoMod { ... }
# pkgs.crucible.zig.toolchain { ... }, .fromBuildZigZon { ... }
```

## CLI

`crucible` is a small Rust program, a Cargo workspace in `crates/`, built with this flake's own
toolchain:

```sh
nix run github:kiyors/crucible                             # doctor: is cargo the fork, and is anything working against it?
nix run github:kiyors/crucible -- init                     # a flake.nix for this project, see Templates
nix run github:kiyors/crucible#crucible -- pin rust 1.99   # the rust-manifest input and sha256 for a channel
nix run github:kiyors/crucible#crucible -- pin node 24.11.0 # the SHASUMS256.txt input and sha256 for a release
nix run github:kiyors/crucible#crucible -- pin go 1.25.3   # the per-system hashes for a Go release
nix run github:kiyors/crucible#crucible -- pin python 3.14.0 # the SHA256SUMS input for a Python release
```

The bare `nix run` builds a doctor-only binary (`packages.<system>.crucible-doctor`, cargo
feature `net` off), which leaves out the HTTP and TLS stack that `update` and `pin` need and so
builds faster. It runs `doctor` with no command, and also has `init`, which needs no network
either. `#crucible` is the full CLI.

`doctor` checks that the first `cargo` on PATH is the fork, and warns about other cargos and
rustup proxies on PATH, `CARGO_INCREMENTAL=0`, `RUSTC_WRAPPER` caches, `CARGO_TARGET_DIR`, a
`RUST_SRC_PATH` from another release, and a `rust-toolchain.toml` that disagrees with `rustc`. It
exits non-zero when cargo is not the fork, and on any other `error` line, such as a toolchain
that a project's pin rules out. It is what `crucible` runs with no command.

For Node.js it checks the active `node` against the nearest `.node-version` or `.nvmrc`, and warns
about state that leaks between projects or makes vp ignore the toolchain's Node: `VP_HOME`, a
`VP_ENV_FORBIDDEN` other than 1, `NODE_PATH`, and an npm prefix inside the read-only store. A
missing Node.js is an `info` line, not an error: `doctor` is useful in Rust-only projects too.

The package manager is checked against the nearest `package.json`: its `packageManager`, or else
the first `devEngines.packageManager` entry with a name and a version, the order
`fromPackageJson` uses. An exact release, a major (`pnpm@10`) and an npm range (`pnpm@^10.2`,
`pnpm@>=9 <11`) are all compared; a spec doctor cannot read (`pnpm@latest`, a pre-release) is an
`info` line rather than a mismatch. `doctor` also warns about:

- **A second copy further down PATH**, such as a pnpm from `extraPackages` next to the
  toolchain's. It lists both and says which one wins.
- **A package manager on another Node.js.** A wrapper or shebang that names a node other than the
  one on PATH (a nixpkgs pnpm built on its own `nodejs`, say) runs install scripts on that one.
  Only what the files state counts: `#!/usr/bin/env node`, an explicit `.../bin/node`, and
  makeWrapper's `.<name>-wrapped` target. Native executables (bun, pnpm's standalone build) are
  reported as not applicable, and anything else is left alone rather than guessed at.

Each warning says what to do: use crucible's `packageManager = "pnpm@<major>"` in
`lib.<system>.node.toolchain`, pass a package built from the release you need, or remove the extra
copy.

The other languages get the same treatment, each one only when its executable or its project file
is there, so a Rust-only project sees no Python or Go lines. A project file without the
executable is a warning that names the function to add (`fromPyprojectToml`, `fromGoMod`,
`fromBuildZigZon`, `deno.toolchain`) or the `programs.crucible.<lang>` option (Deno has none
yet), and a second copy further down PATH is warned about as above.

- **Python and uv.** The active `python3` is compared with the nearest `.python-version` (read as
  `fromPythonVersion` reads it) and with `requires-python` in the nearest `pyproject.toml`, both
  as PEP 440; a `3.14t` entry also needs `python3` to be a freethreaded build. `doctor` warns
  about `PYTHONHOME`, a `UV_PYTHON` that is not the `python3` on PATH (or, when unset, a uv
  wrapper that defaults it to another toolchain's python3), a
  `UV_PYTHON_DOWNLOADS` other than `never`, and any `UV_MANAGED_PYTHON` or
  `UV_PYTHON_PREFERENCE`. A uv that sets no `UV_PYTHON` at all is not crucible's, and gets an
  `info` line. The OS's own `/usr/bin/python3` further down PATH is not reported, since it cannot
  be removed.
- **Go.** `go version` is compared with go.mod's (or go.work's) `go` and `toolchain` directives,
  read as minimums as `fromGoMod` reads them. An older go than the `go` directive is an error,
  since it refuses the module; an older one than the `toolchain` directive is a warning. It warns
  about `GOROOT`, a `GOTOOLCHAIN` other than `local` (from the environment, a `go env -w` file, or
  the GOROOT's `go.env`, which says which go is not crucible's), and a `go install` directory
  (`GOBIN`, or GOPATH's `bin`) missing from PATH. Every go command doctor runs has
  `GOTOOLCHAIN=local`, so it never downloads a toolchain.
- **Zig.** `zig version` is compared with build.zig.zon's `.minimum_zig_version`. An older zig
  is an error; a newer 0.x line is a warning, since Zig changes its language and build API
  between them.
- **Deno.** `deno --version` is reported but not compared: `deno.json` has no field for the Deno
  release a project needs.

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
nix run .#crucible -- update node                     # every supported Node.js line, and Vite+
nix run .#crucible -- update deno                     # the newest stable Deno release
nix run .#crucible -- update python                   # every supported CPython line (and its freethreaded line), and uv
nix run .#crucible -- update go                       # the newest release of each supported Go line
nix run .#crucible -- update zig                      # every tagged Zig release line
```

The fork's hashes come from each release's `SHA256SUMS`, the Rust ones from the channel
manifests, Node.js's from each release's `SHASUMS256.txt`, Vite+'s from its `vp-checksums.txt`
and its pnpm lockfile, Deno's from the sha256 digests GitHub records for denoland/deno's release
assets, and Zig's from ziglang.org's per-platform `index.json`, so nothing is downloaded but
metadata. Every `SHASUMS256.txt` is verified against its GPG signature before its hashes are
used, with the active Node.js release keys vendored in `data/node/release-keys.asc`.
Package-manager pins come from the npm registry's `dist.integrity`, and bun's from oven-sh/bun's
`SHASUMS256.txt`. Go's hashes come from go.dev's release index (`?mode=json&include=all`), which
also decides the supported lines: the two newest with a stable release, as Go's release policy
has it. Which Node.js lines are supported comes from the Release working group's
`schedule.json`. CPython's hashes come from the newest python-build-standalone build's
`SHA256SUMS` and uv's from its release's `sha256.sum`, each checked against the sha256 GitHub
records for the release asset, since neither file is signed. Which CPython lines are supported
comes from python.org's `release-cycle.json`. The nightly search fetches candidate dates in
parallel.

The `update` workflow runs at 00:17, 10:17 and 20:17 UTC (about every 10 hours): it downloads
the latest released CLI (`crucible-<version>-<target>.tar.gz`, verified against the release's
`SHA256SUMS` and its attestation with `gh attestation verify`), runs `crucible update`, and when
`data/` changed, runs the check workflow's builds on x86_64-linux, aarch64-linux and macOS. When
those pass it commits the refreshed `data/` and pushes it straight to `main`: a pin refresh is
data only, so it needs no pull request, review or merge. Before the first release it falls back
to `nix run .#crucible -- update` with the repo's own CLI.

A push made with the default `GITHUB_TOKEN` does not start other workflows, so `check` does not
re-run on that commit; the update workflow's `checks` job is the check for it. A refresh whose
checks fail, or that no longer applies because `main` moved under it, is left for the next
scheduled run.

## Development

```sh
direnv allow          # or: nix develop ./dev
nix flake check -L    # builds a crate with every toolchain
cd dev && nix fmt
nix shell .#rust.stock -c cargo test --workspace                 # the CLI's tests, from the repo root
nix shell .#rust.stock -c cargo clippy --workspace --all-targets # warnings are errors ([workspace.lints])
```

`nix flake check` also runs clippy on the workspace (`checks.<system>.crucible-clippy`), so a new
warning fails CI. `AGENTS.md` lists every check to run before a commit.
