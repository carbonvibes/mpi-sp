{
  writeShellApplication,
  mprocs,
  replaceVars,
  lib,
  git,
  openssh,
}:
writeShellApplication {
  name = "soft-serve-reproduce";

  runtimeInputs = [
    mprocs
    git
    openssh
  ];

  text = lib.readFile (
    replaceVars ./reproduce.sh {
      CONFIG = ./reproduce-config.yaml;
    }
  );
}
