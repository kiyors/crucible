# Builds, tests and runs Go programs with each kind of toolchain, and builds a
# package with nixpkgs' buildGoModule, so a release that cannot run in the
# store (or a go that reaches for a toolchain of its own) fails CI rather than
# a dev shell.
{ pkgs, crucible }:
let
  inherit (pkgs) lib;
  inherit (crucible) go;

  pins = lib.importJSON ../../data/go/go.json;
  stable = pins.releases.${pins.stable};
  # attrNames sorts as strings, which would put 1.100 before 1.99.
  lines = lib.sort (a: b: builtins.compareVersions a b < 0) (builtins.attrNames pins.releases);
  oldest = pins.releases.${builtins.head lines};

  # Whether a toolchain is rejected at evaluation time.
  rejected = toolchain: !(builtins.tryEval toolchain.drvPath).success;

  # go.mod files written from the pins, so these checks follow the daily
  # update instead of breaking when a line leaves support. builtins.toFile
  # needs no build, so reading them back is not import-from-derivation.
  goMod = name: directives: builtins.toFile "go.mod" "module example.com/${name}\n\n${directives}\n";
  fromGoMod = file: go.fromGoMod { inherit file; };
  oldestLine = lib.versions.majorMinor oldest.version;

  # The sandbox has no network; GOPROXY=off makes a module download fail with
  # a clear message instead of a DNS error.
  goRun =
    name: toolchain: script:
    pkgs.runCommand "crucible-check-go-${name}"
      {
        nativeBuildInputs = [ toolchain ];
      }
      ''
        export HOME=$TMPDIR GOPROXY=off
        ${script}
        touch $out
      '';
in
{
  go-stable = goRun "stable" go.stable ''
    test "$(go env GOVERSION)" = "go${stable.version}"
    test "$(go env GOTOOLCHAIN)" = local
    test "$(go env GOROOT)" = "${go.stable.goroot}"
    cp -r ${./hello} hello
    chmod -R u+w hello
    cd hello
    test -z "$(gofmt -l .)"
    go vet ./...
    go test ./...
    go build -o hello .
    test "$(./hello)" = "hello from crucible"
    test "$(go run .)" = "hello from crucible"
  '';

  # Every pinned line runs, not just the default one.
  go-lines = goRun "lines" go.stable ''
    ${lib.concatMapStrings (r: ''
      test "$(${go.toolchain { inherit (r) version; }}/bin/go env GOVERSION)" = "go${r.version}"
    '') (builtins.attrValues pins.releases)}
  '';

  # A module asking for a newer Go stops with a message that names
  # GOTOOLCHAIN=local, instead of trying to download that toolchain. A go
  # with GOTOOLCHAIN=auto would fail differently here: on the download.
  go-toolchain-local = goRun "toolchain-local" go.stable ''
    cp -r ${./future} future
    chmod -R u+w future
    cd future
    ! go build . 2> err
    cat err
    grep -q 'requires go >= 1.99' err
    grep -q 'GOTOOLCHAIN=local' err
    # So does an explicit GOTOOLCHAIN=auto in go's own config file, because
    # the setup hook sets the environment, which wins over it.
    go env -w GOTOOLCHAIN=auto
    test "$(go env GOTOOLCHAIN)" = local
  '';

  # The setup hook overwrites what a login shell may have exported.
  go-setup-hook =
    pkgs.runCommand "crucible-check-go-setup-hook"
      {
        nativeBuildInputs = [ go.stable ];
        GOTOOLCHAIN = "auto";
        GOROOT = "/nonexistent/go";
      }
      ''
        test "$GOTOOLCHAIN" = local
        test -z "''${GOROOT+set}"
        touch $out
      '';

  # cgo through the shell's C compiler. On Linux the binary must name the
  # store's dynamic linker, not the /lib64 one the official linker would use
  # by default, or it would not start on NixOS.
  #
  # runCommandCC, not runCommand plus a compiler: like mkShell it builds with
  # stdenv, whose macOS SDK brings the libresolv that net's cgo resolver links.
  go-cgo =
    pkgs.runCommandCC "crucible-check-go-cgo"
      {
        nativeBuildInputs = [ go.stable ];
      }
      (
        ''
          export HOME=$TMPDIR GOPROXY=off CGO_ENABLED=1
          cp -r ${./cgo} cgo
          chmod -R u+w cgo
          cd cgo
          go build -o cgo .
          test "$(./cgo)" = 42
        ''
        + lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
          interpreter=$(${pkgs.patchelf}/bin/patchelf --print-interpreter cgo)
          echo "$interpreter"
          case "$interpreter" in
            ${builtins.storeDir}/*) ;;
            *) exit 1 ;;
          esac
        ''
        + ''
          touch $out
        ''
      );

  # The toolchain directive wins over the go directive; both are minimums,
  # met by the pinned release of their line; `toolchain default` and
  # comments are ignored.
  go-from-go-mod =
    let
      byToolchain = fromGoMod (
        goMod "toolchain" "go ${oldestLine}.0\ntoolchain go${stable.version} // pinned"
      );
      byGo = fromGoMod (goMod "go" "go ${oldestLine}.0 // the minimum\ntoolchain default");
      byLine = fromGoMod (goMod "line" "go ${oldestLine}");
      exactPin = fromGoMod (goMod "exact" "go ${oldest.version}");
    in
    assert byToolchain.version == stable.version;
    assert byGo.version == oldest.version;
    assert byLine.version == oldest.version;
    assert exactPin.version == oldest.version;
    # A version with a go prefix, and a line, through `toolchain`.
    assert (go.toolchain { version = "go${stable.version}"; }).version == stable.version;
    assert (go.toolchain { version = pins.stable; }).version == stable.version;
    goRun "from-go-mod" byGo ''
      test "$(go env GOVERSION)" = "go${oldest.version}"
    '';

  # Mistakes that would otherwise fail later, or not at all.
  go-errors =
    # A line or exact release that is not pinned, without its hashes.
    assert rejected (go.toolchain { version = "1.20"; });
    assert rejected (go.toolchain { version = "${oldestLine}.0"; });
    assert rejected (go.toolchain { version = "banana"; });
    # hashes pin one exact release.
    assert rejected (
      go.toolchain {
        version = pins.stable;
        hashes.${pkgs.stdenv.hostPlatform.system} = lib.fakeHash;
      }
    );
    # ...and must cover the system being built.
    assert rejected (
      go.toolchain {
        version = "1.25.3";
        hashes = { };
      }
    );
    # go.mod asking for a patch release newer than the pin, for a line that
    # is not pinned, or for nothing at all.
    assert rejected (fromGoMod (goMod "newer" "go ${pins.stable}.999"));
    assert rejected (fromGoMod (goMod "old" "go 1.16"));
    assert rejected (fromGoMod (goMod "none" ""));
    # A cross package set.
    assert rejected
      (import ../../lib/go {
        pkgs = if pkgs.stdenv.hostPlatform.isLinux then pkgs.pkgsCross.mingwW64 else pkgs.pkgsCross.gnu64;
      }).stable;
    # Any exact release evaluates with its hashes, also from go.mod.
    assert
      !rejected (
        go.toolchain {
          version = "1.25.3";
          hashes.${pkgs.stdenv.hostPlatform.system} = lib.fakeHash;
        }
      );
    assert
      !rejected (
        go.fromGoMod {
          file = goMod "hashes" "go 1.25.3";
          hashes.${pkgs.stdenv.hostPlatform.system} = lib.fakeHash;
        }
      );
    pkgs.runCommand "crucible-check-go-errors" { } "touch $out";

  # The release `crucible pin go` documents, built from its hashes.
  go-from-hashes = goRun "from-hashes" (go.toolchain {
    version = "1.25.3";
    hashes = {
      x86_64-linux = "sha256-AzXzFLbnv+CMPQz6p8GduWG3uZ+yC+YrCoJsmSrRTg8=";
      aarch64-linux = "sha256-HULryEmZteIGn14xtn1vxdZzCK2tPheNWi7iyf8gAfU=";
      aarch64-darwin = "sha256-fAg+PSwA3r/rL3fZpMAKGqyXETuJuczEKpBIevNDc4I=";
    };
  }) "test \"$(go env GOVERSION)\" = go1.25.3";

  # nixpkgs' buildGoModule on the toolchain's go. A module without
  # dependencies needs no vendorHash.
  go-build-go-module = go.stable.buildGoModule {
    pname = "hello";
    version = "0.1.0";
    src = ./hello;
    vendorHash = null;
    doInstallCheck = true;
    installCheckPhase = ''
      test "$($out/bin/hello)" = "hello from crucible"
    '';
  };
}
