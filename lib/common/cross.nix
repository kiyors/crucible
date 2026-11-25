# Several toolchains here are binaries that run on the build machine. A cross
# package set (pkgsCross.*) would hand back the binary for the host instead, so
# reject that at eval time instead of building a wrong-host toolchain. Shared
# so every language gives the same actionable error.
{ lib, pkgs }:
name: hint:
lib.assertMsg (pkgs.stdenv.buildPlatform.config == pkgs.stdenv.hostPlatform.config) ''
  crucible: ${name} is built for the machine that runs it; take the toolchain from the native
  package set, not a cross package set (pkgsCross.*).${if hint == "" then "" else "\n${hint}"}
''
