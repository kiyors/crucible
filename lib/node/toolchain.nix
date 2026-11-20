# The public toolchain API: an official Node.js release from nodejs.org, with
# Vite+ (vp) beside it.
#
# A toolchain joins the Node.js (node.nix), vp for that Node.js
# (vite-plus.nix) and, when asked for, a package manager.
# Each one carries:
#   .node            the Node.js alone: node, npm, npx (and corepack before 25)
#   .vite-plus       vp for this Node.js, or null (vitePlus = false)
#   .packageManager  the package manager from packageManager, or null
#   .buildNpmPackage nixpkgs' buildNpmPackage on this Node.js, for package builds
# and a setup hook that keeps vp on the shell's Node.js (see vite-plus.nix).
{
  lib,
  pkgs,
}:
let
  inherit (builtins)
    attrValues
    elemAt
    filter
    head
    match
    readFile
    ;

  semver = import ./semver.nix { inherit lib; };
  nodeFor = pkgs.callPackage ./node.nix { };
  vitePlusPkgs = pkgs.callPackage ./vite-plus.nix { };

  # Pinned by `crucible update`: the newest release of every supported line,
  # and the exact package-manager release kept for each currently-relevant
  # major.
  pinned = lib.importJSON ../../data/node/node.json;
  pinnedVersions = map (r: r.version) (attrValues pinned.releases);
  pinnedPackageManagers = lib.importJSON ../../data/node/package-managers.json;

  # Nix system -> nodejs.org platform, as `PLATFORMS` in crates/crucible/src/update/node.rs.
  platforms = {
    x86_64-linux = "linux-x64";
    aarch64-linux = "linux-arm64";
    aarch64-darwin = "darwin-arm64";
  };

  # Must match the URL `crucible pin node` prints, in crates/crucible/src/pin.rs.
  shasumsUrl = version: "https://nodejs.org/dist/v${version}/SHASUMS256.txt";

  exact = spec: match "v?([0-9]+\\.[0-9]+\\.[0-9]+)" spec;

  # A release from its SHASUMS256.txt, which names the version in every
  # tarball and holds their hashes.
  fromShasums =
    file:
    let
      lines = lib.splitString "\n" (readFile file);
      entries = filter (m: m != null) (
        map (match "([0-9a-f]{64})  (node-v([0-9.]+)-([a-z0-9-]+)\\.tar\\.xz)") lines
      );
      version =
        if entries == [ ] then
          throw "crucible: ${toString file} lists no node-v*.tar.xz; is it a Node.js SHASUMS256.txt?"
        else
          elemAt (head entries) 2;
      tarball =
        platform:
        lib.findFirst (m: elemAt m 3 == platform) (throw ''
          crucible: ${toString file} has no node-v${version}-${platform}.tar.xz.
        '') entries;
    in
    {
      inherit version;
      platforms = lib.mapAttrs (
        _: platform:
        let
          m = tarball platform;
        in
        {
          url = "https://nodejs.org/dist/v${version}/${elemAt m 1}";
          hash = "sha256:${elemAt m 0}";
        }
      ) platforms;
    };

  notPinned = spec: ''
    crucible: Node.js "${spec}" matches no pinned release (${lib.concatStringsSep ", " pinnedVersions}).
    For another release, pass its SHASUMS256.txt as `shasums` (a `flake = false` input) or its
    `sha256`; `nix run github:kiyors/crucible#crucible -- pin node <version>` prints both.
  '';

  # `version` is "lts", "current", "lts/<codename>", a line ("24"), an npm
  # range ("^22.12.0 || >=24") or an exact release. Pinned releases need
  # nothing else; any other exact release needs its SHASUMS256.txt.
  resolve =
    {
      version,
      shasums,
      sha256,
    }:
    let
      spec = lib.trim version;
      codename = match "lts/(.+)" spec;
      byVersion = v: lib.findFirst (r: r.version == v) null (attrValues pinned.releases);
      file =
        if shasums != null then
          shasums
        else if exact spec != null then
          builtins.fetchurl {
            url = shasumsUrl (head (exact spec));
            inherit sha256;
          }
        else
          throw ''
            crucible: `sha256` pins one exact Node.js release, so `version` must be one
            (such as "22.11.0"), not "${spec}".
          '';
      unpinned = fromShasums file;
    in
    if shasums != null || sha256 != null then
      if spec == "lts" || semver.satisfies unpinned.version spec then
        unpinned
      else
        throw ''
          crucible: the SHASUMS256.txt given is for Node.js ${unpinned.version}, which "${spec}" does not allow.
        ''
    else if spec == "lts" || spec == "lts/*" then
      pinned.releases.${pinned.lts}
    else if spec == "current" || spec == "latest" || spec == "node" then
      pinned.releases.${pinned.current}
    else if codename != null then
      lib.findFirst (r: lib.toLower (r.lts or "") == lib.toLower (head codename)) (throw (
        notPinned spec
      )) (attrValues pinned.releases)
    else if exact spec != null then
      if byVersion (head (exact spec)) != null then
        byVersion (head (exact spec))
      else
        throw (notPinned spec)
    else
      let
        newest = semver.newest spec pinnedVersions;
      in
      if newest == null then throw (notPinned spec) else byVersion newest;

  # The package manager a toolchain asks for: a spec such as "pnpm@12" or a
  # range such as "pnpm@^10", or a package with the manager's binary for an
  # exact release crucible does not pin. A major with a pin gets that exact
  # release from npm; otherwise nixpkgs' one-release-per-major build is used,
  # but only when its release is inside the requested range. npm ships with
  # Node.js.
  #
  # Matching the range rather than the major alone matters: package.json pins
  # an exact release, and silently substituting the newest release of its major
  # makes the shell disagree with package.json and `crucible doctor`.
  packageManagerFor =
    node: packageManager:
    if lib.isDerivation packageManager then
      packageManager
    else
      let
        spec = packageManager;
        m = match "(npm|pnpm|yarn|bun)@(.+)" (lib.trim spec);
        name = if m == null then null else elemAt m 0;
        # Corepack may append +sha512.<digest> to packageManager; it is an
        # integrity suffix, not part of the version range.
        requested = if m == null then null else head (lib.splitString "+" (lib.trim (elemAt m 1)));
        majorMatch = if requested == null then null else match "[\\^~>=v ]*([0-9]+).*" requested;
        major = if majorMatch == null then null else head majorMatch;
        exact = if requested == null then null else match "v?([0-9]+\\.[0-9]+\\.[0-9]+)" requested;
        pin =
          if name == null || major == null then null else pinnedPackageManagers.${name}.${major} or null;
        attr =
          if name == "pnpm" then
            "pnpm_${major}"
          else if name == "yarn" then
            (if major == "1" then "yarn" else "yarn-berry_${major}")
          else
            "bun";
        nixpkgsPackage = pkgs.${attr} or null;
        onNode =
          package:
          let
            args = lib.functionArgs (package.override or (_: { }));
          in
          if args ? nodejs-slim then
            package.override { nodejs-slim = node; }
          else if args ? nodejs then
            package.override { nodejs = node; }
          else
            package;
        # A pin is usable when it is the exact release requested, or when the
        # request is a range that contains it. A pin for another release of the
        # same major is not silently accepted.
        pinMatches =
          pin != null
          && (exact == null || head exact == pin.version)
          && semver.satisfies pin.version requested;
        nixpkgsMatches =
          nixpkgsPackage != null
          && (name != "bun" || lib.versions.major nixpkgsPackage.version == major)
          && semver.satisfies nixpkgsPackage.version requested;
        chosen =
          if pinMatches then
            pkgs.callPackage ./package-manager.nix {
              nodejs = node;
              inherit name major;
              inherit pin;
            }
          else if nixpkgsMatches then
            onNode nixpkgsPackage
          else
            null;
        whatIsAvailable =
          (
            if pin != null then
              "crucible pins ${name} ${pin.version}"
            else
              "crucible has no ${name} ${major} pin"
          )
          + lib.optionalString (nixpkgsPackage != null) " and nixpkgs has ${nixpkgsPackage.version}";
      in
      if m == null then
        throw ''
          crucible: packageManager "${spec}" is not <npm|pnpm|yarn|bun>@<version>.
        ''
      # npm ships with Node.js.
      else if name == "npm" then
        null
      else if chosen != null then
        chosen
      else
        throw ''
          crucible: packageManager "${spec}" asks for ${name} ${requested}, but ${whatIsAvailable}.
          Use a major or range that crucible or nixpkgs satisfies, or pass a package built
          from the release you need as `packageManager`.
        '';

  assemble =
    {
      release,
      vitePlus ? true,
      packageManager ? null,
    }:
    # node runs on the build machine, so a cross package set (pkgsCross.*)
    # would select a binary for the wrong one.
    assert lib.assertMsg (pkgs.stdenv.buildPlatform.config == pkgs.stdenv.hostPlatform.config) ''
      crucible: Node.js toolchains are built for the machine that runs them; take one from the
      native package set.
    '';
    let
      node = nodeFor release;
      vp = if vitePlus then vitePlusPkgs.withNode node else null;
      pm = if packageManager == null then null else packageManagerFor node packageManager;
    in
    assert lib.assertMsg (!vitePlus || semver.satisfies node.version vitePlusPkgs.engines.node) ''
      crucible: Vite+ needs Node.js ${vitePlusPkgs.engines.node}, and this toolchain has ${node.version}.
      Pick another `version`, or pass `vitePlus = false`.
    '';
    pkgs.symlinkJoin {
      name = "node-toolchain-${node.version}";
      paths = [ node ] ++ lib.optional (vp != null) vp ++ lib.optional (pm != null) pm;
      # Overwritten, not defaulted: a VP_ENV_FORBIDDEN=0 from the login shell
      # would let vp run a Node.js it downloaded for the project, and a VP_HOME
      # from a standalone vp install would bring that install's "managed"
      # config, instead of this toolchain's Node.js. A shellHook still runs
      # after setup hooks.
      postBuild = lib.optionalString (vp != null) ''
        mkdir -p $out/nix-support
        cat > $out/nix-support/setup-hook <<'EOF'
        export VP_ENV_FORBIDDEN=1
        unset VP_HOME
        EOF
      '';
      passthru = {
        inherit (node) version;
        inherit node;
        vite-plus = vp;
        packageManager = pm;
        buildNpmPackage = pkgs.buildNpmPackage.override { nodejs = node; };
      };
      meta = {
        description = "Node.js ${node.version}${lib.optionalString (vp != null) " with Vite+"}";
        mainProgram = "node";
        inherit (node.meta) platforms;
      };
    };

  toolchain =
    {
      version ? "lts",
      vitePlus ? true,
      packageManager ? null,
      shasums ? null,
      sha256 ? null,
    }:
    assemble {
      release = resolve { inherit version shasums sha256; };
      inherit vitePlus packageManager;
    };

  # Reads the Node.js version and package manager a project asks for:
  # devEngines.runtime (npm's newer field), then the project's .node-version or
  # .nvmrc, then engines.node, then the pinned LTS; and packageManager, then
  # devEngines.packageManager, as corepack does. The version files are the
  # explicit pins a version manager (nvm, nodenv, asdf, volta) reads, so they
  # beat engines.node's advisory range. Nothing here touches the network.
  fromPackageJson =
    {
      file,
      vitePlus ? true,
      # Override the file's packageManager, for a project whose exact release
      # is not pinned: pass a package built from it.
      packageManager ? null,
      shasums ? null,
      sha256 ? null,
    }:
    let
      json = lib.importJSON file;
      dir = builtins.dirOf (toString file);
      # .node-version is the asdf/nodenv/volta name and takes precedence over
      # nvm's .nvmrc when both are present.
      versionFileName = lib.findFirst (name: builtins.pathExists (dir + "/${name}")) null [
        ".node-version"
        ".nvmrc"
      ];
      versionFileLines =
        if versionFileName == null then
          [ ]
        else
          filter (line: line != "" && !lib.hasPrefix "#" line) (
            map lib.trim (lib.splitString "\n" (readFile (dir + "/${versionFileName}")))
          );
      fileVersion = if versionFileLines == [ ] then null else head versionFileLines;
      # devEngines fields are one { name, version } or a list of them.
      named =
        name: field: lib.findFirst (e: (e.name or null) == name && e ? version) null (lib.toList field);
      dev = json.devEngines or { };
      runtime = named "node" (dev.runtime or [ ]);
      devManager = lib.findFirst (e: e ? name && e ? version) null (
        lib.toList (dev.packageManager or [ ])
      );
      version =
        if runtime != null then
          runtime.version
        else if fileVersion != null then
          fileVersion
        else
          json.engines.node or "lts";
      packageManagerFromJson =
        json.packageManager
          or (if devManager != null then "${devManager.name}@${devManager.version}" else null);
      packageManager' = if packageManager != null then packageManager else packageManagerFromJson;
    in
    toolchain {
      inherit
        version
        vitePlus
        shasums
        sha256
        ;
      packageManager = packageManager';
    };
in
{
  inherit toolchain fromPackageJson;
  vite-plus-unwrapped = vitePlusPkgs.unwrapped;
  lts = toolchain { };
  current = toolchain { version = "current"; };
}
