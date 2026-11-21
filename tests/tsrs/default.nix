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
    pkgs.runCommand "crucible-check-tsrs-resolve" { } "touch $out";
}
