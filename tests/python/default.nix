# Runs python, pip and uv from each kind of toolchain and builds a C extension
# against its headers, so a release that cannot run in the store (or a uv that
# reaches for a Python of its own) fails CI rather than a dev shell.
{ pkgs, crucible }:
let
  inherit (pkgs) lib;
  inherit (crucible) python;

  pins = lib.importJSON ../../data/python/python.json;
  uvPins = lib.importJSON ../../data/python/uv.json;
  newest = lib.last (
    lib.sort (a: b: lib.versionOlder a.version b.version) (builtins.attrValues pins.releases)
  );
  line = r: lib.versions.majorMinor r.version;

  # Whether a toolchain is rejected at evaluation time.
  rejected = toolchain: !(builtins.tryEval toolchain.drvPath).success;

  # .python-version beats requires-python, which must still allow it.
  fromApp = python.fromPyprojectToml { file = ./app/pyproject.toml; };
  fromRequires = python.fromPyprojectToml { file = ./requires-only/pyproject.toml; };
  fromVersionFile = python.fromPythonVersion { file = ./version-files/cpython/.python-version; };

  # The path projects pinning a release crucible does not take: the
  # SHA256SUMS of the python-build-standalone build that has it (trimmed here
  # to the archives of one release, plus files that must be ignored).
  fromSha256sums = python.toolchain {
    version = "3.14.0";
    sha256sums = ./SHA256SUMS;
  };

  # uv keeps its cache and data under $HOME; the sandbox has no network, so
  # anything uv tried to download would fail the check.
  pythonRun =
    name: toolchain: script:
    pkgs.runCommand "crucible-check-python-${name}"
      {
        nativeBuildInputs = [ toolchain ];
      }
      ''
        export HOME=$TMPDIR
        ${script}
        touch $out
      '';
in
{
  python-latest = pythonRun "latest" python.latest ''
    test "$(python3 --version)" = "Python ${newest.version}"
    test "$(python --version)" = "Python ${newest.version}"
    # The prefix is the store copy, reached through the toolchain's symlink.
    test "$(python3 -c 'import sys; print(sys.prefix)')" = "${python.latest.python}"
    python3 -c 'import ssl, sqlite3, ctypes, zlib, lzma, bz2, decimal, readline, venv'
    pip --version | grep -q 'python ${line newest}'
    # pip refuses the read-only store with PEP 668's message, not a
    # permission error halfway through an install.
    ! pip install --no-index nothing-at-all 2> pip.err
    grep -q 'crucible' pip.err
    # A venv made by the standard library works without uv.
    python3 -m venv venv
    venv/bin/python -c 'import sys; assert sys.prefix != sys.base_prefix'
  '';

  # Every pinned line runs, not just the default one.
  python-lines = pythonRun "lines" python.latest ''
    ${lib.concatMapStrings (r: ''
      test "$(${
        python.toolchain {
          inherit (r) version;
          uv = false;
        }
      }/bin/python3 --version)" = "Python ${r.version}"
    '') (builtins.attrValues pins.releases)}
  '';

  # A project's loop through uv, on the toolchain's Python and offline.
  python-uv = pythonRun "uv" fromApp ''
    cp -r ${./app} app
    chmod -R u+w app
    cd app
    uv --version | grep -q '^uv ${uvPins.version}'
    # `uv python find` searches PATH, where the toolchain's python3 is a
    # symlink to the store copy.
    test "$(readlink -f "$(uv python find)")" = "$(readlink -f ${fromApp.python}/bin/python3)"
    uv sync --offline
    test "$(uv run --offline python main.py)" = "${fromApp.pythonVersion}"
    test "$(.venv/bin/python -c 'import sys; print(sys.base_prefix)')" = "${fromApp.python}"
    uvx --version | grep -q "${uvPins.version}"
    # A request the toolchain cannot meet fails instead of downloading.
    ! uv python find 3.11
    test ! -e "$HOME/.local/share/uv/python"
    # The wrapper's settings hold outside a dev shell too.
    # The wrapper's settings hold outside a dev shell too, with another
    # python3 first on PATH.
    mkdir -p "$TMPDIR/other"
    ln -s ${pkgs.python3}/bin/python3 "$TMPDIR/other/python3"
    PATH="$TMPDIR/other:$PATH" env -u UV_PYTHON -u UV_NO_MANAGED_PYTHON -u UV_PYTHON_DOWNLOADS \
      ${fromApp.uv}/bin/uv venv --offline "$TMPDIR/venv"
    test "$("$TMPDIR/venv/bin/python" -c 'import sys; print(sys.base_prefix)')" = "${fromApp.python}"
  '';

  # The setup hook overwrites what a login shell may have exported.
  python-setup-hook =
    pkgs.runCommand "crucible-check-python-setup-hook"
      {
        nativeBuildInputs = [ python.latest ];
        UV_PYTHON = "/nonexistent/python3";
        UV_PYTHON_DOWNLOADS = "automatic";
        UV_MANAGED_PYTHON = "1";
        UV_PYTHON_PREFERENCE = "only-managed";
      }
      ''
        test "$UV_PYTHON" = ${python.latest.python}/bin/python3
        test "$UV_PYTHON_DOWNLOADS" = never
        test "$UV_NO_MANAGED_PYTHON" = 1
        test -z "''${UV_MANAGED_PYTHON+set}"
        test -z "''${UV_PYTHON_PREFERENCE+set}"
        touch $out
      '';

  # sysconfig names this prefix and the shell's compiler, so a C extension
  # builds as setuptools would build it, and imports.
  python-c-extension =
    pkgs.runCommand "crucible-check-python-c-extension"
      {
        nativeBuildInputs = [
          python.latest
          pkgs.stdenv.cc
        ];
      }
      ''
        python3 ${./extension/build.py} ${./extension/greet.c}
        test "$(python3 -c 'import greet; print(greet.hello())')" = "hello from crucible"
        # Through the toolchain's symlinks on Linux, the store copy on macOS.
        for flag in $(python3-config --includes); do
          test -f "''${flag#-I}/Python.h"
        done
        touch $out
      '';

  # .python-version, requires-python and the default, resolved offline.
  python-project-files =
    assert line fromApp == "3.13";
    assert line fromRequires == "3.13";
    # Only the first entry of .python-version counts.
    assert line fromVersionFile == "3.14";
    assert python.latest.version == newest.version;
    assert (python.toolchain { version = "3.12"; }).version == pins.releases."3.12".version;
    assert (python.toolchain { version = "~=3.12.0"; }).version == pins.releases."3.12".version;
    assert
      (python.toolchain { version = ">=3.11,!=3.15.*,<4"; }).version == pins.releases."3.14".version;
    assert (python.toolchain { version = "==3.13.*"; }).version == pins.releases."3.13".version;
    assert (python.toolchain { uv = false; }).uv == null;
    pkgs.runCommand "crucible-check-python-project-files" { } "touch $out";

  python-from-sha256sums = pythonRun "from-sha256sums" fromSha256sums ''
    test "$(python3 --version)" = "Python 3.14.0"
    uv venv --offline venv
    test "$(venv/bin/python -c 'import sys; print(sys.base_prefix)')" = "${fromSha256sums.python}"
  '';

  # Mistakes that would otherwise fail later, or not at all.
  python-errors =
    # A line or release that is not pinned, without a SHA256SUMS.
    assert rejected (python.toolchain { version = "3.9"; });
    assert rejected (python.toolchain { version = "3.14.0"; });
    # A release the SHA256SUMS given does not have.
    assert rejected (
      python.toolchain {
        version = "3.13";
        sha256sums = ./SHA256SUMS;
      }
    );
    # Not a version, and specifiers PEP 440 does not allow.
    assert rejected (python.toolchain { version = "latest"; });
    assert rejected (python.toolchain { version = "~=3"; });
    assert rejected (python.toolchain { version = ">=3.14.*"; });
    assert rejected (python.toolchain { version = ">=3.14.0rc1"; });
    # Interpreters crucible does not ship.
    assert rejected (python.fromPythonVersion { file = ./version-files/freethreaded/.python-version; });
    assert rejected (python.fromPythonVersion { file = ./version-files/pypy/.python-version; });
    # A .python-version that requires-python does not allow.
    assert rejected (python.fromPyprojectToml { file = ./mismatch/pyproject.toml; });
    pkgs.runCommand "crucible-check-python-errors" { } "touch $out";

  inherit (python) uv-unwrapped;
}
