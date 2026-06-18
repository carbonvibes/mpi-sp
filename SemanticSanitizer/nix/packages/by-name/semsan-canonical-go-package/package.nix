{
  lib,
  buildGoModule,
}:
buildGoModule {
  pname = "semsan-canonical-go-package";
  version = lib.semsanVersion;

  src = lib.repoRootSrc [
    "go.mod"
    "go.sum"
  ];

  vendorHash = "sha256-vuDaQPCi0ui9xuP+sMahd3sl22h2fcCHkLnhKuBCufA=";

  doCheck = false;

  proxyVendor = true;
}
