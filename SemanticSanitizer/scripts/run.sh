#!/usr/bin/env bash

set -euo pipefail

echo "Running..."
go build -o main cli/main.go

sudo ./main "$@"
