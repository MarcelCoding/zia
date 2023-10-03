# Zia

Proxy udp over websocket, useful to use Wireguard in restricted networks.

Basic example:

```mermaid
graph LR
    WC[Wireguard Client] ---|UDP| B[Zia Client]
    B ---|Websocket| C[Zia Server]
    C ---|UDP| D[Wireguard Server]
```

The benefit is that Websocket uses Http. If you are in a restricted network where you can only access external services,
and you can only use a provided Http proxy you can proxy your Wireguard Udp traffic over Websocket.

```mermaid
graph LR
    WC[Wireguard Client] ---|UDP| B[Zia Client]
    B ---|Websocket| C[Http Proxy]
    C ---|Websocket| D[Zia Server]
    D ---|UDP| E[Wireguard Server]
```

## Mode

| Name      | Description                                                                                                                                                                                                 |
|-----------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Websocket | The UDP datagrams are wrapped inside websocket frames. These frames are then transmitted to the server and there unwrapped.                                                                                 |
| TCP       | The UDP datagrams are prefixed with a 16 bit length of the datagram and then transmitted to the server in TCP packages. At the server these packages are unwrapped and forwarded to the actual UDP upstream. |

The client is capable of doing a TLSv2 or TLSv3 handshake, the server isn't able to handle TLS requests. The client is also able to do a TLS
handshake for a HTTPS proxy. In a case where an end to end (zia-client <-> zia-server) TLS
encryption should happen, you have to proxy the traffic for the server using a reverse proxy.

## Client

Just download the correct binary from the latest release or use the docker image:

```
ghcr.io/marcelcoding/zia-client
```

Environment variables:

```bash
ZIA_LISTEN_ADDR=127.0.0.1:8080 # local udp listener
ZIA_UPSTREAM=ws://domain.tld:1234 # your zia server instance (ws(s) or tcp(s))
# ZIA_PROXY=http://user:pass@proxy.tld:8080 # optional http(s) proxy
```

## Server

Just download the correct binary from the latest release or use the docker image:

```
ghcr.io/marcelcoding/zia-server
```

Environment variables:

```bash
ZIA_LISTEN_ADDR=0.0.0.0:1234 # public websocket listener (client -> ZIA_UPSTREAM)
ZIA_UPSTREAM=domain.tld:9999 # your actual udp service e.g. wireguard listener
ZIA_MODE=WS # WS or TCP see client -> ZIA_UPSTREAM
```
