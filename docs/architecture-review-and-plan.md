# Crucible — architecture review & unified implementation plan

**Status:** review and plan, produced 2026-11-24 and corrected 2026-11-25 after Wave 1
(Phases 0–2) landed. Findings contradicted by the landed code are marked as such; the plan is
kept for context.
**Scope:** the whole repository — Rust workspace, Nix library/flake/side-car files, pin
generation and verification paths, and all templates — with the staged working-tree changes
treated as the current design.
**Constraint observed during review:** a sibling thread was simultaneously rewriting
`templates/`, so the review itself edited nothing. The 2026-11-25 corrections cover the landed
template work (4.5–4.7), the enforced sync check and the revised template checks; they do not
touch `data/**/*.json`.

---

## Table of contents

1. [Executive summary](#1-executive-summary)
2. [Current shape — what the plan must preserve](#2-current-shape--what-the-plan-must-preserve)
3. [Part A — implementation review](#part-a--implementation-review)
   - [A.1 Rust workspace](#a1-rust-workspace)
   - [A.2 Nix side](#a2-nix-side)
   - [A.3 How the two sides stay in sync](#a3-how-the-two-sides-stay-in-sync)
   - [A.4 Pin generation and verification](#a4-pin-generation-and-verification)
4. [Part B — templates and toolchain support](#part-b--templates-and-toolchain-support)
5. [Part C — the plan](#part-c--the-plan)
   - [Phase 0 — land and harden the in-flight verification work](#phase-0--land-and-harden-the-in-flight-verification-work-do-now)
   - [Phase 1 — make the documented invariants real](#phase-1--make-the-documented-invariants-real-next-depends-on-0)
   - [Phase 2 — structural cleanup](#phase-2--structural-cleanup-next-parallel-with-1-after-its-check-exists)
   - [Phase 3 — close verification and pin-path gaps](#phase-3--close-verification-and-pin-path-gaps-later-depends-on-12)
   - [Phase 4 — template and toolchain coverage](#phase-4--template-and-toolchain-coverage-later-depends-on-01-sibling-set-first)
6. [Part D — risks, tradeoffs and open questions](#part-d--risks-tradeoffs-and-open-questions)
7. [Appendix A — verification command reference](#appendix-a--verification-command-reference)
8. [Appendix B — file index of findings](#appendix-b--file-index-of-findings)

---

## 1. Executive summary

Crucible is in good architectural health: a clean acyclic Cargo workspace whose user build
excludes the entire TLS stack, a single `default.nix` wiring point, offline Nix evaluation over
generated JSON, and genuine upstream-artifact verification on several data families. The staged
changes (pnpm GitHub-artifact attestation, npm-registry signature verification for yarn and tsrs)
materially improve provenance.

The dominant problems are **drift and duplication**, not structural faults:

1. **Sync pairs were unenforced when reviewed, and are now checked.** The eight "two copies must
   stay in sync" rules in AGENTS.md plus the newer pairs are enforced by
   `crates/init/tests/sync.rs`, which reads the sources as text; `tests/init.nix` also checks the
   systems list. The 13 copies of the platform/system tables still exist (Phase 2.1).
2. **Verification is uneven.** Node (GPG) and Python/uv (GitHub asset digest) are strong; cargo's
   `SHA256SUMS`, Vite+'s `vp-checksums.txt` and bun's `SHASUMS256.txt` are trusted over HTTPS with
   no independent anchor; pnpm's new attestation check does not verify the Sigstore signature
   locally (a documented tradeoff).
3. **The gaps found here were closed in Wave 1, except one.** `tests/init.nix` now derives the
   template list from `templates.nix`, compares it with the directories and evaluates every
   attr's outputs; `tests/default.nix` now evaluates the Node template's dev shell with a stub
   toolchain that requires a pinned `pnpm` instead of grepping the text.
   `attestation::verify` now has branch tests, but `vite_plus.rs` still has none.
4. **Templates are integrated, and nixpkgs-only is an explicit policy.** Nine template attrs
   call `crucible.lib` (`default`, `rust`, `node`, `bun`, `python`, `uv`, `go`, `zig`, `deno`);
   all templates are dev-shell-only except `default`, which also has a package. The nixpkgs-only
   templates are listed with reasons in `docs/template-policy.md`, and this flake is now the
   single template source: the CLI uses `github:kiyors/crucible#<name>`.
5. **One confirmed broken build at review time, now fixed:** `dev/flake.nix` had included
   `x86_64-darwin`, which pinned nixpkgs dropped; Phase 0.3 removed it and the sync check now
   keeps its systems list aligned with `lib/common/systems.nix`.

The plan is five phases: harden the in-flight security work, enforce invariants with a sync
check, remove structural duplication, close verification/pin gaps, then expand template coverage
one full language stack at a time (Elm first).

---

## 2. Current shape — what the plan must preserve

### 2.1 Nix

- One flake input (nixpkgs). [flake.nix](/Users/gaurav/personal/projects/crucible/flake.nix:8) does
  no work itself; everything goes through [default.nix](/Users/gaurav/personal/projects/crucible/default.nix:1),
  which is also what the overlay and non-flake users call.
- `lib/<lang>/` for Rust, Node (+Vite+, pnpm/yarn/bun), Deno, Python (+uv), Go, Zig, tsrs.
- `lib/common/` contains only [systems.nix](/Users/gaurav/personal/projects/crucible/lib/common/systems.nix:1):
  `x86_64-linux`, `aarch64-linux`, `aarch64-darwin`.
- Evaluation reads `data/**/*.json` offline; network fetches live in the CLI.

### 2.2 Rust

- Workspace crates at the repo root ([Cargo.toml](/Users/gaurav/personal/projects/crucible/Cargo.toml:1)):
  `crucible` → `doctor`, `init`, optional `pins` → `upstream`. Acyclic.
- `net = ["dep:pins"]` in [crucible/Cargo.toml](/Users/gaurav/personal/projects/crucible/crates/crucible/Cargo.toml:10)
  means the user build leaves out rustls/ring by _not depending on them_, not via `#[cfg]` in
  library code. `crucible` and `crucible-full` are built by
  [package.nix](/Users/gaurav/personal/projects/crucible/crates/crucible/package.nix:1) with
  crucible's own `rust.stable.rustPlatform`.
- `upstream` owns HTTP/GPG/SRI/npm-signature/attestation/pnpm-lock logic; `pins` owns `update`
  and `pin` and writes `data/`.

### 2.3 Supported toolchain set today

Rust, Node, Python, Go, Zig, Deno, tsrs, plus bun/pnpm/yarn as Node package managers.

- `init` detects six project-file kinds ([init/lib.rs](/Users/gaurav/personal/projects/crucible/crates/init/src/lib.rs:91)):
  `rust-toolchain.toml`/`rust-toolchain`, `package.json`, `deno.json(c)`, `pyproject.toml` or
  `.python-version`, `go.mod`, `build.zig.zon`.
- `doctor` covers Rust, Node, Python, Go, Zig, Deno ([doctor/lib.rs](/Users/gaurav/personal/projects/crucible/crates/doctor/src/lib.rs:28)).
- Home Manager has six options (rust, node, python, go, zig, tsrs) — **no `deno` option**.
- `pin` commands exist only for rust, node, go, python ([main.rs](/Users/gaurav/personal/projects/crucible/crates/crucible/src/main.rs:84)).

### 2.4 One template source (reconciled in 4.6)

| Consumer                                            | Source                                          | Location                                                                             |
| --------------------------------------------------- | ----------------------------------------------- | ------------------------------------------------------------------------------------ |
| `nix flake init -t github:kiyors/crucible#<name>`   | this flake's `templates/`                       | `flake.nix` / `templates.nix` (25 dirs, `c`/`cpp` aliases, `hash` → `hashi`)         |
| `crucible init <dir> [template]` and `crucible add` | the same flake, `github:kiyors/crucible#<name>` | `crates/crucible/src/templates.rs` (explicit accepted-name list from the same attrs) |

The old external `github:kiyors/templates` repo and its local `nix-templates/` clone are no
longer a second source. The names are now checked offline: `tests/init.nix` compares the
`templates.nix` attrs with the directories, and `crates/init/tests/sync.rs` compares them with
the README list and the CLI's names.

---

# Part A — implementation review

## A.1 Rust workspace

### A.1.1 Module boundaries and layering

The graph is acyclic and largely matches [ARCHITECTURE.md](/Users/gaurav/personal/projects/crucible/ARCHITECTURE.md:302):

```
crucible ─► doctor
         ─► init
         ─► pins (only with `net`) ─► upstream
```

Good decisions to keep:

- `upstream` knows nothing about pins or data layout and holds the whole TLS/GPG/hashing stack.
- `doctor` is genuinely network-free and keeps language checks in sibling modules behind
  `pub(super)` ([doctor/lib.rs](/Users/gaurav/personal/projects/crucible/crates/doctor/src/lib.rs:12)).
- `init` being network-free and owning `channel::manifest_url` means `init` and `pin` share one
  URL rule ([channel.rs](/Users/gaurav/personal/projects/crucible/crates/init/src/channel.rs:1)).
- npm signature verification correctly lives in `upstream/npm.rs` and is reused by both yarn
  ([package_managers.rs](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/package_managers.rs:345))
  and tsrs ([tsrs.rs](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/tsrs.rs:139)).

Problems:

1. **Over-broad re-exports.** [pins/lib.rs:4](/Users/gaurav/personal/projects/crucible/crates/pins/src/lib.rs:4)
   publicly re-exports `attestation, gpg, http, manifest, npm, pnpm_lock, sri` purely so internal
   code can keep writing `crate::http`. The staged change had to edit this list. Likewise
   [pin.rs:17](/Users/gaurav/personal/projects/crucible/crates/pins/src/pin.rs:17) publicly re-exports
   `init::channel::manifest_url`. Make both `pub(crate)`.
2. **`pin` and `update` are not clean siblings.** `pin` reaches into `update` internals
   ([pin.rs:14](/Users/gaurav/personal/projects/crucible/crates/pins/src/pin.rs:14) imports
   `update::python::{self, PBS}` and `update::node::{shasums, sources, DIST}`), while `pin go` is
   implemented in [update/go.rs:84](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/go.rs:84).
   Extract shared release/source helpers.
3. **CLI concern in a library:** `pins` depends on `clap` only for `ValueEnum` on `Scope`
   ([update/mod.rs:28](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/mod.rs:28)).
4. **`attestation.rs` is pnpm-specific despite its generic name** — `API`, `RELEASE`, `REPOSITORY`
   are hardcoded at [attestation.rs:25](/Users/gaurav/personal/projects/crucible/crates/upstream/src/attestation.rs:25).
   Rename or parameterise.
5. **`doctor` is asymmetric.** Node checks occupy ~380 lines in `doctor/lib.rs`, while
   Python/Go/Zig/Deno have their own modules, and those modules use a testable `Env`. Thread `Env`
   through the Node/Rust checks and move them to `doctor/src/node.rs`.

### A.1.2 Duplication inventory

| Duplicated item                           | Exact sites                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Platform/system tables (13 copies)        | [systems.nix:3](/Users/gaurav/personal/projects/crucible/lib/common/systems.nix:3), [manifest.rs:12](/Users/gaurav/personal/projects/crucible/crates/upstream/src/manifest.rs:12) (`HOSTS`), [pnpm_lock.rs:28](/Users/gaurav/personal/projects/crucible/crates/upstream/src/pnpm_lock.rs:28), [init/lib.rs:21](/Users/gaurav/personal/projects/crucible/crates/init/src/lib.rs:21) (`SYSTEMS`), and `update/{cargo.rs:20, node.rs:23, vite_plus.rs:26, package_managers.rs:35, deno.rs:24, python.rs:34, go.rs:30, zig.rs:22, tsrs.rs:28}` |
| npm semver subset                         | [doctor/lib.rs:990](/Users/gaurav/personal/projects/crucible/crates/doctor/src/lib.rs:990) vs [semver.nix](/Users/gaurav/personal/projects/crucible/lib/node/semver.nix)                                                                                                                                                                                                                                                                                                                                                                   |
| PEP 440 subset                            | [doctor/python.rs:374](/Users/gaurav/personal/projects/crucible/crates/doctor/src/python.rs:374) vs [pep440.nix](/Users/gaurav/personal/projects/crucible/lib/python/pep440.nix)                                                                                                                                                                                                                                                                                                                                                           |
| rustup channel parsing                    | [init/channel.rs:20](/Users/gaurav/personal/projects/crucible/crates/init/src/channel.rs:20) vs [doctor/lib.rs:506](/Users/gaurav/personal/projects/crucible/crates/doctor/src/lib.rs:506)                                                                                                                                                                                                                                                                                                                                                 |
| `Source { url, hash }`                    | Seven separate definitions: `node.rs`, `cargo.rs`, `deno.rs`, `go.rs`, `package_managers.rs`, `tsrs.rs`, `zig.rs`; python/vite-plus import `node::Source`                                                                                                                                                                                                                                                                                                                                                                                  |
| `version_key` / numeric version splitters | `update/{node.rs:179, zig.rs:172, python.rs:340, go.rs:174, package_managers.rs:368}`, `pin.rs:115,131`                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| "newest GitHub release" + `release_url`   | `cargo.rs:79`, `vite_plus.rs:87`, `python.rs:294`, plus inline in `deno.rs` and `package_managers.rs`                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| npm integrity check                       | [package_managers.rs:333](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/package_managers.rs:333) and [tsrs.rs:129](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/tsrs.rs:129)                                                                                                                                                                                                                                                                                                                          |
| `REGISTRY = "https://registry.npmjs.org"` | [pnpm_lock.rs:16](/Users/gaurav/personal/projects/crucible/crates/upstream/src/pnpm_lock.rs:16), [package_managers.rs:24](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/package_managers.rs:24), [tsrs.rs:24](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/tsrs.rs:24)                                                                                                                                                                                                                                |
| Hinnant date arithmetic                   | [rust.rs:195](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/rust.rs:195) vs [node.rs:187](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/node.rs:187)                                                                                                                                                                                                                                                                                                                                                   |

### A.1.3 Error handling

- The `crucible:` prefix and "tell the user what to do next" rule
  ([AGENTS.md:38](/Users/gaurav/personal/projects/crucible/AGENTS.md:38)) is followed in Nix but
  almost absent in Rust: only `gpg.rs` uses it; [main.rs:113](/Users/gaurav/personal/projects/crucible/crates/crucible/src/main.rs:113)
  prints `error: {err:#}` and roughly 100 context/bail strings have no prefix. Cheapest fix:
  prefix once at the CLI top level and add `crucible:` context to the new `npm.rs`/`attestation.rs`
  messages.
- Two library panics: [http.rs:97](/Users/gaurav/personal/projects/crucible/crates/upstream/src/http.rs:97)
  `unreachable!` and [rust.rs:157](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/rust.rs:157)
  `join().expect("nightly fetch thread panicked")`. The latter turns a fetch panic into a process
  panic instead of `NightlyOutcome::Failed`, undermining the carefully designed rule at
  `rust.rs:110–133` that any failure newer than the chosen nightly is fatal.
- 404 handling is correct in the nightly search but bare elsewhere: e.g.
  [attestation.rs:82](/Users/gaurav/personal/projects/crucible/crates/upstream/src/attestation.rs:82)
  lets a 404 escape as `GET https://… returned 404` instead of the helpful `bail!` at line 104.
- `parse_sha256sums` ([update/mod.rs:98](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/mod.rs:98))
  silently drops malformed lines and uppercase hex despite its "strict on purpose" comment. A
  needed file still fails later, but diagnosis is harder.
- `version_key` functions use `.parse().unwrap_or(0)`, silently changing ordering of a malformed
  upstream version; `newest_pnpm`/`newest_stable` rely on GitHub's first-match ordering rather
  than max-version.
- **Atomicity gap:** `update::run` builds the whole plan before writing (good), but the write loop
  at [update/mod.rs:82](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/mod.rs:82)
  has no rollback — if write 7 of 13 fails, 1–6 are already replaced.

### A.1.4 Test coverage

Well covered: every `upstream` module has unit tests (including real GPG and npm signature
fixtures); every `pins/update/*` has pure-helper tests; `doctor` uses fake executables and a
replaceable `Env`; `init` has 10 golden flakes checked both by Cargo and Nix; Nix checks build real
code for each language.

Concrete gaps:

1. [vite_plus.rs](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/vite_plus.rs) has
   **no tests at all**, including the lockfile/checksum comparison that is its only defence.
2. `attestation::verify` has one test, for JSON parsing only ([attestation.rs:129](/Users/gaurav/personal/projects/crucible/crates/upstream/src/attestation.rs:129)).
   Predicate/repo/tag filtering, subject matching, the terminal `bail!` and the 404 path are
   untested in _new security code_.
3. `package_managers.rs` tests only release selection and two rejection cases; `pnpm_pins`,
   `bun_pins`, `shasums`, `js_pins` and the successful signed path are untested. `tsrs.rs` never
   tests a successful `npm::verify` through `source()`.
4. `pin.rs` output snippets are untested, yet ARCHITECTURE.md requires `shasumsUrl`/`releaseUrl` to
   match Nix — only byte-level fixture builds check that indirectly.
5. `doctor::run()` and clap dispatch have no Rust tests; `Http` is concrete so no HTTP error path
   (404, 429, retry) has a unit test.
6. No `cargo test` asserts that `SYSTEMS`/`HOSTS` agree with `systems.nix`.

### A.1.5 CLI ergonomics

- Good: `arg_required_else_help = true` gives the deliberate "bare command prints help" behaviour
  checked in [tests/default.nix:51](/Users/gaurav/personal/projects/crucible/tests/default.nix:51);
  the `Missing` external-subcommand handler ([main.rs:79](/Users/gaurav/personal/projects/crucible/crates/crucible/src/main.rs:79))
  gives a useful "use `#crucible-full`" message.
- `--version` help says "`update cargo` only" ([main.rs:43](/Users/gaurav/personal/projects/crucible/crates/crucible/src/main.rs:43))
  but `Scope::All` also passes it to `cargo::plan` ([update/mod.rs:62](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/mod.rs:62)).
- `init` is overloaded: no args writes a flake; with a directory it shells out to `nix flake new`;
  `--force` applies only to the first path, and an existing directory is always refused.
- Help text had said "Template name from the nix-templates repo" while the code used
  `github:kiyors/templates`; Phase 2.9 changed it to the latter, but 4.6 made
  `github:kiyors/crucible` canonical. The two `main.rs` help lines still name the old repo and
  need a follow-up edit after merge; the README link to the local `nix-templates` directory is
  already gone.
- `pin` mixes prose and Nix snippets on stdout with inconsistent labels (`rustc:`/`node:`/`python:`
  vs `go:`); no machine-readable mode.

### A.1.6 Public API and dependency hygiene

- `publish = false` throughout; all versions declared once in root `[workspace.dependencies]`;
  resolver 3; warnings denied via `[workspace.lints]`. Good.
- `pgp` with `default-features = false` avoids bzip2; `p256` features are documented and add only
  one Cargo.lock line; no network dependency leaks into the user build.
- Nits: `serde_json` is both a normal and dev dependency of `upstream`; `pins/update/*` marks nearly
  everything `pub` though the CLI uses only `update::run`, `Scope`, `update::go::pin` and
  `pin::{run,node,python}`; `doctor` is the good model (`pub fn run` only).

---

## A.2 Nix side

### A.2.1 Per-language `toolchain.nix` comparison

| Language | `resolve`                                                                                  | Join / `assemble`                          | Setup hook                                      | Cross rejection | Notable                                              |
| -------- | ------------------------------------------------------------------------------------------ | ------------------------------------------ | ----------------------------------------------- | --------------- | ---------------------------------------------------- |
| Rust     | [toolchain.nix:98](/Users/gaurav/personal/projects/crucible/lib/rust/toolchain.nix:98)     | `symlinkJoin` + real `bin/*`/driver copies | `export RUST_SRC_PATH` (overwrite)              | Yes             | eval-time `fetchurl` for explicit `sha256`           |
| Node     | [toolchain.nix:92](/Users/gaurav/personal/projects/crucible/lib/node/toolchain.nix:92)     | `symlinkJoin`                              | `VP_ENV_FORBIDDEN=1`, unset `VP_HOME`           | Yes             | eval-time `fetchurl` for exact release               |
| Python   | [toolchain.nix:126](/Users/gaurav/personal/projects/crucible/lib/python/toolchain.nix:126) | `symlinkJoin`                              | 3 `UV_*` exports, 2 unsets                      | Yes             | `+` must be `%2B` in URLs                            |
| Go       | [toolchain.nix:60](/Users/gaurav/personal/projects/crucible/lib/go/toolchain.nix:60)       | No join                                    | `GOTOOLCHAIN=local`, unset `GOROOT` in `go.nix` | Yes             | no hashes escape hatch                               |
| Zig      | [toolchain.nix:30](/Users/gaurav/personal/projects/crucible/lib/zig/toolchain.nix:30)      | Single-path join                           | None                                            | Yes             | `cruisel` typo fixed in Phase 0.5                    |
| Deno     | [toolchain.nix:42](/Users/gaurav/personal/projects/crucible/lib/deno/toolchain.nix:42)     | None                                       | None                                            | Yes             | `.deno` passthru now present; stale ordering comment |
| tsrs     | [toolchain.nix:15](/Users/gaurav/personal/projects/crucible/lib/tsrs/toolchain.nix:15)     | None                                       | None                                            | Yes             | —                                                    |

Findings:

- **Deno and tsrs now reject cross package sets.** Phase 1.3 added the shared
  `lib/common/cross.nix` assert, so `pkgsCross` misuse fails with a `crucible:` hint.
- **`.deno` passthru is now present.** `lib/deno/deno.nix` sets both `passthru.version` and
  `passthru.deno = drv`, as README and `deno/toolchain.nix` document.
- The non-conforming `cargo.nix` throw was fixed in Phase 0.5; most "no build for ${system}"
  throws still lack a next step.
- Ready-made toolchains are inconsistently placed: Rust's in `default.nix`, Node/Python/Go's in
  `toolchain.nix` despite comments in their `default.nix` claiming otherwise; Deno/Zig/tsrs have
  none and are built at the call site in [flake.nix:44](/Users/gaurav/personal/projects/crucible/flake.nix:44).
- `notPinned`/`resolve` error shaping is reimplemented 5–6 times;
  `symlinkJoin` + setup-hook heredoc is repeated 4 times.

**Justified divergence (do not "fix"):** Rust's rust-lld wrapper and real-copy sysroot, Node's
`--no-sort` patchelf probe, Go's `dontFixup`, Python's sysconfig rewrite, and the per-format regex
parsing all follow from genuinely different upstream artifacts.

### A.2.2 Fragile patterns

1. `chmod u+w` on store copies in four files ([components.nix:254](/Users/gaurav/personal/projects/crucible/lib/rust/components.nix:254),
   [python.nix:62](/Users/gaurav/personal/projects/crucible/lib/python/python.nix:62),
   [go.nix:44](/Users/gaurav/personal/projects/crucible/lib/go/go.nix:44),
   [vite-plus.nix:83](/Users/gaurav/personal/projects/crucible/lib/node/vite-plus.nix:83)).
2. String-matching upstream text: Go's `go.env` replacement, Python's sysconfig `sed`, vp's
   `create/bin.js` patch, vp's `.vp-setup-complete` marker. These silently no-op or fail if
   upstream changes wording; `doInstallCheck` partly covers them.
3. Eval-time `builtins.fetchurl` in [rust/toolchain.nix:105](/Users/gaurav/personal/projects/crucible/lib/rust/toolchain.nix:105)
   and [node/toolchain.nix:102](/Users/gaurav/personal/projects/crucible/lib/node/toolchain.nix:102).
   Pinned evaluation stays offline, but these paths do network I/O at eval.
4. Regex parsing of TOML/SHASUMS/ZON/PEP440/semver; `builtins.match` anchors the whole string and
   uses POSIX ERE ([AGENTS.md:77](/Users/gaurav/personal/projects/crucible/AGENTS.md:77)).
5. **Wrapper environment hole:** setup hooks overwrite/unset variables, but
   [uv.nix:80](/Users/gaurav/personal/projects/crucible/lib/python/uv.nix:80) and
   [vite-plus.nix:151](/Users/gaurav/personal/projects/crucible/lib/node/vite-plus.nix:151) use
   `--set-default`, so outside a dev shell an inherited `UV_PYTHON` / `VP_ENV_FORBIDDEN=0` wins.
   This is a narrow breach of the "vp never manages Node.js" invariant; `--set`/unset would close it.

### A.2.3 Flake outputs, checks, overlay

- Packages: `default, rust, rust-nightly, cargo, node, node-current, vite-plus, python, uv, go,
  zig, tsrs, crucible, crucible-full`. **No `packages.<system>.deno`** (the overlay has one).
  `lib.rust.defaultComponents` is exported but missing from the README output table.
- Apps: only `default`; `crucible-full` works as `nix run .#crucible-full` via `meta.mainProgram`.
  `apps.default` itself is not checked.
- **Broken at review time, fixed in Phase 0.3:** `dev/flake.nix` included `x86_64-darwin`, so
  `nix eval ./dev#checks.x86_64-darwin.home-manager-module.drvPath` failed with "Nixpkgs 26.11 has
  dropped support for x86_64-darwin". Its systems now follow `lib/common/systems.nix` and sync.rs
  enforces that.
- **Check gaps that were closed:** `tests/init.nix` now derives the template list from
  `templates.nix`, compares it with the directories and evaluates every attr's outputs with stub
  inputs; `tests/default.nix` evaluates the Node template's dev shell with a stub toolchain that
  requires a pinned `pnpm` instead of grepping its text (Phase 1.4 and 0.4). Phase 1.5 added
  `programs.crucible.tsrs` to the Home Manager check, and Phase 0.3 removed `x86_64-darwin` from
  `dev/flake.nix`. Still unchecked: that the overlay equals `lib`/`packages`; Deno's test only
  runs `deno --version`.
- Overlay is a thin `import ./default.nix` ([overlay.nix:1](/Users/gaurav/personal/projects/crucible/overlay.nix:1)),
  so no drift there by construction.

### A.2.4 Documentation drift

- The template claims now agree: ARCHITECTURE describes the stub-input evaluation accurately,
  and only `default` has a package output; every other template is dev-shell-only. The README and
  ARCHITECTURE both point at `docs/template-policy.md` for the nixpkgs-only list.
- `.deno` is now really a passthru in `lib/deno/deno.nix`; `flake.nix`'s description now names
  Deno, Zig and tsrs; and `crates/init/tests/sync.rs` enforces that the README's pnpm, Deno, Go,
  Python and Zig versions appear in `data/**/*.json`. Still unresolved: "Nix never touches the
  network except with `sha256`" ignores Node's exact-release fetch, and `zig.rs` references a
  `PLATFORMS` table that `lib/zig/toolchain.nix` does not define.

---

## A.3 How the two sides stay in sync

### A.3.1 Existing eight pairs — current state and enforcement

The original eight pairs, and their status when reviewed (Phase 1.1 added the check in the last
column):

| #   | Pair (AGENTS.md:54–66)                              | Matches?                                   | Automated check now                              |
| --- | --------------------------------------------------- | ------------------------------------------ | ------------------------------------------------ |
| 1   | `systems.nix` ↔ `HOSTS` ↔ `SYSTEMS`                 | Yes (same order; triples differ by syntax) | `sync.rs`, plus `tests/init.nix` for init's list |
| 2   | `manifestUrl` ↔ `manifest_url`                      | Yes                                        | `sync.rs`                                        |
| 3   | Node `platforms` ↔ `PLATFORMS`                      | Yes                                        | `sync.rs`                                        |
| 4   | `shasumsUrl` ↔ `pin node` output                    | Yes                                        | `sync.rs`                                        |
| 5   | Python `platforms` ↔ `PLATFORMS`                    | Yes                                        | `sync.rs`                                        |
| 6   | `releaseUrl` ↔ `release_url`                        | Yes (both `%2B`)                           | `sync.rs`                                        |
| 7   | `-freethreaded-` infix / `python3.14t` ↔ `archive`  | Yes                                        | `sync.rs`                                        |
| 8   | Go `platforms` + `downloadUrl` ↔ `PLATFORMS` + `DL` | Yes                                        | `sync.rs`                                        |

**Conclusion: the pairs are now enforced cross-language.** `crates/init/tests/sync.rs` reads both
sides as text and names the file to edit on drift; the text-check fragility in Part D, RISK 5
remains.

### A.3.2 New pairs that should be documented and checked

- `layout` field in `data/node/package-managers.json` ↔ `pin.layout or "tarball"` in
  [package-manager.nix:28](/Users/gaurav/personal/projects/crucible/lib/node/package-manager.nix:28)
  (introduced by the staged work).
- npm keys endpoint ↔ pinned `KEYS` array ([npm.rs:7](/Users/gaurav/personal/projects/crucible/crates/upstream/src/npm.rs:7),
  [npm.rs:35](/Users/gaurav/personal/projects/crucible/crates/upstream/src/npm.rs:35)).
- GitHub attestation API, `REPOSITORY`, `RELEASE` and asset names
  ([attestation.rs:25](/Users/gaurav/personal/projects/crucible/crates/upstream/src/attestation.rs:25))
  ↔ the asset Nix fetches.
- The five undocumented `PLATFORMS` copies (`cargo`, `vite_plus`, `deno`, `zig`, `tsrs`,
  `package_managers`) and `pnpm_lock.rs` — now all checked by `sync.rs`.
- Template name lists: flake attrs ↔ `templates/` directories ↔ README list ↔ CLI accepted-name
  list. The external CLI repo is gone, so this is now an offline check (4.6).
- `dev/flake.nix` systems ↔ `systems.nix` — now checked and no longer divergent or broken.
- Hardcoded versions in README vs `data/**/*.json` — now checked by `sync.rs`, which reads the
  version from the prose so a pin refresh does not need a source edit.

### A.3.3 What adding a new language actually takes

There is no generator. The full stack is:

1. `crates/upstream/src/<format>.rs` if a new metadata format is needed.
2. `crates/pins/src/update/<lang>.rs`, a `Scope` variant, usually a `Pin` variant, and `data/<lang>/*.json`.
3. `lib/<lang>/{default,toolchain,<artifact>}.nix`; flake `lib`/`packages`/overlay exports.
4. `tests/<lang>/` plus import in [tests/default.nix](/Users/gaurav/personal/projects/crucible/tests/default.nix:6).
5. `crates/doctor/src/<lang>.rs` plus the call in `run()`.
6. A `programs.crucible.<lang>` option in [home-manager.nix](/Users/gaurav/personal/projects/crucible/modules/home-manager.nix:19).
7. An `init` `Project` field, `detect()` branch, shell rendering and golden fixture.
8. AGENTS.md sync entries, README output/usage docs, ARCHITECTURE.md, and a template.

This is precisely why the 16 gap templates remain nixpkgs-only.

---

## A.4 Pin generation and verification

### A.4.1 Per-family trust anchors

| Data family                         | Verification before hash is trusted                                                                                                                    | Code                                                                                                                                                                                    |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Rust manifests (`data/rust/*.json`) | None cryptographic — HTTPS TOML; completeness check for 9 required components on all hosts; any failure newer than the pick is fatal                   | [rust.rs:40](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/rust.rs:40), [manifest.rs:80](/Users/gaurav/personal/projects/crucible/crates/upstream/src/manifest.rs:80) |
| Fork cargo (`data/rust/cargo.json`) | **None** — `SHA256SUMS` fetched over HTTPS and parsed                                                                                                  | [cargo.rs:42](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/cargo.rs:42)                                                                                              |
| Node (`node.json`)                  | **GPG** detached signature against vendored 8-key allow-list — strongest path                                                                          | [gpg.rs:18](/Users/gaurav/personal/projects/crucible/crates/upstream/src/gpg.rs:18)                                                                                                     |
| Vite+ (`vite-plus.json`)            | Lockfile SRI checked against `vp-checksums.txt`; the checksum file itself is **unverified**, and lockfile tarballs are not npm-signature-checked       | [vite_plus.rs:68](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/vite_plus.rs:68)                                                                                      |
| pnpm (new)                          | GitHub attestation metadata: in-toto type, repo, tag, subject name+digest. **Sigstore signature not verified locally** — TLS-authenticated lookup only | [attestation.rs:80](/Users/gaurav/personal/projects/crucible/crates/upstream/src/attestation.rs:80)                                                                                     |
| bun                                 | **None** — `SHASUMS256.txt` trusted over HTTPS, asymmetric with python/uv                                                                              | [package_managers.rs:295](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/package_managers.rs:295)                                                                      |
| yarn / tsrs (new)                   | npm ECDSA signature vs 2 pinned keys; retired-key expiry checked against packument `time` for yarn                                                     | [npm.rs:59](/Users/gaurav/personal/projects/crucible/crates/upstream/src/npm.rs:59)                                                                                                     |
| Deno                                | GitHub API asset digest; missing digest fatal, no fallback                                                                                             | [deno.rs:77](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/deno.rs:77)                                                                                                |
| Python / uv                         | GitHub recorded digest of the checksum file — good                                                                                                     | [python.rs:310](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/python.rs:310)                                                                                          |
| Go                                  | go.dev index sha256 over HTTPS; no signed sums exist upstream (documented)                                                                             | [go.rs:150](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/go.rs:150)                                                                                                  |
| Zig                                 | ziglang.org index `shasum` over HTTPS                                                                                                                  | [zig.rs:51](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/zig.rs:51)                                                                                                  |

### A.4.2 Gaps

1. Cargo's `SHA256SUMS` is unverified — highest impact, since fork cargo is joined into every Rust
   toolchain. Use the pnpm attestation pattern or at least python's GitHub asset-digest check.
2. Vite+'s `vp-checksums.txt` is unverified — the vp binary and the entire JS dependency graph rest
   on that one file.
3. Bun's `SHASUMS256.txt` is unverified, now inconsistent with pnpm/yarn.
4. pnpm's attestation is not an independent cryptographic trust anchor (documented at
   [attestation.rs:13](/Users/gaurav/personal/projects/crucible/crates/upstream/src/attestation.rs:13));
   any workflow in `pnpm/pnpm` with a matching tag/predicate suffices.
5. npm key selection uses `find_map` over pinned keys — first _pinned_ key with any signature wins,
   not the first valid one ([npm.rs:67](/Users/gaurav/personal/projects/crucible/crates/upstream/src/npm.rs:67)).
   Timestamps are compared as strings ([npm.rs:84](/Users/gaurav/personal/projects/crucible/crates/upstream/src/npm.rs:84)).
6. No verification failure is logged-and-ignored: all verification functions return `Result` and
   call sites propagate. The only deliberate non-fatal cases are nightly `Missing`/`Incomplete`
   and Zig lines without all-system builds — both documented.
7. Writes are not transactional across files (see A.1.3).
8. **Systemic:** Nix checks prove pins _build_, not that their provenance was verified. A
   compromised update workflow can write attacker hashes that pass all checks.

### A.4.3 Simplification opportunities

- Add `Http::get_json<T>()` and `Http::get_text()`; the same
  `from_slice(...).context("failed to parse …")` appears in 8 modules, and checksum-file fetching
  in 4.
- Add `upstream::npm::checked_integrity(...)` and a shared `Dist` to remove the duplicated
  sha512-prefix → verify → return-integrity dance.
- Centralise `REGISTRY`, GitHub API bases, `Source`, `PLATFORMS` and `version_key`.
- Use `pub(crate) use upstream::*` in `pins` so new verification modules need only one edit.
- Replace repeated "newest stable of major" implementations with one helper and consistent
  max-version selection.
- Consider a `--check` dry run that re-fetches and re-verifies provenance for CI.

### A.4.4 `pin` command coverage

`pin` exists for rust, node, go, python only
([main.rs:84](/Users/gaurav/personal/projects/crucible/crates/crucible/src/main.rs:84)).
Cargo, Deno, Zig, tsrs, Vite+ and package managers accept only pinned releases, with error
messages pointing at the maintainer-only `crucible update`. This is a usability gap, not a false
README claim. Either add project-level pin paths with a hashes escape hatch, or make the error
explicitly say "not supported; wait for the next update" instead of naming a maintainer command.

---

# Part B — templates and toolchain support

## B.1 Headline

**Nine template attrs call `crucible.lib`:** `default`, `rust`, `node`, `bun`, `python`, `uv`,
`go`, `zig` and `deno`. The sibling rewrite is done: those dev shells take their toolchains from
the pinned releases (4.1), and `default` is the only one with a package output — all templates are
dev-shell-only otherwise. The other attrs are nixpkgs-only by the policy in
`docs/template-policy.md`; their vestigial `crucible` input has been removed (4.7, with
`jupyter`/`elixir` cleaned up by their owner).

## B.2 Full template map

| Template                     | Files / target                                       | `crucible` input | Uses `crucible.lib` | Classification & need                                                                                                                                              |
| ---------------------------- | ---------------------------------------------------- | ---------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `default`                    | 7 files, Rust + `rust-toolchain.toml`                | Yes              | **Yes**             | Direct: `rust.fromToolchainFile` + `rustPlatform.buildRustPackage`; the only package output                                                                        |
| `rust`                       | `.envrc`, `flake.nix`                                | Yes              | **Yes**             | Direct: `rust.toolchain`; fenix is gone                                                                                                                            |
| `node`                       | Node via nixpkgs `nodejs`, `pnpm`                    | Yes              | **Yes**             | Direct: `node.toolchain` with pinned pnpm                                                                                                                          |
| `bun`                        | nixpkgs `bun`                                        | Yes              | **Yes**             | Direct: `node.toolchain { packageManager = "bun@1"; }` using the pinned bun                                                                                        |
| `python`                     | hardcoded `python313`                                | Yes              | **Yes**             | Direct: `python.toolchain`                                                                                                                                         |
| `uv`                         | hardcoded `python313`, `uv`                          | Yes              | **Yes**             | Direct: `python.toolchain` (carries uv) and `node.toolchain`                                                                                                       |
| `go`                         | overlay `go_1_24`                                    | Yes              | **Yes**             | Direct: `go.toolchain`                                                                                                                                             |
| `zig`                        | nixpkgs `zig`, `zls`                                 | Yes              | **Yes**             | Direct: `zig.toolchain` (the comment points at `fromBuildZigZon`)                                                                                                  |
| `deno`                       | nixpkgs `deno`                                       | Yes              | **Yes**             | Direct: `deno.toolchain {}`                                                                                                                                        |
| `tsrs`                       | no template                                          | —                | —                   | Toolchain exists; a trivial template could be added                                                                                                                |
| `jupyter`                    | Python 3.11 + Poetry + Jupyter, has `pyproject.toml` | No               | No                  | **Partial:** runtime covered, but Poetry's `[tool.poetry.dependencies].python = "^3.11"` isn't read by `fromPyprojectToml`; Poetry/ipykernel un-pinned (app-level) |
| `elm`                        | nixpkgs `elmPackages.elm`, `elm2nix`                 | No               | No                  | **Partial → feasible:** reuse npm-signature pin pattern; needs `data/elm`, `lib/elm`, `pins update elm`, `elm.json` detection, doctor/HM/checks                    |
| `swift`                      | nixpkgs `swift`, `sourcekit-lsp`                     | No               | No                  | Nixpkgs-only today; technically feasible (SDK integration is large)                                                                                                |
| `elixir`                     | nixpkgs BEAM: `erlang_27`, `elixir_1_17`             | No               | No                  | Nixpkgs-only; needs two-axis OTP × Elixir pin model, `mix.exs` detection                                                                                           |
| `hashi` (`hash` attr)        | 10 HashiCorp tools; `allowUnfree`                    | No               | No                  | Nixpkgs-only; ten independent APIs, no project version file                                                                                                        |
| `c-cpp` (`c`, `cpp` aliases) | clang-tools, cmake, conan, gtest, lcov, vcpkg, …     | No               | No                  | Nixpkgs-only; no version file, many independent ecosystems. Only existing link is `doctor::check_cc`                                                               |
| `java`, `kotlin`, `scala`    | JDK 25 + Maven/Gradle/sbt                            | No               | No                  | Nixpkgs-only; vendor JDK licensing, build-time registry resolution conflicts with offline eval                                                                     |
| `laravel`, `php`             | PHP + Composer                                       | No               | No                  | Nixpkgs-only; no per-system upstream toolchain archive, Packagist resolution                                                                                       |
| `nim`, `odin`                | nixpkgs source-bootstrap compilers                   | No               | No                  | Nixpkgs-only; no prebuilt release-archive index                                                                                                                    |
| `nix`                        | nixd, cachix, statix, …                              | No               | No                  | Nixpkgs-only by design; self-referential to pin Nix tooling                                                                                                        |
| `shell`                      | shellcheck only                                      | No               | No                  | N/A — no toolchain version to pin                                                                                                                                  |
| `empty`                      | empty shell                                          | No               | No                  | N/A — `DEFAULT_TEMPLATE` in `crates/crucible/src/templates.rs`                                                                                                     |

## B.3 Classification summary

- **Direct, and now shipped as supported templates:** `default`, `rust`, `node`, `bun`, `python`,
  `uv`, `go`, `zig`, `deno`. A `tsrs` template would be trivial but does not exist yet.
- **Partial:** `jupyter` (project-file reading), `elm` (feasible new pin pipeline).
- **Feasible but large:** `swift`, `elixir`, `hashi`.
- **Nixpkgs-only, with reasons (the policy in `docs/template-policy.md`):** `c-cpp` (and its `c`
  / `cpp` aliases), `java`, `kotlin`, `scala`, `laravel`, `php`, `nim`, `odin`, `nix`, `shell`,
  `empty`, `hash`, plus the four partial/feasible ones above.

## B.4 `init` detection gaps vs templates

`init` detects only rust toolchain files, `package.json`, `deno.json(c)`, `pyproject.toml` /
`.python-version`, `go.mod`, `build.zig.zon`. None of the 16 gap templates' project files are
detected: no `CMakeLists.txt`, `mix.exs`, `elm.json`, `*.tf`, `pom.xml`, `build.gradle[.kts]`,
`.java-version`, `composer.json`, `.nimble`, `Package.swift`, `build.sbt`, `*.nix` or shell files.
`Cargo.toml` alone is deliberately not detected — only rustup files are, matching `default`.
Promoting any gap template therefore requires the full stack from A.3.3 including an `init` detector.

## B.5 Template checks after Wave 1

- `tests/init.nix` derives the list from `templates.nix`, compares it with the template
  directories (including `hash` → `hashi` and `c`/`cpp` → `c-cpp`) and evaluates every attr's
  outputs for every supported system with stub inputs. It forces the dev shells only for templates
  that actually call `crucible.lib`; nixpkgs-only ones are checked for output structure.
- `tests/default.nix` evaluates the Node template's dev shell with a stub toolchain that asserts a
  pinned `pnpm@<major>` package manager: a comment mentioning pnpm cannot satisfy it, and removing
  the package manager fails evaluation.
- `crates/init/src/lib.rs` `include_str!`s six template flakes and `crates/crucible/package.nix`
  lists them in the CLI source filter — template edits rebuild the CLI and can break `cargo test`.
  `crates/init/tests/sync.rs` also checks the template attrs against the README list and the CLI's
  accepted-name list.

---

# Part C — the plan

Phases are ordered by dependency. Phase 0 assumes the sibling template work lands first; no phase
before Phase 4 touches `templates/`.

## Phase 0 — land and harden the in-flight verification work (do now)

**Goal:** make the staged security work trustworthy and fix bugs already identified.

| #   | Task                                                                                                                                                                                          | Files                                                                                                                                                                                                                                                                                                                 | Acceptance                                                                  |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| 0.1 | Add negative + success tests for `attestation::verify`: real-shaped response, wrong tag/repo/asset/digest, no attestations, 404; wrap `http.get()` with `crucible:` context                   | [attestation.rs:80](/Users/gaurav/personal/projects/crucible/crates/upstream/src/attestation.rs:80)                                                                                                                                                                                                                   | All branches covered; 404 yields a helpful message                          |
| 0.2 | Add signed-success tests through `integrity()`/`source()` for yarn and tsrs; extract `npm::checked_integrity` + shared `Dist`; prefer a valid key; parse timestamps instead of string compare | [package_managers.rs:333](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/package_managers.rs:333), [tsrs.rs:129](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/tsrs.rs:129), [npm.rs:67](/Users/gaurav/personal/projects/crucible/crates/upstream/src/npm.rs:67)                   | Success and rejection paths both tested                                     |
| 0.3 | Remove `x86_64-darwin` from the dev flake's systems                                                                                                                                           | [dev/flake.nix:39](/Users/gaurav/personal/projects/crucible/dev/flake.nix:39)                                                                                                                                                                                                                                         | `nix flake check` in `dev/` evaluates on Intel Macs                         |
| 0.4 | Replace the vacuous Node-template grep with a structural check or evaluation                                                                                                                  | [tests/default.nix:34](/Users/gaurav/personal/projects/crucible/tests/default.nix:34)                                                                                                                                                                                                                                 | Check fails if `pnpm` is removed from the dev shell                         |
| 0.5 | Fix `cruisel` typo and non-conforming cargo throw                                                                                                                                             | [zig/toolchain.nix:23](/Users/gaurav/personal/projects/crucible/lib/zig/toolchain.nix:23), [cargo.nix:16](/Users/gaurav/personal/projects/crucible/lib/rust/cargo.nix:16)                                                                                                                                             | All user-facing errors start with `crucible:` and say what to do            |
| 0.6 | Remove library panics; map fetch-thread panic to `NightlyOutcome::Failed`                                                                                                                     | [http.rs:97](/Users/gaurav/personal/projects/crucible/crates/upstream/src/http.rs:97), [rust.rs:157](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/rust.rs:157)                                                                                                                                     | No `panic!`/`expect` in library code outside tests                          |
| 0.7 | Reconcile docs with the landed design                                                                                                                                                         | [ARCHITECTURE.md](/Users/gaurav/personal/projects/crucible/ARCHITECTURE.md:469), [README.md](/Users/gaurav/personal/projects/crucible/README.md:444), [flake.nix:2](/Users/gaurav/personal/projects/crucible/flake.nix:2), [docs/node-research.md](/Users/gaurav/personal/projects/crucible/docs/node-research.md:95) | No contradictions about templates, `.deno`, network-at-eval, or trust model |

**Verify:** `cargo test --workspace`; `cargo clippy --workspace --all-targets`;
`cd dev && nix fmt -- --fail-on-change`; `nix flake check -L --keep-going`.

## Phase 1 — make the documented invariants real (next; depends on Phase 0)

**Goal:** convert fragile "mirrors" into enforced invariants.

| #   | Task                                                                                                                                              | Acceptance                                         |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| 1.1 | Add `checks.<system>.sync` (or a Rust test reading Nix sources) asserting all eight AGENTS.md pairs plus new ones from A.3.2                      | Changing any pair on one side fails a check        |
| 1.2 | Extend AGENTS.md with the new pairs (layout field, npm keys, attestation API, extra platform tables, template name lists, dev-flake systems)      | Documented and checked                             |
| 1.3 | Add cross-system assert to Deno and tsrs via a `lib/common` helper                                                                                | `pkgsCross` misuse fails with `crucible:` hint     |
| 1.4 | Make `tests/init.nix`'s `templates` check evaluate every flake attr (incl. `hash`→`hashi`, `c`/`cpp`) and compare attr list to the directory list | All 27 attrs evaluate for all 3 systems            |
| 1.5 | Add `programs.crucible.tsrs` to the Home Manager check                                                                                            | HM module check covers all six options             |
| 1.6 | Transaction boundary for `update`: stage all temp files, then rename; restore on failure; warn on dropped checksum lines                          | An interrupted update cannot leave a mixed pin set |
| 1.7 | Standardise the `crucible:` prefix at the CLI top level                                                                                           | All CLI errors carry the prefix                    |

**Verify:** `nix flake check --all-systems --no-build`; `nix flake check -L --keep-going`;
`cd dev && nix flake check`; `actionlint`; `shellcheck scripts/linux-from-macos.sh`.

## Phase 2 — structural cleanup (next; can run alongside Phase 1 once its sync check exists)

**Goal:** remove duplication without breaking the intentional design.

| #   | Task                                                                                                             | Files                                                                                                                                                                               |
| --- | ---------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2.1 | Canonical `upstream::platforms` table; delete 13 copies                                                          | `upstream`, all `pins/update/*`, `init/lib.rs`                                                                                                                                      |
| 2.2 | `pins/src/source.rs` holding `Source`, hash lookup, `release_url`, `checked_asset`; move `pin go` into `pin.rs`  | `pins/src/{lib,pin}.rs`, `pins/src/update/*`                                                                                                                                        |
| 2.3 | `pub(crate)` re-exports; reduce `pins/update/*` and `upstream` model `pub`s; drop `clap` from `pins`             | [pins/lib.rs:4](/Users/gaurav/personal/projects/crucible/crates/pins/src/lib.rs:4), [pins/Cargo.toml](/Users/gaurav/personal/projects/crucible/crates/pins/Cargo.toml:1)            |
| 2.4 | Move Node checks to `doctor/src/node.rs`; thread `Env` through all checks                                        | `doctor/src/*`                                                                                                                                                                      |
| 2.5 | `lib/common` helpers: `crucible:` throw, `resolveRelease`/`notPinned`, join+setup-hook, cross-system assert      | `lib/common/`                                                                                                                                                                       |
| 2.6 | Unify ready-made toolchain placement across `default.nix`/`toolchain.nix`                                        | `lib/<lang>/`                                                                                                                                                                       |
| 2.7 | Decide the `--set-default` wrapper env question; keep documented or switch vp to `--set`/unset                   | [uv.nix:80](/Users/gaurav/personal/projects/crucible/lib/python/uv.nix:80), [vite-plus.nix:151](/Users/gaurav/personal/projects/crucible/lib/node/vite-plus.nix:151)                |
| 2.8 | Shared corpora for semver/PEP440 copies; tests for `vite_plus.rs`, `pin.rs` output, `doctor::run`, clap dispatch | `tests/`, `crates/`                                                                                                                                                                 |
| 2.9 | CLI fixes: `--version` help, `init` new-project semantics, template-repo naming; remove duplicate dev-dep        | [main.rs:43](/Users/gaurav/personal/projects/crucible/crates/crucible/src/main.rs:43), [upstream/Cargo.toml](/Users/gaurav/personal/projects/crucible/crates/upstream/Cargo.toml:1) |

**Acceptance:** no duplicate platform/`Source`/`version_key` definitions; the public surface is
reduced to what callers use; both parser copies are covered by one corpus.
**Verify:** same as Phase 1 plus `cargo build -p crucible --no-default-features` (user build must
stay network-free).

## Phase 3 — close verification and pin-path gaps (later; depends on Phases 1–2)

| #   | Task                                                                                                                        | Acceptance                                                                      |
| --- | --------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| 3.1 | Verify cargo's `SHA256SUMS` and Vite+'s `vp-checksums.txt` against GitHub asset digests/attestations                        | No unverified checksum file remains except documented HTTPS-only upstreams      |
| 3.2 | Extend attestation to bun/deno; consider local Sigstore bundle verification                                                 | pnpm's check becomes an independent trust anchor, or the tradeoff is documented |
| 3.3 | Add project-level `pin` paths (or explicit "unsupported" errors) for cargo, deno, zig, tsrs, vite-plus and package managers | `pin` coverage matches documented resolution rules                              |
| 3.4 | Add `crucible update --check` dry run re-verifying provenance; wire into CI                                                 | A compromised updater cannot push unchecked hashes past `checks`                |
| 3.5 | Consider uniform data shape and `pin --json`                                                                                | Either adopted consistently or exceptions documented                            |

**Verify:** full AGENTS.md block plus `scripts/linux-from-macos.sh`; `nix-templates/` is now in
its rsync exclusions, and x86_64-linux remains CI-only.

## Phase 4 — template and toolchain coverage (later; depends on Phases 0–1; sibling set first)

| #   | Task                                                                                                                                                                     | Acceptance                                                                                        |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------- |
| 4.1 | Finish the sibling set (default/rust, node, bun, python, uv, go, zig, deno)                                                                                              | Each dev shell evaluates and uses `crucible.lib`                                                  |
| 4.2 | Fix `jupyter`'s Poetry `pyproject.toml` reading in `fromPyprojectToml` (or document fallback)                                                                            | Jupyter's Python version resolves to 3.11 as the file states                                      |
| 4.3 | Promote `elm` as the first new language, following the full A.3.3 checklist                                                                                              | `lib/elm`, `data/elm`, `pins update elm`, `tests/elm`, doctor, HM, init detection, docs, template |
| 4.4 | Then Swift, Elixir, HashiCorp in that feasibility order                                                                                                                  | Same full-stack acceptance per language                                                           |
| 4.5 | **Landed:** `docs/template-policy.md`, README and ARCHITECTURE list the nixpkgs-only templates with reasons and define "supported template" as the intersection above    | "Supported template" is an explicit, listed intersection                                          |
| 4.6 | **Landed:** the CLI uses `github:kiyors/crucible#<name>` and validates names against an explicit list from `templates.nix`; the leftover is the two `main.rs` help lines | One canonical source; helper validation                                                           |
| 4.7 | **Landed:** the nixpkgs-only templates declare no `crucible` input and point at the policy doc instead (`jupyter`/`elixir` handled by their owner)                       | No misleading unused inputs                                                                       |

**Verify:** full AGENTS.md block; `nix flake check --all-systems --no-build`;
Linux-from-macOS for aarch64-linux; CI for x86_64-linux.

---

# Part D — risks, tradeoffs and open questions

## Risks

1. **Concurrency:** templates are being edited in this checkout. Phases 0–3 deliberately avoid
   `templates/`. Re-read the sibling diff before Phase 4. Note the `include_str!`/source-filter
   coupling: template edits rebuild the CLI and can break `cargo test`.
2. **Trust model:** pnpm's "attestation" check is not a locally verified signature. Calling it
   verification without qualification overstates it. Node and npm key rotations are manual and
   fatal until updated — good for safety, bad for availability; there is no alerting.
3. **GitHub API limits:** unauthenticated attestation lookups (10+ per pnpm update) hit 60 req/hr;
   403 is non-retryable. The error should hint at `GITHUB_TOKEN`.
4. **Cost of new languages:** each adds a 3-system pin matrix, real CI builds, doc/sync pairs and
   roughly eight files. Zig `master` and x86_64-darwin are precedents for deliberately _not_
   supporting something.
5. **Text-based sync checks are fragile.** The alternative — generated constants — conflicts with
   "nixpkgs is the only flake input" and offline evaluation. Accept the text check but keep it
   narrow and well-commented.
6. **Offline eval invariant:** all provenance trust is concentrated in the scheduled CLI run. Any
   change that lets Nix fetch at eval time for pinned releases erodes the core design.

## Tradeoffs

- **Keep the deliberate parser copies** (semver, PEP 440) rather than forcing a shared crate; add
  a shared test corpus instead. The Rust and Nix sides have different consumers and failure modes.
- **Keep Go/Zig/Rust-manifest HTTPS-only trust** — upstream publishes no signed index; document it
  rather than inventing a fake verification layer.
- **`--set-default` wrappers** are convenient for standalone `uv`/`vp`; changing them to `--set`
  may surprise users who intentionally override. Decide explicitly rather than drifting.
- **nixpkgs-only templates** should be a documented policy, not an embarrassment; forcing a pin
  pipeline for `shellcheck` or `nixd` would add no value.

## Open questions

1. ~~Should `.deno` passthru be added?~~ Resolved: the passthru now exists in `lib/deno/deno.nix`.
2. Is `--set-default` acceptable for standalone `uv`/`vp`, or must the "vp never manages Node.js"
   invariant hold outside dev shells too?
3. ~~Is `github:kiyors/templates` canonical?~~ Resolved by 4.6: this flake owns the templates
   used by `crucible init <dir>` / `add`; no second flake input.
4. Should `pin` gain a machine-readable mode and new language paths, or should unpinned versions be
   explicitly unsupported with a clear message?
5. Should `x86_64-darwin` be dropped from the nixpkgs-only gap templates too, or left alone because
   they never use crucible's pins?
6. ~~Should the `crucible` input be removed from gap templates?~~ Resolved by 4.7: removed and
   replaced with a pointer to `docs/template-policy.md`.

---

## Appendix A — verification command reference

From [AGENTS.md](/Users/gaurav/personal/projects/crucible/AGENTS.md:6), run before each commit:

```sh
nix flake check -L --keep-going           # every check for this machine's system
nix flake check --all-systems --no-build  # everything evaluates on all three systems
cd dev && nix fmt -- --fail-on-change     # formatting, deadnix, statix, typos
cd dev && nix flake check                 # git hooks (treefmt, gitleaks)
nix shell .#rust.stock -c cargo test --workspace
nix shell .#rust.stock -c cargo clippy --workspace --all-targets
nix shell .#rust.stock -c cargo build -p crucible --no-default-features
actionlint
shellcheck scripts/linux-from-macos.sh
```

- macOS sandbox builds print a harmless `xcrun` warning.
- Use `.#rust.stock` (stock cargo); the fork shares one build directory between worktrees.
- CI covers x86_64-linux, aarch64-linux and macOS; locally cover at least one system plus the
  `--all-systems --no-build` evaluation. `scripts/linux-from-macos.sh` covers aarch64-linux from
  macOS and now excludes `nix-templates/` along with the other reference clones.
- New files must be `git add`ed before Nix can see them. This document and
  `docs/template-policy.md` are Markdown files in `docs/` and are not referenced by the flake, so
  no flake change is needed for them.

**Statement of what was actually built during this review:** the review itself was static and
built no system; the `dev#x86_64-darwin` failure was confirmed by a read-only `nix eval`, not a
build. The 2026-11-25 corrections report separately on the commands run to verify the landed
template work.

## Appendix B — file index of findings

| Area                    | Key files                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Layering / re-exports   | [pins/lib.rs](/Users/gaurav/personal/projects/crucible/crates/pins/src/lib.rs:4), [pin.rs](/Users/gaurav/personal/projects/crucible/crates/pins/src/pin.rs:14), [update/mod.rs](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/mod.rs:28)                                                                                                                                                                                                                                                                                                                                                                                  |
| New verification code   | [attestation.rs](/Users/gaurav/personal/projects/crucible/crates/upstream/src/attestation.rs:1), [npm.rs](/Users/gaurav/personal/projects/crucible/crates/upstream/src/npm.rs:1), [package_managers.rs](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/package_managers.rs:1), [tsrs.rs](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/tsrs.rs:1)                                                                                                                                                                                                                                                        |
| Error handling / panics | [main.rs:113](/Users/gaurav/personal/projects/crucible/crates/crucible/src/main.rs:113), [http.rs:97](/Users/gaurav/personal/projects/crucible/crates/upstream/src/http.rs:97), [rust.rs:157](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/rust.rs:157), [update/mod.rs:98](/Users/gaurav/personal/projects/crucible/crates/pins/src/update/mod.rs:98)                                                                                                                                                                                                                                                                   |
| Nix toolchain APIs      | [rust/toolchain.nix](/Users/gaurav/personal/projects/crucible/lib/rust/toolchain.nix:1), [node/toolchain.nix](/Users/gaurav/personal/projects/crucible/lib/node/toolchain.nix:1), [python/toolchain.nix](/Users/gaurav/personal/projects/crucible/lib/python/toolchain.nix:1), [go/toolchain.nix](/Users/gaurav/personal/projects/crucible/lib/go/toolchain.nix:1), [zig/toolchain.nix](/Users/gaurav/personal/projects/crucible/lib/zig/toolchain.nix:1), [deno/toolchain.nix](/Users/gaurav/personal/projects/crucible/lib/deno/toolchain.nix:1), [tsrs/toolchain.nix](/Users/gaurav/personal/projects/crucible/lib/tsrs/toolchain.nix:1) |
| Sync definition         | [AGENTS.md:54](/Users/gaurav/personal/projects/crucible/AGENTS.md:54), [systems.nix](/Users/gaurav/personal/projects/crucible/lib/common/systems.nix:1), [tests/init.nix:72](/Users/gaurav/personal/projects/crucible/tests/init.nix:72)                                                                                                                                                                                                                                                                                                                                                                                                    |
| Checks                  | [tests/default.nix](/Users/gaurav/personal/projects/crucible/tests/default.nix:1), [tests/init.nix:130](/Users/gaurav/personal/projects/crucible/tests/init.nix:130)                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Templates               | [flake.nix:100](/Users/gaurav/personal/projects/crucible/flake.nix:100), [templates.rs](/Users/gaurav/personal/projects/crucible/crates/crucible/src/templates.rs:1), [init/lib.rs:91](/Users/gaurav/personal/projects/crucible/crates/init/src/lib.rs:91)                                                                                                                                                                                                                                                                                                                                                                                  |
| Home Manager            | [home-manager.nix](/Users/gaurav/personal/projects/crucible/modules/home-manager.nix:19)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Dev flake / workflows   | [dev/flake.nix:39](/Users/gaurav/personal/projects/crucible/dev/flake.nix:39), [check.yaml](/Users/gaurav/personal/projects/crucible/.github/workflows/check.yaml:1), [update.yaml](/Users/gaurav/personal/projects/crucible/.github/workflows/update.yaml:1)                                                                                                                                                                                                                                                                                                                                                                               |
