# Template policy

Templates are starter projects exposed as `templates.<name>` by this flake's
[`templates.nix`](../templates.nix), with the files under [`templates/`](../templates).
This document says which templates crucible supports with pinned toolchains, why
the rest deliberately use nixpkgs, and where the names come from. The README has
the user-facing list of `nix flake init -t` commands.

## One canonical source

The attrs in `templates.nix` are the names, and the directories they point at are
the content. `hash` is the flake attr for the `hashi` directory; `c` and `cpp`
alias `c-cpp` and have no directories of their own. There is no second template
repo: `crucible init <directory> [template]` and `crucible add [template]` shell
out to `nix flake new`/`nix flake init --template` with
`github:kiyors/crucible#<name>`, and `crates/crucible/src/templates.rs` keeps an
explicit accepted-name list derived from the same attrs. A separate external
flake as the CLI's template source would add a second flake input and need an
offline check of its names, against both the one-input rule and offline
evaluation. The accepted names are validated before `nix` runs, so a typo gets a
`crucible:` error listing the choices instead of a flake-not-found error.

`tests/init.nix` compares every `templates.nix` attr with the template
directories (including the aliases) and evaluates each flake's outputs with stub
inputs. `crates/init/tests/sync.rs` compares the attrs with the README list. Add
a name to all of those places together.

## What "supported template" means

A **supported template** is the intersection of:

1. an attr in `templates.nix`,
2. a toolchain whose releases crucible pins in `data/` and exposes through
   `lib.<system>.<lang>`, and
3. a template whose dev shell actually calls that `crucible.lib` function with
   no project-supplied archive hashes.

The intersection is exactly these ten attrs:

| Attr      | Toolchain                               | Pin                   |
| --------- | --------------------------------------- | --------------------- |
| `default` | `rust.fromToolchainFile`                | `rust-toolchain.toml` |
| `rust`    | `rust.toolchain`                        | pinned stable         |
| `node`    | `node.toolchain` with pnpm              | pinned LTS + pnpm     |
| `bun`     | `node.toolchain` with bun               | pinned bun            |
| `python`  | `python.toolchain`                      | pinned CPython + uv   |
| `uv`      | `python.toolchain` and `node.toolchain` | pinned Python + Node  |
| `go`      | `go.toolchain`                          | pinned Go line        |
| `zig`     | `zig.toolchain`                         | pinned Zig line       |
| `deno`    | `deno.toolchain`                        | pinned Deno 2         |
| `elm`     | `elm.fromElmJson`                       | pinned Elm compiler   |

These are the only templates whose dev shells `tests/init.nix` forces to
evaluate with a real `crucible.lib`. `default` is also the only template with a
package output (`buildRustPackage`); every other template, supported or not, is
dev-shell-only. There is no tsrs template yet, although the toolchain exists.

Everything else in `templates.nix` is **nixpkgs-only**: it installs packages
straight from nixpkgs and is not a claim that crucible pins, detects or verifies
that language. A nixpkgs-only template should declare only the nixpkgs input.

## Why the other templates stay nixpkgs-only

| Attr(s)                   | What it installs                             | Why there is no pin pipeline                                                                                                                                                                                                                                                           |
| ------------------------- | -------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `c-cpp`, `c`, `cpp`       | clang, CMake, Conan, vcpkg, linters, gdb     | No version file and many independent ecosystems; only `doctor`'s C/C++ check ties in.                                                                                                                                                                                                  |
| `java`, `kotlin`, `scala` | JDK 25 with Maven, Gradle, sbt               | Vendor JDK licensing; build-time registry resolution conflicts with offline evaluation.                                                                                                                                                                                                |
| `laravel`, `php`          | PHP, Composer, Laravel                       | No per-system upstream toolchain archive; Composer/Packagist resolves at build time.                                                                                                                                                                                                   |
| `nim`, `odin`             | Nim/Nimble, Odin                             | nixpkgs source-bootstraps them; there is no prebuilt release-archive index for all three systems.                                                                                                                                                                                      |
| `nix`                     | nixd, nixfmt, statix, cachix, vulnix         | Nix tooling is self-referential: pinning it would mean pinning Nix with itself.                                                                                                                                                                                                        |
| `shell`                   | shellcheck                                   | No toolchain version to pin; the checker is the whole shell.                                                                                                                                                                                                                           |
| `empty`                   | an empty dev shell                           | Nothing to pin; it is the CLI's `DEFAULT_TEMPLATE` and a starting point to adapt.                                                                                                                                                                                                      |
| `hash`                    | ten HashiCorp tools, with `allowUnfree`      | No project version file to read, so a pin would install the newest release without a project-level guarantee. Seven share `releases.hashicorp.com`'s GPG-signed index, but damon has no release assets and tflint/terragrunt need bespoke GitHub handlers; four tools are unfree/BUSL. |
| `jupyter`                 | Python 3.11, Poetry, Jupyter, ipykernel      | App-level stack: Poetry's `[tool.poetry.dependencies].python = "^3.11"` is not read by `fromPyprojectToml`, and Poetry/ipykernel are not pinned.                                                                                                                                       |
| `swift`                   | Swift, SourceKit-LSP                         | Swift's `releases.json` has no toolchain checksum; the macOS artifact is an Apple Developer-ID `.pkg` that cannot be verified or unpacked on Linux CI, and the official artifacts need Xcode/Apple SDK setup.                                                                          |
| `elixir`                  | Erlang/OTP 27, Elixir 1.17, Hex, mix, rebar3 | The compiler zips are easy, but OTP ships only as source tarballs, so a two-axis OTP x Elixir pin would mean per-system source builds; `mix.exs` cannot name OTP (only `.tool-versions` can).                                                                                          |

These are a policy, not a backlog: forcing a pin pipeline for `shellcheck`,
`nixd` or a JDK would add no value. Promote one only when it has a real upstream
release index, a project version file to read and a complete cross-language pin
pipeline.

### Swift, Elixir and HashiCorp are explicitly unsupported

Phase 4.4 of the plan is **resolved by policy**: these three stay nixpkgs-only
rather than receiving invented pins. `crucible update swift|elixir|hashi`,
`crucible update --check swift|elixir|hashi` and
`crucible pin swift|elixir|hashi <version>` all fail with a `crucible:` error
that names the tool, gives the short reason in the table above and points back
to this document. There is deliberately no `data/swift`, `data/elixir` or
`data/hashi`, and `crates/init/tests/sync.rs` guards that their templates
declare no `crucible` input. `crucible doctor` prints an info line when it sees
`Package.swift`/`.swift-version`, `mix.exs`/a BEAM `.tool-versions`, or a `*.tf`
file; those lines never fail the run.

## Promoting a template

Adding a pinned language means the full stack, not just a template edit: a new
upstream metadata parser in `crates/upstream/` if needed, an update/pin path in
`crates/pins/src/update/` plus `data/<lang>/`, `lib/<lang>/` and flake/overlay
exports, `tests/<lang>/`, a `doctor` check and Home Manager option, an `init`
detector and golden fixture, the AGENTS.md sync pairs, README/ARCHITECTURE docs,
and finally the template. Then move the attr into the supported list above and
give it a row in the CLI's accepted-name list if it is a new name or alias.
