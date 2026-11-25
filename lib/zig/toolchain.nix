# The public API: an official Zig release from ziglang.org, pinned in
# data/zig/zig.json by `crucible update`.
{
  lib,
  pkgs,
}:
let
  inherit (builtins)
    attrValues
    compareVersions
    head
    match
    readFile
    ;

  zigFor = pkgs.callPackage ./zig.nix { };
  pinned = lib.importJSON ../../data/zig/zig.json;
  pinnedLines = attrValues pinned.releases;
  pinnedVersions = map (r: r.version) pinnedLines;

  notPinned = spec: ''
    crucible: Zig "${spec}" is not a pinned release (${lib.concatStringsSep ", " pinnedVersions}).
    Pass a line ("0.15") or an exact pinned release ("0.15.2"); `crucible update zig`
    pins the newest patch of each minor line in ziglang.org's index.json.
  '';

  # The versions `toolchain` accepts: "stable" (the default: the line of the
  # newest pinned release), a pinned line ("0.15") or a pinned exact release
  # ("0.15.2").
  resolve =
    { version }:
    let
      spec = lib.trim version;
      # A line is "0.15": it resolves to the release pinned for that line,
      # which is the newest patch of it.
      line = match "v?([0-9]+\\.[0-9]+)$" spec;
      # An exact release is "0.15.2".
      exact = match "v?([0-9]+\\.[0-9]+\\.[0-9]+)" spec;
      byLine = if line != null then pinned.releases.${head line} or null else null;
      byVersion =
        if exact != null then lib.findFirst (r: r.version == head exact) null pinnedLines else null;
    in
    if spec == "stable" then
      pinned.releases.${pinned.stable}
    else if byLine != null then
      byLine
    else if byVersion != null then
      byVersion
    else
      throw (notPinned spec);

  # `.minimum_zig_version` from a project's build.zig.zon: the string form
  # Zig has written since 0.13 ("0.15.2", or with a leading v), or the tuple
  # form before it (.{ .major = 0, .minor = 15, .patch = 2 }).
  #
  # builtins.match is POSIX ERE: no non-capturing groups or \s, and it
  # returns null unless the regex matches the entire text.
  minimumZigVersion =
    file:
    let
      text = readFile file;
      stringVersion = match ".*minimum_zig_version[[:space:]]*=[[:space:]]*[^0-9]*v?([[:digit:]]+\\.[[:digit:]]+(\\.[[:digit:]]+)?)[[:space:]]*.*" text;
      tupleVersion = match ".*minimum_zig_version[[:space:]]*=[[:space:]]*\\.[{][^}]*\\.major[[:space:]]*=[[:space:]]*([[:digit:]]+)[^}]*\\.minor[[:space:]]*=[[:space:]]*([[:digit:]]+)[^}]*\\.patch[[:space:]]*=[[:space:]]*([[:digit:]]+)[^}]*[}].*" text;
    in
    if stringVersion != null then
      head stringVersion
    else if tupleVersion != null then
      lib.concatStringsSep "." tupleVersion
    else
      throw ''
        crucible: ${toString file} has no .minimum_zig_version, so there is no version to
        resolve. Pass the version your project needs to `toolchain { version = ...; }` instead.
      '';

  assemble =
    { release }:
    # zig runs on the build machine, so a cross package set (pkgsCross.*)
    # would select a binary for the wrong one.
    assert lib.assertMsg (pkgs.stdenv.buildPlatform.config == pkgs.stdenv.hostPlatform.config) ''
      crucible: Zig toolchains are built for the machine that runs them; take one from the
      native package set.
    '';
    let
      zig = zigFor release;
    in
    pkgs.symlinkJoin {
      name = "zig-toolchain-${release.version}";
      paths = [ zig ];
      passthru = {
        inherit (release) version;
        zig = zigFor release;
      };
      meta = {
        description = "Zig ${release.version}";
        mainProgram = "zig";
        inherit (zig.meta) platforms;
      };
    };

  toolchain =
    {
      version ? "stable",
    }:
    assemble { release = resolve { inherit version; }; };

  # Reads `.minimum_zig_version` from a project's build.zig.zon and returns
  # the toolchain for it. It is the oldest release that will do, so the
  # pinned release of its line satisfies it when it is the same or newer, and
  # a build.zig.zon keeps working as the update moves the pin to each new
  # patch. Another line does not: Zig changes its language between 0.x lines.
  fromBuildZigZon =
    { file }:
    let
      minimum = minimumZigVersion file;
      line = lib.versions.majorMinor minimum;
      pinnedLine = pinned.releases.${line} or null;
    in
    if pinnedLine == null then
      throw (notPinned minimum)
    else if compareVersions pinnedLine.version minimum < 0 then
      throw ''
        crucible: ${toString file} needs Zig ${minimum}, which is newer than the pinned
        ${pinnedLine.version}. Update crucible, or lower .minimum_zig_version.
      ''
    else
      assemble { release = pinnedLine; };
in
{
  inherit toolchain fromBuildZigZon;
}
