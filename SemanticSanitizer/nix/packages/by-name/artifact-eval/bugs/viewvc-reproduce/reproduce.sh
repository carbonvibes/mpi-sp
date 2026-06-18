#!/usr/bin/env bash

set -euo pipefail

if [ ! -f "./semsan-cli" ]; then
  echo "Error: ./semsan-cli not found. Please build semsan-cli first as per the instructions in the README."
  exit 1
fi
cli="$(realpath ./semsan-cli)"

if [ ! -f "nix/packages/by-name/artifact-eval/micro-benchmark/config-general.yaml" ]; then
  echo "Error: This script must be run from the root of the repository."
  exit 1
fi

dir="$(mktemp -d)"
echo "Running reproduction in temporary directory: ${dir}"
cleanup() {
  rm -rf "${dir}"
}
trap cleanup EXIT
cd "${dir}"

sample_rcs='@VULN@/lib/vclib/ccvs/rcsparse/test-data/default,v'

make_repo() {
  local repo="$1"
  local filename="$2"

  mkdir -p "${repo}/CVSROOT"
  : >"${repo}/CVSROOT/config"
  cp "${sample_rcs}" "${repo}/${filename},v"
}

unprivileged_repo="${dir}/unprivileged"
privileged_repo="${dir}/privileged"
make_repo "${unprivileged_repo}" public.txt
make_repo "${privileged_repo}" secret.txt

config="${dir}/viewvc.conf"
cat >"${config}" <<EOF
[general]
cvs_roots = unprivileged: ${unprivileged_repo}, privileged: ${privileged_repo}
default_root = unprivileged

[options]
template_dir = @VULN@/templates/default
authorizer = forbiddenre

[utilities]
rcs_dir = @RCS@/bin
diff = @DIFF@/bin/diff

[authz-forbiddenre]
forbiddenre = ^privileged(/|$)
EOF

sudo "$(command -v mprocs)" "@PYTHON@/bin/python @VULN@/bin/standalone.py -h 127.0.0.1 -p 49152 -c ${config}" "${cli} attach --config @CONFIG@"
