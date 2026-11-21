# Builds and runs a hello program with the default toolchain, checks that
# every pinned line resolves to its release, `fromBuildZigZon` on the zon
# fixtures, and the evaluation-time errors.
{ pkgs, crucible }:
let
  inherit (pkgs) lib;
  inherit (crucible) zig;

  pins = lib.importJSON ../../data/zig/zig.json;
  stable = pins.releases.${pins.stable};

  # A toolchain is rejected at evaluation time when building it throws.
  rejected = toolchain: !(builtins.tryEval toolchain.drvPath).success;

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

  # `fromBuildZigZon` reads .minimum_zig_version and resolves it to a pin.
  zig-from-build-zig-zon =
    let
      # The fixtures name the newest pin of the newest line; update them
      # when `crucible update zig` moves it.
      stringForm = zig.fromBuildZigZon { file = ./hello/build.zig.zon; };
      tupleForm = zig.fromBuildZigZon { file = ./tuple/build.zig.zon; };
    in
    assert stringForm.version == stable.version;
    assert tupleForm.version == stable.version;
    pkgs.runCommand "crucible-check-zig-from-build-zig-zon" { } "touch $out";

  # Mistakes that would otherwise fail later, or not at all.
  zig-errors =
    # Unpinned patches: the data pins the newest patch of each line.
    assert rejected (zig.toolchain { version = "0.15.1"; });
    # Master's line is not pinned, and a garbage spec is nothing.
    assert rejected (zig.toolchain { version = "0.18"; });
    assert rejected (zig.toolchain { version = "banana"; });
    # A release without .minimum_zig_version: pass `version` instead.
    assert rejected (zig.fromBuildZigZon { file = ./no-minimum/build.zig.zon; });
    # A line without a build on this system is an error here.
    assert (rejected (zig.toolchain { version = "0.7"; })) == (pkgs.system == "aarch64-darwin");
    pkgs.runCommand "crucible-check-zig-errors" { } "touch $out";
}
