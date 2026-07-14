# cheetah-message-local

In-process message bus implementation for single-node Cheetah Signaling deployments.

`InProcessMessageBus` implements the same Sans-I/O `RawCommandBus` and
`RawEventBus` ports as the NATS backend. Commands and events are encoded to
proto envelopes before being placed on bounded `tokio` channels, preserving the
exact serialization boundary used in cluster mode.

## Features

- Bounded `mpsc` command channel with explicit `Busy` back-pressure.
- `broadcast` event channel; events published without subscribers are dropped
  silently.
- Implements `cheetah_domain::CommandBus` and `cheetah_domain::EventPublisher`.

## Usage

```rust
use cheetah_message_local::InProcessMessageBus;

let bus = InProcessMessageBus::new(256, 1024);
```
