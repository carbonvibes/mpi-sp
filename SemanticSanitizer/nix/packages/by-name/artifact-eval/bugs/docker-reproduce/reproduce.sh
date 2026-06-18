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

sudo "$(command -v mprocs)" 'sleep 2 && @CLOBBER@' 'sleep 2 && @ATTACK@' "${cli} attach --config @CONFIG@"
