# cheetah-signal-grpc

Generated [`tonic`](https://docs.rs/tonic) gRPC client/server bindings for the
Cheetah Signaling protocol definitions under `proto/`.

This crate is a transport adapter (layer 2). It depends on
`cheetah-signal-contracts` for the underlying Protocol Buffer message types so
the foundation crate can remain free of `tonic`.
