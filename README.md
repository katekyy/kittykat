# kittykat

`kittykat` is an on-demand rotating HTTP proxy that routes traffic through the Tor network.
It dynamically creates isolated Tor circuits for each authorization token, ensuring better anonymity, reduced cross-request correlation
and it can provide a good way to stop your scrapers from getting blocked or rate limited if you juggle the tokens right.

## Features

  * __Tor Circuit Isolation__: For each authentication token in the `Proxy-Authorization` header we generate a new isolation token which ensures that connection
    will use a different circuit (if you want to know more about that, see Arti's [TorClient](https://tpo.pages.torproject.net/core/doc/rust/arti_client/struct.TorClient.html)).

  * __Anoynmous Circuit Isolation__: When the authorization header is not found, `kittykat` will generate a temporary isolation token so you can use it even when your client doesn't support proxy authorization (it is highly unrecommended though).

  * __Automatic Cleanup__: Idle circuits are purged after a configurable TTL (default: 10s).

  * __Bidirectional Tunneling__: HTTPs/CONNECT support for secure traffic.


## Usage

### Basic Proxy Setup

Configure your client to use a HTTP proxy.

```bash
# Using curl.
curl -x http://localhost:8118 https://check.torproject.org/api/ip
```

As you can see we can use `kittykat` without authorization as it'll generate an anonymous token for us internally.

### Session

```bash
# Using curl.

curl -x http://user:token@localhost:8118 https://check.torproject.org/api/ip
#=> { "isTor": true, "IP": "8.8.4.4" }

curl -x http://user:token@localhost:8118 https://check.torproject.org/api/ip
#=> { "isTor": true, "IP": "8.8.4.4" }
```

We got the same IP, yay! :3

## Configuration

_Work-in-progress_
