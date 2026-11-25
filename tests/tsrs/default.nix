# Runs the pinned tsrs on a small project, so a package that stops installing
# (or loses the lib.*.d.ts files the executable reads from beside itself)
# fails a check rather than a downstream shell, and checks the version
# resolution.
{ pkgs, crucible }:
let
  inherit (crucible.tsrs) toolchain;
  inherit (pkgs) lib;
  pins = lib.importJSON ../../data/tsrs/tsrs.json;
  tsrs = toolchain { };
  rejected = toolchain: !(builtins.tryEval toolchain.drvPath).success;

  # tryEval reports failure but not the thrown text, so assert the wording in
  # the source that throws it; the rejection below covers the error path.
  # Collapse whitespace so a phrase can span nixfmt's line wrapping.
  says =
    phrase:
    let
      source = builtins.readFile ../../lib/tsrs/toolchain.nix;
      flat = lib.concatStringsSep " " (
        builtins.filter builtins.isString (builtins.split "[[:space:]]+" source)
      );
    in
    lib.strings.hasInfix phrase flat;
in
{
  # The project type-checks; a type error is reported with its code and fails
  # the run, through tsrs and through the Oxlint backend's tsrs-lint.
  tsrs =
    pkgs.runCommand "crucible-check-tsrs"
      {
        nativeBuildInputs = [ tsrs ];
      }
      ''
        export HOME=$TMPDIR
        tsrs --version | grep -qF "tsrs ${pins.version},"
        cp -R ${./app} app && chmod -R u+w app && cd app
        tsrs -p .
        tsrs -p . --fast
        echo 'export const wrong: number = "1";' > src/wrong.ts
        if tsrs -p . > out; then
          echo "tsrs accepted a type error" >&2
          exit 1
        fi
        cat out
        grep -q 'src/wrong.ts(1,14): error TS2322' out
        tsrs-lint --help 2>&1 | grep -q 'Usage: tsrs-lint headless'
        touch $out
      '';

  # "latest" and the pinned version give the pin; nothing else does.
  tsrs-resolve =
    assert tsrs.version == pins.version;
    assert (toolchain { inherit (pins) version; }).version == pins.version;
    assert rejected (toolchain {
      version = "7.0.0";
    });
    # The error tells the user the version must come from this flake's pin.
    assert says "Only versions in this flake's pinned data are supported";
    assert says "wait for the next scheduled crucible update";
    assert says "or, if you maintain the pins yourself, use";
    assert says "#crucible-full";
    assert says "crucible update tsrs";
    pkgs.runCommand "crucible-check-tsrs-resolve" { } "touch $out";
}
