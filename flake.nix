{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    flake-utils.url = "github:numtide/flake-utils";

    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

    crane.url = "github:ipetkov/crane";
    crane.inputs.nixpkgs.follows = "nixpkgs";
    crane.inputs.flake-compat.follows = "flake-compat";
    crane.inputs.flake-utils.follows = "flake-utils";
    crane.inputs.rust-overlay.follows = "rust-overlay";

    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.flake-utils.follows = "flake-utils";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

    flake-compat.url = "github:edolstra/flake-compat";
    flake-compat.flake = false;
  };

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;}
    ({
      self,
      inputs,
      lib,
      ...
    }: {
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
      perSystem = {
        config,
        self',
        pkgs,
        system,
        ...
      }: let
        craneLib = inputs.crane.lib.${system};
        src = craneLib.cleanCargoSource ./.;
        bareCommonArgs = {
          inherit src;
          nativeBuildInputs = with pkgs; [
            pkg-config
          ];
          buildInputs = with pkgs; [
            openssl
            sqlite
          ];
        };
        cargoArtifacts = craneLib.buildDepsOnly bareCommonArgs;
        commonArgs = bareCommonArgs // {inherit cargoArtifacts;};
      in {
        packages = {
          oranc = craneLib.buildPackage commonArgs;
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
              Entrypoint = ["${pkgs.tini}/bin/tini" "--"];
              Cmd = let
                start = pkgs.writeShellScript "start-oranc" ''
                  exec ${config.packages.oranc}/bin/oranc server \
                    --listen "[::]:80" \
                    --upstream "$UPSTREAM" \
                    --ignore-upstream "$IGNORE_UPSTREAM" \
                    $EXTRA_ARGS "$@"
                '';
              in ["${start}"];
              Env = [
                "RUST_LOG=oranc=info"
                "UPSTREAM=https://cache.nixos.org"
                "IGNORE_UPSTREAM=nix-cache-info"
                "EXTRA_ARGS="
              ];
              ExposedPorts = {
                "80/tcp" = {};
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
        };
        overlayAttrs.oranc = config.packages.oranc;
        checks = {
          inherit (self'.packages) oranc dockerImage;
          doc = craneLib.cargoDoc commonArgs;
          fmt = craneLib.cargoFmt {inherit src;};
          nextest = craneLib.cargoNextest commonArgs;
          clippy = craneLib.cargoClippy (commonArgs
            // {
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            });
        };
        treefmt = {
          projectRootFile = "flake.nix";
          programs = {
            alejandra.enable = true;
            rustfmt.enable = true;
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
    });
}
