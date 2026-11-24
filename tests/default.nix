# The flake's checks, one set per language.
{ pkgs, crucible }:
let
  inherit (pkgs) lib;
in
import ./rust { inherit pkgs crucible; }
// import ./node { inherit pkgs crucible; }
// import ./init.nix { inherit pkgs crucible; }
// import ./deno { inherit pkgs crucible; }
// import ./python { inherit pkgs crucible; }
// import ./go { inherit pkgs crucible; }
// import ./zig { inherit pkgs crucible; }
// import ./tsrs { inherit pkgs crucible; }
// {
  # Built and unit-tested by the stable toolchain's rustPlatform, with and
  # without the `net` feature.
  crucible = crucible.cli;
  crucible-full = crucible.cli-full;

  # `cargo clippy --workspace --all-targets` with every warning an error.
  # It reuses the CLI package's vendored dependencies, since the sandbox
  # cannot download crates, and its stock toolchain, which carries clippy.
  crucible-clippy = crucible.cli-full.overrideAttrs {
    pname = "crucible-clippy";
    buildPhase = ''
      runHook preBuild
      cargo clippy --offline --workspace --all-targets -- -D warnings
      runHook postBuild
    '';
    doCheck = false;
    installPhase = "touch $out";
  };

  # The Node template ships a dev shell with pnpm and biome in it.
  node-template =
    let
      nodeTemplate = builtins.readFile ../templates/node/flake.nix;
    in
    assert lib.assertMsg (
      builtins.match ".*nodePackages.pnpm.*" nodeTemplate != null
    ) "the node template must ship pnpm in its dev shell";
    pkgs.runCommand "crucible-check-node-template" { } "touch $out";

  # `crucible doctor`, run here with the fork first on PATH so it passes. The
  # user build is what `nix run github:kiyors/crucible` starts, so it must say
  # the same thing as the full one, list only its own commands, and point
  # `pin` and `update` at `crucible-full`. A bare `crucible` prints the
  # commands. It runs in a project with every language's files and none of
  # their toolchains, so each language's checks speak up without failing the
  # run.
  crucible-commands =
    pkgs.runCommand "crucible-check-commands"
      {
        nativeBuildInputs = [ crucible.rust.stable ];
      }
      ''
        export HOME=$TMPDIR
        full=${lib.getExe crucible.cli-full}
        slim=${lib.getExe crucible.cli}
        mkdir project && cd project
        echo 3.14 > .python-version
        printf 'module example.com/m\n\ngo 1.26\n' > go.mod
        echo '.{ .name = .m, .minimum_zig_version = "0.15.2" }' > build.zig.zon
        echo '{}' > deno.json
        # A bare `crucible` lists the commands and does nothing else.
        ! $slim > help 2>&1
        for cmd in init add doctor; do grep -qE "^  $cmd " help; done
        ! grep -qE '^  (update|pin) ' help
        ! $full > help-full 2>&1
        for cmd in update pin; do grep -qE "^  $cmd " help-full; done
        $full doctor > full
        grep -q 'huacnlee/cargo' full
        # The user build must still carry the Node.js checks,
        grep -q 'VP_HOME' full
        # and the Python, Go, Zig and Deno ones.
        grep -q 'no python3 is on PATH' full
        grep -q 'no go is on PATH' full
        grep -q 'no zig is on PATH' full
        grep -q 'deno.json is here, but no deno is on PATH' full
        $slim doctor > slim
        diff full slim
        ! $slim pin rust 1.99 2> missing
        grep -q 'github:kiyors/crucible#crucible-full -- pin rust 1.99' missing
        ! $slim update 2> missing
        grep -q 'github:kiyors/crucible#crucible-full -- update' missing
        touch $out
      '';

  # The package manager checks with fake executables: devEngines as the
  # fallback for packageManager, a range, a second pnpm further down PATH, and
  # a pnpm wrapper that runs on another node than the one on PATH.
  crucible-doctor-package-manager = pkgs.runCommand "crucible-check-doctor-package-manager" { } ''
    export HOME=$TMPDIR
    root=$PWD
    mkdir -p first second other/bin project
    printf '#!${pkgs.runtimeShell}\necho v24.11.0\n' > first/node
    cp first/node other/bin/node
    printf '#!${pkgs.runtimeShell}\n[ "$1" = --version ] && exec echo 10.4.1\nexec "%s" "$@"\n' \
      "$root/other/bin/node" > first/pnpm
    printf '#!${pkgs.runtimeShell}\necho 9.15.0\n' > second/pnpm
    chmod +x first/* second/pnpm other/bin/node
    echo '{"devEngines":{"packageManager":[{"name":"pnpm","version":">=10 <11"}]}}' \
      > project/package.json
    cd project
    PATH=$root/first:$root/second:$PATH ${lib.getExe crucible.cli} doctor > out || true
    cat out
    grep -q 'is pnpm 10.4.1, which matches pnpm@>=10 <11' out
    grep -q 'another pnpm is on PATH: .*/second/pnpm' out
    grep -q 'runs on .*/other/bin/node, not the node on PATH' out
    touch $out
  '';
}
