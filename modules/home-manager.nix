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
    mkOption
    optional
    types
    ;

  cfg = config.programs.crucible.rust;
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

  config = mkIf cfg.enable {
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
  };
}
