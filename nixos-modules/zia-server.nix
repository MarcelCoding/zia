{ config, pkgs, lib, ... }:

let
  cfg = config.services.zia-server;
  ziaServerName = name: "zia-server-${name}";
  enabledServers = lib.filterAttrs (_: conf: conf.enable) config.services.zia-server.servers;
in
{
  options = {
    services.zia-server = {
      package = lib.mkPackageOption pkgs "zia-server" { };

      servers = lib.mkOption {
        type = lib.types.attrsOf (lib.types.submodule {
          options = {
            enable = lib.mkEnableOption "Zia Server";
            listen = {
              addr = lib.mkOption {
                type = lib.types.str;
                description = "The ip address zia should be listening on.";
                default = "::";
              };
              port = lib.mkOption {
                type = lib.types.port;
                description = "The port zia should be listening on.";
                default = null;
              };
            };
            upstream = lib.mkOption {
              type = lib.types.str;
              description = "The socket address of the udp upstream zia should redirect all traffic to.";
              default = null;
            };
            mode = lib.mkOption {
              type = lib.types.enum [ "ws" ];
              description = "The mode zia should be listening with.";
              default = "ws";
            };
            openFirewall = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Whether to open ports in the firewall for the server.";
            };
          };
        });
      };
    };
  };

  config = lib.mkIf (enabledServers != { }) {
    networking.firewall.allowedTCPPorts = lib.mkMerge (lib.mapAttrsToList (_: conf: if conf.openFirewall then [ conf.listen.port ] else [ ]) enabledServers);

    systemd.services = lib.mapAttrs'
      (name: conf: lib.nameValuePair (ziaServerName name) {
        description = "Zia Server (${name})";

        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];

        environment = {
          ZIA_LISTEN_ADDR = "${if (lib.hasInfix ":" conf.listen.addr) then "[${conf.listen.addr}]" else conf.listen.addr}:${toString conf.listen.port}";
          ZIA_UPSTREAM = conf.upstream;
          ZIA_MODE = conf.mode;
        };

        serviceConfig = {
          ExecStart = lib.getExe cfg.package;
          DynamicUser = true;
          User = "zia-server-${name}";
        };
      })
      enabledServers;
  };
}
