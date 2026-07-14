# cheetah-message-api

Sans-I/O messaging ports and proto envelope mapping for Cheetah Signaling.

This crate defines the transport-neutral boundary used by both the in-process
and NATS message bus implementations. Domain [`Command`] and
[`Event<DomainEvent>`] values are encoded as proto
[`CommandEnvelope`]/[`EventEnvelope`] so the same serialization contract is used
regardless of backend.

## Public entry points

- `bus`: `RawCommandBus`, `RawEventBus`, `Subscription`, `AckHandle`, and
  `Delivery`.
- `mapper`: `encode_command`, `decode_command`, `encode_event`, `decode_event`.
- `subject`: NATS-style subject helpers using tenant buckets.

## Design constraints

- `domain` must not depend on concrete message systems; this crate only depends
  on `cheetah-domain`, `cheetah-signal-contracts`, and `cheetah-signal-types`.
- All envelopes carry `operation_id` and `step_id` so command results can be
  correlated back to the originating `Operation`.
