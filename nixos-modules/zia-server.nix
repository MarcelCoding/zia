{ config, pkgs, lib, ... }:

let
  cfg = config.services.zia-server;
  ziaServerName = name: "zia-server" + "-" + name;
  enabledServers = lib.filterAttrs (name: conf: conf.enable) config.services.zia-server.servers;
in
{
  options = {
    services.zia-server = {
      package = lib.mkOption {
        type = lib.types.package;
        default = pkgs.zia-server;
        defaultText = lib.literalExpression "pkgs.zia-server";
        description = lib.mdDoc "Which Zia Server derivation to use.";
      };

      servers = lib.mkOption {
        type = lib.types.attrsOf (lib.types.submodule ({ config, name, ... }: {
          options = {
            enable = lib.mkEnableOption (lib.mdDoc "Zia Server.");
            listen = {
              addr = lib.mkOption {
                type = lib.types.str;
                description = lib.mkDoc "The ip address zia should be listening on.";
                default = "0.0.0.0";
              };
              port = lib.mkOption {
                type = lib.types.port;
                description = lib.mkDoc "The port zia shuld be listening on.";
                default = null;
              };
            };
            upstream = lib.mkOption {
              type = lib.types.str;
              description = lib.mkDoc "The socket address of the udp upstream zia should redirect all traffic to.";
              default = null;
            };
            mode = lib.mkOption {
              type = lib.types.enum [ "ws" ];
              description = lib.mkDoc "The mode zia sould be listening with.";
              default = "ws";
            };
            openFirewall = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = lib.mdDoc "Whether to open ports in the firewall for the server.";
            };
          };
        }));
      };
    };
  };

  config = lib.mkIf (enabledServers != { }) {
    environment.systemPackages = [ cfg.package ];
    networking.firewall.allowedTCPPorts = lib.mapAttrsToList (_: conf: conf.listen.port) enabledServers;

    systemd.services = lib.mapAttrs'
      (name: conf: lib.nameValuePair (ziaServerName name) {
        description = "Zia Server - ${ziaServerName name}";

        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];

        serviceConfig = {
          ExecStart = "${cfg.package}/bin/zia-server";
          DynamicUser = true;
          User = "zia-server";

          Environment = [
            "ZIA_LISTEN_ADDR=${conf.listen.addr}:${toString conf.listen.port}"
            "ZIA_UPSTREAM=${conf.upstream}"
            "ZIA_MODE=${conf.mode}"
          ];
        };
      })
      enabledServers;
  };
}
