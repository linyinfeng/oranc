{
  config,
  pkgs,
  lib,
  ...
}: let
  cfg = config.services.oranc;
in {
  options = {
    services.oranc = {
      enable = lib.mkEnableOption "oranc";
      package = lib.mkOption {
        type = lib.types.package;
        default = pkgs.oranc;
        defaultText = "pkgs.oranc";
        description = ''
          Which oranc package to use.
        '';
      };
      listen = lib.mkOption {
        type = lib.types.str;
        default = "[::]:8080";
        description = ''
          Socket address to listen on.
        '';
      };
      log = lib.mkOption {
        type = lib.types.str;
        default = "oranc=info";
        description = ''
          Log configuration in RUST_LOG format.
        '';
      };
    };
  };
  config = lib.mkIf cfg.enable {
    systemd.services.oranc = {
      script = ''
        ${cfg.package}/bin/oranc --listen "${cfg.listen}"
      '';
      serviceConfig = {
        DynamicUser = true;
      };
      environment.RUST_LOG = cfg.log;
      wantedBy = ["multi-user.target"];
    };
  };
}
