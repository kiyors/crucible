{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    literalExpression
    mkEnableOption
    mkIf
    mkMerge
    mkOption
    optional
    types
    ;

  cfg = config.programs.crucible.rust;
  nodeCfg = config.programs.crucible.node;
  pythonCfg = config.programs.crucible.python;
  goCfg = config.programs.crucible.go;
  zigCfg = config.programs.crucible.zig;
  tsrsCfg = config.programs.crucible.tsrs;
  tomlFormat = pkgs.formats.toml { };
  cargoHome = "${config.home.homeDirectory}/${cfg.cargoHomeDir}";
  sessionVariables = config.home.sessionVariables;
in
{
  options.programs.crucible.rust = {
    enable = mkEnableOption "a Rust toolchain whose cargo is huacnlee/cargo";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.rust;
      defaultText = literalExpression "crucible.packages.\${system}.rust";
      example = literalExpression ''
        crucible.lib.''${system}.rust.toolchain {
          targets = [ "wasm32-unknown-unknown" ];
        }
      '';
      description = ''
        The toolchain to install: one from crucible's `packages` or `lib.<system>.rust`,
        or any package with a `bin/cargo`. To leave components out, build it with
        `components` or `exclude`; a toolchain built with
        `exclude = [ "rust-analyzer" ]` installs no rust-analyzer.
        The module does not set `RUST_SRC_PATH`: rust-analyzer finds the
        standard library in the sysroot of whichever `rustc` a project uses.
      '';
    };

    rust-analyzer = mkOption {
      type = types.nullOr types.package;
      default = null;
      example = literalExpression "pkgs.rust-analyzer";
      description = ''
        A rust-analyzer to install next to the toolchain. crucible's toolchains
        already include the one built for their rustc, so this is only for a
        `package` that lacks it.
      '';
    };

    extraPackages = mkOption {
      type = types.listOf types.package;
      default = [ ];
      example = literalExpression "[ pkgs.cargo-edit pkgs.bacon ]";
      description = "Cargo subcommands and other tools installed next to the toolchain.";
    };

    cargoHomeDir = mkOption {
      type = types.str;
      default = ".cargo";
      description = "CARGO_HOME, relative to the home directory.";
    };

    gc = mkOption {
      inherit (tomlFormat) type;
      default = { };
      example = {
        max-size = "auto";
        shared-max-size = "50 GiB";
        max-unused = "1 day";
        frequency = "1 hour";
      };
      description = ''
        The fork's `[build.gc]` settings for its shared build directory, see
        <https://github.com/huacnlee/cargo#what-changes>.
      '';
    };

    settings = mkOption {
      inherit (tomlFormat) type;
      default = { };
      example = {
        net.git-fetch-with-cli = true;
      };
      description = ''
        Cargo configuration written to {file}`$CARGO_HOME/config.toml`, merged
        with {option}`programs.crucible.rust.gc`.
      '';
    };
  };

  options.programs.crucible.node = {
    enable = mkEnableOption "a Node.js toolchain from nodejs.org, with Vite+ (vp)";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.node;
      defaultText = literalExpression "crucible.packages.\${system}.node";
      example = literalExpression ''
        crucible.lib.''${system}.node.toolchain {
          version = "24";
          packageManager = "pnpm@10";
        }
      '';
      description = ''
        The toolchain to install: one from crucible's `packages` or `lib.<system>.node`
        (node, npm and vp, the pinned LTS by default), or any package with a `bin/node`.
        Its vp uses this Node.js and never downloads one of its own.
      '';
    };

    extraPackages = mkOption {
      type = types.listOf types.package;
      default = [ ];
      example = literalExpression "[ pkgs.typescript ]";
      description = ''
        Package managers and other tools installed next to the toolchain. A second
        Node.js or package manager here shadows the toolchain's.
      '';
    };

    npmPrefix = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = ".npm-global";
      description = ''
        Where `npm install --global` puts packages, relative to the home directory, set
        as NPM_CONFIG_PREFIX with its {file}`bin` on PATH. By default npm installs next
        to node, which is the read-only store, so global installs fail.
      '';
    };

    pnpmHome = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = ".local/share/pnpm";
      description = ''
        Where pnpm keeps its global packages, relative to the home directory, set as
        PNPM_HOME with its {file}`bin` on PATH. pnpm 12 puts globally installed binaries
        in `$PNPM_HOME/bin`, so that directory is the one to add, not `$PNPM_HOME` itself.
      '';
    };
  };

  options.programs.crucible.python = {
    enable = mkEnableOption "a CPython toolchain from python-build-standalone, with uv";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.python;
      defaultText = literalExpression "crucible.packages.\${system}.python";
      example = literalExpression ''
        crucible.lib.''${system}.python.toolchain {
          version = "3.13";
        }
      '';
      description = ''
        The toolchain to install: one from crucible's `packages` or `lib.<system>.python`
        (python3, pip and uv, the newest pinned CPython by default), or any package with a
        `bin/python3`. Its uv uses this Python for every project, whatever the project's
        `.python-version` says, and never downloads one of its own; a project that needs
        another release gets it from a dev shell built with `fromPythonVersion` or
        `fromPyprojectToml`, whose setup hook takes over.
      '';
    };

    extraPackages = mkOption {
      type = types.listOf types.package;
      default = [ ];
      example = literalExpression "[ pkgs.ruff pkgs.basedpyright ]";
      description = ''
        Linters, language servers and other tools installed next to the toolchain. A
        second Python here shadows the toolchain's. Python libraries do not belong here:
        install them into a virtual environment, or command-line tools with
        `uv tool install`.
      '';
    };
  };

  options.programs.crucible.go = {
    enable = mkEnableOption "a Go toolchain from go.dev";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.go;
      defaultText = literalExpression "crucible.packages.\${system}.go";
      example = literalExpression ''
        crucible.lib.''${system}.go.toolchain { version = "1.26"; }
      '';
      description = ''
        The toolchain to install: one from crucible's `packages` or `lib.<system>.go`
        (the newest pinned release by default), or any package with a `bin/go`.
        crucible's toolchains default to `GOTOOLCHAIN=local` in their own
        {file}`go.env`, so `go` never downloads the toolchain a module asks for.
        gopls is not included; add it to {option}`programs.crucible.go.extraPackages`.
      '';
    };

    extraPackages = mkOption {
      type = types.listOf types.package;
      default = [ ];
      example = literalExpression "[ pkgs.gopls pkgs.golangci-lint ]";
      description = ''
        gopls and other tools installed next to the toolchain. A second Go here
        shadows the toolchain's.
      '';
    };

    goPath = mkOption {
      type = types.nullOr types.str;
      default = "go";
      example = ".local/share/go";
      description = ''
        GOPATH, relative to the home directory, with its {file}`bin` (where
        `go install` puts programs) on PATH. The default is Go's own default,
        {file}`~/go`. `null` leaves GOPATH and PATH alone.
      '';
    };
  };

  options.programs.crucible.zig = {
    enable = mkEnableOption "a Zig toolchain from ziglang.org";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.zig;
      defaultText = literalExpression "crucible.packages.\${system}.zig";
      example = literalExpression "crucible.lib.\${system}.zig.toolchain { version = \"0.15\"; }";
      description = ''
        The toolchain to install: one from crucible's `packages` or
        `lib.<system>.zig`, or any package with a `zig` executable.
      '';
    };

    extraPackages = mkOption {
      type = types.listOf types.package;
      default = [ ];
      example = literalExpression "[ pkgs.zls ]";
      description = ''
        Other tools installed next to the toolchain. nixpkgs' `zls`, the Zig
        Language Server, is built against nixpkgs' zig and does not track
        crucible's pins, so it is not wired in by the toolchain itself.
      '';
    };
  };

  options.programs.crucible.tsrs = {
    enable = mkEnableOption "tsrs, the Rust port of the TypeScript 7 compiler and language server";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.tsrs;
      defaultText = literalExpression "crucible.packages.\${system}.tsrs";
      description = ''
        The tsrs to install: `tsrs` (a drop-in for `tsc`, and `tsrs --lsp` for editors)
        and `tsrs-lint`, the backend Oxlint uses for its type-aware rules. It is the
        native executable alone and needs no Node.js. A project that lists `tsrs` in its
        own `devDependencies` runs that release through `pnpm exec` or `npx`; this one is
        for checking outside such projects and for editors.
      '';
    };
  };

  config = mkMerge [
    (mkIf cfg.enable {
      warnings =
        optional ((sessionVariables.CARGO_INCREMENTAL or null) == "0") ''
          programs.crucible.rust: CARGO_INCREMENTAL=0 turns off the incremental caches
          huacnlee/cargo shares between worktrees, so undoing an edit or switching
          back to a branch rebuilds from scratch.
        ''
        ++ optional (sessionVariables ? CARGO_TARGET_DIR) ''
          programs.crucible.rust: huacnlee/cargo ignores CARGO_TARGET_DIR and always
          builds in ./target and ~/.cargo/targets.
        ''
        ++ optional (sessionVariables ? RUSTC_WRAPPER) ''
          programs.crucible.rust: RUSTC_WRAPPER is set. huacnlee/cargo already shares
          compiled dependencies between projects, so a compiler cache such as
          sccache or kache stores everything a second time.
        '';

      programs.crucible.rust.settings = mkIf (cfg.gc != { }) { build.gc = cfg.gc; };

      home = {
        packages = [
          cfg.package
        ]
        ++ optional (cfg.rust-analyzer != null) cfg.rust-analyzer
        ++ cfg.extraPackages;

        # No RUST_SRC_PATH: rust-analyzer finds the sources through `rustc
        # --print sysroot`, and a global value would point rust-analyzer at this
        # release in projects that use another one (rustup, a pinned dev shell).
        sessionVariables.CARGO_HOME = cargoHome;

        sessionPath = [ "${cargoHome}/bin" ];

        file."${cfg.cargoHomeDir}/config.toml" = mkIf (cfg.settings != { }) {
          source = tomlFormat.generate "cargo-config.toml" cfg.settings;
        };
      };
    })

    (mkIf nodeCfg.enable {
      warnings = optional (sessionVariables ? VP_HOME) ''
        programs.crucible.node: VP_HOME is set, so crucible's vp shares that root (and its
        config.json) with any other vp. If that config manages Node.js itself, vp stops
        with "VP_ENV_FORBIDDEN=1: Vite+ does not manage Node.js"; run `vp env off` there.
      '';

      home = {
        packages = [ nodeCfg.package ] ++ nodeCfg.extraPackages;
        sessionVariables = mkMerge [
          (mkIf (nodeCfg.npmPrefix != null) {
            NPM_CONFIG_PREFIX = "${config.home.homeDirectory}/${nodeCfg.npmPrefix}";
          })
          (mkIf (nodeCfg.pnpmHome != null) {
            PNPM_HOME = "${config.home.homeDirectory}/${nodeCfg.pnpmHome}";
          })
        ];
        sessionPath =
          optional (nodeCfg.npmPrefix != null) "${config.home.homeDirectory}/${nodeCfg.npmPrefix}/bin"
          ++ optional (nodeCfg.pnpmHome != null) "${config.home.homeDirectory}/${nodeCfg.pnpmHome}/bin";
      };
    })

    (mkIf pythonCfg.enable {
      warnings =
        optional (sessionVariables ? PYTHONHOME) ''
          programs.crucible.python: PYTHONHOME is set, so python3 looks for its standard
          library there instead of next to itself, and fails to start or runs another
          release's modules.
        ''
        ++
          optional
            (lib.any (name: sessionVariables ? ${name}) [
              "UV_PYTHON"
              "UV_PYTHON_DOWNLOADS"
              "UV_MANAGED_PYTHON"
              "UV_PYTHON_PREFERENCE"
            ])
            ''
              programs.crucible.python: a UV_PYTHON* or UV_MANAGED_PYTHON session variable
              overrides what crucible's uv sets, so uv may use or download another Python
              than the toolchain's.
            '';

      # `uv tool install` puts its commands here.
      home.packages = [ pythonCfg.package ] ++ pythonCfg.extraPackages;
      home.sessionPath = [ "${config.home.homeDirectory}/.local/bin" ];
    })

    (mkIf goCfg.enable {
      warnings =
        optional (sessionVariables ? GOROOT) ''
          programs.crucible.go: GOROOT is set, so the toolchain's go uses the standard library
          and tools under it instead of its own. Go finds its GOROOT by itself; remove it.
        ''
        ++ optional ((sessionVariables.GOTOOLCHAIN or "local") != "local") ''
          programs.crucible.go: GOTOOLCHAIN=${toString sessionVariables.GOTOOLCHAIN} lets go
          download and run the toolchain a module asks for instead of this one.
        '';

      home = {
        packages = [ goCfg.package ] ++ goCfg.extraPackages;
        sessionVariables = mkIf (goCfg.goPath != null) {
          GOPATH = "${config.home.homeDirectory}/${goCfg.goPath}";
        };
        sessionPath = optional (goCfg.goPath != null) "${config.home.homeDirectory}/${goCfg.goPath}/bin";
      };
    })

    (mkIf zigCfg.enable {
      home.packages = [ zigCfg.package ] ++ zigCfg.extraPackages;
    })

    (mkIf tsrsCfg.enable {
      home.packages = [ tsrsCfg.package ];
    })
  ];
}
