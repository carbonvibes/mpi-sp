{
  writeShellApplication,
  mprocs,
  replaceVars,
  curl,
  lib,
}:
writeShellApplication {
  name = "docker-reproduce";

  runtimeInputs = [
    mprocs
    curl
  ];

  text = lib.readFile (
    replaceVars ./reproduce.sh {
      ATTACK = ./attack.sh;
      CLOBBER = ./clobber.py;
      CONFIG = ./reproduce-config.yaml;
    }
  );
}
