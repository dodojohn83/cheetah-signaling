# cheetah-message-nats

NATS JetStream message bus implementation for clustered Cheetah Signaling.

`NatsBus` connects to a NATS server, ensures `CHEETAH_COMMANDS` and
`CHEETAH_EVENTS` streams exist, and implements the same `RawCommandBus` and
`RawEventBus` ports as the in-process backend.

## Features

- Durable pull consumers with explicit acknowledgement.
- `NATS-Msg-Id` header for JetStream message deduplication on the producer.
- Producer acknowledgement waits for server confirmation.
- `ack` / `nak` / `term` handle success, transient failure redelivery, and
  dead-lettering of unprocessable messages.

## Subject layout

- `sig.v1.command.{tenant_bucket}.{owner_node}`
- `sig.v1.event.{tenant_bucket}.{event_type}`

## Usage

```rust
use std::sync::Arc;
use cheetah_message_nats::NatsBus;

let bus = NatsBus::connect("nats://localhost:4222", node_id, resolver).await?;
```
