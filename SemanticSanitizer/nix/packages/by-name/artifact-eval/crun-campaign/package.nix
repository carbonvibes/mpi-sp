{
  writeShellApplication,
  artifact-eval,
  mprocs,
  replaceVars,
  lib,
}:
writeShellApplication {
  name = "crun-campaign";

  runtimeInputs = [ mprocs ];

  text = lib.readFile (
    replaceVars ./fuzz.sh {
      FUZZER = artifact-eval.crun-fuzzer;
      HARNESS = artifact-eval.crun-harness;
      CONFIG = ./fuzz-config.yaml;
    }
  );
}
