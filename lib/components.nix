# Installs Rust components from a channel manifest the way rustup does: every
# component's tarball unpacked into one prefix. The official binaries find their
# libraries through relative paths ($ORIGIN/../lib, @loader_path/../lib), so one
# prefix is all macOS needs and Linux only needs its interpreter and libraries
# patched in.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  zlib,
  path,
}:
let
  inherit (lib)
    concatStringsSep
    head
    optionalString
    splitString
    ;

  host = stdenv.hostPlatform.rust.rustcTarget;
  inherit (stdenv.cc) bintools;
in
{
  # A channel manifest: data/<channel>.json, or a channel-rust-*.toml read with importTOML.
  manifest,
  # Components by manifest or short name ("clippy" for "clippy-preview").
  components,
  # Extra targets whose rust-std is installed next to the host's.
  targets ? [ ],
  name ? "rust",
}:
let
  canonical = c: manifest.renames.${c}.to or c;

  # The tarball of one package for one target, or null when the manifest has
  # none (an entry the manifest marks unavailable counts as none).
  source =
    pkg: target:
    let
      targets' = manifest.pkg.${pkg}.target or { };
      src = targets'.${target} or targets'."*" or null;
    in
    if src == null || !(src.available or true) then
      null
    else
      fetchurl {
        url = src.xz_url or src.url;
        sha256 = src.xz_hash or src.hash;
      };

  has = pkg: target: source pkg target != null;

  requested = map (c: {
    inherit c;
    pkg = canonical c;
    target = host;
  }) components;

  stdFor = map (target: {
    c = "rust-std for ${target}";
    pkg = "rust-std";
    inherit target;
  }) targets;

  wanted = requested ++ stdFor;
  missing = builtins.filter (w: !(has w.pkg w.target)) wanted;

  srcs =
    assert lib.assertMsg (missing == [ ])
      "nix-cargo: the ${manifest.version} (${manifest.date}) manifest has no ${
        concatStringsSep ", " (map (w: w.c) missing)
      } for ${host}.";
    map (w: source w.pkg w.target) wanted;

  version = head (splitString " " manifest.version);

  # The x86_64-unknown-linux-gnu target links through the bundled rust-lld
  # (`gcc-ld/ld.lld`), which bypasses the nixpkgs ld wrapper: the binaries it
  # links get the FHS dynamic linker and no RUNPATH for store libraries. Route
  # it through that wrapper, keeping rust-lld behind it.
  wrapRustLld = ''
    wrap_ld() {
      local dst="$1" wrapper="$2"
      export prog="$3"
      substituteAll "$wrapper" "$dst"
      chmod +x "$dst"
    }

    shopt -s nullglob
    lds=( $out/lib/rustlib/*/bin/gcc-ld/ld.lld )
    shopt -u nullglob
    if [ ''${#lds[@]} -gt 0 ]; then
      mkdir -p $out/nix-support
      substituteAll ${
        path + "/pkgs/build-support/wrapper-common/utils.bash"
      } $out/nix-support/utils.bash
      substituteAll ${
        path + "/pkgs/build-support/bintools-wrapper/add-flags.sh"
      } $out/nix-support/add-flags.sh
      substituteAll ${
        path + "/pkgs/build-support/bintools-wrapper/add-hardening.sh"
      } $out/nix-support/add-hardening.sh

      for ld in "''${lds[@]}"; do
        # ld.lld finds rust-lld relative to itself and takes its flavor from
        # its name, so the original keeps both in a sibling directory.
        unwrapped="$(dirname "$ld")-unwrapped/ld.lld"
        mkdir -p "$(dirname "$unwrapped")"
        mv "$ld" "$unwrapped"
        wrap_ld "$ld" ${path + "/pkgs/build-support/bintools-wrapper/ld-wrapper.sh"} "$unwrapped"
      done
    fi
  '';

  # llvm-tools ships its binaries in lib/rustlib/<host>/bin, linked against
  # @rpath/libLLVM.dylib one directory over, which only rustc's lib/ has.
  linkDarwinLLVM = ''
    if [ -e $out/lib/libLLVM.dylib ]; then
      for dir in $out/lib/rustlib/*-apple-darwin; do
        if [ -d "$dir/bin" ] && [ ! -e "$dir/lib/libLLVM.dylib" ]; then
          mkdir -p "$dir/lib"
          ln -s $out/lib/libLLVM.dylib "$dir/lib/libLLVM.dylib"
        fi
      done
    fi
  '';
in
stdenv.mkDerivation {
  pname = name;
  inherit version srcs;

  dontUnpack = true;
  dontConfigure = true;
  dontBuild = true;
  # The binaries are release builds already; stripping or shrinking their
  # RUNPATHs would only break what autoPatchelf sets.
  dontStrip = true;
  dontPatchELF = true;
  # rust-src would otherwise have its CI scripts rewritten, pulling bash into
  # the closure; the few launchers in bin/ use /bin/sh, which every host has.
  dontPatchShebangs = true;
  # rust-std of other Linux targets holds foreign-architecture ELF files, so
  # patching is limited to the host's own files below.
  dontAutoPatchelf = true;

  nativeBuildInputs = lib.optionals stdenv.hostPlatform.isLinux [ autoPatchelfHook ];
  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
    zlib
    stdenv.cc.cc.lib
  ];

  installPhase = ''
    runHook preInstall
    mkdir -p $out

    for src in $srcs; do
      work=$(mktemp -d)
      tar xf "$src" -C "$work"
      top=$(echo "$work"/*)
      # Each tarball lists its components; each component directory is a tree
      # rooted at the prefix, plus the installer's manifest.in.
      while read -r component; do
        cp -R "$top/$component/." $out/
        rm -f $out/manifest.in
      done < "$top/components"
      rm -rf "$work"
    done
    chmod -R u+w $out

    runHook postInstall
  '';

  postFixup =
    optionalString stdenv.hostPlatform.isLinux ''
      patch_paths=()
      for p in $out/bin $out/libexec $out/lib/rustlib/${host}; do
        [ -e "$p" ] && patch_paths+=("$p")
      done
      for lib in $out/lib/*.so*; do
        [ -e "$lib" ] && patch_paths+=("$lib")
      done
      addAutoPatchelfSearchPath --no-recurse $out/lib
      autoPatchelf -- "''${patch_paths[@]}"

      ${wrapRustLld}
    ''
    + optionalString stdenv.hostPlatform.isDarwin linkDarwinLLVM;

  # Read by the substituted ld wrapper, as the nixpkgs bintools wrapper sets them.
  env = lib.optionalAttrs stdenv.hostPlatform.isLinux {
    inherit (bintools)
      coreutils_bin
      expandResponseParams
      shell
      wrapperName
      ;
    mktemp = bintools.mktemp or "mktemp";
    rm = bintools.rm or "rm";
    suffixSalt = lib.replaceStrings [ "-" "." ] [ "_" "_" ] stdenv.targetPlatform.config;
    hardening_unsupported_flags = "";
    darwinMinVersion = "";
    fallback_sdk = "";
    use_response_file_by_default = "0";
  };

  passthru = {
    inherit manifest components targets;
    channelDate = manifest.date;
  };

  meta = {
    description = "Rust ${manifest.version}: ${concatStringsSep ", " (map (w: w.c) wanted)}";
    homepage = "https://www.rust-lang.org";
    license = with lib.licenses; [
      mit
      asl20
    ];
    sourceProvenance = with lib.sourceTypes; [ binaryNativeCode ];
    platforms = import ./systems.nix;
  };
}
