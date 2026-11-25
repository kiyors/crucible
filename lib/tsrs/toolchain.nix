# The public toolchain API: the tsrs release pinned in data/tsrs/tsrs.json.
#
# tsrs has no release lines to choose between: every version is a prerelease
# against one TypeScript release, so one pin is kept, the one npm tags latest.
{
  lib,
  pkgs,
}:
let
  tsrsFor = pkgs.callPackage ./tsrs.nix { };
  assertNative = import ../common/cross.nix { inherit lib pkgs; };
  pinned = lib.importJSON ../../data/tsrs/tsrs.json;

  # "latest" or the pinned version itself; anything else is an error rather
  # than a silent substitution, like the other pins.
  resolve =
    { version }:
    let
      spec = lib.trim version;
    in
    if spec == "latest" || spec == pinned.version then
      pinned
    else
      throw ''
        crucible: tsrs "${spec}" is not pinned; the pin is ${pinned.version}.
        Use version = "latest", or run `crucible update tsrs` to pin the release npm tags latest.
      '';
in
{
  toolchain =
    {
      version ? "latest",
    }:
    assert (assertNative "tsrs" "");
    tsrsFor (resolve {
      inherit version;
    });
}
