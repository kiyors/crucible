{
  description = "HashiCorp tools from nixpkgs: Terraform, Packer, Nomad, Vault and Terragrunt (allowUnfree)";

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
              config.allowUnfree = true;
            };
          }
        );
    in
    {
      devShells = forEachSupportedSystem (
        { pkgs }:
        {
          default = pkgs.mkShellNoCC {
            packages = with pkgs; [
              packer
              terraform
              tflint
              nomad
              vault
              nomad-autoscaler
              nomad-pack
              levant
              damon
              terragrunt
            ];
          };
        }
      );
    };
}
