{
  stdenvNoCC,
  replaceVars,
  formats,
  dockerImage,
  oranc,
  dockerTools,
  nix,
  bash,
  tini,
  buildEnv,
  coreutils,
  curl,
  findutils,
  shadow,
  gnused,
  tcping-go,
}:
let
  packageForTest = "github:nixos/nixpkgs/nixos-unstable#coreutils";
  composeFile = (formats.yaml { }).generate "container-compose.yml" {
    services = {
      registry = {
        image = "registry";
      };
      oranc = {
        image = "oranc:${dockerImage.imageTag}";
        environment = [
          "EXTRA_ARGS=--no-ssl"
          "RUST_LOG=oranc=info"
        ];
        depends_on = {
          registry = {
            condition = "service_started";
          };
        };
      };
      oranc-test-script = {
        image = "oranc-test-script:${dockerImage.imageTag}";
        environment = [
          "PACKAGE_FOR_TEST=${packageForTest}"
        ];
        depends_on = {
          registry = {
            condition = "service_started";
          };
          oranc = {
            condition = "service_started";
          };
        };
      };
    };
  };
  testScript = replaceVars ./test.sh {
    inherit (stdenvNoCC) shell;
  };
  testScriptDockerImage = dockerTools.buildImageWithNixDb {
    name = "oranc-test-script";
    tag = dockerImage.imageTag;
    copyToRoot = buildEnv {
      name = "image-root";
      paths = [
        coreutils
        oranc
        nix
        curl
        findutils
        shadow
        tcping-go
        gnused
        dockerTools.caCertificates
      ];
    };
    config = {
      Entrypoint = [
        "${tini}/bin/tini"
        "--"
      ];
      Cmd = [
        "${bash}/bin/bash"
        "${testScript}"
      ];
      Env = [
        "RUST_LOG=oranc=info"
        # required by nix
        "USER=nobody"
      ];
    };
  };
  driver = replaceVars ./driver.sh {
    inherit (stdenvNoCC) shell;
    inherit composeFile dockerImage testScriptDockerImage;
  };
in
stdenvNoCC.mkDerivation (self: {
  name = "oranc-integration-test";
  dontUnpack = true;
  installPhase = ''
    install -D "${driver}" "$out/bin/${self.name}"
    install -D "${composeFile}" "$out/share/oranc/container-compose.yml"
  '';
})
