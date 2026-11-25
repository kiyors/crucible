# The public toolchain API: a Deno release, pinned in data/deno/deno.json.
#
# A toolchain carries:
#   .deno    the deno executable, from the release zip
#   .version the pinned release's version
{
  lib,
  pkgs,
}:
let
  inherit (builtins)
    attrNames
    elem
    getAttr
    head
    match
    ;

  denoFor = pkgs.callPackage ./deno.nix { };
  assertNative = import ../common/cross.nix { inherit lib pkgs; };

  pinned = lib.importJSON ../../data/deno/deno.json;
  pinnedMajors = attrNames pinned;

  # The newest release is the one of the highest major: `crucible update`
  # writes the majors oldest first, so this is the last of them.
  newestMajor = lib.last (lib.sort lib.versionOlder pinnedMajors);

  # "latest" is the newest pinned release, a major line ("2") picks that
  # major's pin, and an exact release ("2.9.8") is used only as the one pinned
  # for its major. Nothing else: an unpinned release is an error rather than a
  # silent substitution, like the package-manager pins.
  notPinned =
    spec:
    let
      pins = map (major: "${major} ${(getAttr major pinned).version}") pinnedMajors;
    in
    throw ''
      crucible: Deno "${spec}" is not pinned; the pins are: ${lib.concatStringsSep ", " pins}.
      Only versions in this flake's pinned data are supported: wait for the next
      scheduled crucible update, or, if you maintain the pins yourself, use
      `#crucible-full` and run `crucible update deno`.
    '';

  resolve =
    { version }:
    let
      spec = lib.trim version;
      # `builtins.match` anchors the whole string, so the patterns cover the
      # full spec.
      exact = match "([0-9]+\\.[0-9]+\\.[0-9]+)" spec;
      # ["2"] for "2.9.8", or null for anything that is not a 3-part version.
      major = match "([0-9]+)\\..*" spec;
      pinnedExact =
        exact != null
        && major != null
        && elem (head major) pinnedMajors
        && (getAttr (head major) pinned).version == head exact;
    in
    if spec == "latest" then
      getAttr newestMajor pinned
    else if elem spec pinnedMajors then
      getAttr spec pinned
    else if pinnedExact then
      getAttr (head major) pinned
    else
      throw (notPinned spec);
in
{
  toolchain =
    {
      version ? "latest",
    }:
    assert (assertNative "Deno" "");
    denoFor (resolve {
      inherit version;
    });
}
