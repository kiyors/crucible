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
  assertNative = import ../common/cross.nix { inherit lib pkgs; };
  pythonFor = pkgs.callPackage ./python.nix { };
  uvPkgs = pkgs.callPackage ./uv.nix { };

  # Pinned by `crucible update`: the newest release of every line python.org
  # still fixes, all from one python-build-standalone build. The freethreaded
  # builds have their own lines, keyed with the trailing "t" next to the
  # regular ones, at the same versions.
  pinned = lib.importJSON ../../data/python/python.json;
  pinnedReleases = attrValues pinned.releases;
  # A freethreaded release is named by its version with the "t" a request
  # for it takes.
  label = r: r.version + lib.optionalString (r.freethreaded or false) "t";

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
  # archive of for each system, as pins: `{ version, freethreaded,
  # platforms }`. The build tag is part of every file name; the freethreaded
  # archives name the GIL builds' versions with a "-freethreaded" infix, so
  # they parse as freethreaded with the GIL triple.
  fromSha256sums =
    file:
    let
      # The groups of one archive line: [hex, file, version, build, triple].
      parse =
        m:
        let
          triple = lib.removeSuffix "-freethreaded" (elemAt m 4);
        in
        {
          version = elemAt m 2;
          build = elemAt m 3;
          file = elemAt m 1;
          hash = "sha256:${elemAt m 0}";
          inherit triple;
          freethreaded = triple != elemAt m 4;
        };
      entries = filter (e: e != null) (
        map (m: if m == null then null else parse m) (
          map (match "([0-9a-f]{64})  (cpython-([0-9]+\\.[0-9]+\\.[0-9]+)\\+([0-9]+)-(.+)-install_only\\.tar\\.gz)") (
            lib.splitString "\n" (readFile file)
          )
        )
      );
      archive =
        freethreaded: version: triple:
        lib.findFirst (
          e: e.version == version && e.triple == triple && e.freethreaded == freethreaded
        ) null entries;
      complete =
        freethreaded: version:
        lib.all (triple: archive freethreaded version triple != null) (attrValues platforms);
      # The pins of one kind: every release that has complete archives.
      pins =
        freethreaded:
        map (version: {
          inherit version freethreaded;
          platforms = lib.mapAttrs (
            _: triple:
            let
              e = archive freethreaded version triple;
            in
            {
              url = releaseUrl e.build e.file;
              inherit (e) hash;
            }
          ) platforms;
        }) (lib.unique (filter (v: complete freethreaded v) (map (e: e.version) entries)));
    in
    if entries == [ ] then
      throw "crucible: ${toString file} lists no cpython-*-install_only.tar.gz; is it a python-build-standalone SHA256SUMS?"
    else
      pins false ++ pins true;

  notPinned = spec: ''
    crucible: Python "${spec}" matches no pinned release (${
      lib.concatMapStringsSep ", " label pinnedReleases
    }).
    For another release, pass the SHA256SUMS of a python-build-standalone release that has it as
    `sha256sums`, for example a `flake = false` input with the url
    https://github.com/astral-sh/python-build-standalone/releases/download/<build>/SHA256SUMS
  '';

  # `version` is a line ("3.14"), "3" (the newest pinned line), a PEP 440
  # specifier (">=3.12,<3.15") or an exact release. Pinned releases need
  # nothing else; others come from a build's SHA256SUMS. A trailing "t"
  # ("3.14t", "3.14.10t") asks for the freethreaded build of what the rest
  # allows. The two kinds share versions, so each request only ever sees the
  # releases of its own kind.
  resolve =
    {
      version,
      sha256sums,
    }:
    let
      spec = lib.trim version;
      freethreaded = lib.hasSuffix "t" spec;
      releases = filter (r: (r.freethreaded or false) == freethreaded) (
        if sha256sums != null then fromSha256sums sha256sums else pinnedReleases
      );
      newest = pep440.newest (lib.removeSuffix "t" spec) (map (r: r.version) releases);
    in
    if newest != null then
      lib.findFirst (r: r.version == newest) null releases
    else if sha256sums != null then
      throw ''
        crucible: the SHA256SUMS given has no ${lib.optionalString freethreaded "freethreaded "}CPython that "${spec}" allows for every system
        (it has ${lib.concatMapStringsSep ", " label releases}).
      ''
    else
      throw (notPinned spec);

  assemble =
    {
      release,
      uv ? true,
    }:
    assert (assertNative "Python" "");
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
  # "3.14.2", "cpython@3.14", "cpython-3.14", "3.14t" for the freethreaded
  # build or a specifier. Only the first entry counts. Other implementations
  # (PyPy, GraalPy) are not something crucible ships, so they fail here
  # rather than resolving to a different interpreter.
  readPythonVersion =
    file:
    let
      lines = filter (line: line != "" && !lib.hasPrefix "#" line) (
        map lib.trim (lib.splitString "\n" (readFile file))
      );
      entry =
        if lines == [ ] then throw "crucible: ${toString file} names no Python version." else head lines;
      plain = match "(cpython[-@]|python)?([0-9]+(\\.[0-9]+){0,2}t?)" (lib.toLower entry);
    in
    if plain != null then
      elemAt plain 1
    else if match "[<>=!~].*" entry != null then
      entry
    else
      throw ''
        crucible: ${toString file} asks for "${entry}", but crucible ships CPython only.
        Use a version such as "3.14", "3.14.2" or "3.14t" for the freethreaded build.
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
