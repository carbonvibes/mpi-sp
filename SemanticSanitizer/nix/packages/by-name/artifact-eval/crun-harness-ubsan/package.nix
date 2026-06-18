{ callPackage, ... }:
let
  base = callPackage ../crun-harness/package.nix { };
in
base.overrideAttrs (old: {
  pname = "crun-harness-ubsan";

  # mirror of crun-harness-asan, with UBSan. abort-on-error isn't baked in (the
  # patch has no ENABLE_UBSAN hook), so the launcher sets UBSAN_OPTIONS at runtime.
  env = (old.env or { }) // {
    AFL_USE_UBSAN = "1";
    NIX_CFLAGS_COMPILE = "-g";
  };
})
