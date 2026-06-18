#!/usr/bin/env bash

set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
  echo "This script must be run as root." >&2
  exit 1
fi

version="0.9.1"
arch="$(uname -m)"
case "${arch}" in
x86_64 | amd64)
  asset="soft-serve_${version}_Linux_x86_64.tar.gz"
  ;;
*)
  echo "Unsupported architecture ${arch}; the artifact evaluation expects x86_64 Linux." >&2
  exit 1
  ;;
esac

tmp="$(mktemp -d)"
cleanup() {
  rm -rf "${tmp}"
}
trap cleanup EXIT

url="https://github.com/charmbracelet/soft-serve/releases/download/v${version}/${asset}"
echo "Downloading Soft-Serve ${version} from ${url}"
curl -fL "${url}" -o "${tmp}/${asset}"
tar -xzf "${tmp}/${asset}" -C "${tmp}"
install -Dm755 "${tmp}/soft-serve_${version}_Linux_x86_64/soft" /usr/local/bin/soft

/usr/local/bin/soft --version
