# One CPython release from python-build-standalone's install_only archive, the
# relocatable build uv installs: python, the pip it bundles, headers and the
# standard library, unpacked as upstream ships them.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  libxcrypt-legacy,
}:
# { version, platforms.<system> = { url, hash } }: an entry of
# data/python/python.json, or one read from a release's SHA256SUMS.
release:
let
  inherit (stdenv.hostPlatform) system isLinux;
  source =
    release.platforms.${system}
      or (throw "crucible: Python ${release.version} has no build for ${system}.");
  line = lib.versions.majorMinor release.version;
  # The freethreaded builds install under the same versions with a "t": the
  # include dir, the standard library and the interpreter are python3.14t.
  free = lib.optionalString (release.freethreaded or false) "t";
  stdlib = "lib/python${line}${free}";

  # PEP 668: pip and `uv pip install --system` refuse to write into this
  # Python, which is in the read-only store anyway, and say what to do instead.
  externallyManaged = ''
    [externally-managed]
    Error=This Python is crucible's, in the read-only Nix store. Install packages into a
     virtual environment instead (`uv venv` or `python3 -m venv .venv`), add them to the
     project with `uv add`, or install command-line tools with `uv tool install`.
  '';
in
stdenv.mkDerivation {
  pname = "cpython";
  inherit (release) version;

  src = fetchurl { inherit (source) url hash; };

  nativeBuildInputs = lib.optionals isLinux [ autoPatchelfHook ];

  # OpenSSL, SQLite, Tcl/Tk and the rest are bundled. Besides glibc, the Linux
  # build links libgcc_s, and up to 3.12 its _crypt module links libcrypt.so.1,
  # which glibc no longer ships; the macOS one only links system libraries.
  buildInputs = lib.optionals isLinux [
    stdenv.cc.cc.lib
    libxcrypt-legacy
  ];

  # sysconfig still describes the build machine: a /install prefix, clang
  # (which a Linux dev shell rarely has) and an llvm-ar under /tools. Building
  # a C extension reads those, so they are rewritten to this prefix and the
  # shell's cc, c++ and ar, as uv does when it installs these builds.
  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -R bin include lib share $out/

    for file in $out/${stdlib}/_sysconfigdata_*.py $out/${stdlib}/_sysconfig_vars_*.json \
      $out/${stdlib}/config-${line}*/Makefile; do
      [ -e "$file" ] || continue
      chmod u+w "$file"
      # Twice, because neighbouring paths share the separator between them.
      for _ in 1 2; do
        sed -E -i \
          -e "s#(^|[[:space:]\"'=])/install(/|[[:space:]\"']|\$)#\1$out\2#g" \
          "$file"
      done
      sed -E -i \
        -e "s#[^[:space:]\"'=]*/tools/llvm/bin/llvm-ar#ar#g" \
        -e "s#(^|[[:space:]\"'=])clang\+\+([[:space:]\"']|\$)#\1c++\2#g" \
        -e "s#(^|[[:space:]\"'=])clang([[:space:]\"']|\$)#\1cc\2#g" \
        "$file"
    done
    # Compiled copies of the old sysconfig data would shadow the fixed source.
    rm -f $out/${stdlib}/__pycache__/_sysconfigdata_*.pyc

    printf '%s' ${lib.escapeShellArg externallyManaged} > $out/${stdlib}/EXTERNALLY-MANAGED
    runHook postInstall
  '';

  # The bundled scripts' shebangs (pip, pydoc, python3-config) are already
  # relocatable; the standard library's `#!/usr/bin/env python3` scripts get
  # this python.
  preFixup = ''
    addToSearchPath HOST_PATH $out/bin
  '';

  # Stripping would invalidate the macOS binaries' signatures, which arm64
  # refuses to run, and gains little on an already release-built python.
  dontStrip = true;

  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    test "$($out/bin/python3 --version)" = "Python ${release.version}"
    test "$($out/bin/python3 -c 'import sys; print(sys.prefix)')" = "$out"
    $out/bin/python3 -c 'import ssl, sqlite3, ctypes, zlib, lzma, bz2, decimal, readline'
    ! grep -qE "[[:space:]\"'=]/install(/|[[:space:]\"']|\$)" $out/${stdlib}/_sysconfigdata_*.py
    runHook postInstallCheck
  '';

  passthru = {
    pythonVersion = line;
    sitePackages = "${stdlib}/site-packages";
    executable = "python${line}${free}";
  };

  meta = {
    description = "CPython ${release.version}, the python-build-standalone build uv installs";
    homepage = "https://github.com/astral-sh/python-build-standalone";
    license = lib.licenses.psfl;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    mainProgram = "python3";
    platforms = import ../common/systems.nix;
  };
}
