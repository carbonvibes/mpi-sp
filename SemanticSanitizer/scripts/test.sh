#!/usr/bin/env bash

set -euo pipefail

echo "Running Unit Tests"
go test -v ./...

echo "Running Integration Tests"
go test -c -o main_test -tags integration test/main_test.go
trap 'rm -f main_test' EXIT

sudo ./main_test -test.v
