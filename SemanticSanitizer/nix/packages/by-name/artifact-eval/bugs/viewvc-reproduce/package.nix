{
  writeShellApplication,
  mprocs,
  replaceVars,
  lib,
  rcs,
  diffutils,
  python2,
  fetchFromGitHub,
}:
let
  viewvc-vuln = fetchFromGitHub {
    owner = "viewvc";
    repo = "viewvc";
    rev = "1.2.3";
    sha256 = "sha256-ojaQ5BY1wzcShUmPMnVPYSoICS9a+O/v7AxCAtsD32I=";
  };
in
writeShellApplication {
  name = "viewvc-reproduce";

  runtimeInputs = [
    mprocs
    rcs
    diffutils
    python2
  ];

  text = lib.readFile (
    replaceVars ./reproduce.sh {
      VULN = viewvc-vuln;
      PYTHON = python2;
      RCS = rcs;
      DIFF = diffutils;
      CONFIG = ./reproduce-config.yaml;
    }
  );
}
