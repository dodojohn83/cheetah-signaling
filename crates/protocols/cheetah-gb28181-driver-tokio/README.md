# cheetah-gb28181-driver-tokio

Tokio-based UDP/TCP driver for the GB28181 protocol module. It owns the
network sockets, transaction timer injection and transport-side NAT address
handling, and forwards parsed SIP messages to `cheetah-gb28181-module`.
