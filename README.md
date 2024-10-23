# surge-ping

A Ping (ICMP) detection tool, you can personalize the Ping parameters. Since version `0.4.0`, a new `Client` data structure
has been added. This structure wraps the `socket` implementation and can be passed between any task cheaply. If you have multiple
addresses to detect, you can easily complete it by creating only one system socket(Thanks @wladwm).

[![Crates.io](https://img.shields.io/crates/v/surge-ping.svg)](https://crates.io/crates/surge-ping)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/kolapapa/surge-ping/blob/main/LICENSE)
[![API docs](https://docs.rs/surge-ping/badge.svg)](http://docs.rs/surge-ping)

rust ping libray based on `tokio` + `socket2` + `pnet_packet`.

## Usage

```
Usage: surge-ping [OPTIONS] <HOST>

Arguments:
  <HOST>  Destination host or address

Options:
  -4                                 Use IPv4
  -6                                 Use IPv6
  -i, --interval <INTERVAL>          Wait time in seconds between sending each packet [default: 1.0]
  -s, --size <SIZE>                  Specify the number of data bytes to be sent [default: 56]
  -c, --count <COUNT>                Stop after sending <count> ECHO_REQUEST packets [default: 5]
  -I, --interface <INTERFACE>        Source packets with the given interface ip address
  -w, --wait-timeout <WAIT_TIMEOUT>  Specify a timeout in seconds, beginning once the last ping is sent [default: 1.0]
  -h, --help                         Print help
```

## License

This project is licensed under the [MIT license].

[MIT license]: https://github.com/kolapapa/surge-ping/blob/main/LICENSE
