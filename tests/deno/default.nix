# Runs the deno executable from the pinned release, so a zip that stops
# installing fails a check rather than a downstream shell, and checks the
# version resolution.
{ pkgs, crucible }:
let
  inherit (crucible.deno) toolchain;
  inherit (pkgs) lib;
  pins = lib.importJSON ../../data/deno/deno.json;
  majors = lib.attrNames pins;
  newestMajor = lib.last (lib.sort lib.versionOlder majors);
  newest = toolchain { };
  rejected = toolchain: !(builtins.tryEval toolchain.drvPath).success;

  # One check per pinned major: the toolchain's deno prints the pinned
  # version.
  denoScript =
    lib.concatStringsSep "\n" (
      map (
        major:
        let
          inherit ((lib.getAttr major pins)) version;
        in
        "${toolchain { version = major; }}/bin/deno --version | grep -q \"^deno ${version} \""
      ) majors
    )
    + "\ntouch $out";
in
{
  deno = pkgs.runCommand "crucible-check-deno" {
    nativeBuildInputs = map (major: toolchain { version = major; }) majors;
  } denoScript;

  # "latest" is the newest pinned release, and an exact pinned release
  # resolves as itself.
  deno-resolve =
    assert newest.version == (lib.getAttr newestMajor pins).version;
    pkgs.runCommand "crucible-check-deno-resolve" { } ''
      test "${
        (toolchain { version = newestMajor; }).version
      }" = "${(lib.getAttr newestMajor pins).version}"
      touch $out
    '';

  # Nothing but the pins: an unpinned major or release is an error rather
  # than a silent substitution.
  deno-errors =
    assert
      !rejected (toolchain {
        version = "${newestMajor}";
      });
    assert
      !rejected (toolchain {
        version = "${newest.version}";
      });
    assert rejected (toolchain {
      version = "1";
    });
    pkgs.runCommand "crucible-check-deno-errors" { } "touch $out";
}
