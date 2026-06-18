{ callPackage, llvmPackages_21, ... }:
let
  base = callPackage ../crun-harness/package.nix { };
  profileRtDir = "${llvmPackages_21.compiler-rt-libc}/lib/linux";
in
base.overrideAttrs (old: {
  pname = "crun-harness-cov";

  # CC stays as afl-clang-lto so the AFL++ forkserver runtime is linked in.
  # __AFL_LOOP detects "no AFL++ parent" at runtime (forkserver FD not open)
  # and falls back to dumb mode — runs the loop body exactly once then exits.
  # This is identical to the stub behaviour but keeps the binary compatible
  # with the Rust fuzzer for longer coverage campaigns if needed.
  #
  # Coverage flags go in preBuild, NOT in env.  If they were in
  # NIX_CFLAGS_COMPILE they would reach the configure-phase compiler probes
  # (simple `int main(){return 0;}` link tests) which then fail because the
  # profiling runtime is not in scope yet.  preBuild runs after configure has
  # already written the Makefile so every source file gets the flags but the
  # probe tests are unaffected.
  #
  # -fprofile-instr-generate stays out of NIX_LDFLAGS.  libtool calls ld.bfd
  # directly (bypassing the clang driver) and ld.bfd misparses it as its own
  # -f (auxiliary shared lib) flag which requires -shared.  Instead we link
  # libclang_rt.profile-x86_64.a explicitly via -L + -l which ld.bfd handles.
  preBuild = (old.preBuild or "") + ''
    export NIX_CFLAGS_COMPILE="$NIX_CFLAGS_COMPILE -fprofile-instr-generate -fcoverage-mapping -g"
    export NIX_LDFLAGS="$NIX_LDFLAGS -L${profileRtDir} -lclang_rt.profile-x86_64"
  '';
})
