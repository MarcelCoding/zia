{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/release-23.05";
    flake-utils.url = "github:numtide/flake-utils";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, fenix }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = (import nixpkgs) {
          inherit system;
        };
      in
      rec {
        packages = {
          zia-client = let manifest = (pkgs.lib.importTOML ./zia-client/Cargo.toml).package; in pkgs.rustPlatform.buildRustPackage {
            pname = manifest.name;
            version = manifest.version;
            cargoLock.lockFile = ./Cargo.lock;
            src = pkgs.lib.cleanSource ./.;
            cargoBuildFlags = "-p ${manifest.name}";
            nativeBuildInputs = [ fenix.packages.${system}.stable.toolchain ];
          };

          zia-server = let manifest = (pkgs.lib.importTOML ./zia-server/Cargo.toml).package; in pkgs.rustPlatform.buildRustPackage {
            pname = manifest.name;
            version = manifest.version;
            cargoLock.lockFile = ./Cargo.lock;
            src = pkgs.lib.cleanSource ./.;
            cargoBuildFlags = "-p ${manifest.name}";
            nativeBuildInputs = [ fenix.packages.${system}.stable.toolchain ];
          };
        };
      }
    );
}
