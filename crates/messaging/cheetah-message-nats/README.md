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
- All connect, publish, subscribe and stream operations are bounded by
  configurable timeouts.
- TLS is required for cluster communication; plaintext `nats://` URLs are
  rejected.
- Optional token or username/password authentication via `SecretStore` references.
- In-process subject allow-listing before publish and subscribe, defaulting to
  the command/event subjects required by this node.

## Subject layout

- `sig.v1.command.{tenant_bucket}.{owner_node}`
- `sig.v1.event.{tenant_bucket}.{event_type}`

## Usage

```rust
use std::sync::Arc;
use std::time::Duration;
use cheetah_message_nats::NatsBus;
use cheetah_signal_types::config::MessagingConfig;

let mut config = MessagingConfig::default();
config.backend = cheetah_signal_types::config::MessagingBackend::Nats;
config.nats_url = "tls://localhost:4222".to_string();

let bus = NatsBus::connect(
    &config,
    node_id,
    resolver,
    secret_store,
    Duration::from_secs(5),
    Duration::from_secs(5),
)
.await?;
```
