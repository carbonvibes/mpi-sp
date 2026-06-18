# Returns the current SemanticSanitizer version, as defined in `version.txt`.

{ lib }:
let
  versionFile = import ../../../../version.nix;

  version =
    if (lib.hasAttr "version" versionFile) then
      versionFile.version
    else
      builtins.throw "The `version` attribute must be set in `version.nix`";
in
version
