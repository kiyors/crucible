# Several toolchains are a symlink tree over their parts with a setup hook
# that fixes the environment for shells using them. Keep the derivation shape
# here so each toolchain only supplies its paths, hook text and passthru.
{ lib, pkgs }:
{
  name,
  paths,
  # Null means no setup hook at all; a string is the hook's body (the leading
  # and trailing whitespace is trimmed so the heredoc gets exactly one final
  # newline, as the hand-written hooks had).
  setupHook ? null,
  passthru ? { },
  meta ? { },
}:
pkgs.symlinkJoin {
  inherit
    name
    paths
    passthru
    meta
    ;
  postBuild = lib.optionalString (setupHook != null) ''
    mkdir -p $out/nix-support
    cat > $out/nix-support/setup-hook <<'EOF'
    ${lib.trim setupHook}
    EOF
  '';
}
