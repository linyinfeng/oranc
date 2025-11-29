{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

    crane.url = "github:ipetkov/crane";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } (
      {
        self,
        inputs,
        lib,
        ...
      }:
      {
        systems = [
          "x86_64-linux"
          "aarch64-linux"
        ];
        imports = [
          inputs.flake-parts.flakeModules.easyOverlay
          inputs.treefmt-nix.flakeModule
        ];
        flake = {
          nixosModules.oranc = ./nixos/module.nix;
        };
        perSystem =
          {
            config,
            self',
            pkgs,
            system,
            ...
          }:
          let
            craneLib = inputs.crane.mkLib pkgs;
            src = craneLib.cleanCargoSource (craneLib.path ./.);
            bareCommonArgs = {
              inherit src;
              nativeBuildInputs = with pkgs; [
                pkg-config
                installShellFiles
              ];
              buildInputs = with pkgs; [
                openssl
                sqlite
              ];
              # TODO https://github.com/ipetkov/crane/issues/385
              doNotLinkInheritedArtifacts = true;
            };
            cargoArtifacts = craneLib.buildDepsOnly bareCommonArgs;
            commonArgs = bareCommonArgs // {
              inherit cargoArtifacts;
            };
          in
          {
            packages = {
              oranc = craneLib.buildPackage (
                commonArgs
                // {
                  postInstall = ''
                    installShellCompletion --cmd oranc \
                      --bash <($out/bin/oranc completion bash) \
                      --fish <($out/bin/oranc completion fish) \
                      --zsh  <($out/bin/oranc completion zsh)
                  '';
                  meta.mainProgram = "oranc";
                }
              );
              default = config.packages.oranc;
              dockerImage = pkgs.dockerTools.buildImage {
                name = "oranc";
                tag = self.sourceInfo.rev or null;
                copyToRoot = pkgs.buildEnv {
                  name = "oranc-env";
                  paths = [
                    pkgs.dockerTools.caCertificates
                  ];
                };
                config = {
                  Entrypoint = [
                    "${pkgs.tini}/bin/tini"
                    "--"
                  ];
                  Cmd =
                    let
                      start = pkgs.writeShellScript "start-oranc" ''
                        exec ${config.packages.oranc}/bin/oranc server \
                          --listen "[::]:80" \
                          $EXTRA_ARGS "$@"
                      '';
                    in
                    [ "${start}" ];
                  Env = [
                    "RUST_LOG=oranc=info"
                    "EXTRA_ARGS="
                  ];
                  ExposedPorts = {
                    "80/tcp" = { };
                  };
                  Labels =
                    {
                      "org.opencontainers.image.title" = "oranc";
                      "org.opencontainers.image.description" = "OCI Registry As Nix Cache";
                      "org.opencontainers.image.url" = "https://github.com/linyinfeng/oranc";
                      "org.opencontainers.image.source" = "https://github.com/linyinfeng/oranc";
                      "org.opencontainers.image.licenses" = "MIT";
                    }
                    // lib.optionalAttrs (self.sourceInfo ? rev) {
                      "org.opencontainers.image.revision" = self.sourceInfo.rev;
                    };
                };
              };
              integrationTestScript = pkgs.callPackage ./integration-test {
                inherit (config.packages) dockerImage oranc;
              };
            };
            overlayAttrs.oranc = config.packages.oranc;
            checks = {
              inherit (self'.packages) oranc dockerImage;
              doc = craneLib.cargoDoc commonArgs;
              fmt = craneLib.cargoFmt { inherit src; };
              nextest = craneLib.cargoNextest commonArgs;
              clippy = craneLib.cargoClippy (
                commonArgs
                // {
                  cargoClippyExtraArgs = "--all-targets -- --deny warnings";
                }
              );
              defaultNix = import ./default.nix { config = { inherit system; }; };
            };
            treefmt = {
              projectRootFile = "flake.nix";
              programs = {
                nixfmt.enable = true;
                rustfmt.enable = true;
                shfmt.enable = true;
                prettier.enable = true;
              };
            };
            devShells.default = pkgs.mkShell {
              inputsFrom = lib.attrValues self'.checks;
              packages = with pkgs; [
                rustup
                rust-analyzer
              ];
            };
          };
      }
    );
}
