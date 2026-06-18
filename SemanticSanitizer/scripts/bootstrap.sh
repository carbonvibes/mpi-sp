#!/usr/bin/env bash

set -euo pipefail

echo "Running..."
go build -o bootstrap-cli bootstrap/main.go

./bootstrap-cli "$@"
