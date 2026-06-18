_: {
  projectRootFile = "flake.nix";
  programs = {
    # keep-sorted start block=true
    actionlint.enable = true;
    clang-format.enable = true;
    deadnix.enable = true;
    formatjson5 = {
      enable = true;
      indent = 2;
      oneElementLines = true;
      sortArrays = true;
    };
    gofumpt.enable = true;
    keep-sorted.enable = true;
    nixfmt.enable = true;
    rustfmt.enable = true;
    shellcheck.enable = true;
    shfmt.enable = true;
    statix.enable = true;
    terraform.enable = true;
    # keep-sorted end
  };
  settings.global.excludes = [
    "case-studies/oci/nautilus"
    "case-studies/oci/crun"
  ];
}
