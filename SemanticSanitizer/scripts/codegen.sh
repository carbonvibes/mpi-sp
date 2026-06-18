#!/usr/bin/env bash

set -euo pipefail

go mod tidy
go generate ./...

nix-update --version=skip --flake legacyPackages.x86_64-linux.semsan-canonical-go-package
