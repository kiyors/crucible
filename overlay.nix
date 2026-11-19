final: _prev:
let
  nix-cargo = import ./default.nix { pkgs = final; };
in
{
  inherit nix-cargo;
  huacnlee-cargo = nix-cargo.cargo;
}
