{
  lib,
  pkgs,
}:
let
  libs = with pkgs; [
    libbpf
    linuxHeaders
  ];

  clang-wrapped = pkgs.llvmPackages_21.clang.overrideAttrs (oldAttrs: {
    postFixup = oldAttrs.postFixup + ''
      for o in $out/bin/*; do
        mv $o $out/bin/$(basename $o)-wrapped
      done
    '';
  });
in
pkgs.writeShellApplication {
  name = "semsan-codegen";

  runtimeInputs =
    with pkgs;
    [
      go
      clang-wrapped
      llvmPackages_21.clang-unwrapped
      llvmPackages_21.llvm
      nix-update
    ]
    ++ libs;

  runtimeEnv = {
    C_INCLUDE_PATH = lib.concatStringsSep ":" (lib.map (x: "${x}/include") libs);
    LIBRARY_PATH = lib.concatStringsSep " " (lib.map (x: "${x}/lib") libs);
  };

  text = lib.readFile (lib.path.append lib.repoRoot "scripts/codegen.sh");
}
