# Node / JS toolchain from upstream metadata — research

Verified 2026-11-19 on aarch64-darwin; commands/URLs shown.

## 1. Node.js

- `https://nodejs.org/dist/index.json`: newest-first array; fields `version`, `date`,
  `files[]`, `npm`, `v8`, `uv`, `zlib`, `openssl`, `modules` (ABI), `lts` (codename or `false`),
  `security`. **No hashes** (`jq -r '.[0]|keys|join(",")'`). Newest: `v26.13.1` (2026-11-10,
  lts=Lithium), `v24.22.3` (Krypton); `v27.0.0-alpha.*` is `lts:false`.
- Hashes are per version in `https://nodejs.org/dist/v26.13.1/SHASUMS256.txt`, GPG-signed
  (`.sig`/`.asc`, keys at `github.com/nodejs/release-keys`); nixpkgs verifies this in `update.nix`.
- `files[]` ≠ artifact name: it says `osx-arm64-tar`, the file is `node-v26.13.1-darwin-arm64.tar.xz`.
  Map: `x86_64-linux→linux-x64`, `aarch64-linux→linux-arm64`, `aarch64-darwin→darwin-arm64`.
  `.tar.xz` exists for all three (checked v18–v26).
- Sizes (`curl -w '%{size_download}'`): linux-x64 34.1 MB, linux-arm64 33.1 MB, darwin-arm64 xz
  30.6 MB (gz 58.3 MB); unpacked 226/237 MB, `bin/node` 150 MB unstripped. Layout:
  `bin/{node,npm,npx}`, `lib/node_modules/npm`, `include/node`.
- **NixOS patching** (`nix run nixpkgs#patchelf -- --print-interpreter/--print-needed`, linux-arm64):
  `/lib/ld-linux-aarch64.so.1`, empty RPATH, NEEDED `libatomic libdl libm libstdc++ libgcc_s
  libpthread libc` → `autoPatchelfHook` + `stdenv.cc.cc.lib`. Trap: patchelf ≥0.19 header sorting
  crashes official Node; nix-openclaw sets `patchelfFlags = [ "--no-sort" ]`.
- Darwin: only `/usr/lib/libSystem`, `libc++`, frameworks; no LC_RPATH, ad-hoc signed — no patching.
  Official Linux binaries need glibc ≥ 2.34, libstdc++ ≥ `GLIBCXX_3.4.29` (`BUILDING.md`).
- nixpkgs builds from source (`v26.nix`: 26.13.0, ~7 patches); `nodejs` = 24.22.2 vs upstream
  24.22.3: correct, slow, late.

## 2. Package managers

- **pnpm** (`gh api repos/pnpm/pnpm/releases/latest` → v12.21.0): assets
  `pnpm-{linux-x64,linux-arm64,linux-arm64-musl,darwin-x64,darwin-arm64,win32-*,…}.tar.gz`,
  24–32 MB; `digest` present (`sha256:6c5913a4…`; downloaded, shasum matched). Tarball is a 50 MB
  Node SEA + `dist/`; interpreter `/lib/ld-linux-aarch64.so.1`, NEEDED
  `libgcc_s libpthread libm libdl libc` → patch on NixOS. Registry alternative:
  `pnpm-12.21.0.tgz` (`sha512-Zv2CeO4o…`, `bin.pnpm` JS, needs Node).
- **yarn berry** (`gh api repos/yarnpkg/berry/releases/latest` → `@yarnpkg/cli/4.18.1`):
  **0 assets**, no digest. Use npm `@yarnpkg/cli-dist@4.18.1` (`sha512-EH+A4TxA…`,
  `bin.yarn = bin/yarn.js`) with Node. nixpkgs bootstraps it from the repo with node+yarn.
- **bun** (`gh api repos/oven-sh/bun/releases/latest` → `bun-v1.4.5`): assets
  `bun-{linux-x64,linux-aarch64,linux-aarch64-musl,darwin-x64,darwin-aarch64,…}[,-baseline][,-profile].zip`
  - `SHASUMS256.txt`; `digest` present (`sha256:18656a3b…`, matched). Zip holds only
    `bun-linux-aarch64/bun` (107 MB; 49 MB zipped); interpreter `/lib/ld-linux-aarch64.so.1`,
    NEEDED `libc libpthread libdl libm` → patch. nixpkgs' `bun` does zip + `autoPatchelfHook`.
- `digest` is **not retroactive**: `null` for pnpm v9.0.0 and bun v1.0.0 assets. Verify by
  download/SHASUMS.
- **corepack**: removed in Node ≥25 (`bundlesCorepack = !lib.versionAtLeast version "25.0.0"`,
  nixpkgs `nodejs.nix`; v26 tarball has none). nixpkgs ships only `corepack_22/24`.

## 3. Project version files

- `packageManager`: `"pnpm@10.12.1+sha512.…"`; hash = SHA-512 of the **registry tarball**.
  Verified: sha512 of `pnpm-10.12.1.tgz` = `sha512-8N2oWA8O…` = `dist.integrity`. Corepack hashes
  the tarball (or the URL file).
- `devEngines`: keys `cpu`, `os`, `libc`, `runtime`, `packageManager`; object or array of
  `{name, version, onFail}`, `onFail` ∈ `warn|error|ignore` (default `error`); npm checks it before
  `install`/`ci`/`run`. Corepack falls back to `devEngines.packageManager` when `packageManager` is
  absent.
- `engines.node`/`engines.npm`: semver ranges, advisory.
- `.nvmrc`: one version or alias (`lts/*`, `node`), `#` comments, parent lookup (nvm README).
  `.node-version`: same shape, read by nodenv/asdf/volta (nodenv README).
- Precedence: `packageManager` > `devEngines.packageManager`; `.node-version`/`.nvmrc` >
  `engines.node` > pinned LTS. Ranges resolve against the pinned index (highest match, `lts/*` =
  newest `lts`), never the network; no match = eval error.

## 4. Ecosystem options

| Option                                              | Strength / weakness                                                          |
| --------------------------------------------------- | ---------------------------------------------------------------------------- |
| `nodejs_22/24/26`, `nodejs-slim`                    | patched, reproducible / hours to build, lags upstream, no per-project pin    |
| `pnpm_10/11/12`                                     | maintained; 12 from Rust source / lag (12.9.0 vs 12.21.0), CVEs on old lines |
| `fetchPnpmDeps` + `pnpmConfigHook`                  | offline lockfile deps / builds apps, not supplies the CLI                    |
| `yarn-berry`                                        | correct bundle / heavyweight source bootstrap                                |
| nixpkgs `bun`                                       | shows binaries are acceptable / version lag, patch work                      |
| community flakes (`nix-openclaw`, `microscope-dev`) | exact, fast / ad-hoc, partial platforms, no signature checks                 |

## 5. What `doctor` should catch

- `npm config get prefix` in `/nix/store` or unwritable → `npm i -g` fails; `NPM_CONFIG_PREFIX`
  silently overrides it (set in this environment).
- `readlink -f $(command -v pnpm)` → corepack dist: a shim, not the pinned pnpm; corepack is absent
  on Node 26.
- `pnpm --version` ≠ `packageManager` (lockfile drift, pnpm 10 vs 12).
- Multiple `node` on PATH (three here), `node_modules/.bin` precedence, `NODE_OPTIONS`/`NODE_PATH`.
- ABI/glibc: `process.versions.modules` vs prebuilt addons, musl vs glibc.

## Recommended design

- `data/node.json`: trimmed index entries + per-system `{url, hash}` from SHASUMS, keyed by version
  (`lts`, `modules`, `npm` kept). `data/{pnpm,yarn,bun,corepack}.json`: version, `{url, hash}` per
  system, plus npm `{version, integrity}` for `packageManager` checks.
- Artifacts: `nodejs` = official tarball unpacked, `autoPatchelfHook` on Linux with `--no-sort`,
  `dontStrip`; passthru `npm`, `headers`. `pnpm` = standalone tarball + patchelf. `yarn` =
  `@yarnpkg/cli-dist` + node wrapper. `bun` = zip + patchelf. `corepack` optional, never default.
- `fromPackageJson` reads `packageManager` → `devEngines.packageManager` → node pin files →
  `engines.node`.
- CLI: `update` (index + SHASUMS, GPG optional, compact JSON), `pin <spec>` (adds verified hashes),
  `resolve` (range → pinned), `doctor` (checks above).
- Risks: index.json is hash-free (trust HTTPS + signature), GitHub `digest` gaps, pnpm/yarn release
  format churn, patchelf header bug, corepack removal, glibc floor, 226 MB per Node.

## Not verified

No aarch64-Linux builder: patchelf findings are static, no runtime test of patched node/pnpm/bun.
NixOS npm-prefix behaviour untested there. Yarn asset history checked only recently.
Corepack-as-npm-package digest unchecked.

## Summary

1. Node tarballs cover all three targets, but `index.json` lacks hashes; SHASUMS + GPG is the anchor.
2. Linux binaries need interpreter/RPATH patching (`--no-sort`); darwin none.
3. pnpm/bun expose GitHub `digest` only on recent releases; yarn berry has no release assets.
4. `packageManager` hashes the registry tarball — matches npm integrity, checkable offline.
5. Design mirrors crucible: pinned JSON, per-component derivations, `update`/`pin`/`doctor`.

Report: `/private/tmp/claude-501/-Users-gaurav-personal-projects-crucible/65bfd540-06e8-4f3a-a83d-96f2aa1b3798/scratchpad/node-research.md`
