# kittykat

`kittykat` is an on-demand rotating HTTP proxy that routes traffic through the Tor network.
It dynamically creates isolated Tor circuits for each client, ensuring better anonymity and reduced cross-request correlation.

## Features

  * __Rotating Tor Circuits__: Each client gets an independent Tor connection.

  * __UUID-Based Session Management__: Send a `x-kitty` header with a valid UUIDv4 to reuse an existing circuit/session or leave it empty for a new one.

  * __Automatic Client Expiration__: Tor clients are purged after a set duration (default: 10 seconds).

## Usage

To use `kittykat`, send HTTP requests through the proxy. To persist a Tor circuit, include the `x-kitty` header with a valid UUID.

Example:

```http
GET http://example.com
host: example.com
x-kitty: 57a6aa49-3ae4-491c-a954-42cfb2014042
```

If `x-kitty` is omitted or empty, a new Tor circuit is assigned and the response will contain a new `x-kitty` header.
The same will occur when the UUID is valid but got purged or did not exist at all.
