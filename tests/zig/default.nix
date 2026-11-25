# Builds and runs a hello program with the default toolchain, checks that
# every pinned line resolves to its release, `fromBuildZigZon` on the zon
# fixtures, and the evaluation-time errors.
{ pkgs, crucible }:
let
  inherit (pkgs) lib;
  inherit (crucible) zig;

  pins = lib.importJSON ../../data/zig/zig.json;
  stable = pins.releases.${pins.stable};

  # build.zig.zon files written from the pins, so these checks follow the
  # update instead of breaking when it moves a pin. builtins.toFile needs no
  # build, so reading them back is not import-from-derivation.
  zon =
    name: minimum:
    builtins.toFile "build.zig.zon" ".{ .name = .${name}, .minimum_zig_version = \"${minimum}\" }\n";
  fromZon = name: minimum: zig.fromBuildZigZon { file = zon name minimum; };

  # A toolchain is rejected at evaluation time when building it throws.
  rejected = toolchain: !(builtins.tryEval toolchain.drvPath).success;

  # tryEval reports failure but not the thrown text, so assert the wording in
  # the source that throws it; the rejections below cover the error paths.
  # Collapse whitespace so a phrase can span nixfmt's line wrapping.
  says =
    phrase:
    let
      source = builtins.readFile ../../lib/zig/toolchain.nix;
      flat = lib.concatStringsSep " " (
        builtins.filter builtins.isString (builtins.split "[[:space:]]+" source)
      );
    in
    lib.strings.hasInfix phrase flat;

  # Build and run a hello program with a toolchain on PATH.
  zigRun =
    name: toolchain: script:
    pkgs.runCommand "crucible-check-zig-${name}" { nativeBuildInputs = [ toolchain ]; } ''
      # zig's global cache is ~/.cache/zig, which is read-only in a build.
      export ZIG_GLOBAL_CACHE_DIR=$TMPDIR/zig-cache
      mkdir -p $out
      cp ${./hello/main.zig} main.zig
      zig build-exe -femit-bin=$out/hello main.zig
      # std.debug.print writes to stderr.
      test "$($out/hello 2>&1)" = "hello from crucible"
      test "$(zig version)" = "${toolchain.version}"
      ${script}
    '';
in
{
  # The default toolchain is the newest pinned release.
  zig-hello =
    let
      toolchain = zig.toolchain { };
    in
    assert toolchain.version == stable.version;
    assert toolchain.zig.version == stable.version;
    zigRun "hello" toolchain "";

  # Every line that has a build on this system resolves to its pinned
  # release; a line without one is rejected (see zig-errors).
  zig-lines =
    let
      withBuild = lib.filter (line: builtins.hasAttr pkgs.system pins.releases.${line}.platforms) (
        lib.attrNames pins.releases
      );
      ok = lib.all (
        line: (zig.toolchain { version = line; }).version == pins.releases.${line}.version
      ) withBuild;
    in
    assert ok;
    pkgs.runCommand "crucible-check-zig-lines" { } "touch $out";

  # `fromBuildZigZon` reads .minimum_zig_version as the oldest release that
  # will do, met by the pinned release of its line.
  zig-from-build-zig-zon =
    let
      # Both fixtures name 0.17.1, which a newer 0.17 patch still meets.
      stringForm = zig.fromBuildZigZon { file = ./hello/build.zig.zon; };
      tupleForm = zig.fromBuildZigZon { file = ./tuple/build.zig.zon; };
    in
    assert stringForm.version == pins.releases."0.17".version;
    assert tupleForm.version == pins.releases."0.17".version;
    assert (fromZon "older" "${pins.stable}.0").version == stable.version;
    assert (fromZon "exact" stable.version).version == stable.version;
    assert (fromZon "oldline" "0.15.0").version == pins.releases."0.15".version;
    pkgs.runCommand "crucible-check-zig-from-build-zig-zon" { } "touch $out";

  # Mistakes that would otherwise fail later, or not at all.
  zig-errors =
    # Unpinned patches: the data pins the newest patch of each line.
    assert rejected (zig.toolchain { version = "0.15.1"; });
    # Master's line is not pinned, and a garbage spec is nothing.
    assert rejected (zig.toolchain { version = "0.18"; });
    assert rejected (zig.toolchain { version = "banana"; });
    # A minimum newer than its line's pin, or on a line that is not pinned.
    assert rejected (fromZon "newer" "${pins.stable}.999");
    assert rejected (fromZon "unpinned" "0.999.0");
    # A release without .minimum_zig_version: pass `version` instead.
    assert rejected (zig.fromBuildZigZon { file = ./no-minimum/build.zig.zon; });
    # A line without a build on this system is an error here.
    assert (rejected (zig.toolchain { version = "0.7"; })) == (pkgs.system == "aarch64-darwin");
    # The error tells the user the version must come from this flake's pins.
    assert says "Only versions in this flake's pinned data are supported";
    assert says "wait for the next scheduled crucible update";
    assert says "or, if you maintain the pins yourself, use";
    assert says "#crucible-full";
    assert says "crucible update zig";
    pkgs.runCommand "crucible-check-zig-errors" { } "touch $out";
}
