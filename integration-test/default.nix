{
  stdenvNoCC,
  substituteAll,
  formats,
  dockerImage,
  oranc,
  dockerTools,
  nix,
  tini,
  buildEnv,
  coreutils,
  curl,
  findutils,
  shadow,
}: let
  packageForTest = "github:nixos/nixpkgs/nixos-unstable#coreutils";
  composeFile = (formats.yaml {}).generate "container-compose-yml" {
    services = {
      registry = {
        image = "registry";
      };
      oranc = {
        image = "oranc:${dockerImage.imageTag}";
        environment = [
          "EXTRA_ARGS=--no-ssl"
        ];
      };
      oranc-test-script = {
        image = "oranc-test-script:${dockerImage.imageTag}";
        environment = [
          "PACKAGE_FOR_TEST=${packageForTest}"
        ];
        depends_on = {
          registry = {condition = "service_started";};
          oranc = {condition = "service_started";};
        };
      };
    };
  };
  testScript = substituteAll {
    src = ./test.sh;
    isExecutable = true;
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
        dockerTools.caCertificates
      ];
    };
    config = {
      Entrypoint = ["${tini}/bin/tini" "--"];
      Cmd = ["${testScript}"];
      Env = [
        "RUST_LOG=oranc=info"
        # required by nix
        "USER=nobody"
      ];
    };
  };
  driver = substituteAll {
    src = ./driver.sh;
    isExecutable = true;
    inherit (stdenvNoCC) shell;
    inherit composeFile dockerImage testScriptDockerImage;
  };
in
  stdenvNoCC.mkDerivation (self: {
    name = "oranc-integration-test";
    dontUnpack = true;
    installPhase = ''
      install -D "${driver}" "$out/bin/${self.name}"
    '';
  })
