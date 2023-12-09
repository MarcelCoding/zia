{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/release-23.11";
    flake-utils.url = "github:numtide/flake-utils";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, fenix }:
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
              inherit fenix;
              cargoToml = ./zia-client/Cargo.toml;
            };

            zia-server = pkgs.callPackage ./derivation.nix {
              inherit fenix;
              cargoToml = ./zia-server/Cargo.toml;
            };
          };
        }
      ) // {
      overlays.default = _: prev: {
        zia-client = self.packages."${prev.system}".zia-client;
        zia-server = self.packages."${prev.system}".zia-server;
      };

      nixosModules = {
        zia-server = import ./nixos-modules/zia-server.nix;
      };
    };
}
