{
  description = "Elixir 1.17 and Erlang/OTP 27 from nixpkgs, with Hex, mix and rebar3";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable"; # unstable Nixpkgs
  # Nixpkgs-only by policy: this template installs no toolchain from crucible's
  # pins, so it deliberately takes no crucible input (see docs/template-policy.md).

  outputs =
    inputs:

    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forEachSupportedSystem =
        f:
        inputs.nixpkgs.lib.genAttrs supportedSystems (
          system:
          f {
            pkgs = import inputs.nixpkgs {
              inherit system;
              overlays = [ inputs.self.overlays.default ];
            };
          }
        );
    in
    {
      overlays.default = final: _prev: rec {
        # documentation
        # https://nixos.org/manual/nixpkgs/stable/#sec-beam

        # ==== ERLANG ====

        # use whatever version is currently defined in nixpkgs
        # erlang = pkgs.beam.interpreters.erlang;

        # use latest version of Erlang 27
        erlang = final.beam.interpreters.erlang_27;

        # specify exact version of Erlang OTP
        # erlang = pkgs.beam.interpreters.erlang.override {
        #   version = "26.2.2";
        #   sha256 = "sha256-7S+mC4pDcbXyhW2r5y8+VcX9JQXq5iEUJZiFmgVMPZ0=";
        # }

        # ==== BEAM packages ====

        # all BEAM packages will be compile with your preferred erlang version
        pkgs-beam = final.beam.packagesWith erlang;

        # ==== Elixir ====

        # use whatever version is currently defined in nixpkgs
        # elixir = pkgs-beam.elixir;

        # use latest version of Elixir 1.17
        elixir = pkgs-beam.elixir_1_17;

        # specify exact version of Elixir
        # elixir = pkgs-beam.elixir.override {
        #   version = "1.17.1";
        #   sha256 = "sha256-a7A+426uuo3bUjggkglY1lqHmSbZNpjPaFpQUXYtW9k=";
        # };
      };

      devShells = forEachSupportedSystem (
        { pkgs }:
        {
          default = pkgs.mkShellNoCC {
            packages =
              with pkgs;
              [
                # use the Elixr/OTP versions defined above; will also install OTP, mix, hex, rebar3
                elixir

                # mix needs it for downloading dependencies
                git

                # probably needed for your Phoenix assets; nixpkgs removed nodejs_20
                nodejs_22
              ]
              ++
                # Linux only
                pkgs.lib.optionals pkgs.stdenv.isLinux (
                  with pkgs;
                  [
                    gigalixir
                    inotify-tools
                    libnotify
                  ]
                )
              ++
                # macOS only
                pkgs.lib.optionals pkgs.stdenv.isDarwin (
                  with pkgs;
                  [
                    terminal-notifier
                  ]
                );
          };
        }
      );
    };
}
