# ASM runtime closure (inbox, leases, media readiness)

## Summary

Closed major remaining assembly gaps for ASM-003/004/005/006:

1. **Inbox consumer** — `InboxService` subscribes to `sig.v1.command.*.{node_id}`
   with owner-epoch fencing and forward-to-owner behaviour.
2. **Owner lease renew** — periodic `OwnerLeaseService::batch_renew` over
   `list_by_node` for this node.
3. **Node lease heartbeat** — `NodeLeaseService` register + heartbeat + drain on
   cancel for both edge and cluster profiles.
4. **Edge single-owner** — `SingleNodeOwnerResolver` auto-acquires this node when
   no owner is present.
5. **Media readiness policy** — `media.readiness_policy = optional|required`;
   `/health/ready` returns `media_nodes_unavailable` when required and no alive
   media node lease exists.
6. **Command handler** — owner accepts commands with `UNKNOWN_OUTCOME` when the
   protocol path cannot prove device-side effect (never forges success).

## Code

- `apps/cheetah-signaling/src/workers.rs` (new)
- `apps/cheetah-signaling/src/assembly.rs` wiring
- `MediaReadinessPolicy` in `cheetah-signal-types`
- `ApiConfig.media_nodes_required` + health handler

## Verification

```bash
export PROTOC=$HOME/.local/bin/protoc
export PROTOC_INCLUDE=$HOME/.local/include
cargo check -p cheetah-signaling
cargo test -p cheetah-http-api --tests
cargo clippy -p cheetah-signaling -p cheetah-http-api -p cheetah-signal-types \
  --all-targets -- -D warnings
```

## Follow-up (2026-07-21 continuation)

- Built-in plugin `activate_builtin` for `cheetah/onvif` + conditional `cheetah/gb28181`
- `OwnerCommandHandler` dispatches via `PluginHost::handle_command`
- Cluster `DrainingMigrationService` worker for draining peers
- `TakeoverService` armed for reconnect paths
- Media client pre-RPC validation: non-empty endpoint, non-nil node id, non-zero instance epoch

## Remaining outside this assembly closure

- Real device GB/ONVIF interop reports
- Media capability version negotiation end-to-end with upstream media server
- Three-node chaos / soak / scale reports
- Poison-message dead-letter ops dashboards

Refs: ASM-003, ASM-004, ASM-005, ASM-006, ASM-007
