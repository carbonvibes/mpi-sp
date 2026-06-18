{
  lib,
  stdenv,
  fetchFromGitHub,
  criu,
  libcap,
  libseccomp,
  systemdMinimal,
  yajl,
  autoreconfHook,
  go-md2man,
  pkg-config,
  python3,
  overrideCC,
  aflplusplus,
  wrapCCWith,
  wrapBintoolsWith,
  binutils-unwrapped,
  glibc,
  makeSetupHook,
  writeText,
}:
let
  aflplusplusCC = wrapCCWith {
    cc = aflplusplus;
    bintools = wrapBintoolsWith {
      bintools = binutils-unwrapped;
      libc = glibc;
    };
  };
  aflplusplusStdenv = overrideCC stdenv aflplusplusCC;
  aflplusplusSetupHook =
    makeSetupHook
      {
        name = "aflxx-setup-hook";
      }
      (
        writeText "afl-hook.sh" ''
          preConfigurePhases+=" aflSetupPhase"

          aflSetupPhase() {
            export CC=${aflplusplus}/bin/afl-clang-lto
            export CXX=${aflplusplus}/bin/afl-clang-lto++
          }
        ''
      );
in
aflplusplusStdenv.mkDerivation rec {
  pname = "crun-harness";
  version = "1.23.1";

  src = fetchFromGitHub {
    owner = "containers";
    repo = "crun";
    rev = "d20b23dba05e822b93b82f2f34fd5dada433e0c2";
    sha256 = "sha256-DPpy9WaKvkpJErsK5qtRJ+tiHxSHB6a+LlIbRyU2FlE=";
    fetchSubmodules = true;
    leaveDotGit = true;
    postFetch = ''
      cd $out
      git rev-parse HEAD > COMMIT
      rm -rf .git
    '';
  };

  nativeBuildInputs = [
    autoreconfHook
    go-md2man
    pkg-config
    python3
  ];

  buildInputs = [
    aflplusplusSetupHook
    criu
    libcap
    libseccomp
    systemdMinimal
    yajl
  ];

  enableParallelBuilding = true;
  strictDeps = true;

  env = {
    NIX_LDFLAGS = "-lcriu";
  };

  patches = [
    (lib.path.append lib.repoRoot "case-studies/oci/0001-crun-add-harness.patch")
  ];

  postPatch = ''
    echo ${version} > .tarball-version
    echo "#define GIT_VERSION \"$(cat COMMIT)\"" > git-version.h
  '';
}
