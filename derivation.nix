{ lib, pkgs, cargoToml, ... }:
let
  manifest = (lib.importTOML cargoToml).package;
in
pkgs.rustPlatform.buildRustPackage {
  pname = manifest.name;
  version = manifest.version;
  cargoLock.lockFile = ./Cargo.lock;
  src = lib.cleanSource ./.;
  cargoBuildFlags = "--package ${manifest.name} --bin ${manifest.name}";
  checkFlags = "--package ${manifest.name} --bin ${manifest.name}";
}
