# Returns a package set originating from the root of the repository.
# The `files` attribute is a list of paths relative to the root of the repository.

{ lib }:
files:
let
  filteredFiles = lib.map (subpath: lib.path.append lib.repoRoot subpath) files;
in
lib.fileset.toSource {
  root = lib.repoRoot;
  fileset = lib.fileset.unions filteredFiles;
}
