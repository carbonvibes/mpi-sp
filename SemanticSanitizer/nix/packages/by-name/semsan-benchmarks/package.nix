{
  lib,
  stdenv,
}:
stdenv.mkDerivation {
  name = "semsan-benchmarks";
  src = lib.repoRootSrc [ "eval/testsuite" ];

  buildPhase = ''
    runHook preBuild
    cd eval/testsuite
    make all
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    cp build/* $out/bin/
    runHook postInstall
  '';
}
