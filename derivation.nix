{ lib, pkgs, cargoToml, ... }:
let
  manifest = (lib.importTOML cargoToml).package;
in
pkgs.rustPlatform.buildRustPackage {
  pname = manifest.name;
  version = manifest.version;
  cargoLock.lockFile = ./Cargo.lock;
  src = lib.cleanSource ./.;
  cargoBuildFlags = "-p ${manifest.name}";
}
