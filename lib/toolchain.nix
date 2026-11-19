# The public toolchain API: a Rust channel's components from static.rust-lang.org,
# with huacnlee/cargo as their cargo.
#
# A toolchain is the components (lib/components.nix) joined with the fork's cargo.
# Each one carries:
#   .stock         the same components with the channel's own cargo; the fork
#                  pins its build directory under ~/.cargo/targets, which the
#                  Nix sandbox does not have
#   .rustPlatform  makeRustPlatform over .stock, for buildRustPackage
#   .rust-analyzer the channel's rust-analyzer, built for its rustc, or null
#   .rustSrcPath   the value for RUST_SRC_PATH
{
  lib,
  pkgs,
  # huacnlee/cargo, from package.nix.
  cargo,
}:
let
  inherit (lib)
    filter
    importJSON
    importTOML
    unique
    ;

  mkRust = pkgs.callPackage ./components.nix { };
  host = pkgs.stdenv.hostPlatform.rust.rustcTarget;

  defaultComponents = [
    "rustc"
    "rust-std"
    "rust-src"
    "clippy"
    "rustfmt"
    "llvm-tools"
  ];

  # Pinned by update.nu; no hash needed. `nightly` is the newest nightly that
  # ships every component on every supported host.
  pinned = {
    stable = importJSON ../data/stable.json;
    beta = importJSON ../data/beta.json;
    nightly = importJSON ../data/nightly.json;
  };

  # A rustup channel name to its manifest URL: "1.99", "1.99.0", "stable",
  # "nightly-2026-11-01", "beta-2026-11-14".
  manifestUrl =
    channel:
    let
      m = builtins.match "(stable|beta|nightly|[0-9]+\\.[0-9]+(\\.[0-9]+)?)(-([0-9]{4}-[0-9]{2}-[0-9]{2}))?" channel;
      name = builtins.elemAt m 0;
      date = builtins.elemAt m 3;
    in
    if m == null then
      throw "nix-cargo: \"${channel}\" is not a channel rustup understands"
    else
      "https://static.rust-lang.org/dist${
        lib.optionalString (date != null) "/${date}"
      }/channel-rust-${name}.toml";

  # An upstream channel-rust-*.toml carries rustc's version only as
  # "1.99.0 (b940084d7 2026-09-28)" under pkg.rustc; data/*.json has it trimmed
  # at the top level already.
  importManifest =
    file:
    let
      m = importTOML file;
    in
    m // { version = lib.head (lib.splitString " " m.pkg.rustc.version); };

  # `manifest` is a channel-rust-*.toml file, for example a `flake = false`
  # input, which `nix flake update` refreshes; `sha256` pins one fetched at
  # evaluation time. The pinned channels need neither.
  resolveManifest =
    {
      channel,
      manifest ? null,
      sha256 ? null,
    }:
    if manifest != null then
      importManifest manifest
    else if sha256 != null then
      importManifest (
        builtins.fetchurl {
          url = manifestUrl channel;
          inherit sha256;
        }
      )
    else
      pinned.${channel} or (throw ''
        nix-cargo: channel "${channel}" is not pinned (stable, beta, nightly), so it needs
        ${manifestUrl channel}
        as `manifest` (a `flake = false` input) or its `sha256` (nix-prefetch-url <url>).
      '');

  assemble =
    {
      manifest,
      components,
      targets,
    }:
    let
      wanted = unique (filter (c: c != "cargo") components);
      targets' = unique (filter (t: t != host) targets);
      # rust-analyzer links rustc's librustc_driver through a relative path, so
      # it has to share the components' prefix rather than get one of its own.
      hasAnalyzer = (manifest.pkg.rust-analyzer-preview.target or { }) ? ${host};
      base = mkRust {
        inherit manifest;
        name = "rust-components";
        components = unique (wanted ++ lib.optional hasAnalyzer "rust-analyzer");
        targets = targets';
      };
      ownCargo = mkRust {
        inherit manifest;
        name = "rust-cargo";
        components = [ "cargo" ];
      };
      withCargo =
        name: cargo':
        pkgs.symlinkJoin {
          name = "${name}-${base.version}";
          paths = [
            base
            cargo'
          ];
          passthru = {
            inherit (base) version;
            inherit manifest;
            components = wanted;
            targets = targets';
            # Read by buildRustPackage for its default meta.platforms, as on
            # nixpkgs' rustc. The target set is whatever rust-std was installed,
            # so no platform is ruled out up front.
            targetPlatforms = lib.platforms.all;
            tier1TargetPlatforms = lib.platforms.all;
            badTargetPlatforms = [ ];
          };
        };

      stock = withCargo "rust-stock" ownCargo;
      toolchain = withCargo "rust" cargo;
    in
    toolchain
    // {
      inherit stock base;
      rustPlatform = pkgs.makeRustPlatform {
        cargo = stock;
        rustc = stock;
      };
      # Just the binary, for adding to a shell or profile on its own.
      rust-analyzer =
        if hasAnalyzer then
          pkgs.runCommand "rust-analyzer-${base.version}"
            {
              meta.mainProgram = "rust-analyzer";
            }
            ''
              mkdir -p $out/bin
              ln -s ${base}/bin/rust-analyzer $out/bin/rust-analyzer
            ''
        else
          null;
      rustSrcPath = "${base}/lib/rustlib/src/rust/library";
    };

  mkToolchain =
    {
      channel ? "stable",
      components ? defaultComponents,
      targets ? [ ],
      manifest ? null,
      sha256 ? null,
    }:
    assemble {
      manifest = resolveManifest { inherit channel manifest sha256; };
      inherit components targets;
    };

  # Reads a rust-toolchain(.toml) file as rustup does: channel, profile,
  # components and targets. The legacy one-line format works too.
  fromToolchainFile =
    {
      file,
      manifest ? null,
      sha256 ? null,
    }:
    let
      text = builtins.readFile file;
      spec =
        if builtins.match ".*\\[toolchain].*" text != null then
          (importTOML file).toolchain
        else
          { channel = lib.trim text; };
      channel = spec.channel or (throw "nix-cargo: ${toString file} sets no toolchain.channel");
      manifest' = resolveManifest { inherit channel manifest sha256; };
      profile = manifest'.profiles.${spec.profile or "default"};
      onHost =
        c:
        let
          target = manifest'.pkg.${manifest'.renames.${c}.to or c}.target or { };
          src = target.${host} or target."*" or null;
        in
        src != null && src.available or true;
    in
    assert lib.assertMsg (!(spec ? path)) "nix-cargo: toolchain.path is not supported";
    assemble {
      manifest = manifest';
      # A profile names components some hosts lack (rust-mingw), so those are
      # skipped; a component the file itself names must exist.
      components = filter onHost profile ++ spec.components or [ ];
      targets = spec.targets or [ ];
    };
in
{
  inherit mkToolchain fromToolchainFile defaultComponents;
}
