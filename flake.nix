{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem
      (system:
        let
          pkgs = (import nixpkgs) {
            inherit system;
          };
        in
        {
          packages = {
            zia-client = pkgs.callPackage ./derivation.nix {
              cargoToml = ./zia-client/Cargo.toml;
            };

            zia-server = pkgs.callPackage ./derivation.nix {
              cargoToml = ./zia-server/Cargo.toml;
            };
          };
        }
      ) // {
      overlays.default = _: prev: {
        inherit (self.packages."${prev.system}") zia-client zia-server;
      };

      nixosModules = {
        zia-server = import ./nixos-modules/zia-server.nix;
      };
    };
}
