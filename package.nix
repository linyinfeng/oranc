{
  rustPlatform,
  lib,
  pkg-config,
  installShellFiles,
  aws-lc,
  openssl,
  sqlite,
  zstd,
}:
let
  inherit (builtins) readFile fromTOML;
  cargoTOML = fromTOML (readFile ./Cargo.toml);
in
rustPlatform.buildRustPackage (_finalAttrs: {
  inherit (cargoTOML.package) name version;
  src =
    with lib.fileset;
    toSource {
      root = ./.;
      fileset = unions [
        ./Cargo.toml
        ./Cargo.lock
        ./src
      ];
    };
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [
    pkg-config
    installShellFiles
  ];
  buildInputs = [
    aws-lc
    openssl
    sqlite
    zstd
  ];

  postInstall = ''
    installShellCompletion --cmd oranc \
      --bash <($out/bin/oranc completion bash) \
      --fish <($out/bin/oranc completion fish) \
      --zsh  <($out/bin/oranc completion zsh)
  '';

  meta = {
    inherit (cargoTOML.package) description homepage;
    mainProgram = "oranc";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [ yinfeng ];
  };
})
