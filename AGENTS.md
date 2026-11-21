# AGENTS.md

Guidance for coding agents working in this repo. Read `README.md` for what crucible does and
`ARCHITECTURE.md` for how it is built.

## Verify before you commit

```sh
nix flake check -L --keep-going           # every check for this machine's system
nix flake check --all-systems --no-build  # everything evaluates on all three systems
cd dev && nix fmt -- --fail-on-change     # formatting, deadnix, statix, typos
cd dev && nix flake check                 # git hooks (treefmt, gitleaks)
# The CLI is a Cargo workspace at the repo root (crates/*):
nix shell .#rust.stock -c cargo test --workspace                         # unit tests
nix shell .#rust.stock -c cargo clippy --workspace --all-targets         # lints
nix shell .#rust.stock -c cargo build -p crucible --no-default-features  # doctor-only build
```

- **New files must be `git add`ed** before Nix can see them. An untracked file looks like a
  missing file to the flake.
- CI builds on x86_64-linux, aarch64-linux and macOS. Locally you usually cover only one system.
  Run the `--all-systems --no-build` evaluation at least, and say which systems you actually
  built.
- macOS sandbox builds print an `xcrun` warning. It is old and harmless.
- Use `.#rust.stock` (stock cargo) for the cargo commands. The fork shares one build directory
  between every worktree of the repo, so parallel worktrees race on it.

## Conventions

- **Commits**: Conventional Commits with a scope where one fits (`feat(rust):`,
  `fix(cli):`, `refactor!:`), and `!` plus a `BREAKING CHANGE:` note when a public output, an
  option or a `lib` function changes. Wrap bodies at 72 columns. Commit only when asked, and
  never push, force-push or rename the GitHub repo without asking first.
- **Comments explain why, not what.** Match how densely the surrounding code is commented: most
  Nix functions open with a comment saying why they exist.
- **Error messages** begin with `crucible:` and tell the user what to do next (see
  `resolveManifest` and `assemble` in `lib/rust/toolchain.nix`).
- **Document what users can see.** Any change to an argument, output or option updates the
  README, the Home Manager option `description`, and a check in `tests/`.
- **Per-language layout**: `lib/<lang>/`, `data/<lang>/`, `tests/<lang>/`, and
  `lib.<system>.<lang>.*` in the flake. Shared code goes in `lib/common/`. The public function
  names are `toolchain` and `from<ProjectFile>` (for example `fromToolchainFile`, and later
  `fromPackageJson`). Do not use `mk*` names.

## Rules that are easy to break

- **Never edit `data/**/*.json` by hand.** `crucible update` writes them, and treefmt excludes
  them, as it does `crates/*/tests/fixtures/*.json`. Reformatting them would churn every update.
- **The main flake has one input, nixpkgs.** Development tools go in `dev/flake.nix`.
- **Nix evaluation stays offline.** Network fetches belong in the CLI, and their results go into
  `data/`.
- **Two copies must stay in sync**:
  - `lib/common/systems.nix` ↔ `HOSTS` in `crates/upstream/src/manifest.rs` (same systems, same
    order)
  - `manifestUrl` in `lib/rust/toolchain.nix` ↔ `manifest_url` in `crates/pins/src/pin.rs`
  - `platforms` in `lib/node/toolchain.nix` ↔ `PLATFORMS` in `crates/pins/src/update/node.rs`
  - `shasumsUrl` in `lib/node/toolchain.nix` ↔ the URL `pin::node` prints in
    `crates/pins/src/pin.rs`
- **Package builds use `.rustPlatform`** (stock cargo). The fork cannot build in the sandbox,
  because it keeps its build directory under `~/.cargo/targets`.
- **The sysroot's real copies are deliberate** (`bin/*`, `librustc_driver`). Do not "optimize"
  them into symlinks. rustc finds its sysroot from the location of the driver library.

## Nix gotchas

- `runCommand` has no C compiler. A check that links needs `pkgs.stdenv.cc` in
  `nativeBuildInputs`.
- Store copies are read-only, so `chmod u+w` before `patchelf` or editing them.
- POSIX regex in `builtins.match`: `[=` inside a bracket expression starts an equivalence class.
- patchelf ≥ 0.19 sorts headers, which crashes the official `node` binary, so `lib/node/node.nix`
  passes `--no-sort`, but only where patchelf knows it (nixpkgs' stdenv still has 0.15).

## Shell gotchas (the maintainer's machine)

- The shell is zsh. A `$VAR` that holds several flags is not word-split, so use arrays:
  `F=(...); cmd "${F[@]}"`.
- `ls` is aliased to eza. Use `command ls` when the output gets parsed.

## Linux from macOS

aarch64-linux can be tested in an Apple `container`:

```sh
container system start
container run -d --name nixlinux --memory 8G --cpus 6 -v "$PWD":/src docker.io/nixos/nix:latest sleep infinity
container exec nixlinux sh -c 'echo "experimental-features = nix-command flakes" >> /etc/nix/nix.conf'
container exec nixlinux sh -c 'cd /src && nix flake check -L --keep-going'
```

Mount a copy made with `rsync -a --delete --exclude fenix --exclude nix-vite-plus --exclude
/target --exclude result` rather than the working tree itself. Only CI covers x86_64-linux.

## Out of scope

- `fenix/` and `nix-vite-plus/` are gitignored reference clones. Read them, but do not change or
  import them.
- Downstream consumers (other projects, the maintainer's dotfiles) are not part of this repo.
  Change them only when asked, and never touch other people's repos without asking.
