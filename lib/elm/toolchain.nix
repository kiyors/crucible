# The public toolchain API: the Elm compiler from the npm package that carries
# its binary, pinned in data/elm/elm.json.
#
# A toolchain is the release itself (elm.nix), one derivation per version, so
# every toolchain on the same version shares it. Each one carries:
#   .elm      the elm executable, for callers that want the derivation itself
#   .version  the compiler version elm.json and `elm --version` use
{
  lib,
  pkgs,
}:
let
  inherit (builtins)
    match
    readFile
    ;

  elmFor = pkgs.callPackage ./elm.nix { };
  assertNative = import ../common/cross.nix { inherit lib pkgs; };
  errors = import ../common/errors.nix { };

  # `crucible update elm` writes one pin, keyed by the elm.json-form version.
  pinned = lib.importJSON ../../data/elm/elm.json;
  versions = lib.attrNames pinned;
  newest = lib.last (lib.sort lib.versionOlder versions);

  # Nix system -> the npm package holding that system's executable, mirroring
  # `PLATFORMS` in crates/pins/src/update/elm.rs in lib/common/systems.nix order.
  platforms = {
    x86_64-linux = "@elm_binaries/linux_x64";
    aarch64-linux = "@elm_binaries/linux_arm64";
    aarch64-darwin = "@elm_binaries/darwin_arm64";
  };

  # npm wraps the compiler as elm@<version>-0; elm.json and `elm --version`
  # say <version>. `registry` and `npmSuffix` match `REGISTRY` and `NPM_SUFFIX`
  # in crates/pins/src/update/elm.rs, and `crucible pin elm` prints this URL.
  registry = "https://registry.npmjs.org";
  npmSuffix = "-0";
  tarballUrl =
    version: package:
    let
      name = lib.last (lib.splitString "/" package);
    in
    "${registry}/${package}/-/${name}-${version}${npmSuffix}.tgz";

  # Only "latest" and exact pinned versions are supported; nothing is silently
  # substituted for a version no pin covers.
  notPinned =
    spec:
    errors.notPinned {
      name = "Elm";
      inherit spec;
      releases = lib.concatStringsSep ", " versions;
      next = ''
        Only versions in this flake's pinned data are supported: wait for the next
        scheduled crucible update, or, if you maintain the pins yourself, use
        `#crucible-full` and run `crucible pin elm ${spec}` for the `hashes` to pass.
      '';
    };

  exact = spec: match "([0-9]+\\.[0-9]+\\.[0-9]+)" spec;

  # `version` is "latest" or an exact compiler version. An unpinned exact
  # release needs `hashes`, its per-system SRI hashes from `crucible pin elm`.
  resolve =
    {
      version,
      hashes,
    }:
    let
      spec = lib.trim version;
      release = pinned.${spec} or null;
      fromHashes =
        if hashes == null then
          null
        else if exact spec == null then
          throw ''
            crucible: `hashes` pin one exact Elm release, so `version` must be one
            (such as "0.19.3"), not "${spec}".
          ''
        else
          {
            version = spec;
            platforms = lib.mapAttrs (system: package: {
              url = tarballUrl spec package;
              hash =
                hashes.${system} or (throw ''
                  crucible: `hashes` for Elm ${spec} has no entry for ${system}.
                '');
            }) platforms;
          };
    in
    if fromHashes != null then
      fromHashes
    else if spec == "latest" then
      pinned.${newest}
    else if release != null then
      release
    else
      throw (notPinned spec);
  assemble =
    release:
    assert (assertNative "Elm" "");
    elmFor release;
in
{
  toolchain =
    {
      version ? "latest",
      hashes ? null,
    }:
    assemble (resolve {
      inherit version hashes;
    });

  # Reads the compiler version from an elm.json. The field is matched in the
  # file text so a malformed file gives a crucible: error pointing at the fix,
  # rather than a generic attribute-missing one. Nothing here touches the
  # network: the version must be pinned or come with `hashes`.
  fromElmJson =
    {
      file,
      hashes ? null,
    }:
    let
      source = readFile file;
      matched = match ".*\"elm-version\"[[:space:]]*:[[:space:]]*\"([0-9]+\\.[0-9]+\\.[0-9]+)\".*" source;
      version =
        if matched == null then
          throw ''
            crucible: ${toString file} has no "elm-version" compiler version; add one
            (such as "elm-version": "${newest}") to your elm.json.
          ''
        else
          lib.head matched;
    in
    assemble (resolve {
      inherit version hashes;
    });
}
