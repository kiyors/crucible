# Runs node, npm and vp from each kind of toolchain, and builds a package with
# nixpkgs' npm helpers, so a release that cannot run in the store (or a vp that
# reaches for a Node.js of its own) fails CI rather than a dev shell.
{ pkgs, crucible }:
let
  inherit (pkgs) lib;
  inherit (crucible) node;

  pins = lib.importJSON ../../data/node/node.json;
  lts = pins.releases.${pins.lts};
  pmPins = lib.importJSON ../../data/node/package-managers.json;

  # PEP 440 and semver have two deliberate parser copies (Rust in doctor, Nix
  # here). They share a corpus so a new case has to get the same answer on both
  # sides. A throw is the Nix side of Rust's None, so both read as null.
  semver = import ../../lib/node/semver.nix { inherit lib; };
  semverCorpus = lib.importJSON ../corpora/semver.json;
  semverCase =
    {
      range,
      version,
      expected,
      ...
    }:
    let
      result = builtins.tryEval (semver.satisfies version range);
      got = if result.success then result.value else null;
    in
    assert lib.assertMsg (got == expected) ''
      crucible: the shared semver corpus disagrees for ${range} / ${version}:
      expected ${builtins.toJSON expected}, got ${builtins.toJSON got}; edit
      lib/node/semver.nix or crates/doctor/src/lib.rs
    '';
    true;

  # Resolve a pinned major without building it, for the evaluation checks.
  pinnedPm = name: major: (node.toolchain { packageManager = "${name}@${major}"; }).packageManager;

  # Whether a toolchain is rejected at evaluation time.
  rejected = toolchain: !(builtins.tryEval toolchain.drvPath).success;

  # package.json picks the line; the release in it is whatever is pinned.
  fromApp = node.fromPackageJson { file = ./app/package.json; };
  fromDevEngines = node.fromPackageJson { file = ./dev-engines.json; };
  # The version-manager files are explicit project pins: they beat the
  # advisory engines.node range, and .node-version wins over .nvmrc.
  fromNodeVersion = node.fromPackageJson { file = ./version-files/package.json; };
  fromNvmrc = node.fromPackageJson { file = ./version-files-nvmrc/package.json; };

  # The path projects pinning an exact release take: its SHASUMS256.txt.
  fromShasums = node.toolchain {
    version = "24.11.0";
    sha256 = "sha256-7hr+SEoySW/XLCLwKsyA4o5q9VlJG0bN+9HHo5IsQr0=";
  };

  # vp keeps its config and caches under $HOME; the sandbox has no network,
  # so anything vp tried to download would fail the check.
  nodeRun =
    name: toolchain: script:
    pkgs.runCommand "crucible-check-node-${name}"
      {
        nativeBuildInputs = [ toolchain ];
      }
      ''
        export HOME=$TMPDIR CI=1
        ${script}
        touch $out
      '';
in
{
  node-lts = nodeRun "lts" node.lts ''
    test "$(node --version)" = "v${lts.version}"
    test "$(node -p process.versions.modules)" = "${lts.modules}"
    test "$(npm --version)" = "${lts.npm}"
    test "$(npx --version)" = "${lts.npm}"
    # npm runs on the node it ships with, whatever else is on PATH.
    head -1 "$(readlink -f "$(command -v npm)")" | grep -q '${node.lts.node}/bin/node'
  '';

  # Every pinned line runs, not just the default one.
  node-lines = nodeRun "lines" node.lts ''
    ${lib.concatMapStrings (r: ''
      test "$(${
        node.toolchain {
          inherit (r) version;
          vitePlus = false;
        }
      }/bin/node --version)" = "v${r.version}"
    '') (builtins.attrValues pins.releases)}
  '';

  # A project's whole loop through vp, on the toolchain's node.
  node-vite-plus = nodeRun "vite-plus" node.lts ''
    cp -r ${./app} app
    chmod -R u+w app
    cd app
    vp --version | grep -q 'Node.js *v${lts.version}'
    vp fmt --check src
    vp lint src
    vp build
    test -f dist/index.html
    vp test --globals
    # vp kept its state in its own root, seeded as system-first, and set
    # nothing up for itself: no shims, no managed runtimes.
    grep -q system_first "$HOME/.local/share/crucible/vite-plus/config.json"
    test ! -e "$HOME/.local/share/vite-plus"
    test ! -e "$HOME/.vite-plus"
    test ! -e "$HOME/.local/share/crucible/vite-plus/js_runtime"
  '';

  # The setup hook overwrites what a login shell may have exported.
  node-setup-hook =
    pkgs.runCommand "crucible-check-node-setup-hook"
      {
        nativeBuildInputs = [ node.lts ];
        VP_ENV_FORBIDDEN = "0";
        VP_HOME = "/nonexistent/.vite-plus";
      }
      ''
        test "$VP_ENV_FORBIDDEN" = 1
        test -z "''${VP_HOME+set}"
        touch $out
      '';

  # engines.node and packageManager from package.json.
  node-from-package-json =
    assert builtins.all semverCase semverCorpus.cases;
    assert lib.versions.major fromApp.version == "24";
    assert fromDevEngines.packageManager.pname == "pnpm";
    nodeRun "from-package-json" fromApp ''
      node --version | grep -q '^v24\.'
      pnpm --version | grep -q '^10\.'
      # pnpm runs on the toolchain's node: the pinned native pnpm's wrapper
      # puts that node first on PATH for the scripts pnpm runs.
      grep -q '${fromApp.node}/bin' "$(readlink -f "$(command -v pnpm)")"
      echo '{}' > package.json
      test "$(pnpm exec node --version)" = "$(node --version)"
    '';

  # devEngines.runtime wins over engines.node, and a list may name other
  # runtimes before node.
  node-dev-engines =
    assert lib.versions.major fromDevEngines.version == "22";
    pkgs.runCommand "crucible-check-node-dev-engines" { } "touch $out";

  node-version-files =
    assert lib.versions.major fromNodeVersion.version == "24";
    assert lib.versions.major fromNvmrc.version == "22";
    pkgs.runCommand "crucible-check-node-version-files" { } "touch $out";

  node-from-shasums = nodeRun "from-shasums" fromShasums ''
    test "$(node --version)" = v24.11.0
    vp --version | grep -q 'Node.js *v24.11.0'
  '';

  # A pinned major gets its exact upstream release, not nixpkgs' build of that
  # major.
  node-pnpm-exact =
    let
      inherit (pmPins.pnpm."12") version;
      toolchain = node.toolchain { packageManager = "pnpm@${version}"; };
    in
    assert toolchain.packageManager.version == version;
    nodeRun "pnpm-exact" toolchain ''
      pnpm --version | grep -qx '${version}'
    '';

  # The other pinned majors resolve on this node without being built, and the
  # nixpkgs fallback still handles majors that are not pinned.
  node-package-manager-pins =
    # Every pnpm line upstream still releases is pinned.
    assert
      (builtins.attrNames pmPins.pnpm) == [
        "10"
        "11"
        "12"
      ];
    assert (pinnedPm "pnpm" "10").version == pmPins.pnpm."10".version;
    assert (pinnedPm "pnpm" "11").version == pmPins.pnpm."11".version;
    assert (pinnedPm "pnpm" "12").version == pmPins.pnpm."12".version;
    assert (pinnedPm "bun" "1").version == pmPins.bun."1".version;
    assert (pinnedPm "yarn" "1").version == pmPins.yarn."1".version;
    assert (pinnedPm "yarn" "4").version == pmPins.yarn."4".version;
    # bun@1.x never depends on nixpkgs' one bun: every supported major is
    # pinned, with a hash for every system of lib/common/systems.nix.
    assert (builtins.attrNames pmPins.bun) == [ "1" ];
    assert builtins.elem "x86_64-linux" (builtins.attrNames pmPins.bun."1".platforms);
    assert builtins.elem "aarch64-linux" (builtins.attrNames pmPins.bun."1".platforms);
    assert builtins.elem "aarch64-darwin" (builtins.attrNames pmPins.bun."1".platforms);
    # A major with no pin comes from nixpkgs; the range must contain it.
    assert (node.toolchain { packageManager = "yarn@3"; }).packageManager.pname == "yarn-berry";
    # fromPackageJson's packageManager argument overrides the file's spec.
    assert
      (node.fromPackageJson {
        file = ./app/package.json;
        packageManager = "pnpm@${pmPins.pnpm."12".version}";
      }).packageManager.version == pmPins.pnpm."12".version;
    pkgs.runCommand "crucible-check-node-package-manager-pins" { } "touch $out";

  # Build and run every pinned pnpm, yarn and bun, so a tarball layout
  # change fails a check instead of a downstream shell. pnpm 10 is a raw
  # binary, 11 and 12 tarballs with dist/, all from their GitHub releases.
  node-pinned-package-manager-builds =
    let
      pnpm10 = pinnedPm "pnpm" "10";
      pnpm11 = pinnedPm "pnpm" "11";
      classic = pinnedPm "yarn" "1";
      berry = pinnedPm "yarn" "4";
      bun = pinnedPm "bun" "1";
    in
    pkgs.runCommand "crucible-check-node-pinned-package-manager-builds"
      {
        nativeBuildInputs = [
          classic
          berry
          bun
        ];
      }
      ''
        export HOME=$TMPDIR
        ${pnpm10}/bin/pnpm --version | grep -q '^${pmPins.pnpm."10".version}$'
        ${pnpm11}/bin/pnpm --version | grep -q '^${pmPins.pnpm."11".version}$'
        ${classic}/bin/yarn --version | grep -q '^${pmPins.yarn."1".version}$'
        ${berry}/bin/yarn --version | grep -q '^${pmPins.yarn."4".version}$'
        ${bun}/bin/bun --version | grep -q '^${pmPins.bun."1".version}$'
        touch $out
      '';

  # Mistakes that would otherwise fail later, or not at all.
  node-errors =
    # A line or release that is not pinned, without its SHASUMS256.txt.
    assert rejected (node.toolchain { version = "18"; });
    assert rejected (node.toolchain { version = "24.0.0"; });
    assert rejected (node.toolchain { version = "lts/argon"; });
    # sha256 pins one exact release.
    assert rejected (
      node.toolchain {
        version = "^24";
        sha256 = lib.fakeHash;
      }
    );
    # A package manager major nixpkgs does not have, and a malformed one.
    assert rejected (node.toolchain { packageManager = "pnpm@3.0.0"; });
    assert rejected (node.toolchain { packageManager = "deno@2.0.0"; });
    # An exact release that is neither the pin nor nixpkgs' release is an
    # error, not a silent substitution; the same version as a range is fine.
    assert rejected (node.toolchain { packageManager = "pnpm@12.3.4"; });
    assert !rejected (node.toolchain { packageManager = "pnpm@^12.3.4"; });
    # A package built from the exact release is the escape hatch.
    assert
      (node.toolchain {
        packageManager = pkgs.writeShellScriptBin "pnpm" "echo 12.3.4";
      }).packageManager.name == "pnpm";
    # Vite+ with a Node.js outside its engines (^24.11.0 on that line)...
    assert rejected (
      node.toolchain {
        version = "24.10.0";
        sha256 = "sha256-7wf+XZQvw4/tttOlwiAQk2M0JY+YY5qNBMCFd80DvY4=";
      }
    );
    # ...which builds without it.
    assert
      !rejected (
        node.toolchain {
          version = "24.10.0";
          sha256 = "sha256-7wf+XZQvw4/tttOlwiAQk2M0JY+YY5qNBMCFd80DvY4=";
          vitePlus = false;
        }
      );
    # A pinned line and npm (which ships with Node.js) evaluate.
    assert !rejected (node.toolchain { version = pins.current; });
    assert !rejected (node.toolchain { packageManager = "npm@11.0.0"; });
    pkgs.runCommand "crucible-check-node-errors" { } "touch $out";

  # nixpkgs' buildNpmPackage on the toolchain's node, with dependencies
  # fetched by their lockfile integrity (importNpmLock), so there is no
  # npmDepsHash to keep up to date.
  node-build-npm-package = node.lts.buildNpmPackage {
    pname = "hello";
    version = "0.1.0";
    src = ./hello;
    npmDeps = pkgs.importNpmLock { npmRoot = ./hello; };
    inherit (pkgs.importNpmLock) npmConfigHook;
    dontNpmBuild = true;
    doInstallCheck = true;
    installCheckPhase = ''
      test "$($out/bin/hello)" = "hello from crucible"
      grep -q '${node.lts.node}/bin/node' "$(readlink -f $out/bin/hello)"
    '';
  };

  inherit (node) vite-plus-unwrapped;
}
