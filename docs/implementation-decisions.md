# Implementation decisions

This record settles the open questions and deferred items in
[`architecture-review-and-plan.md`](architecture-review-and-plan.md). It is a
decision log, not a status report: items marked **deferred** are not implemented
and are not claimed as shipped. The phase/task numbers refer to the plan.

## Phase 3.5 — data shape and `pin --json`

**Decision: keep the per-language shapes, document the exceptions, and defer
`pin --json`.**

`data/**/*.json` is written by `crucible update` and read by the Nix library.
Each reader expects its own schema: `lib/rust`, `lib/node`, `lib/python`,
`lib/go`, `lib/zig`, `lib/deno` and `lib/tsrs` pattern-match per-family fields
(version lists, per-system asset URLs, hashes, checksum metadata). A uniform
shape would require changing every reader in lockstep and flattening families
that genuinely differ — a Rust manifest's component matrix is not a Node
release's asset table — breaking offline Nix evaluation for no user-visible
gain. The per-language shapes are the documented format, not an oversight.

A machine-readable `pin --json` is deferred for the same reason: `pin` mixes
prose and Nix snippets on stdout today, and a JSON mode would need a stable
schema covering every `pin` command (rust, node, go, python, plus the
project-level paths from 3.3), a flag threaded through clap, and a guarantee
that the emitted URLs keep matching the Nix `releaseUrl`/`shasumsUrl` sync
pairs checked by `crates/init/tests/sync.rs`. With no consumer that needs it,
the prose output plus those text checks are the contract.

## Phase 3.2 — Sigstore tradeoff

**Decision: keep the GitHub-attestation lookup as a documented tradeoff; local
Sigstore bundle verification is deferred.**

The pnpm check fetches in-toto attestation metadata from the GitHub API and
matches in-toto type, repository, tag, subject name and digest. That is a
TLS-authenticated lookup, **not a locally verified signature**: any workflow in
`pnpm/pnpm` with a matching tag and predicate would satisfy it. Calling it
"signature verification" without qualification overstates the guarantee.
Bun's `SHASUMS256.txt` is now anchored to a GitHub asset digest, but that is a
digest anchor, not a Sigstore signature either.

Making the check an independent trust anchor would require the updater to
verify a Sigstore bundle locally: check the Fulcio certificate's identity
against the expected workflow and OIDC issuer, verify the Rekor inclusion
proof/transparency-log entry, and verify the signature against the subject
digest, using pinned trust roots. The current design keeps provenance inside
the scheduled `crucible update` run and stores only digests in `data/`; local
bundle verification is deliberately not partly implemented.

## Phase 2.7 — wrapper environment (`--set-default`)

**Decision: keep `--set-default`; the dev-shell setup hook is authoritative.**

[`uv.nix`](../lib/python/uv.nix) and
[`vite-plus.nix`](../lib/node/vite-plus.nix) use `--set-default` so standalone
`uv`/`vp` invocations still work; an inherited `UV_PYTHON` or
`VP_ENV_FORBIDDEN=0` can therefore win outside a dev shell. Switching to
`--set`/unset would close that hole but break users who intentionally override
those variables. Inside a dev shell the setup hook overwrites/unsets them, so
the "vp never manages Node.js" and uv version invariants hold where crucible
owns the environment. That is the supported path; the standalone override is a
documented tradeoff, not an outstanding bug.

## Phase 1.4 / 4.6 — one canonical template source

**Decision: this flake is the single source of template names; the CLI
validates names in Rust.**

`templates.nix` attrs are the names and the `templates/` directories are the
content; `crucible init <dir> [template]` and `crucible add` use
`github:kiyors/crucible#<name>`. The external `github:kiyors/templates` repo is
gone, so there is no second flake input. `crates/crucible/src/templates.rs`
carries an explicit accepted-name list and rejects a typo with a `crucible:`
error before `nix` runs. `tests/init.nix` compares every attr with the
directories (including `hash`→`hashi` and `c`/`cpp`) and evaluates its outputs;
`crates/init/tests/sync.rs` compares the attrs with the README and CLI lists.
The names are therefore checked offline. See
[`template-policy.md`](template-policy.md).

## Phase 4.3 — Elm (landed)

**Decision: Elm is promoted; the full-stack work has landed.**

`crucible update elm` pins the compiler from the npm packages the official
`elm` launcher installs. The trust anchor is npm's signature over the
packument: each `@elm_binaries/<platform>` tarball's SHA-512
`dist.integrity` is checked against that signature before its hash is written
to `data/elm/elm.json`. npm versions the compiler as `0.19.3-0`, while
`elm.json` and `elm --version` say `0.19.3`; the suffix is stripped from the
version and kept in the tarball URLs. `crucible pin elm 0.19.3` prints the
verified per-system hashes for an explicit `toolchain` call.

The compiler is exposed as `lib.<system>.elm.toolchain` and
`lib.<system>.elm.fromElmJson`, which reads `elm-version` from a project's
`elm.json`; the three supported systems are the ones npm ships binaries for.
The user-facing half is in place too: a `programs.crucible.elm` Home Manager
option, an Elm template backed by the pinned compiler with `elm2nix` from
nixpkgs, `init` detection and `doctor` checks for `elm.json`, and the
README/ARCHITECTURE/template-policy documentation. Elm has moved out of the
nixpkgs-only table in [`template-policy.md`](template-policy.md). The plan's
"partial / deferred" rows describe the state before this promotion.

## Phase 4.4 — Swift, Elixir and HashiCorp (resolved by policy)

**Decision: all three stay nixpkgs-only. None is pinned, and that is now an
explicit, testable user-facing policy rather than a silent gap.**

They stay in the nixpkgs-only table of
[`template-policy.md`](template-policy.md). The verified blockers are
structural, not just missing glue:

- **Swift** — `releases.json` has no toolchain checksum to anchor a pin; the
  macOS artifact is an Apple Developer-ID `.pkg` that cannot be verified or
  unpacked on Linux CI, and the official artifacts need Xcode/Apple SDK setup.
  There is no per-system release-archive plus version-file pair to pin.
- **Elixir** — the compiler zips are easy (one platform-independent zip per
  compiler × OTP-major, with GitHub digests), but OTP ships only as source
  tarballs. A two-axis OTP × Elixir pin therefore needs per-system source
  builds, and `mix.exs` names the compiler range but no OTP version; only
  `.tool-versions` names both.
- **HashiCorp** — no project version file to read, so a pin would install the
  newest release without a project-level guarantee. Seven tools share
  `releases.hashicorp.com`'s GPG-signed index, but damon has no release assets,
  tflint and terragrunt need bespoke GitHub handlers, and four tools are
  unfree/BUSL. A subset promotion (Terraform first) is conceivable.

Rather than invent pins, the CLI accepts these names and explains them:
`crucible update <lang>`, `crucible update --check <lang>` and
`crucible pin <lang> <version>` reach one shared `pin::unsupported` error that
names the tool, gives the short reason and points at `docs/template-policy.md`.
`doctor` adds info-only checks for `Package.swift`/`.swift-version`, `mix.exs`/
a BEAM `.tool-versions`, and `*.tf`; they change no exit code or existing
diagnostic. `crates/init/tests/sync.rs` guards both the messages and the absence
of a `crucible` input from those three templates. There are no fake data pins
to assert.

Each promotion would still need the exact A.3.3 checklist from the plan, in
full:

1. `crates/upstream/src/<format>.rs` if a new metadata format is needed.
2. `crates/pins/src/update/<lang>.rs`, a `Scope` variant, usually a `Pin`
   variant, and `data/<lang>/*.json`.
3. `lib/<lang>/{default,toolchain,<artifact>}.nix`; flake `lib`/`packages`/
   overlay exports.
4. `tests/<lang>/` plus import in `tests/default.nix`.
5. `crates/doctor/src/<lang>.rs` plus the call in `run()`.
6. A `programs.crucible.<lang>` option in `modules/home-manager.nix`.
7. An `init` `Project` field, `detect()` branch, shell rendering and golden
   fixture.
8. AGENTS.md sync entries, README output/usage docs, ARCHITECTURE.md, and a
   template.

On top of that shared stack, Swift needs the SDK integration, Elixir needs the
two-axis OTP × Elixir resolver plus `mix.exs` detection, and HashiCorp needs
ten API clients and version inputs. Promote a template only when it has a real
upstream release index and a project version file, as the policy doc says.
