{
  writeShellApplication,
  mprocs,
  replaceVars,
  lib,
  git,
  lighttpd,
}:
let
  # At this time, the Git version shipped with nixpkgs is not patched,
  # so we use the default version for reproduction.
  git-vuln = git;
in
writeShellApplication {
  name = "git-reproduce";

  runtimeInputs = [
    mprocs
    lighttpd
  ];

  text = lib.readFile (
    replaceVars ./reproduce.sh {
      VULN = git-vuln;
      LIGHTTPD = lighttpd;
      CONFIG = ./reproduce-config.yaml;
    }
  );
}
