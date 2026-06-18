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
echo "Running campaign in temporary directory: ${dir}"
cd "$dir"

mkdir corpus
cp @FUZZER@/share/corpus corpus/1

sudo "$(command -v mprocs)" 'tail --retry --follow=name fuzzer_stats' '@FUZZER@/bin/forkserver_simple -g @FUZZER@/share/grammar.py @HARNESS@/bin/crun @@' "${cli} attach --config @CONFIG@"
