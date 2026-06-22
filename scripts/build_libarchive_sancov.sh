#!/usr/bin/env bash
# build libarchive 3.7.2 static with clang sancov trace-pc-guard.
# the .a is linked into fuzz_libafl (not the system .so) so the observer sees
# edges inside libarchive's parsers, not just the 3 in the harness wrapper.
# run once from any dir: bash scripts/build_libarchive_sancov.sh
# output: vendor/libarchive-sancov/{include,lib}

set -euo pipefail

VERSION=3.7.2
TARBALL="libarchive-${VERSION}.tar.gz"
URL="https://github.com/libarchive/libarchive/releases/download/v${VERSION}/${TARBALL}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENDOR_DIR="${REPO_ROOT}/vendor"
INSTALL_DIR="${VENDOR_DIR}/libarchive-sancov"
TMP_DIR="/tmp/libarchive-sancov-build"

if [[ -f "${INSTALL_DIR}/lib/libarchive.a" ]]; then
    echo "[libarchive-sancov] already built at ${INSTALL_DIR} — skipping"
    exit 0
fi

echo "[libarchive-sancov] downloading ${TARBALL}..."
mkdir -p "${TMP_DIR}"
wget -q --show-progress "${URL}" -O "${TMP_DIR}/${TARBALL}"

echo "[libarchive-sancov] extracting..."
tar xf "${TMP_DIR}/${TARBALL}" -C "${TMP_DIR}"

echo "[libarchive-sancov] configuring..."
BUILD_DIR="${TMP_DIR}/libarchive-${VERSION}/build"
mkdir -p "${BUILD_DIR}"
cmake -S "${TMP_DIR}/libarchive-${VERSION}" -B "${BUILD_DIR}" \
    -DCMAKE_C_COMPILER=clang \
    -DCMAKE_C_FLAGS="-fsanitize-coverage=trace-pc-guard -O1 -fno-omit-frame-pointer" \
    -DBUILD_SHARED_LIBS=OFF \
    -DCMAKE_BUILD_TYPE=RelWithDebInfo \
    -DCMAKE_INSTALL_PREFIX="${INSTALL_DIR}" \
    -DENABLE_TEST=OFF \
    -DENABLE_ICONV=OFF \
    -DENABLE_OPENSSL=OFF \
    -DENABLE_NETTLE=OFF \
    -DENABLE_MBEDTLS=OFF \
    -DENABLE_EXPAT=OFF \
    -DENABLE_LIBXML2=OFF \
    -DENABLE_TAR=OFF \
    -DENABLE_CPIO=OFF \
    -DENABLE_CAT=OFF \
    -DENABLE_UNZIP=OFF \
    -Wno-dev \
    -DCMAKE_POLICY_DEFAULT_CMP0042=NEW \
    2>&1

echo "[libarchive-sancov] building (this takes ~1 min)..."
cmake --build "${BUILD_DIR}" --parallel "$(nproc)"

echo "[libarchive-sancov] installing to ${INSTALL_DIR}..."
cmake --install "${BUILD_DIR}"

echo ""
echo "[libarchive-sancov] done."
echo "  Headers : ${INSTALL_DIR}/include"
echo "  Library : ${INSTALL_DIR}/lib/libarchive.a"
echo ""
echo "Now rebuild the fuzzer:"
echo "  cd mutator && cargo build --bin fuzz_libafl"
