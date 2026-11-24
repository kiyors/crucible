# `crucible init`'s output must evaluate, and so must the templates whose
# text it shares. Its cargo tests pin the generated text to the golden flakes
# in crates/init/tests/fixtures/init/, and this check calls those flakes'
# `outputs` with crucible's own lib: together they show that what init writes
# is a working flake, without IFD or a network.
{ pkgs, crucible }:
let
  inherit (pkgs) lib;
  inherit (pkgs.stdenv.hostPlatform) system;
  systems = import ../lib/common/systems.nix;
  fixtures = ../crates/init/tests/fixtures/init;
  # The systems as init writes them, one per line of the genAttrs list.
  listed = lib.concatMapStrings (s: "        \"${s}\"\n") systems;

  # The numbered channel's real manifest is a download, so the check only
  # makes sure the flake hands its rust-manifest input to fromToolchainFile.
  rustManifest = {
    outPath = "/rust-manifest";
  };
  rust = crucible.rust // {
    fromToolchainFile =
      args:
      if args ? manifest then
        assert args.manifest == rustManifest;
        assert builtins.pathExists args.file;
        crucible.rust.stable
      else
        crucible.rust.fromToolchainFile args;
  };

  # A flake's outputs, called with crucible's lib as the flake exposes it.
  outputsOf =
    dir:
    (import (dir + "/flake.nix")).outputs (
      {
        nixpkgs = {
          inherit lib;
          legacyPackages.${system} = pkgs;
        };
        crucible.lib.${system} = {
          inherit rust;
          inherit (crucible)
            node
            deno
            python
            go
            zig
            ;
        };
        rust-manifest = rustManifest;
      }
      // {
        # The moved templates are self-contained dev shells: their `outputs`
        # call `self`, so the check passes a stub whose overlay is the one
        # output the `go` template reads through it.
        self = {
          overlays.default = final: _prev: {
            go = final."go_1_24";
          };
        };
      }
    );

  evaluate =
    name:
    let
      dir = fixtures + "/${name}";
      flake = import (dir + "/flake.nix");
      outputs = outputsOf dir;
      text = builtins.readFile (dir + "/flake.nix");
    in
    assert lib.assertMsg (
      builtins.attrNames outputs.devShells == lib.sort lib.lessThan systems
    ) "crucible: init's ${name} flake must cover lib/common/systems.nix";
    # The same order, too: SYSTEMS in crates/init/src/lib.rs follows systems.nix.
    assert lib.assertMsg (lib.hasInfix listed text)
      "crucible: init's ${name} flake lists the systems out of order";
    assert (flake.inputs ? rust-manifest) == (name == "rust-numbered");
    # Only the derivation path: evaluating the shell is the point, building
    # its toolchains is what the other checks do.
    builtins.unsafeDiscardStringContext outputs.devShells.${system}.default.drvPath;
in
{
  init-flakes = pkgs.runCommand "crucible-check-init-flakes" { } ''
    # ${
      lib.concatMapStringsSep " " evaluate [
        "rust"
        "rust-numbered"
        "node"
        "rust-node"
        "python"
        "python-version"
        "go"
        "zig"
        "deno"
        "mixed"
      ]
    }
    touch $out
  '';

  # init is part of the user build, so `nix run github:kiyors/crucible
  # -- init` works on a cold cache. It must refuse to replace a flake.nix.
  crucible-init = pkgs.runCommand "crucible-check-init" { } ''
    slim=${lib.getExe crucible.cli}
    cp -r ${fixtures}/rust-numbered project
    chmod -R u+w project
    cd project
    ! $slim init 2> refused
    grep -q 'pass --force' refused
    mv flake.nix golden.nix
    $slim init > out
    diff golden.nix flake.nix
    grep -q 'through the rust-manifest input' out
    grep -q 'nix develop' out
    # Every language at once, with Zig left on the newest pin.
    cd ..
    cp -r ${fixtures}/mixed mixed
    chmod -R u+w mixed
    cd mixed
    mv flake.nix golden.nix
    $slim init > out
    diff golden.nix flake.nix
    grep -q 'python: pyproject.toml (version from requires-python)' out
    grep -q 'go: go.mod (go 1.27.0)' out
    grep -q 'note: build.zig.zon has no .minimum_zig_version' out
    touch $out
  '';

  # The root flake exposes the moved templates as `templates.<name>`; the
  # directories they point at must be flake dirs. These are directory names:
  # `templates.hash` lives in `hashi`.
  templates =
    let
      moved = [
        "bun"
        "c-cpp"
        "deno"
        "elixir"
        "elm"
        "empty"
        "go"
        "hashi"
        "java"
        "jupyter"
        "kotlin"
        "laravel"
        "nim"
        "nix"
        "node"
        "odin"
        "php"
        "python"
        "rust"
        "scala"
        "shell"
        "swift"
        "uv"
        "zig"
      ];
      missing = lib.filter (name: !builtins.pathExists (../templates + "/${name}/flake.nix")) (
        moved ++ [ "default" ]
      );
    in
    assert missing == [ ];
    pkgs.runCommand "crucible-check-templates" { } "touch $out";
}
