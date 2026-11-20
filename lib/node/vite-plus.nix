# Vite+ (vp): the native CLI from its GitHub release, next to the JavaScript
# packages it runs, which vp's own installer would otherwise download into the
# user's data directory on first use.
#
# The packages are laid out the way pnpm's isolated linker does it, from the
# release's own lockfile (data/node/vite-plus.json): each one in
# node_modules/.pnpm/<id>/node_modules/<name>, with its dependencies as
# symlinks beside it. Every tarball is fetched by the registry's integrity
# hash, so there is no fixed-output hash to learn by building.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  runCommand,
  writeText,
  coreutils,
}:
let
  inherit (stdenv.hostPlatform) system isLinux;
  pins = lib.importJSON ../../data/node/vite-plus.json;
  inherit (pins) version;

  source =
    pins.platforms.${system} or (throw "crucible: Vite+ ${version} has no build for ${system}.");

  packages = lib.filterAttrs (_: p: !(p ? systems) || builtins.elem system p.systems) pins.packages;

  # A dependency link at .pnpm/<id>/node_modules/<link> points back up to
  # .pnpm: past node_modules, <id>, and the scope of a scoped name.
  linkTarget =
    link: target:
    lib.concatStrings (lib.replicate (1 + lib.length (lib.splitString "/" link)) "../")
    + "${target}/node_modules/${packages.${target}.name}";

  install =
    id: p:
    let
      dir = "$out/node_modules/.pnpm/${id}/node_modules";
    in
    ''
      mkdir -p "${dir}/${p.name}"
      tar xzf ${fetchurl { inherit (p) url hash; }} --strip-components=1 --delay-directory-restore \
        -C "${dir}/${p.name}"
    ''
    # Optional dependencies for other systems are not installed here.
    + lib.concatStrings (
      lib.mapAttrsToList (link: target: ''
        mkdir -p "$(dirname "${dir}/${link}")"
        ln -s "${linkTarget link target}" "${dir}/${link}"
      '') (lib.filterAttrs (_: target: packages ? ${target}) (p.dependencies or { }))
    );

  unwrapped = stdenv.mkDerivation {
    pname = "vite-plus-unwrapped";
    inherit version;

    src = fetchurl { inherit (source) url hash; };
    # The archive has no top-level directory.
    unpackPhase = ''
      mkdir vp
      tar xzf $src -C vp
      cd vp
    '';

    nativeBuildInputs = lib.optionals isLinux [ autoPatchelfHook ];
    # The Linux vp and the native addons link libgcc_s and libstdc++.
    buildInputs = lib.optionals isLinux [ stdenv.cc.cc.lib ];

    installPhase = ''
      runHook preInstall

      # vp looks for its sidecars (sync-versions/bin.mjs, toolchain.json)
      # next to itself, so the archive goes in whole.
      mkdir -p $out/bin
      cp -R . $out/bin/
      chmod 755 $out/bin/vp
      ${lib.concatStrings (lib.mapAttrsToList install packages)}
      ln -s .pnpm/${pins.root}/node_modules/vite-plus $out/node_modules/vite-plus
      # Some tarballs record directories without the execute bit (hence the
      # delayed restore above), which would make them unreadable.
      chmod -R u+w,a+rX $out/node_modules

      # `vp create` copies its templates with copyFileSync, which keeps the
      # store's read-only mode, so the new project's files could not be edited.
      substituteInPlace $out/node_modules/vite-plus/dist/create/bin.js \
        --replace-fail \
          'else fs.copyFileSync(src, dest);' \
          'else { fs.copyFileSync(src, dest); fs.chmodSync(dest, fs.statSync(dest).mode | 0o200); }'

      # Without this marker beside the binary, vp's first run treats the store
      # path as a fresh download and installs itself: shims under
      # ~/.local/share/vite-plus and PATH entries in every shell profile, all
      # pointing into a store path that garbage collection later removes.
      touch $out/bin/.vp-setup-complete

      runHook postInstall
    '';

    # The packages' `#!/usr/bin/env node` scripts must keep finding the node
    # of whichever shell runs them, not one fixed here.
    dontPatchShebangs = true;
    dontStrip = true;

    meta = {
      description = "Vite+, the unified toolchain for the web (without a Node.js)";
      homepage = "https://viteplus.dev";
      license = lib.licenses.mit;
      sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
      mainProgram = "vp";
      platforms = import ../common/systems.nix;
    };
  };
in
{
  inherit unwrapped;
  inherit (pins) engines;

  # vp for one Node.js, which it takes from PATH, after the shell's own.
  #
  # vp otherwise downloads a Node.js and package managers per project (from
  # .node-version or engines) into its data directory. VP_ENV_FORBIDDEN=1 stops
  # that, but vp 1.7 only runs a Node.js from PATH when its config.json says
  # "system_first" (what `vp env off` writes, along with shims), and reads
  # that file only from VP_HOME or the user's config directory. So vp gets a
  # root of its own, apart from any standalone vp install, seeded with that
  # config on first run.
  withNode =
    node:
    let
      systemFirst = writeText "vite-plus-config.json" (
        builtins.toJSON {
          nodeShimMode = "system_first";
          packageManagerShimModes = lib.genAttrs [ "bun" "npm" "pnpm" "yarn" ] (_: "system_first");
        }
      );
    in
    runCommand "vite-plus-${version}"
      {
        nativeBuildInputs = [ makeWrapper ];
        passthru = { inherit unwrapped; };
        meta = unwrapped.meta // {
          description = "Vite+, the unified toolchain for the web";
        };
      }
      ''
        # vp finds its packages from its own location, so it is wrapped in
        # place rather than linked.
        makeWrapper ${unwrapped}/bin/vp $out/bin/vp \
          --set-default VP_ENV_FORBIDDEN 1 \
          --run 'export VP_HOME="''${VP_HOME:-''${XDG_DATA_HOME:-$HOME/.local/share}/crucible/vite-plus}"' \
          --run '[ -e "$VP_HOME/config.json" ] || { ${coreutils}/bin/mkdir -p "$VP_HOME" && ${coreutils}/bin/cat ${systemFirst} > "$VP_HOME/config.json"; } 2>/dev/null || true' \
          --suffix PATH : ${lib.makeBinPath [ node ]}
        # vpr and vpx are vp under another name; the wrapper keeps argv[0].
        ln -s vp $out/bin/vpr
        ln -s vp $out/bin/vpx
      '';
}
