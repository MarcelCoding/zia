{ lib, rustPlatform, cargoToml, ... }:

let
  manifest = (lib.importTOML cargoToml).package;
in
rustPlatform.buildRustPackage {
  pname = manifest.name;
  inherit (manifest) version;

  src = lib.cleanSource ./.;
  cargoLock.lockFile = ./Cargo.lock;

  cargoBuildFlags = "-p ${manifest.name}";
  cargoTestFlags = "-p ${manifest.name}";

  meta = {
    mainProgram = manifest.name;
  };
}
