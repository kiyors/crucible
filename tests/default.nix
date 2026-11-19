# Builds a small crate with each toolchain, so a fork release that cannot drive
# the channel's rustc fails CI rather than a dev shell.
{ pkgs, nixCargo }:
let
  inherit (pkgs) lib;
  host = pkgs.stdenv.hostPlatform.rust.rustcTarget;

  src = ./hello;

  wasm = nixCargo.mkToolchain { targets = [ "wasm32-unknown-unknown" ]; };
  fromFile = nixCargo.fromToolchainFile { file = ./rust-toolchain.toml; };

  # The path projects pinning a numbered channel take: the upstream
  # channel-rust-*.toml as `manifest`.
  fromManifest = nixCargo.fromToolchainFile {
    file = ./rust-toolchain-1.99.toml;
    manifest = builtins.fetchurl {
      url = "https://static.rust-lang.org/dist/channel-rust-1.99.0.toml";
      sha256 = "sha256-zm3dyIY2T414ZRR3EhLOvptzG6gta4WZUcawzMUWtqI=";
    };
  };

  # The fork keeps its build directory under $HOME/.cargo/targets, so it gets a
  # writable HOME; it never touches the network for a crate without deps.
  cargoRun =
    name: toolchain: script:
    pkgs.runCommand "nix-cargo-check-${name}"
      {
        # runCommand has no C compiler, which rustc needs as its linker driver;
        # a dev shell from mkShell always has one.
        nativeBuildInputs = [
          toolchain
          pkgs.stdenv.cc
        ];
      }
      ''
        export HOME=$TMPDIR
        cp -r ${src} hello
        chmod -R u+w hello
        cd hello
        ${lib.optionalString (name != "stock") "cargo -V | grep -q huacnlee/cargo"}
        ${script}
        touch $out
      '';

  runHello = ''
    cargo -V
    test "$(cargo run --offline --quiet)" = "hello from nix-cargo"
  '';
in
{
  stable = cargoRun "stable" nixCargo.default ''
    ${runHello}
    cargo clippy --offline --quiet -- -D warnings
    cargo fmt --check
    # llvm-tools live in the sysroot, linked against rustc's libLLVM.
    "$(rustc --print sysroot)/lib/rustlib/${host}/bin/llvm-nm" --version
    ${nixCargo.default.rust-analyzer}/bin/rust-analyzer --version
    # Exported by the toolchain's setup hook.
    test -f "$RUST_SRC_PATH/core/src/lib.rs"
  '';

  # The channel's own cargo, as packages are built with.
  stock = cargoRun "stock" nixCargo.default.stock ''
    cargo -V | grep -vq huacnlee/cargo
    ${runHello}
  '';

  nightly = cargoRun "nightly" nixCargo.nightly runHello;

  # A linked .wasm module, not just an rlib: the link goes through the
  # sysroot's rust-lld (as wasm-ld) on every host.
  wasm32 = cargoRun "wasm32" wasm ''
    cargo rustc --offline --lib --crate-type cdylib --target wasm32-unknown-unknown
    find -L ~/.cargo/targets target -name 'hello.wasm' -path '*wasm32*' 2>/dev/null | grep -q .
  '';

  from-toolchain-file = cargoRun "from-toolchain-file" fromFile ''
    ${runHello}
    rustc --print target-list >/dev/null
    test -d "$(rustc --print sysroot)/lib/rustlib/wasm32-unknown-unknown"
  '';

  from-manifest = cargoRun "from-manifest" fromManifest ''
    rustc -V | grep -q "rustc 1.99.0 "
    ${runHello}
    cargo clippy --offline --quiet -- -D warnings
  '';

  # The stock-cargo sibling builds packages in the sandbox.
  rust-platform = nixCargo.default.rustPlatform.buildRustPackage {
    pname = "hello";
    version = "0.1.0";
    inherit src;
    cargoLock.lockFile = ./hello/Cargo.lock;
    doInstallCheck = true;
    installCheckPhase = ''
      test "$($out/bin/hello)" = "hello from nix-cargo"
    '';
  };

  inherit (nixCargo) cargo;

  # Built and unit-tested by the default toolchain's rustPlatform.
  inherit (nixCargo) cli;
}
// lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
  # Links through rustc's bundled rust-lld (gcc-ld/ld.lld), the default on
  # x86_64-linux and forced here so every Linux host covers it. The sandbox has
  # no /lib64, so the binary only runs if the ld wrapper gave it the store's
  # dynamic linker.
  rust-lld = cargoRun "rust-lld" nixCargo.nightly ''
    export RUSTFLAGS="-Zunstable-options -Clinker-features=+lld -Clink-self-contained=+linker"
    ${runHello}
    exe=$(cargo build --offline --quiet --message-format=json \
      | grep -o '"executable":"[^"]*"' | cut -d'"' -f4 | tail -1)
    # rust-lld did the link, not a fallback to the system linker...
    ${pkgs.binutils-unwrapped}/bin/readelf -p .comment "$exe" | grep -q 'Linker: LLD'
    # ...and the binary asks for the store's ld.so.
    ${pkgs.patchelf}/bin/patchelf --print-interpreter "$exe" | grep -q '^/nix/store/'
  '';
}
