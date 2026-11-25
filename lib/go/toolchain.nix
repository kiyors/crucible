# The public toolchain API: an official Go release from go.dev.
#
# A toolchain is the release itself (go.nix), one derivation per release, so
# every toolchain on the same release shares it. Each one carries:
#   .goroot        its GOROOT, for tools that ask for one
#   .buildGoModule nixpkgs' buildGoModule on this Go, for package builds
# and a setup hook that sets GOTOOLCHAIN=local, as its go.env already does.
{
  lib,
  pkgs,
}:
let
  inherit (builtins)
    attrValues
    compareVersions
    elemAt
    filter
    head
    match
    readFile
    ;

  goFor = pkgs.callPackage ./go.nix { };
  assertNative = import ../common/cross.nix { inherit lib pkgs; };
  errors = import ../common/errors.nix { };

  # Pinned by `crucible update`: the newest release of every supported line.
  pinned = lib.importJSON ../../data/go/go.json;
  pinnedVersions = map (r: r.version) (attrValues pinned.releases);

  # Nix system -> Go GOOS-GOARCH, as `PLATFORMS` in crates/pins/src/update/go.rs.
  platforms = {
    x86_64-linux = "linux-amd64";
    aarch64-linux = "linux-arm64";
    aarch64-darwin = "darwin-arm64";
  };

  # Must match the URLs `crucible update go` pins, from `DL` in
  # crates/pins/src/update/go.rs.
  downloadUrl = version: platform: "https://dl.google.com/go/go${version}.${platform}.tar.gz";

  # "1.27"; and "1.27.4" or a pre-release such as "1.27rc1", which only an
  # explicit `hashes` can pin.
  lineOf = spec: match "([0-9]+\\.[0-9]+)" spec;
  exact = spec: match "(([0-9]+\\.[0-9]+)(\\.[0-9]+|(rc|beta)[0-9]+))" spec;

  notPinned =
    spec:
    errors.notPinned {
      name = "Go";
      inherit spec;
      releases = lib.concatStringsSep ", " pinnedVersions;
      next = ''
        For another release, pass its per-system `hashes`;
        `nix run github:kiyors/crucible#crucible-full -- pin go <version>` prints them.
      '';
    };

  # `version` is "stable", a line ("1.27") or an exact release, with or
  # without the "go" prefix. Pinned releases need nothing else; any other
  # exact release needs `hashes`.
  #
  # `minimum` reads a version the way the go command reads go.mod: as the
  # oldest release that will do. It gets the oldest pinned release that is
  # the same or newer, so go.mod files keep working as the daily update moves
  # the pins to each new patch release, and as old lines leave support: Go
  # keeps building modules that ask for an older release.
  resolve =
    {
      version,
      hashes,
      minimum ? false,
    }:
    let
      spec = lib.removePrefix "go" (lib.trim version);
      line = lineOf spec;
      release = exact spec;
      pinnedLine = if release == null then null else pinned.releases.${elemAt release 1} or null;
    in
    if hashes != null then
      if release == null then
        throw ''
          crucible: `hashes` pin one exact Go release, so `version` must be one
          (such as "1.25.3"), not "${spec}".
        ''
      else
        {
          version = spec;
          platforms = lib.mapAttrs (system: platform: {
            url = downloadUrl spec platform;
            hash =
              hashes.${system} or (throw ''
                crucible: `hashes` for Go ${spec} has no entry for ${system}.
              '');
          }) platforms;
        }
    else if spec == "stable" || spec == "latest" then
      pinned.releases.${pinned.stable}
    else if minimum && (line != null || release != null) then
      let
        # The pinned releases that will do, oldest first.
        enough = lib.sort (a: b: compareVersions a.version b.version < 0) (
          filter (r: compareVersions r.version spec >= 0) (attrValues pinned.releases)
        );
      in
      if enough != [ ] then
        head enough
      else
        throw ''
          crucible: Go ${spec} is newer than every pinned release (${lib.concatStringsSep ", " pinnedVersions}).
          Update crucible, or pass that release's `hashes`
          (`nix run github:kiyors/crucible#crucible-full -- pin go ${spec}`).
        ''
    else if line != null then
      pinned.releases.${head line} or (throw (notPinned spec))
    else if release == null then
      throw ''
        crucible: "${version}" is not a Go version; use "stable", a line such as "1.27", or a
        release such as "1.27.4".
      ''
    else if pinnedLine != null && pinnedLine.version == spec then
      pinnedLine
    else
      throw (notPinned spec);

  assemble =
    release:
    assert (assertNative "Go" "Cross-compile with GOOS and GOARCH instead.");
    goFor release;

  toolchain =
    {
      version ? "stable",
      hashes ? null,
    }:
    assemble (resolve {
      inherit version hashes;
    });

  # Reads the release a module asks for: its `toolchain` directive, then its
  # `go` directive, the order the go command uses to pick a toolchain. Both
  # name the oldest release that will do. `go.work` has the same directives
  # and works too. Nothing here touches the network.
  fromGoMod =
    {
      file,
      # Per-system hashes for the exact release the file names, when no pinned
      # release of its line satisfies it.
      hashes ? null,
    }:
    let
      # Directives may be followed by `//` comments, and `toolchain default`
      # means none.
      lines = map (line: lib.trim (head (lib.splitString "//" line))) (
        lib.splitString "\n" (readFile file)
      );
      directive =
        name:
        let
          values = filter (m: m != null) (map (match "${name}[ \t]+([^ \t]+)") lines);
        in
        if values == [ ] then null else head (head values);
      toolchainDirective = directive "toolchain";
      goDirective = directive "go";
      version =
        if toolchainDirective != null && toolchainDirective != "default" then
          toolchainDirective
        else if goDirective != null then
          goDirective
        else
          throw ''
            crucible: ${toString file} has no `go` directive; add one (such as `go 1.27`) with
            `go mod edit -go=1.27`.
          '';
    in
    assemble (resolve {
      inherit version hashes;
      minimum = true;
    });
in
{
  inherit toolchain fromGoMod;
}
