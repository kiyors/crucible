# Architecture

crucible builds language toolchains for Nix from each language's own upstream release artifacts.
It never builds them from source or goes through another overlay. It covers Rust, Node.js
(with Vite+) and Deno, each in its own `lib/<lang>/`, `data/<lang>/` and `tests/<lang>/`. This
file explains how the pieces fit together. The README covers how to use them.

## The big picture

```
upstream release metadata          crucible CLI (crates/)          Nix (lib/)
static.rust-lang.org manifests ─┐
huacnlee/cargo SHA256SUMS ──────┼─ crucible update ─► data/rust/*.json ─► components ─► sysroot ─► toolchain
                                │   (trim, hash, validate,                 (one drv per    (symlinks +   (+ fork cargo,
                                │    write atomically)                      tarball)        real copies)   or .stock)
project files ──────────────────┴────────────────────────────────────► fromToolchainFile (rust-toolchain.toml)

nodejs.org index + SHASUMS256 ──┐
Release schedule.json ──────────┼─ crucible update ─► data/node/node.json ──────► node (one drv per release)
vp-checksums.txt + lockfile ────┤   node               data/node/vite-plus.json ─► vp + node_modules ─┐
                                │                                                  (one fetch per pkg) ├► toolchain
project files ──────────────────┴────────────────────────────────────► fromPackageJson (package.json)  ┘

denoland/deno releases ──────────┐
denoland/deno GitHub digests ───┴─ crucible update ─► data/deno/deno.json ─► deno (one drv)
```

The design rests on two ideas:

1. **Upstream metadata is pinned as data, and Nix only reads it.** The CLI fetches and trims
   upstream metadata and converts its hashes to SRI. Nix evaluation then imports JSON and never
   touches the network, except when a user explicitly passes `sha256` for an unpinned channel.
2. **One derivation per upstream artifact.** Toolchains are cheap joins over shared component
   derivations, so a second toolchain on the same release costs a symlink tree and a few copies
   instead of a second unpack.

## Code map

### Entry points

- `flake.nix`: the public surface. `packages`, `apps`, `lib`, `checks`, `overlays`,
  `homeManagerModules` and `templates`, for the systems in `lib/common/systems.nix`. It does no
  work of its own. Everything goes through `default.nix`.
- `default.nix`: `{ pkgs } -> { rust, node, cli, cli-doctor }`, everything crucible provides for one nixpkgs
  instance. The flake, the overlay (`overlay.nix` → `pkgs.crucible`) and non-flake users all
  call it, so there is exactly one place where the pieces are wired together.
- `lib/common/systems.nix`: the supported systems (`x86_64-linux`, `aarch64-linux`,
  `aarch64-darwin`), meaning the ones that both nixpkgs and the fork's releases support. The
  CLI's `HOSTS` in `crates/upstream/src/manifest.rs` must list the same systems in the same
  order.

### Rust (`lib/rust/`)

- `default.nix`: puts together the fork's cargo, the toolchain API, and the two ready-made
  toolchains (`stable`, and `nightly` with Cranelift).
- `cargo.nix`: the prebuilt huacnlee/cargo binary from `data/rust/cargo.json`. On Linux it runs
  `autoPatchelf`. It is wrapped with `CARGO_FORK_AUTO_UPDATE=0`, because the fork's self-update
  renames over its own binary, which the read-only store does not allow.
- `components.nix`: the low-level layer.
  - `component`: one tarball from the manifest, installed as a prefix, fetched by the manifest's
    hash. Host tools are patched on Linux. rust-std for other targets is left alone, because it
    holds that target's libraries.
  - **rust-lld wrapping (Linux):** rustc's bundled `gcc-ld/ld.lld` would bypass the nixpkgs ld
    wrapper, so binaries it links would get the FHS dynamic linker. It is routed through a copy
    of the nixpkgs bintools wrapper, with the libc and dynamic-linker `nix-support` files the
    wrapper reads.
  - `sysroot`: a `symlinkJoin` over the components, plus **real copies** of `bin/*` and
    `librustc_driver`. rustc derives its sysroot from where the driver library is loaded, and the
    tools find their libraries relative to their own path, so symlinks into a component would
    point them at the wrong tree. On Linux the copies get this tree's `lib/` first in their
    RUNPATH.
  - `canonical` / `hasComponent`: resolve the manifest's `renames` (`clippy` ↔
    `clippy-preview`), so callers can use either spelling.
- `toolchain.nix`: the public API.
  - `toolchain { channel, components, targets, exclude, manifest, sha256 }`
  - `fromToolchainFile { file, exclude, manifest, sha256 }`, which reads `rust-toolchain(.toml)`
    the way rustup does, profiles included
  - `resolveManifest` picks a pinned `data/rust/*.json`, an explicit `manifest` file (usually a
    `flake = false` input), or a `builtins.fetchurl` checked against `sha256`.
  - `assemble` applies `exclude`, adds rust-analyzer whenever the channel ships it, and builds
    the sysroot. It asserts at evaluation time that `rustc` and `rust-std` are present and that
    `cargo` is not excluded.
  - Each toolchain carries `.stock`, `.rustPlatform`, `.rust-analyzer`, `.rustSrcPath` and
    `.base`, plus a setup hook that exports `RUST_SRC_PATH`, overwriting any inherited value so
    a dev shell's own sources always win.

### Node.js (`lib/node/`)

- `default.nix`: the toolchain API for one nixpkgs instance.
- `node.nix`: one official release (an entry of `data/node/node.json`, or one read from a
  `SHASUMS256.txt`), installed as upstream ships it. On Linux `autoPatchelf` runs, with
  `--no-sort` only where patchelf knows it: 0.19 and later sort program headers, which crashes
  the official node, and older ones (nixpkgs' stdenv has 0.15) reject the flag. `dontStrip`,
  because stripping would break the macOS signature. npm's shebangs point at this node.
- `vite-plus.nix`: `unwrapped` is the `vp` release archive (binary plus its sidecars) next to
  `node_modules`, built from `data/node/vite-plus.json` the way pnpm's isolated linker lays it
  out: `.pnpm/<id>/node_modules/<name>` per package, dependencies as relative symlinks. Each
  tarball is a `fetchurl` by its registry integrity, so there is no fixed-output hash to learn by
  building. Shebangs are left as `/usr/bin/env node`, so scripts run on the shell's node.
  `.vp-setup-complete` beside the binary stops vp's first-run self-install (shims, shell
  profiles), and `create/bin.js` is patched so `vp create` leaves the copied templates writable.
  `withNode` wraps it per Node.js: `VP_ENV_FORBIDDEN=1` (no managed Node.js or package
  manager), a `VP_HOME` of its own under `~/.local/share/crucible/vite-plus` seeded with a
  `system_first` `config.json` (Vite+ 1.7 only takes Node.js from PATH with that config, and only
  reads it from `VP_HOME` or the user's config directory), and the node appended to PATH.
- `semver.nix`: npm range matching (`^`, `~`, x-ranges, hyphens, `||`), enough to pick the
  newest pinned release for `engines.node` offline.
- `toolchain.nix`: the public API.
  - `toolchain { version, vitePlus, packageManager, shasums, sha256 }`
  - `fromPackageJson { file, vitePlus, shasums, sha256 }`: `devEngines.runtime`, then
    `engines.node`; `packageManager`, then `devEngines.packageManager`.
  - `resolve` turns "lts", "current", "lts/<codename>", a line, a range or an exact release into
    a pinned release, or reads one from `shasums` or a `builtins.fetchurl` checked against
    `sha256`. `shasumsUrl` must match the URL `pin node` prints.
  - `packageManagerFor` uses the pinned release of that major from
    `data/node/package-managers.json` (built by `lib/node/package-manager.nix`; bun from oven-sh/bun's
    GitHub release zips, pnpm and yarn from npm registry tarballs) when it satisfies the requested
    range, then nixpkgs' build of that major when it
    does, and otherwise rejects the spec instead of silently substituting another release. A
    package can be passed directly for a release crucible does not pin. npm ships with node.
  - `assemble` asserts that Vite+'s `engines.node` allows the node, joins node, vp and the
    package manager, and adds a setup hook that sets `VP_ENV_FORBIDDEN=1` and unsets `VP_HOME`.
    Each toolchain carries `.node`, `.vite-plus`, `.packageManager` and `.buildNpmPackage`
    (nixpkgs' builder overridden onto the node).

### Deno (`lib/deno/`)

- `default.nix`: the toolchain API for one nixpkgs instance.
- `deno.nix`: one Deno release from `data/deno/deno.json`, the `deno` executable from the release
  zip, installed as upstream ships it; on Linux `autoPatchelf` runs. Deno is a separate runtime,
  not an npm package manager, so it lives in its own language: `lib/deno/`, `data/deno/` and
  `tests/deno/`, with `lib.<system>.deno.toolchain`.
- `toolchain.nix`: the public API.
  - `toolchain { version }` - `"latest"` (the newest pinned release), a major line (`"2"`), or an
    exact pinned release; anything else is an error rather than a silent substitution, like the
    package-manager pins.

### Data (`data/rust/`, `data/node/`, `data/deno/`)

Generated by `crucible update`. Do not edit by hand, and do not format.

- `stable.json`, `beta.json`, `nightly.json`: trimmed channel manifests. They keep the packages,
  targets, profiles and renames, and put `version` at the top level. Package order is kept from
  upstream so that update diffs stay readable.
- `cargo.json`: the fork's version and a per-system `{ url, hash }`.
- `node/node.json`: `lts` and `current` (line numbers), and per supported line its newest
  release: version, date, LTS codename, end of support, npm, ABI (`modules`) and a per-system
  `{ url, hash }` from `SHASUMS256.txt`. A line is supported from its `start` to its `end` in the
  Release working group's `schedule.json`.
- `node/vite-plus.json`: the Vite+ version, a per-system `{ url, hash }` for the `vp` archive
  from `vp-checksums.txt`, Vite+'s `engines`, and the dependency graph of the release's
  `vp-install-lock.yaml`: per snapshot (`id`) its name, tarball URL, sha512 integrity,
  dependency links and, when not every system installs it, `systems`. Sorted by id.
- `node/package-managers.json`: the newest pnpm, yarn (1 and 4) and bun release of each supported
  major, with tarball URLs and SRI integrity hashes from the npm registry - except bun, whose
  hashes come from oven-sh/bun's `SHASUMS256.txt`, one zip per platform; system-independent for
  yarn's single JS file, per-system for the other native binaries.
- `deno/deno.json`: the newest stable release of Deno 2 from denoland/deno's GitHub releases:
  version, and per-system `{ url, hash }` for the `deno` executable from the release zips.
- `node/release-keys.asc`: the active Node.js release keys, vendored so `SHASUMS256.txt.sig` can
  be checked without a system keyring. Replaced deliberately when nodejs rotates signers.

The pinned nightly is the newest one that ships every required component (Cranelift included) on
**every** supported host, not simply the newest nightly.

### CLI (`crates/`)

A Cargo workspace whose root manifest and `Cargo.lock` sit at the repo root (`publish = false`
throughout). Every dependency version is declared once, in the root `[workspace.dependencies]`.
`crates/crucible/package.nix` builds the binary with crucible's own `rust.stable.rustPlatform`,
so every CI run also proves that the toolchain packages a real crate. Its source is the repo root
cut down to the Cargo manifests, `crates/` without Nix or Markdown, and
`data/node/release-keys.asc`, so editing docs or Nix does not rebuild the CLI.

There are five crates. The first cut is the one that matters to users, what the doctor-only
build compiles; the second follows the one layering the network code already has.

- `crates/crucible`: the `crucible` binary, only clap and dispatch. Its default `net` feature
  is nothing but the optional `pins` dependency, so the doctor-only build (`cli-doctor`,
  `packages.crucible-doctor`, what `apps.default` runs) leaves out rustls, ring and every other
  network dependency by not depending on them, not through `#[cfg]` in library code. The
  package builds and tests with `--package`, so that build never compiles `pins` or `upstream`.
- `crates/doctor`: `doctor` alone. It may not reach the network, and its dependencies
  (`serde_json`, `toml`) are all the doctor-only build adds to the binary.
- `crates/init`: `init [--force]`, which writes a `flake.nix` with a dev shell from the current
  directory's `rust-toolchain(.toml)` and `package.json`, with a `rust-manifest` input for
  channels the pins do not cover. Its text is a trimmed copy of `templates/*/flake.nix` (a test
  checks the shared pieces still match), already nixfmt-formatted, and its `SYSTEMS` follow
  `lib/common/systems.nix`. It needs no network, so it is in the doctor-only build. Its
  `channel.rs` holds the rustup channel rules: `manifest_url` mirrors `manifestUrl` in
  `toolchain.nix` and `pins`' `pin` uses it too, so there is one copy.
- `crates/upstream`: downloading, verifying and parsing upstream release metadata. Its modules
  depend on nothing else in the workspace, and they hold the whole TLS, GPG and hashing stack.
- `crates/pins`: `update` and `pin`, the two commands that turn that metadata into pins (in
  `data/`, or printed for a project). They share helpers, so they stay one crate. Its root
  re-exports `upstream`'s modules, so `crate::http` and the rest read as they do in one crate.
  Adding a language means a step in `pins/src/update/` and, when it needs a new upstream
  format, a model in `upstream`.

Everything is a library, so the trimming and parsing rules are tested without a network or a
process.

`upstream`:

- `pnpm_lock.rs`: the pnpm v9 lockfile model and `graph()`, the shape of the graph in
  `data/node/vite-plus.json`. It walks the graph once per system from the root, following
  optional dependencies only where npm's `os`/`cpu`/`libc` rules allow them. Its `PLATFORMS`
  follow `lib/common/systems.nix`, like `PLATFORMS` in `pins`' `update/node.rs` and
  `update/vite_plus.rs`.
- `manifest.rs`: the upstream manifest model and `trim()`, the exact shape of
  `data/rust/*.json`. `tests/trim.rs` compares it against `tests/fixtures/`.
- `gpg.rs`: verifies each `SHASUMS256.txt` against its `.sig` using the active Node.js release
  keys from `data/node/release-keys.asc`. Pure Rust, so `cargo test` needs no system `gpgv`; the
  vendored keyring is the allow-list, not a key someone happened to install locally.
- `http.rs`: the single path for every download. It retries transient failures and never retries
  a 404.
- `sri.rs`: hex/bytes → `sha256-<base64>`, so the CLI never has to shell out to `nix`.

`pins`:

- `update/`: `update [all|cargo|rust|node|deno]`. It fetches and validates **every** pin before writing
  anything, then replaces each file atomically in a fixed order, under a
  `.crucible-update.lock` in the repo root. A half-written pin is worse than a stale one,
  because the flake would fail to evaluate. The nightly search fetches up to a month of dated
  manifests in parallel. Only a 404 counts as "no such nightly". Any other failure newer than
  the pick fails the run, so the pin never silently moves backwards. `node` pins the newest
  release of every supported line, then Vite+, whose lockfile is downloaded and checked against
  `vp-checksums.txt` before it is trimmed, then the package managers from npm registry metadata
  (bun's from oven-sh/bun's `SHASUMS256.txt`), and `update deno` pins the newest stable Deno
  releases from denoland/deno's GitHub API assets. Every `SHASUMS256.txt` is GPG-verified before
  its hashes are used.
- `pin.rs`: `pin rust <channel>` prints the `flake = false` input and the SRI hash for a
  channel, from `init`'s `channel::manifest_url`.
  `pin node <x.y.z>` does the same for a release's `SHASUMS256.txt`, after checking its GPG
  signature and that it lists a tarball for every system.

`doctor` checks the user's environment. Is `cargo` the fork? It also looks for rustup
proxies, `CARGO_INCREMENTAL=0`, `CARGO_TARGET_DIR`, `RUSTC_WRAPPER` caches, a stale
`RUST_SRC_PATH`, a `rust-toolchain.toml` that disagrees with `rustc`, variables the current
`hm-session-vars.sh` no longer sets (a shell older than the last switch), and on macOS a GCC
`cc`. For Node.js it compares the active node with the nearest `.node-version`/`.nvmrc` and the
active package manager with `package.json`'s `packageManager`, and warns about `VP_HOME`,
`VP_ENV_FORBIDDEN`, `NODE_PATH` and an npm prefix in the read-only store.

### Integration

- `modules/home-manager.nix`: `programs.crucible.rust.*`. It installs a toolchain, sets
  `CARGO_HOME`, writes `~/.cargo/config.toml` from `settings` and `gc` (the fork's
  `[build.gc]`), and warns about settings that work against the fork. It deliberately sets no
  `RUST_SRC_PATH`: rust-analyzer finds the sources through `rustc --print sysroot`, and a global
  value would leak this release into every other project.
- `modules/home-manager.nix`: `programs.crucible.node.*` installs a Node.js toolchain and,
  with `npmPrefix`, gives `npm install --global` a writable prefix. It warns when `VP_HOME` is
  set, since crucible's vp would then share another vp's config.
- `templates/default`: a Rust project whose toolchain comes from `rust-toolchain.toml`.
- `templates/node`: a Node.js project whose toolchain comes from `package.json`,
  `.node-version` and `packageManager` (including pnpm), with a `buildNpmPackage` build.

### Verification

- `tests/default.nix` → `checks.<system>`, one set per language (`tests/rust/`) plus the CLI
  build. The Rust checks build `tests/rust/hello` with each kind of toolchain: stable (clippy,
  fmt, llvm-tools, rust-analyzer, `RUST_SRC_PATH`, which must beat an inherited one), stock
  cargo, nightly, wasm32 (a real link), `fromToolchainFile`, a numbered channel through
  `manifest`, `exclude` and its evaluation-time errors, `rustPlatform.buildRustPackage`, and on
  Linux a rust-lld link whose interpreter must be in the store. The Node.js checks run node and
  npm on every pinned line, take `tests/node/app` through `vp fmt`, `lint`, `build` and `test` in
  the sandbox (no network, so a download attempt fails) and check vp set nothing up for itself,
  check the setup hook beats inherited values, the Deno checks run `deno` from each pinned
  major and check `toolchain` resolution, resolve `fromPackageJson` (devEngines,
  `.node-version`/`.nvmrc`, engines, packageManager with pnpm on the toolchain's node, bun's
  new pins from oven-sh/bun's GitHub zips), build from
  a `SHASUMS256.txt`, reject misuse at evaluation time, and build `tests/node/hello` with
  `.buildNpmPackage` and `importNpmLock`. `tests/init.nix` evaluates the golden flakes in
  `crates/init/tests/fixtures/init/` (which `init`'s cargo tests must reproduce byte for byte) against
  crucible's lib, and runs the doctor-only `init` in one of them. `tests/default.nix` also checks the Node template
  resolves and that the doctor-only binary carries the Node checks.
- `dev/flake.nix`: a separate flake for treefmt (nixfmt, deadnix, statix, typos, oxfmt,
  rustfmt, taplo) and git hooks (treefmt, gitleaks), kept separate so consumers never pay for
  those inputs. It also owns the Home Manager module check, since the main flake must keep
  nixpkgs as its only input.
- `.github/workflows/`: `check.yaml` (format and hooks, then a build matrix on x86_64-linux,
  aarch64-linux and macOS), `update.yaml` (daily `crucible update` + `nix flake update`, opening
  a PR), and `hestia-gc.yaml` (cache GC). Nix setup is in `.github/actions/setup-nix`.

## Invariants

- **nixpkgs is the only flake input** of the main flake. Development tooling goes in `dev/`.
- **Evaluation is offline** for pinned channels. Anything that needs the network belongs in the
  CLI and ends up in `data/`.
- **vp never manages Node.js.** It runs on the toolchain's (or the shell's) node, from the
  store copy, without self-setup. Projects' `.node-version` files do not make it download one.
- **The toolchain's cargo is always the fork.** Sandboxed package builds use `.stock` through
  `.rustPlatform`, because the fork keeps its build directory under `~/.cargo/targets`, which
  the sandbox does not have.
- **Toolchains are built for the machine that runs them.** A cross package set is rejected.
  Cross targets go in `targets`.
- **Misuse fails at evaluation time** with a `crucible: ...` message. It must never produce a
  toolchain that builds but cannot compile.
- **Components are shared.** Anything keyed on more than the tarball (the toolchain's component
  list, for example) would break sharing between toolchains.

## Not in the repo

`fenix/` and `nix-vite-plus/` are gitignored local clones kept for reference. They are not
dependencies. `docs/` holds research notes for upcoming work.
