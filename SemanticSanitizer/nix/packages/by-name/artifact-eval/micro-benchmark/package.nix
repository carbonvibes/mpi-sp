{
  lib,
  writeShellApplication,
  semsan-benchmarks,
}:
writeShellApplication {
  name = "micro-benchmark";

  runtimeInputs = [ semsan-benchmarks ];

  text = lib.readFile ./micro-benchmark.sh;
}
