# The public toolchain API: a Rust channel's components from static.rust-lang.org,
# with huacnlee/cargo as their cargo.
#
# A toolchain is a sysroot of the channel's components (lib/components.nix),
# joined with the fork's cargo.
# Each one carries:
#   .stock         the same components with the channel's own cargo; the fork
#                  pins its build directory under ~/.cargo/targets, which the
#                  Nix sandbox does not have
#   .rustPlatform  makeRustPlatform over .stock, for buildRustPackage
#   .rust-analyzer the channel's rust-analyzer, built for its rustc, or null
#                  (the channel has none, or it was excluded)
#   .rustSrcPath   the value for RUST_SRC_PATH (null without rust-src), which
#                  the toolchain's setup hook also exports in shells, replacing
#                  any inherited value
{
  lib,
  pkgs,
  # huacnlee/cargo, from cargo.nix.
  cargo,
}:
let
  inherit (lib)
    filter
    importJSON
    importTOML
    unique
    ;

  rust = pkgs.callPackage ./components.nix { };

  defaultComponents = [
    "rustc"
    "rust-std"
    "rust-src"
    "clippy"
    "rustfmt"
    "llvm-tools"
  ];

  # Pinned by `crucible update`; no hash needed. `nightly` is the newest nightly that
  # ships every component on every supported host.
  pinned = {
    stable = importJSON ../../data/rust/stable.json;
    beta = importJSON ../../data/rust/beta.json;
    nightly = importJSON ../../data/rust/nightly.json;
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
      throw "crucible: \"${channel}\" is not a channel rustup understands"
    else
      "https://static.rust-lang.org/dist${
        lib.optionalString (date != null) "/${date}"
      }/channel-rust-${name}.toml";

  # An upstream channel-rust-*.toml carries rustc's version only as
  # "1.99.0 (b940084d7 2026-09-28)" under pkg.rustc; data/rust/*.json has it trimmed
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
        crucible: channel "${channel}" is not pinned (stable, beta, nightly), so it needs
        ${manifestUrl channel}
        as `manifest` (a `flake = false` input) or its `sha256` (nix-prefetch-url <url>).
      '');

  # Components a toolchain cannot work without. cargo is not among them: the
  # fork is always added, and `.stock` swaps in the channel's own.
  required = [
    "rustc"
    "rust-std"
  ];

  assemble =
    {
      manifest,
      components,
      targets,
      exclude,
    }:
    # The components here run on the build machine, so a cross package set
    # (pkgsCross.*) would select binaries for the wrong one.
    assert lib.assertMsg (pkgs.stdenv.buildPlatform.config == pkgs.stdenv.hostPlatform.config) ''
      crucible: toolchains are built for the machine that runs them. For ${pkgs.stdenv.hostPlatform.config},
      use the native package set's toolchain with `targets = [ "${pkgs.stdenv.hostPlatform.rust.rustcTarget}" ]`.
    '';
    let
      # Compared by the manifest's own names, so `exclude = [ "clippy" ]`
      # also drops a "clippy-preview" that a profile listed.
      excluded = map (rust.canonical manifest) exclude;
      isExcluded = c: builtins.elem (rust.canonical manifest c) excluded;
      wanted = unique (filter (c: c != "cargo" && !isExcluded c) components);
      missing = filter (r: !(builtins.any (c: rust.canonical manifest c == r) wanted)) required;
      targets' = unique (filter (t: t != rust.host) targets);
      # rust-analyzer loads librustc_driver, which it only finds inside a
      # sysroot, so when the channel has it, it is part of every sysroot
      # unless excluded.
      hasAnalyzer = rust.hasComponent manifest "rust-analyzer" && !isExcluded "rust-analyzer";
      hasSrc = builtins.elem "rust-src" wanted;
      base = rust.sysroot {
        inherit manifest;
        components = unique (wanted ++ lib.optional hasAnalyzer "rust-analyzer");
        targets = targets';
      };
      rustSrcPath = "${base}/lib/rustlib/src/rust/library";

      withCargo =
        name: cargo':
        pkgs.symlinkJoin {
          name = "${name}-${base.version}";
          paths = [
            base
            cargo'
          ];
          # rust-analyzer would find these sources through `rustc --print
          # sysroot` on its own, but it reads RUST_SRC_PATH first, so a value
          # inherited from the login shell (another release, or a collected
          # store path) would win over the shell's own rustc. Overwriting it
          # makes the toolchain authoritative; a shellHook still runs after
          # setup hooks if a project really wants something else.
          postBuild = lib.optionalString hasSrc ''
            mkdir -p $out/nix-support
            echo 'export RUST_SRC_PATH="${rustSrcPath}"' > $out/nix-support/setup-hook
          '';
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

      stock = withCargo "rust-stock" (
        rust.component {
          inherit manifest;
          pkg = "cargo";
        }
      );
      toolchain = withCargo "rust" cargo;
    in
    assert lib.assertMsg (!(builtins.elem "cargo" exclude)) ''
      crucible: cargo cannot be excluded: the toolchain's cargo is huacnlee/cargo, and
      `.stock` is the same toolchain with the channel's own cargo.
    '';
    assert lib.assertMsg (missing == [ ]) ''
      crucible: a toolchain needs ${lib.concatStringsSep " and " missing}, which ${
        if lib.length missing == 1 then "is" else "are"
      } missing from its components or excluded.
    '';
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
      rustSrcPath = if hasSrc then rustSrcPath else null;
    };

  toolchain =
    {
      channel ? "stable",
      components ? defaultComponents,
      targets ? [ ],
      exclude ? [ ],
      manifest ? null,
      sha256 ? null,
    }:
    assemble {
      manifest = resolveManifest { inherit channel manifest sha256; };
      inherit components targets exclude;
    };

  # Reads a rust-toolchain(.toml) file as rustup does: channel, profile,
  # components and targets. The legacy one-line format works too. `exclude`
  # removes components the file's profile brings in, which rustup has no
  # syntax for.
  fromToolchainFile =
    {
      file,
      exclude ? [ ],
      manifest ? null,
      sha256 ? null,
    }:
    let
      name = toString file;
      text = lib.trim (builtins.readFile file);
      # rustup reads `rust-toolchain.toml` as TOML, and `rust-toolchain` as TOML
      # unless it is the legacy single line naming a channel.
      legacy = !(lib.hasSuffix ".toml" name) && builtins.match "[^]=\n[]+" text != null;
      spec = if legacy then { channel = text; } else (builtins.fromTOML text).toolchain or { };
      channel = spec.channel or (throw "crucible: ${name} sets no toolchain.channel");
      manifest' = resolveManifest { inherit channel manifest sha256; };
      profileName = spec.profile or "default";
      profile =
        manifest'.profiles.${profileName} or (throw ''
          crucible: ${name} asks for profile "${profileName}"; the ${manifest'.version} manifest has ${lib.concatStringsSep ", " (builtins.attrNames manifest'.profiles)}.
        '');
    in
    assert lib.assertMsg (!(spec ? path)) "crucible: toolchain.path is not supported";
    assemble {
      manifest = manifest';
      # A profile names components some hosts lack (rust-mingw), so those are
      # skipped; a component the file itself names must exist.
      components = filter (rust.hasComponent manifest') profile ++ spec.components or [ ];
      targets = spec.targets or [ ];
      inherit exclude;
    };
in
{
  inherit toolchain fromToolchainFile defaultComponents;
}
