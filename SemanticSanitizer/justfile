codegen:
    nix run .#semsan-codegen

build:
    go build -o semsan-cli cli/main.go

test:
    ./scripts/test.sh

fmt:
    nix fmt

run *args:
    ./scripts/run.sh {{ args }}

bootstrap *args:
    ./scripts/bootstrap.sh {{ args }}