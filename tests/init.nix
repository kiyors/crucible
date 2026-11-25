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

  # The root flake exposes the moved templates as `templates.<name>`: every
  # attr must be a flake that evaluates for every supported system, and the
  # attrs must cover the directories (`hash` lives in `hashi`, while `c` and
  # `cpp` alias `c-cpp`). Inputs are stubbed as in init-flakes so this stays
  # offline: nixpkgs is the pinned store copy and crucible's lib is built from
  # the same one, not a fetched flake.
  templates =
    let
      declared = import ../templates.nix;
      attrs = lib.attrNames declared;
      directories = lib.attrNames (
        lib.filterAttrs (_: kind: kind == "directory") (builtins.readDir ../templates)
      );
      # The attrs whose names are not directory names.
      aliases = {
        hash = "hashi";
        c = "c-cpp";
        cpp = "c-cpp";
      };
      expected = lib.unique (
        [
          "c"
          "cpp"
          "hash"
        ]
        ++ map (d: if d == "hashi" then "hash" else d) directories
      );

      # A flake-like nixpkgs: `import inputs.nixpkgs` follows outPath, while
      # lib/legacyPackages serve the templates that use those instead. Importing
      # the pinned copy for each system needs no network.
      pkgsFor = s: import pkgs.path { system = s; };
      nixpkgsStub = {
        outPath = pkgs.path;
        inherit lib;
        legacyPackages = lib.genAttrs systems pkgsFor;
      };
      # Templates call `crucible.lib.${system}.<lang>.<function>`; build that
      # public surface from default.nix for every system, as flake.nix does.
      crucibleStub = {
        lib = lib.genAttrs systems (s: import ../. { pkgs = pkgsFor s; });
      };
      # Some templates apply their own overlay through `self`. A no-op overlay
      # lets those dev shells evaluate without recursing into their outputs.
      stub = {
        nixpkgs = nixpkgsStub;
        crucible = crucibleStub;
        self = {
          overlays.default = _final: _prev: { };
        };
        rust-manifest = {
          outPath = "/rust-manifest";
        };
      };

      # Forgetting a system or a required output is a template error. Forcing
      # the shells themselves is only meaningful for templates that use
      # crucible: the nixpkgs-only ones name packages that nixpkgs may have
      # removed since the template was written, which is not what this check
      # is about.
      evaluate =
        name:
        let
          dir = declared.${name}.path;
          outputs = (import (dir + "/flake.nix")).outputs stub;
          shells = outputs.devShells;
          packages = outputs.packages or { };
          drvs =
            if lib.hasInfix "crucible.lib" (builtins.readFile (dir + "/flake.nix")) then
              lib.concatMap (
                s:
                [ shells.${s}.default.drvPath ]
                ++ lib.optional (packages ? ${s}.default) packages.${s}.default.drvPath
              ) systems
            else
              [ ];
        in
        assert lib.assertMsg (outputs ? devShells) "crucible: template ${name} has no devShells";
        assert lib.assertMsg (builtins.all (s: shells ? ${s}) systems) ''
          crucible: template ${name} must cover every system in lib/common/systems.nix
        '';
        assert lib.assertMsg (builtins.all (
          s: !(packages ? ${s}) || packages.${s} ? default
        ) systems) "crucible: template ${name}'s packages must have a default for every system";
        lib.concatStringsSep " " (map builtins.unsafeDiscardStringContext drvs);
    in
    assert lib.assertMsg (lib.sort lib.lessThan attrs == lib.sort lib.lessThan expected) ''
      crucible: templates.nix attrs (${lib.concatStringsSep ", " attrs}) must match the
      templates/ directories (${lib.concatStringsSep ", " expected})
    '';
    assert lib.assertMsg (builtins.all
      (alias: declared.${alias}.path == ../templates + "/${aliases.${alias}}")
      (lib.attrNames aliases)
    ) "crucible: template aliases must point at the same flake as their target";
    assert lib.assertMsg (builtins.all (
      name: builtins.pathExists (declared.${name}.path + "/flake.nix")
    ) attrs) "crucible: every templates.nix attr must point at a flake directory";
    pkgs.runCommand "crucible-check-templates" { } ''
      # ${lib.concatMapStringsSep "\n# " evaluate attrs}
      touch $out
    '';
}
