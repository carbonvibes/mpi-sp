# SemanticSanitizer (SemSan)

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://github.com/llvm/llvm-project/blob/release/19.x/LICENSE.TXT)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.20020040.svg)](https://doi.org/10.5281/zenodo.20020040)

This repo contains the code from our paper: `SemSan : a Configurable Sanitizer
for Detecting System-Level Semantic Bugs` published in WOOT'26. Additonally, the
complete artifact can be found on [Zenodo](https://zenodo.org/records/20020040),
[DOI:10.5281/zenodo.20020040](https://doi.org/10.5281/zenodo.20020040). To
replicate our experiments, please check this [page](ARTIFACT_EVAL.md).

> [!WARNING]
> This is an academic prototype, which is not meant for use in
> production environments.

## Prerequisites

- Nix installation with the `nix-command` and `flakes` features
  enabled. For the best installation experience, the [Determinate
  systems Nix Installer](https://docs.determinate.systems/determinate-nix/)
  is recommended, which auto-enables the aforementioned features,
  requiring no further configuration besides running the installation
  script.
- All required dependencies for SemanticSanitizer are available through
  the Nix development shell, which can be entered with:

  ```sh
  nix develop
  ```

- (Although cross-compilation should technically be possible, it's
  not tested in CI. Therefore, an x86_64-linux system is recommended
  for building and working on SemanticSanitizer.)

## How to Build

This section describes how to build SemanticSanitizer from the source
code available in this repository.

### Code Generation

Before the CLI can be built, the BPF C code needs to be compiled to
object files, and the Go binding code needs to be auto-generated.

The code generation is run with the following command:

```sh
just codegen
```

This should leave you with several `*_bpfe{l,b}.go` and `*_bpfe{l,b}.o`
files in the `internal/bpf/*` directories of the repository.

### CLI Build

For most users, the CLI is the core component of SemanticSanitizer that
needs to be built.

It can be built with the following command:

```sh
just build
```

This will leave the resulting (standalone) binary at `./semsan-cli`.

## How to Use

This section describes how to use the SemanticSanitizer CLI.

### Quickstart

First of all, generate a configuration file:

```sh
./semsan-cli config generate
```

Fill out the configuration file according to your use-case. For
example, to find all `dirownership` violations for the `foobar` binary
on the system, use the following configuration:

```yaml
comm: "foobar"
dirOwnership: true
```

Then, run SemanticSanitizer with:

```sh
./semsan-cli attach
```

## How to Work on the Repository

This section describes how to properly work with the code of
SemanticSanitizer.

### Iterating on Changes

For quickly running SemanticSanitizer from the code in this repository,
e.g. to rapidly iterate on local changes, the following command can
be used instead, which provides a shorthand to building and running
SemanticSanitizer with the given arguments from the local code:

```sh
just run <..args>
```

### Adding a new Sanitizer

For adding a new sanitizer, the most trivial way is to use the
[bootstrapping mechanism](./bootstrap/).

First, generate a YAML sanitizer specification with the following
command:

```sh
just bootstrap spec
```

Then, edit the YAML file accordingly based on the sanitizer that
should be built. The YAML layout should be mostly self-explanatory.

Then, generate the required code from the specification with the
following command:

```sh
just bootstrap sanitizer -s spec.yaml
```

Dependending on the sanitizer's complexity, further changes on the
code might be necessary, e.g. to properly implement cross-sanitizer
statefulness. To do so, simply edit the generated `.bpf.c` and `.go`
files accordingly.

Note that the bootstrap mechanism only makes an ease-of-use feature.
New sanitizers can also be added by starting from scratch and writing
the `.bpf.c` and `.go` files by hand.

### Tests

SemanticSanitizer has a comprehensive set of unit and integration
tests for the framework core as well as the individual examplary
sanitizers.

The tests can be run with:

```sh
just test
```

### Formatting

To format the code based on the formatters distributed with the Nix
development shell, the following command can be used:

```sh
just fmt
```

## Credits

Authored by [Moritz Sanft](https://github.com/msanft).
