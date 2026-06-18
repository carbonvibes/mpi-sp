{ callPackage, ... }:
let
  base = callPackage ../crun-harness/package.nix { };
in
base.overrideAttrs (old: {
  pname = "crun-harness-asan";

  # AFL_USE_ASAN=1 tells afl-clang-lto to inject -fsanitize=address and link
  # the ASAN runtime itself — it knows where its own LLVM runtime lives.
  # Passing -fsanitize=address directly via NIX_CFLAGS_COMPILE breaks the
  # configure compiler probe because the ASAN runtime isn't in the sandbox
  # linker path at that stage.
  env = (old.env or { }) // {
    AFL_USE_ASAN = "1";
    NIX_CFLAGS_COMPILE = "-DENABLE_ASAN -g";
  };
})
