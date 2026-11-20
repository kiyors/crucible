# The public toolchain API: CPython from python-build-standalone, with uv beside
# it.
#
# A toolchain joins the Python (python.nix) and uv for that Python (uv.nix).
# Each one carries:
#   .python  the CPython alone: python3, pip, headers and the standard library
#   .uv      uv for this Python, or null (uv = false)
# and a setup hook that keeps uv on this Python (see uv.nix).
{
  lib,
  pkgs,
}:
let
  inherit (builtins)
    attrValues
    elemAt
    filter
    head
    match
    readFile
    ;

  pep440 = import ./pep440.nix { inherit lib; };
  pythonFor = pkgs.callPackage ./python.nix { };
  uvPkgs = pkgs.callPackage ./uv.nix { };

  # Pinned by `crucible update`: the newest release of every line python.org
  # still fixes, all from one python-build-standalone build.
  pinned = lib.importJSON ../../data/python/python.json;
  pinnedReleases = attrValues pinned.releases;
  pinnedVersions = map (r: r.version) pinnedReleases;

  # Nix system -> host triple of the archives, as `PLATFORMS` in
  # crates/pins/src/update/python.rs.
  platforms = {
    x86_64-linux = "x86_64-unknown-linux-gnu";
    aarch64-linux = "aarch64-unknown-linux-gnu";
    aarch64-darwin = "aarch64-apple-darwin";
  };

  # Must match `release_url` in crates/pins/src/update/python.rs.
  releaseUrl =
    build: file:
    "https://github.com/astral-sh/python-build-standalone/releases/download/${build}/${
      lib.replaceStrings [ "+" ] [ "%2B" ] file
    }";

  # Every release a python-build-standalone SHA256SUMS has an install_only
  # archive of for each system. The build tag is part of every file name.
  fromSha256sums =
    file:
    let
      entries = filter (m: m != null) (
        map (match "([0-9a-f]{64})  (cpython-([0-9]+\\.[0-9]+\\.[0-9]+)\\+([0-9]+)-(.+)-install_only\\.tar\\.gz)") (
          lib.splitString "\n" (readFile file)
        )
      );
      archive =
        version: triple: lib.findFirst (m: elemAt m 2 == version && elemAt m 4 == triple) null entries;
      complete = version: lib.all (triple: archive version triple != null) (attrValues platforms);
      versions = lib.unique (filter complete (map (m: elemAt m 2) entries));
    in
    if entries == [ ] then
      throw "crucible: ${toString file} lists no cpython-*-install_only.tar.gz; is it a python-build-standalone SHA256SUMS?"
    else
      map (version: {
        inherit version;
        platforms = lib.mapAttrs (
          _: triple:
          let
            m = archive version triple;
          in
          {
            url = releaseUrl (elemAt m 3) (elemAt m 1);
            hash = "sha256:${elemAt m 0}";
          }
        ) platforms;
      }) versions;

  notPinned = spec: ''
    crucible: Python "${spec}" matches no pinned release (${lib.concatStringsSep ", " pinnedVersions}).
    For another release, pass the SHA256SUMS of a python-build-standalone release that has it as
    `sha256sums`, for example a `flake = false` input with the url
    https://github.com/astral-sh/python-build-standalone/releases/download/<build>/SHA256SUMS
  '';

  # `version` is a line ("3.14"), "3" (the newest pinned line), a PEP 440
  # specifier (">=3.12,<3.15") or an exact release. Pinned releases need
  # nothing else; others come from a build's SHA256SUMS.
  resolve =
    {
      version,
      sha256sums,
    }:
    let
      spec = lib.trim version;
      releases = if sha256sums != null then fromSha256sums sha256sums else pinnedReleases;
      newest = pep440.newest spec (map (r: r.version) releases);
    in
    if newest != null then
      lib.findFirst (r: r.version == newest) null releases
    else if sha256sums != null then
      throw ''
        crucible: the SHA256SUMS given has no CPython that "${spec}" allows for every system
        (it has ${lib.concatMapStringsSep ", " (r: r.version) releases}).
      ''
    else
      throw (notPinned spec);

  assemble =
    {
      release,
      uv ? true,
    }:
    # python runs on the build machine, so a cross package set (pkgsCross.*)
    # would select a binary for the wrong one.
    assert lib.assertMsg (pkgs.stdenv.buildPlatform.config == pkgs.stdenv.hostPlatform.config) ''
      crucible: Python toolchains are built for the machine that runs them; take one from the
      native package set.
    '';
    let
      python = pythonFor release;
      uvPkg = if uv then uvPkgs.withPython python else null;
    in
    pkgs.symlinkJoin {
      name = "python-toolchain-${python.version}";
      paths = [ python ] ++ lib.optional (uvPkg != null) uvPkg;
      # Overwritten, not defaulted: a UV_PYTHON or UV_PYTHON_DOWNLOADS from the
      # login shell (or a Home Manager toolchain on another release) would
      # otherwise win over this dev shell's Python. UV_MANAGED_PYTHON and the
      # older UV_PYTHON_PREFERENCE would contradict UV_NO_MANAGED_PYTHON.
      postBuild = lib.optionalString (uvPkg != null) ''
        mkdir -p $out/nix-support
        cat > $out/nix-support/setup-hook <<'EOF'
        export UV_PYTHON=${python}/bin/python3
        export UV_NO_MANAGED_PYTHON=1
        export UV_PYTHON_DOWNLOADS=never
        unset UV_MANAGED_PYTHON UV_PYTHON_PREFERENCE
        EOF
      '';
      passthru = {
        inherit (python) version pythonVersion sitePackages;
        inherit python;
        uv = uvPkg;
      };
      meta = {
        description = "CPython ${python.version}${lib.optionalString (uvPkg != null) " with uv"}";
        mainProgram = "python3";
        inherit (python.meta) platforms;
      };
    };

  toolchain =
    {
      version ? "3",
      uv ? true,
      sha256sums ? null,
    }:
    assemble {
      release = resolve { inherit version sha256sums; };
      inherit uv;
    };

  # The request in a .python-version, as uv and pyenv write it: "3.14",
  # "3.14.2", "cpython@3.14" or "cpython-3.14", or a specifier. Only the first
  # entry counts. Other implementations and freethreaded builds ("3.14t") are
  # not something crucible ships, so they fail here rather than resolving to a
  # different interpreter.
  readPythonVersion =
    file:
    let
      lines = filter (line: line != "" && !lib.hasPrefix "#" line) (
        map lib.trim (lib.splitString "\n" (readFile file))
      );
      entry =
        if lines == [ ] then throw "crucible: ${toString file} names no Python version." else head lines;
      plain = match "(cpython[-@]|python)?([0-9]+(\\.[0-9]+){0,2})" (lib.toLower entry);
    in
    if plain != null then
      elemAt plain 1
    else if match "[<>=!~].*" entry != null then
      entry
    else
      throw ''
        crucible: ${toString file} asks for "${entry}", but crucible ships CPython only, without
        the freethreaded builds. Use a version such as "3.14" or "3.14.2".
      '';

  fromPythonVersion =
    {
      file,
      uv ? true,
      sha256sums ? null,
    }:
    toolchain {
      version = readPythonVersion file;
      inherit uv sha256sums;
    };

  # Reads the Python a project asks for the way uv does: the .python-version
  # next to pyproject.toml, then project.requires-python, then the newest
  # pinned line. A .python-version outside requires-python is an error, as it
  # is for uv. Nothing here touches the network.
  fromPyprojectToml =
    {
      file,
      uv ? true,
      sha256sums ? null,
    }:
    let
      toml = builtins.fromTOML (readFile file);
      dir = builtins.dirOf (toString file);
      versionFile = dir + "/.python-version";
      fileVersion = if builtins.pathExists versionFile then readPythonVersion versionFile else null;
      requires = toml.project.requires-python or null;
      release = resolve {
        version =
          if fileVersion != null then
            fileVersion
          else if requires != null then
            requires
          else
            "3";
        inherit sha256sums;
      };
    in
    if requires != null && !pep440.satisfies release.version requires then
      throw ''
        crucible: .python-version picks Python ${release.version}, which requires-python
        "${requires}" in ${toString file} does not allow.
      ''
    else
      assemble { inherit release uv; };
in
{
  inherit toolchain fromPythonVersion fromPyprojectToml;
  uv-unwrapped = uvPkgs.unwrapped;
  latest = toolchain { };
}
