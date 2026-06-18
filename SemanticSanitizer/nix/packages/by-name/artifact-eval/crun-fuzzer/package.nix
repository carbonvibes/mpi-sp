{
  lib,
  rustPlatform,
  python3,
}:
rustPlatform.buildRustPackage {
  pname = "crun-fuzzer";
  version = "0.0.1";

  src = lib.repoRootSrc [ "case-studies/oci" ];
  sourceRoot = "source/case-studies/oci/fuzzer";

  nativeBuildInputs = [ python3 ];

  postPatch = ''
    # To satisfy libAFL-bolts
    touch /build/README.md
  '';

  cargoLock = {
    lockFile = lib.path.append lib.repoRoot "case-studies/oci/fuzzer/Cargo.lock";
    outputHashes = {
      "libafl-0.15.4" = "sha256-Y9I2Hh3LreI1Oi1/Y4deywmmU8Ocw1dQiUTRNRI6KPM=";
    };
  };

  postInstall = ''
    mkdir -p $out/share
    cp ../corpus ../grammar.py $out/share/
  '';
}
