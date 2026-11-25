# Architecture

crucible builds language toolchains for Nix from each language's own upstream release artifacts.
It never builds them from source or goes through another overlay. It covers Rust, Node.js
(with Vite+), Deno, Python (with uv), Go and Zig, and the tsrs type checker, each in its own `lib/<lang>/`, `data/<lang>/` and
`tests/<lang>/`. This
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
release-cycle.json ─────────────┐
python-build-standalone ────────┼─ crucible update ─► data/python/python.json ──► cpython (one drv per release)
  SHA256SUMS                    │   python             data/python/uv.json ──────► uv ─┐
uv sha256.sum ──────────────────┤                                                      ├► toolchain
project files ──────────────────┴──────────► fromPythonVersion / fromPyprojectToml ───┘
go.dev release index ───────────┬─ crucible update ─► data/go/go.json ─────────► go (one drv per release)
                                │   go
project files ──────────────────┴────────────────────────────────────► fromGoMod (go.mod, go.work)
ziglang.org index ──────────────┼─ crucible update ─► data/zig/zig.json ──────► zig (one drv per release)
project files ──────────────────┴────────────────────────────────────► fromBuildZigZon (build.zig.zon)
npm registry @tsrs/* integrity ──── crucible update ─► data/tsrs/tsrs.json ────► tsrs (one drv)
```

The design rests on two ideas:

1. **Upstream metadata is pinned as data, and Nix only reads it.** The CLI fetches and trims
   upstream metadata and converts its hashes to SRI. Evaluating pinned releases is offline and
   imports JSON only; `sha256` for an unpinned Rust channel or Node.js release is the exception,
   where `builtins.fetchurl` fetches the manifest or `SHASUMS256.txt` at evaluation time and
   checks it against that hash.
2. **One derivation per upstream artifact.** Toolchains are cheap joins over shared component
   derivations, so a second toolchain on the same release costs a symlink tree and a few copies
   instead of a second unpack.

## Code map

### Entry points

- `flake.nix`: the public surface. `packages`, `apps`, `lib`, `checks`, `overlays`,
  `homeManagerModules` and `templates`, for the systems in `lib/common/systems.nix`. It does no
  work of its own. Everything goes through `default.nix`.
- `default.nix`: `{ pkgs } -> { rust, node, deno, python, go, zig, tsrs, cli, cli-full }`, everything crucible provides for one nixpkgs
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
    `data/node/package-managers.json` (built by `lib/node/package-manager.nix`; pnpm and bun from
    their GitHub releases, yarn from npm registry tarballs) when it satisfies the requested
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

### Python (`lib/python/`)

**Why python-build-standalone.** CPython's own release artifacts are source tarballs plus
macOS and Windows installers; python.org publishes no Linux binaries and its macOS `.pkg`
installs into `/Library/Frameworks`, so neither unpacks into a store path. nixpkgs builds CPython
from source, which is the thing crucible does not do. python-build-standalone (maintained by
Astral) publishes relocatable `install_only` archives of every supported line for all three
systems in each dated build, with a `SHA256SUMS` for the build. They are what `uv python
install` downloads, so crucible's Python is the same binary a uv user elsewhere gets. They bundle
OpenSSL, SQLite, Tcl/Tk and the rest, linking only glibc, libgcc_s and (up to 3.12) libcrypt on
Linux and only system libraries on macOS, so `autoPatchelf` on Linux is the only patching the binaries need. Conda's
builds and the `python-build` tarballs from pyenv were the alternatives: the first pulls in a
separate package ecosystem, the second builds from source.

- `default.nix`: the toolchain API for one nixpkgs instance.
- `python.nix`: one release (an entry of `data/python/python.json`, or one read from a
  `SHA256SUMS`), installed from its `install_only` archive. On Linux `autoPatchelf` runs.
  sysconfig (`_sysconfigdata_*.py`, `_sysconfig_vars_*.json` from 3.14, the config `Makefile`)
  still describes the build machine, so its `/install` prefix becomes the store path and clang,
  clang++ and the build's llvm-ar become `cc`, `c++` and `ar`, as uv does when it installs these
  builds; C extensions then build with the dev shell's compiler. An `EXTERNALLY-MANAGED` file
  (PEP 668) makes pip and `uv pip install --system` refuse the store with a pointer to a venv.
  `dontStrip`, because stripping would break the macOS signatures. The freethreaded builds are
  the same, one "t" further along: `bin/python3.14t`, the stdlib under `lib/python3.14t` and
  `include/python3.14t` (`bin/python3` is there too), and `toolchain.nix`'s `fromSha256sums`
  reads their `-freethreaded-` archive names.
- `uv.nix`: `unwrapped` is the `uv` and `uvx` binaries from uv's GitHub release (the glibc
  build, `autoPatchelf`ed on Linux). `withPython` wraps both for one Python with defaults for
  `UV_PYTHON` (that python3), `UV_NO_MANAGED_PYTHON=1` and `UV_PYTHON_DOWNLOADS=never`, the
  current spellings: uv 0.13 hides `UV_PYTHON_PREFERENCE=only-system`, which they replace.
- `pep440.nix`: PEP 440 specifier matching (`==`, `!=`, `~=`, `>=`, `<=`, `>`, `<`, `===`,
  `.*`, zero padding) plus uv's bare requests ("3", "3.14"), enough to pick the newest pinned
  release for `requires-python` offline. Pre-release specifiers are rejected.
- `toolchain.nix`: the public API.
  - `toolchain { version, uv, sha256sums }`: a `version` ending in "t" is the freethreaded
    line, pinned next to the GIL line of the same versions in `data/python/python.json`.
  - `fromPythonVersion { file, uv, sha256sums }`: the first entry of `.python-version`, with
    uv's `cpython@`/`cpython-` prefixes; a `3.14t` entry picks the freethreaded line. Other
    implementations are rejected.
  - `fromPyprojectToml { file, uv, sha256sums }`: `.python-version` beside it, then
    `project.requires-python`, then "3"; a `.python-version` outside `requires-python` fails.
  - `resolve` picks the newest pinned release a spec allows, or the newest one a `SHA256SUMS`
    has an archive of for every system. A spec ending in "t" looks only at the freethreaded
    releases and the others only at the GIL ones, since the two share versions. `releaseUrl` must match `release_url` in
    `crates/pins/src/update/python.rs`.
  - `assemble` joins python and uv, and adds a setup hook that exports `UV_PYTHON`,
    `UV_NO_MANAGED_PYTHON=1` and `UV_PYTHON_DOWNLOADS=never` and unsets `UV_MANAGED_PYTHON` and
    `UV_PYTHON_PREFERENCE`. Each toolchain carries `.python`, `.uv`, `.pythonVersion` and
    `.sitePackages`.

### Go (`lib/go/`)

- `default.nix`: the toolchain API for one nixpkgs instance.
- `go.nix`: one official release (an entry of `data/go/go.json`, or one built from `hashes`),
  unpacked into `share/go`, which is its GOROOT, with `bin/go` and `bin/gofmt` symlinked into
  it; go finds GOROOT from its resolved path. `dontFixup`: the tools are static Go binaries,
  stripping would break the macOS signatures, and `src/` holds ELF and Mach-O testdata that
  patchelf must not touch. `go.env` gets `GOTOOLCHAIN=local` instead of upstream's `auto`, so
  the default holds wherever the toolchain is installed. The setup hook sets `GOTOOLCHAIN=local`
  (the environment beats both `go.env` and `go env -w`) and unsets `GOROOT`. The official
  linker's default ELF interpreter is the FHS one, but cgo builds link externally through the
  nixpkgs cc wrapper, which sets the store's; `go-cgo` checks that.
- `toolchain.nix`: the public API.
  - `toolchain { version, hashes }` and `fromGoMod { file, hashes }`, which reads `toolchain`,
    then `go`, ignoring `toolchain default` and comments.
  - `resolve` turns "stable", a line or an exact release into a pinned release, or builds one
    from per-system `hashes` with URLs from `downloadUrl`, which must match `DL` in
    `crates/pins/src/update/go.rs`. For go.mod it reads a line or release as a minimum, as the go
    command does: the oldest pinned release that is the same or newer satisfies it, on a newer
    line when the one asked for is no longer pinned.
  - Each toolchain carries `.goroot` and `.buildGoModule` (nixpkgs' builder overridden onto
    it, with `CGO_ENABLED` in passthru for the builder's default). There is no gopls: Go
    releases do not ship one, and it does not have to match the Go it runs.

### Zig (`lib/zig/`)

- `default.nix`: the toolchain API for one nixpkgs instance.
- `zig.nix`: one official release (an entry of `data/zig/zig.json`), installed as upstream
  ships it: the executable and `lib/` side by side under `bin/`. No patching, no cross
  handling: zig runs where the build machine is, and a cross package set is rejected in
  `assemble`.
- `toolchain.nix`: the public API.
  - `toolchain { version }`: "stable" (the default, the line of the newest pinned release),
    a pinned line ("0.15") or a pinned exact release ("0.15.2"). Anything else is an
    evaluation error; `crucible update zig` pins the newest patch of every minor line in
    ziglang.org's `index.json`.
  - `fromBuildZigZon { file }`: reads `.minimum_zig_version` from `build.zig.zon` (the
    string form Zig writes since 0.13, or the tuple form before it) as a minimum: the pinned
    release of its line satisfies it when it is the same or newer. It never moves to another
    line, since Zig changes its language between 0.x lines.
  - `assemble` joins the zig executable with a `symlinkJoin`; each toolchain carries
    `.version` and `.zig`.

### tsrs (`lib/tsrs/`)

- `default.nix`: the toolchain API for one nixpkgs instance.
- `tsrs.nix`: the release in `data/tsrs/tsrs.json`, from the per-platform npm package
  (`@tsrs/linux-x64-gnu` and so on), not the `tsrs` package, whose bin is a Node.js launcher
  for the same executable. The package goes into `lib/tsrs` whole, because the executable reads
  the bundled `lib.*.d.ts` files from beside its real path; `bin/tsrs` links to it and
  `bin/tsrs-lint` wraps `tsrs --lint`. On Linux `autoPatchelf` runs. tsrs is a TypeScript tool
  but needs no Node.js, so it is a tool of its own rather than part of the Node.js toolchain,
  with `lib.<system>.tsrs.toolchain`.
- `toolchain.nix`: `toolchain { version }`: "latest" (the default) or the pinned version. Every
  tsrs version is a prerelease against one TypeScript release, so there are no lines to pick
  from, and one release is pinned.

### Data (`data/rust/`, `data/node/`, `data/deno/`, `data/python/`, `data/go/`, `data/zig/`, `data/tsrs/`)

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
- `node/package-managers.json`: the newest pnpm (10, 11, 12), yarn (1 and 4) and bun release of each supported
  major, with per-system `{ url, hash }` and SRI hashes: pnpm from pnpm's GitHub release assets
  (each checked against GitHub's artifact attestation), bun from oven-sh/bun's `SHASUMS256.txt`,
  yarn from the npm registry's signature-checked integrity. pnpm's `layout` says whether the
  asset is one binary (10, static on Linux so patchelf cannot corrupt its `pkg` snapshot)
  or a tarball with `dist/` (11, 12); yarn's single JS file is
  system-independent. pnpm's attestation is a metadata lookup over authenticated HTTPS (type,
  repository, tag and subject digest); its Sigstore signature is not verified locally. Bun's
  `SHASUMS256.txt` has no signature and is trusted over HTTPS only, unlike Node's GPG-verified
  one. Neither is an independent trust anchor; these are documented tradeoffs, not the same
  guarantee as the Node and npm checks.
- `deno/deno.json`: the newest stable release of Deno 2 from denoland/deno's GitHub releases:
  version, and per-system `{ url, hash }` for the `deno` executable from the release zips.
- `go/go.json`: `stable` (a line), and per supported line its newest release: version and a
  per-system `{ url, hash }` for the `.tar.gz` archive, from go.dev's release index. A line is
  supported while it is one of the two newest with a stable release.
- `node/release-keys.asc`: the active Node.js release keys, vendored so `SHASUMS256.txt.sig` can
  be checked without a system keyring. Replaced deliberately when nodejs rotates signers.
- `python/python.json`: the python-build-standalone `build` (its date tag) and per supported line
  (`bugfix` or `security` in python.org's `release-cycle.json`) its newest release in that
  build: version, status, end of life and a per-system `{ url, hash }` from the build's
  `SHA256SUMS`. The freethreaded lines of the same versions come next to the regular ones,
  marked `"freethreaded": true`.
- `python/uv.json`: the uv version and a per-system `{ url, hash }` from its `sha256.sum`.
- `zig/zig.json`: `stable` (the line of the newest pinned release) and `releases`: minor line
  ("0.15") -> its newest patch release, with the per-system `{ url, hash }` entries from
  `index.json`. A minor whose newest patch has no build for a supported system is not pinned
  (0.1.1 has only a Windows build).
- `tsrs/tsrs.json`: the tsrs version npm's `latest` tag names, and per-system `{ url, hash }` for
  its platform packages' tarballs, with the registry's sha512 integrity.

The pinned nightly is the newest one that ships every required component (Cranelift included) on
**every** supported host, not simply the newest nightly.

### CLI (`crates/`)

A Cargo workspace whose root manifest and `Cargo.lock` sit at the repo root (`publish = false`
throughout). Every dependency version is declared once, in the root `[workspace.dependencies]`.
`crates/crucible/package.nix` builds the binary with crucible's own `rust.stable.rustPlatform`,
so every CI run also proves that the toolchain packages a real crate. Its source is the repo root
cut down to the Cargo manifests, `crates/` without Nix or Markdown, and
`data/node/release-keys.asc`, so editing docs or Nix does not rebuild the CLI.

There are five crates. The first cut is the one that matters to users, what the user build
compiles; the second follows the one layering the network code already has.

- `crates/crucible`: the `crucible` binary, only clap and dispatch. Its default `net` feature
  is nothing but the optional `pins` dependency, so the user build (`cli`,
  `packages.crucible`, what `apps.default` runs) leaves out rustls, ring and every other
  network dependency by not depending on them, not through `#[cfg]` in library code. The
  package builds and tests with `--package`, so that build never compiles `pins` or `upstream`.
- `crates/doctor`: `doctor` alone. It may not reach the network, and its dependencies
  (`serde_json`, `toml`) are all the user build adds to the binary.
- `crates/init`: `init [--force]`, which writes a `flake.nix` with one dev shell from the
  current directory's project files: `rust-toolchain(.toml)`, `package.json`,
  `deno.json(c)`, `pyproject.toml` or `.python-version`, `go.mod` and `build.zig.zon`, each
  through the matching `from<ProjectFile>` (Deno through `toolchain { }`, since `deno.json`
  names no Deno version), with a `rust-manifest` input for channels the pins do not cover. It
  reads only what it needs to pick a function and describe the result (the `go.mod` directive,
  `.minimum_zig_version`, where Python's version comes from); the versions are resolved at
  evaluation time against the pins, which init does not carry. Its text is a trimmed copy of `templates/*/flake.nix` (a test
  checks the shared pieces still match), already nixfmt-formatted, and its `SYSTEMS` follow
  `lib/common/systems.nix`. It needs no network, so it is in the user build. Its
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

- `update/`: `update [all|cargo|rust|node|deno|python|go|zig|tsrs]`. It fetches and validates **every** pin before writing
  anything, then replaces each file atomically in a fixed order, under a
  `.crucible-update.lock` in the repo root. A half-written pin is worse than a stale one,
  because the flake would fail to evaluate. The nightly search fetches up to a month of dated
  manifests in parallel. Only a 404 counts as "no such nightly". Any other failure newer than
  the pick fails the run, so the pin never silently moves backwards. `node` pins the newest
  release of every supported line, then Vite+, whose lockfile is downloaded and checked against
  `vp-checksums.txt` before it is trimmed, then the package managers: pnpm from its GitHub release
  assets (each sha256 checked against GitHub's artifact attestation metadata, whose Sigstore
  signature is not verified locally), bun from oven-sh/bun's `SHASUMS256.txt` (HTTPS-only, not
  signed), yarn from npm registry metadata. `update deno` pins the newest stable Deno
  releases from denoland/deno's GitHub API assets. Node.js's `SHASUMS256.txt` files are
  GPG-verified before their hashes are used; bun's is not. `python` pins the newest release of
  every supported line from
  the latest python-build-standalone build — and the freethreaded lines of the same versions —
  then the latest uv. Neither project signs its
  checksum file, so each is checked against the sha256 digest GitHub records for the release
  asset before its hashes are used. `go` pins the newest release of the two newest Go lines from
  go.dev's release index, which carries every archive's sha256 (Go publishes no signed checksum
  file); `update/go.rs` also holds `pin go`. `zig` pins the newest tagged release of every minor line, from
  ziglang.org's `index.json`, which carries every archive's sha256. `tsrs` pins the release
  npm's `latest` tag names (not the highest version: SemVer sorts `tsrs10` before `tsrs9`),
  after checking that the launcher's optional dependencies name that same release for every
  platform package, with the registry's sha512 integrity for each tarball.
- `pin.rs`: `pin rust <channel>` prints the `flake = false` input and the SRI hash for a
  channel, from `init`'s `channel::manifest_url`.
  `pin node <x.y.z>` does the same for a release's `SHASUMS256.txt`, after checking its GPG
  signature and that it lists a tarball for every system. `pin go <version>` (in
  `update/go.rs`) prints a `toolchain` call with the release's per-system `hashes`.
  `pin python <x.y.z>` (in `pin.rs`) walks the repo's release list from the newest until a build
  carries the release's `install_only` archives for every system, checks the build's
  `SHA256SUMS` against the digest GitHub recorded for it, and prints the `sha256sums` input and
  the file's hash.

`doctor` checks the user's environment. Is `cargo` the fork? It also looks for rustup
proxies, `CARGO_INCREMENTAL=0`, `CARGO_TARGET_DIR`, `RUSTC_WRAPPER` caches, a stale
`RUST_SRC_PATH`, a `rust-toolchain.toml` that disagrees with `rustc`, variables the current
`hm-session-vars.sh` no longer sets (a shell older than the last switch), and on macOS a GCC
`cc`. For Node.js it compares the active node with the nearest `.node-version`/`.nvmrc` and the
active package manager with `package.json`'s `packageManager` (or `devEngines.packageManager`,
in `fromPackageJson`'s order), and warns about `VP_HOME`, `VP_ENV_FORBIDDEN`, `NODE_PATH` and an
npm prefix in the read-only store. Versions go through `version_matches`, a Rust copy of the
subset of npm ranges `lib/node/semver.nix` reads; it returns `None` for what it cannot decide, so
an alias is never reported as a mismatch. Copies of the package manager further down PATH are
warned about like extra cargos. Which node a package manager runs on is read from its file alone
(an `env node` shebang, an explicit `.../bin/node` in a small shell wrapper, makeWrapper's
`.<name>-wrapped` target), never from scanning a bundled script, whose strings mention node paths
that say nothing about how it runs.

The other languages live in their own modules (`python.rs`, `go.rs`, `zig.rs`, `deno.rs`), each
silent when neither its executable nor its project file is found. They read PATH and variables
through `Env`, which the tests replace with their own so they never touch the process
environment, and share `find_nearest_file`, `shadowed`, `stdout_of` and `at_least` (a minimum,
through `version_matches`). Python compares `python3` with `.python-version` and
`requires-python` through `pep440_matches`, the clauses `lib/python/pep440.nix` reads (and
asks `python3` for `Py_GIL_DISABLED` when the entry ends in "t"), and
checks the `UV_*` variables uv's setup hook overwrites, reading the default `UV_PYTHON` from
crucible's makeWrapper script when the variable is unset. Go compares `go version` with the
`go` and `toolchain` directives as minimums, resolves `GOTOOLCHAIN` the way go does
(environment, `GOENV` file, the GOROOT's `go.env`), and runs every go command with
`GOTOOLCHAIN=local` so none of them downloads a toolchain. Zig compares `zig version` with
`.minimum_zig_version`. Deno only reports the version: `deno.json` pins none.

### Integration

- `modules/home-manager.nix`: `programs.crucible.rust.*`. It installs a toolchain, sets
  `CARGO_HOME`, writes `~/.cargo/config.toml` from `settings` and `gc` (the fork's
  `[build.gc]`), and warns about settings that work against the fork. It deliberately sets no
  `RUST_SRC_PATH`: rust-analyzer finds the sources through `rustc --print sysroot`, and a global
  value would leak this release into every other project.
- `modules/home-manager.nix`: `programs.crucible.node.*` installs a Node.js toolchain and,
  with `npmPrefix`, gives `npm install --global` a writable prefix. It warns when `VP_HOME` is
  set, since crucible's vp would then share another vp's config.
- `modules/home-manager.nix`: `programs.crucible.python.*` installs a Python toolchain and puts
  `~/.local/bin` (`uv tool install`'s commands) on PATH. It warns when `PYTHONHOME` is set, or a
  `UV_PYTHON*`/`UV_MANAGED_PYTHON` session variable overrides what crucible's uv sets.
- `modules/home-manager.nix`: `programs.crucible.go.*` installs a Go toolchain and sets
  `GOPATH` (`~/go` by default) with its `bin` on PATH. It warns about `GOROOT` and a
  `GOTOOLCHAIN` other than `local`.
- `modules/home-manager.nix`: `programs.crucible.zig.*` installs a Zig toolchain; its
  `extraPackages` slot fits nixpkgs' `zls`, the Zig Language Server, which does not track
  crucible's pins.
- `modules/home-manager.nix`: `programs.crucible.tsrs.*` installs tsrs and tsrs-lint, for
  editors and checks outside projects that install their own.
- `templates/default`: a Rust project whose toolchain comes from `rust-toolchain.toml`, with a
  `buildRustPackage` package.
- `templates/node`, `templates/python`, `templates/go`, `templates/zig`, `templates/deno`,
  `templates/rust`, `templates/bun`, `templates/uv`: dev shells whose toolchains come from the
  matching `crucible.lib` module. Node pins pnpm from GitHub, Python pins uv with CPython, and
  Bun is the Node toolchain's pinned package manager; the rest take the newest supported line.
  The remaining templates use nixpkgs packages until crucible pins their toolchains.

### Verification

- `tests/default.nix` → `checks.<system>`, one set per language (`tests/rust/`, `tests/node/`,
  `tests/zig/`) plus the CLI
  build and `crucible-clippy`, which runs clippy on the workspace with every warning denied
  (`[workspace.lints]` in `Cargo.toml` sets the same levels for local runs). The Rust checks build `tests/rust/hello` with each kind of toolchain: stable (clippy,
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
  crucible's lib, runs the user build's `init` in the numbered-channel and the all-languages
  fixtures. Its `templates` check compares every `templates.nix` attr (including `hash` → `hashi`
  and the `c`/`cpp` aliases) with the template directories and evaluates each flake's outputs for
  every supported system with stub inputs; it does not build template packages. The nixpkgs-only
  templates are checked for output structure, while the ones that use `crucible.lib` also have
  their dev shells forced. `tests/default.nix` evaluates the Node template's dev shell with a stub
  toolchain to prove it asks for a pinned pnpm, and checks that the user binary carries the Node,
  Python, Go, Zig and Deno checks. The Python checks run python,
  pip and a stdlib venv on the newest line and every pinned line, take `tests/python/app` through
  `uv sync` and `uv run` offline (uv must find the toolchain's python and refuse to download
  another), check the setup hook beats inherited values, build and import a C extension from
  sysconfig's own compiler settings, resolve `.python-version`, `requires-python` and
  specifiers, build from a trimmed `SHA256SUMS`, and reject misuse at evaluation time.
  The Go checks build, vet,
  test and run `tests/go/hello` (gofmt included) and run every pinned line, check that a module
  needing a newer Go fails with `GOTOOLCHAIN=local` and that the setup hook beats inherited
  values, build a cgo program (on Linux its interpreter must be in the store), resolve
  `fromGoMod` from go.mod files generated from the pins, build a release from `hashes`, reject
  misuse at evaluation time, and build `tests/go/hello` with `.buildGoModule`.
  The Zig checks run `zig version` and build and run `tests/zig/hello` with the default
  toolchain, check every pinned line resolves, resolve `fromBuildZigZon` on the zon fixtures,
  and reject misuse at evaluation time.
- `dev/flake.nix`: a separate flake for treefmt (nixfmt, deadnix, statix, typos, oxfmt,
  rustfmt, taplo) and git hooks (treefmt, gitleaks), kept separate so consumers never pay for
  those inputs. It also owns the Home Manager module check, since the main flake must keep
  nixpkgs as its only input.
- `.github/workflows/`: `check.yaml` (format and hooks, then a build matrix on x86_64-linux,
  aarch64-linux and macOS), `update.yaml` (daily `crucible update` + `nix flake update`, opening
  a PR), and `hestia-gc.yaml` (cache GC). Nix setup is in `.github/actions/setup-nix`.

## Invariants

- **nixpkgs is the only flake input** of the main flake. Development tooling goes in `dev/`.
- **Evaluation is offline** for pinned releases. The only Nix-side fetch is `builtins.fetchurl`
  for an explicit `sha256` (a Rust manifest or Node.js `SHASUMS256`), checked against that hash;
  anything else that needs the network belongs in the CLI and ends up in `data/`.
- **uv never manages Python.** It runs the toolchain's python, never one it installed or
  downloaded itself.
- **go never switches toolchains.** `GOTOOLCHAIN=local` in `go.env` and in the setup hook;
  a go.mod that needs a newer Go is an error, not a download.
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
