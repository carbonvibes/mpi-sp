{
  lib,
  writeShellApplication,
  phoronix-test-suite,
  postgresql,
  apacheHttpd,
  bc,
  gnutar,
  xz,
}:
writeShellApplication {
  name = "macro-benchmark";

  runtimeInputs = [
    phoronix-test-suite
    postgresql
    apacheHttpd
    bc
    gnutar
    xz
  ];

  text = lib.readFile ./macro-benchmark.sh;
}
