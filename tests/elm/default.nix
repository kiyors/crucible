# Runs the Elm compiler from the pinned npm tarball, so a tarball that stops
# installing fails a check rather than a downstream shell. `elm --version`
# needs no package registry, so this stays offline; an Elm app would need the
# registry to resolve dependencies. The other checks are pure evaluation of
# version resolution and elm.json reading.
{ pkgs, crucible }:
let
  inherit (crucible.elm) toolchain fromElmJson;
  inherit (pkgs) lib;
  pins = lib.importJSON ../../data/elm/elm.json;
  versions = lib.attrNames pins;
  newest = lib.last (lib.sort lib.versionOlder versions);
  newestVersion = pins.${newest}.version;
  elm = toolchain { };

  # Whether a toolchain is rejected at evaluation time.
  rejected = toolchain: !(builtins.tryEval toolchain.drvPath).success;

  # tryEval reports failure but not the thrown text, so assert the wording in
  # the source that throws it; the rejections below cover the error paths.
  # Collapse whitespace so a phrase can span nixfmt's line wrapping.
  says =
    phrase:
    let
      source = builtins.readFile ../../lib/elm/toolchain.nix;
      flat = lib.concatStringsSep " " (
        builtins.filter builtins.isString (builtins.split "[[:space:]]+" source)
      );
    in
    lib.strings.hasInfix phrase flat;

  # elm.json files written from the pins, like tests/go's go.mod, so these
  # checks follow the daily update instead of breaking when it moves.
  elmJson =
    version:
    builtins.toFile "elm.json" ''
      {
        "type": "application",
        "source-directories": [ "src" ],
        "elm-version": "${version}",
        "dependencies": { "direct": {}, "indirect": {} },
        "test-dependencies": { "direct": {}, "indirect": {} }
      }
    '';
in
{
  elm =
    pkgs.runCommand "crucible-check-elm"
      {
        nativeBuildInputs = [ elm ];
      }
      ''
        export HOME=$TMPDIR
        ${lib.getExe elm} --version | grep -qF "${newestVersion}"
        touch $out
      '';

  # "latest" and the exact pin give the same release; npm's "-0" packaging
  # form and any unpinned release are errors rather than silent substitutions.
  # The escape hatch resolves an unpinned version at eval time from caller
  # hashes (it is not built here: no such tarball need exist).
  elm-resolve =
    assert elm.version == newestVersion;
    assert (toolchain { version = "latest"; }).version == newestVersion;
    assert (toolchain { version = newestVersion; }).version == newestVersion;
    assert rejected (toolchain {
      version = "0.99.0";
    });
    assert rejected (toolchain {
      version = "${newestVersion}-0";
    });
    assert rejected (toolchain {
      version = "latest";
      hashes = { };
    });
    assert
      (toolchain {
        version = "0.99.0";
        hashes = lib.mapAttrs (_: source: source.hash) pins.${newest}.platforms;
      }).version == "0.99.0";
    # The error tells the user where the pin comes from and what to do next.
    assert says "Only versions in this flake's pinned data are supported";
    assert says "wait for the next scheduled crucible update";
    assert says "#crucible-full";
    assert says "crucible pin elm";
    assert says "`hashes` pin one exact Elm release";
    pkgs.runCommand "crucible-check-elm-resolve" { } "touch $out";

  # fromElmJson reads "elm-version" and resolves that exact version. A file
  # without the field, or naming an unpinned version without `hashes`, fails
  # with an actionable error.
  elm-from-elm-json =
    let
      fromFile = file: fromElmJson { inherit file; };
    in
    assert (fromFile (elmJson newestVersion)).version == newestVersion;
    assert rejected (fromFile (elmJson "0.99.0"));
    assert rejected (fromFile (builtins.toFile "elm.json" ''{ "type": "application" }''));
    assert
      (fromElmJson {
        file = elmJson "0.99.0";
        hashes = lib.mapAttrs (_: source: source.hash) pins.${newest}.platforms;
      }).version == "0.99.0";
    assert says "has no \"elm-version\" compiler version; add one";
    pkgs.runCommand "crucible-check-elm-from-elm-json" { } "touch $out";
}
