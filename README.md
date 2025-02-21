# kittykat

`kittykat` is an on-demand rotating HTTP proxy that routes traffic through the Tor network.
It uses a Rust implementation of Tor - [Arti](https://arti.torproject.org/) - instead of C Tor.

`kittykat` is designed for web-scraping and for hiding against API rate-limits.
This project doesn't encourage the second reason for it's creation.
It's just listed as one of the reasons why it was created in the first place.

## Features

  * __Tor Circuit Isolation__: For each `Proxy-Authorization` token we create a new isolation token which ensures
    that our connection will use a different Tor circuit (if you want to know more about that, see Arti's
    [TorClient](https://tpo.pages.torproject.net/core/doc/rust/arti_client/struct.TorClient.html) struct).

  * __Automatic Cleanup__: Idle isolation tokens are purged after a configurable TTL (default: 10s).

  * __Anonymous Circuit Isolation__: When the authorization header is nowhere to be found, `kittykat` will still create
    a new isolation token global for all anonymous connections, which also get's purged after the TTL.

  * __Bidirectional Tunneling__: HTTPs/CONNECT support for secure traffic.

## Building

To build `kittykat` you can simply build it with cargo.
You can also install it.

```bash
# Build in release mode,
cargo build --release

# or install it.
cargo install --path .
```

## Setup

### Configuration

You can configure `kittykat` using either environment variables or a configuration file.
If `kittykat` sees even one environment variable listed below, it will always choose environment variables over a configuration file.

Below is a list of options you can configure, the environment variable name is enclosed in parenthesis:

  * __listen_address__ (__KITTYKAT_LISTEN_ADDRESS__): __required__, used to configure the IPv4 address that the proxy will listen on.

  * __token_lifetime__ (__KITTYKAT_TOKEN_LIFETIME__): *default: `10000`*, used to set the TTL (in milliseconds) for both anonymous and session tokens.

  * __optimistic_stream__ (__KITTYKAT_OPTIMISTIC__): *default: `false`*, used to configure whether `kittykat` should open an "optimistic" stream when connecting to the target via Tor.
    Read [this](https://tpo.pages.torproject.net/core/doc/rust/arti_client/struct.StreamPrefs.html#method.optimistic) for more information.

  * __log_level__ (__KITTYKAT_LOG__): *default: `info`*, used to configure the logging level. Possible values are: `trace`, `debug`, `info`, `warn` or `error`.

  * __max_circuit_dirtiness__ (__KITTYKAT_MAX_CIRC_DIRT__): *default: `15`*, used to configure max dirtiness of a Tor circuit. Read [this](https://tpo.pages.torproject.net/core/doc/rust/arti_client/config/circ/struct.CircuitTimingBuilder.html#method.max_dirtiness) for more information.

## Usage

### Basic usage

Configure your client to use a HTTP proxy.

```bash
# Using cURL.

curl -x http://localhost:8118 https://check.torproject.org/api/ip
#=> { "isTor": true, "IP": "172.18.55.78" }
```

As you can see we can use `kittykat` without authorization as it'll generate an anonymous token for us internally.

### Reusing circuits

Next, we will authorize ourselves into `kittykat` so it knows which cat- hold on, that wasn't in the script. Sorry, my brain is being silly.
We authorize ourselves to create a new, reausable session for ourselves of course.

```bash
# Using cURL.

curl -x http://localhost:8118 --proxy-user "cat:goes meow" https://check.torproject.org/api/ip
#=> { "isTor": true, "IP": "10.72.20.4" }

curl -x http://localhost:8118 --proxy-user "cat:goes meow" https://check.torproject.org/api/ip
#=> { "isTor": true, "IP": "10.72.20.4" }
```

We got the same IP, yay! :3

