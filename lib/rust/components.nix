# Rust components from a channel manifest, one derivation per tarball, and the
# sysroot that joins them.
#
# A component is keyed by its tarball alone, so every toolchain that uses the
# same rustc, rust-src or rust-std shares one copy of it in the store. A
# sysroot is then cheap: a symlink tree over its components, plus real copies
# of the few files whose own location matters (see `sysroot` below).
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  symlinkJoin,
  patchelf,
  zlib,
  path,
}:
let
  inherit (lib)
    concatStringsSep
    head
    optional
    optionalString
    splitString
    ;

  host = stdenv.hostPlatform.rust.rustcTarget;
  inherit (stdenv.cc) bintools;
  darwinSdkSetup = path + "/pkgs/build-support/wrapper-common/darwin-sdk-setup.bash";

  canonical = manifest: c: manifest.renames.${c}.to or c;

  # The tarball entry for one package and target, or null when the manifest
  # has none (an entry marked unavailable counts as none).
  sourceOf =
    manifest: pkg: target:
    let
      targets = manifest.pkg.${pkg}.target or { };
      src = targets.${target} or targets."*" or null;
    in
    if src == null || !(src.available or true) then null else src;

  versionOf = manifest: head (splitString " " manifest.version);

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
      # ld-wrapper.sh sources it unconditionally, Linux included.
      ${optionalString (builtins.pathExists darwinSdkSetup) ''
        substituteAll ${darwinSdkSetup} $out/nix-support/darwin-sdk-setup.bash
      ''}
      # The libc and dynamic linker defaults add-flags.sh reads, as the
      # system bintools wrapper has them, so a link through rust-lld gets the
      # store's ld.so and libc even when nothing else passes them.
      for file in dynamic-linker dynamic-linker-m32 ld-set-dynamic-linker libc-ldflags libc-ldflags-before; do
        if [ -e ${bintools}/nix-support/$file ]; then
          cp ${bintools}/nix-support/$file $out/nix-support/$file
        fi
      done

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

  # Read by the substituted ld wrapper, as the nixpkgs bintools wrapper sets them.
  ldWrapperEnv = {
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

  # One component tarball installed as a prefix.
  #
  # Host tools other than rustc (clippy, rustfmt, rust-analyzer, llvm-tools,
  # Cranelift) link rustc's librustc_driver or libLLVM, so rustc is their
  # input: on Linux autoPatchelf points them at it, and on macOS llvm-tools
  # gets the libLLVM it looks for one directory over.
  component =
    {
      manifest,
      pkg,
      target ? host,
    }:
    let
      src = sourceOf manifest pkg target;
      version = versionOf manifest;
      isRustc = pkg == "rustc";
      # Only these hold executables for this machine. rust-std for another
      # target holds that target's libraries, which must not be patched.
      runsHere = target == host && pkg != "rust-std" && pkg != "rust-src";
      rustc =
        if isRustc || !runsHere then
          null
        else
          component {
            inherit manifest;
            pkg = "rustc";
          };
      patchLinux = stdenv.hostPlatform.isLinux && (runsHere || pkg == "rust-std" && target == host);
    in
    assert lib.assertMsg (
      src != null
    ) "crucible: the ${manifest.version} (${manifest.date}) manifest has no ${pkg} for ${target}";
    stdenv.mkDerivation (
      {
        pname = "${pkg}-${target}";
        inherit version;

        src = fetchurl {
          url = src.xz_url or src.url;
          sha256 = src.xz_hash or src.hash;
        };

        dontConfigure = true;
        dontBuild = true;
        # Release builds already; stripping or shrinking their RUNPATHs would
        # only break what autoPatchelf sets.
        dontStrip = true;
        dontPatchELF = true;
        # rust-src would otherwise have its CI scripts rewritten, pulling bash
        # into the closure; the launchers in bin/ use /bin/sh, which every
        # host has.
        dontPatchShebangs = true;
        dontAutoPatchelf = !patchLinux;

        nativeBuildInputs = optional patchLinux autoPatchelfHook;
        buildInputs = lib.optionals patchLinux [
          zlib
          stdenv.cc.cc.lib
        ];

        # Each tarball lists its components; each component directory is a
        # tree rooted at the prefix, plus the installer's manifest.in.
        installPhase = ''
          runHook preInstall
          mkdir -p $out
          while read -r name; do
            cp -R "$name/." $out/
            rm -f $out/manifest.in
          done < components
          chmod -R u+w $out
          runHook postInstall
        '';

        postFixup =
          optionalString patchLinux ''
            ${optionalString (rustc != null) "addAutoPatchelfSearchPath ${rustc}/lib"}
            autoPatchelf -- $out
          ''
          + optionalString (stdenv.hostPlatform.isLinux && isRustc) wrapRustLld
          + optionalString (stdenv.hostPlatform.isDarwin && rustc != null) ''
            # llvm-tools look for @rpath/libLLVM.dylib in lib/rustlib/<host>/lib;
            # some releases ship it there, the others only in rustc's lib/.
            lib_llvm=$out/lib/rustlib/${host}/lib/libLLVM.dylib
            if [ -d $out/lib/rustlib/${host}/bin ] && [ ! -e $lib_llvm ] && [ -e ${rustc}/lib/libLLVM.dylib ]; then
              mkdir -p $out/lib/rustlib/${host}/lib
              ln -s ${rustc}/lib/libLLVM.dylib $lib_llvm
            fi
          '';

        passthru = {
          inherit pkg target;
        };

        meta = {
          description = "Rust ${manifest.version}: ${pkg} for ${target}";
          homepage = "https://www.rust-lang.org";
          license = with lib.licenses; [
            mit
            asl20
          ];
          sourceProvenance = with lib.sourceTypes; [ binaryNativeCode ];
          platforms = import ../common/systems.nix;
        };
      }
      // lib.optionalAttrs (stdenv.hostPlatform.isLinux && isRustc) { env = ldWrapperEnv; }
    );

  # The components joined into one sysroot.
  #
  # Two kinds of file must be real rather than links into a component:
  # - librustc_driver: rustc (and rustdoc, clippy, rust-analyzer) takes the
  #   sysroot from the directory the driver library is loaded from, and the
  #   sysroot is what gathers rust-std, rust-src and the tools together.
  # - bin/*: the tools find their libraries (and clippy-driver its siblings)
  #   relative to their own path, which is this tree only for a real file.
  sysroot =
    {
      manifest,
      components,
      targets ? [ ],
      name ? "rust-components",
    }:
    let
      hostParts = map (
        c:
        component {
          inherit manifest;
          pkg = canonical manifest c;
        }
      ) components;
      stdParts = map (
        target:
        component {
          inherit manifest target;
          pkg = "rust-std";
        }
      ) targets;
    in
    symlinkJoin {
      name = "${name}-${versionOf manifest}";
      paths = hostParts ++ stdParts;
      nativeBuildInputs = optional stdenv.hostPlatform.isLinux patchelf;
      postBuild = ''
        for file in $out/bin/* $out/lib/librustc_driver-*; do
          if [ -L "$file" ]; then
            # Store files are read-only; the Linux pass below rewrites these.
            cp --remove-destination "$(realpath "$file")" "$file"
            chmod u+w "$file"
          fi
        done
      ''
      # On macOS the copies find the driver through @loader_path/../lib, which
      # now is this tree. On Linux autoPatchelf gave them absolute RUNPATHs
      # into the rustc component, so this tree's lib/ goes first.
      + optionalString stdenv.hostPlatform.isLinux ''
        for file in $out/bin/*; do
          if isELF "$file"; then
            patchelf --set-rpath "$out/lib:$(patchelf --print-rpath "$file")" "$file"
          fi
        done
      '';
      passthru = {
        inherit manifest components targets;
        version = versionOf manifest;
      };
      meta.description = "Rust ${manifest.version}: ${
        concatStringsSep ", " (components ++ map (t: "rust-std for ${t}") targets)
      }";
    };

  # Whether the manifest ships a component (by either name) for this host.
  hasComponent = manifest: c: sourceOf manifest (canonical manifest c) host != null;
in
{
  inherit
    component
    sysroot
    hasComponent
    host
    ;
}
