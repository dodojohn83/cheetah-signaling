# ADR 0001: Migration tool uses deterministic UUIDv5 identifiers

## Status
Accepted

## Context
AGENTS.md section 5 mandates that internal new IDs use UUIDv7 for creation
order and randomness (`TenantId`, `DeviceId`, `ChannelId` and other identity
newtypes expose `generate()` based on `Uuid::now_v7()`).

The standalone migration tool (`apps/cheetah-migration-tool`) imports legacy
records into Cheetah Signaling. The tool is designed to be run repeatedly and
incrementally against the same target database. For re-runs to be safe and
idempotent, the mapping from an old-system `(tenant, external_id, protocol)` to
a Cheetah `DeviceId` (and similarly for tenants and channels) must be stable.
UUIDv7 is time-based and non-deterministic, so it cannot provide this property.

This creates a deliberate, bounded deviation from the "internal new IDs use
UUIDv7" rule for the migration boundary only. The migration tool is not a
runtime creation path: it translates pre-existing external identities into
internal stable identifiers, after which normal runtime creation continues to use
UUIDv7.

## Decision

- The migration tool will derive `TenantId`, `DeviceId` and `ChannelId` using
  version-5 UUIDs (`Uuid::new_v5`) over a fixed namespace and a string input
  composed of the old-system tenant, external identifiers and protocol.
- All runtime creation paths (`Device::new`, `Channel::new`, `Tenant::new`,
  services and HTTP handlers) continue to use UUIDv7 through the identity
  newtype `generate()` methods.
- A stable v5 identifier is only produced inside the migration tool mappers;
  no other crate or runtime path will call `Uuid::new_v5` for identity
  generation.

## Consequences

- Re-importing the same old-system record on a later day produces the same
  Cheetah identity, making dry-run, partial re-run, cutover retry and
  `skip_existing=false` overwrites safe without prior state lookup.
- Imported identifiers are not time-ordered. This is acceptable because they
  carry their own `created_at` timestamps and are never used for ordering
  queries by ID.
- Any future change to the deterministic input format or namespace would
  break idempotency for previously migrated records. The input format and
  namespace are therefore frozen once the migration tool ships.

## Risks

- An engineer could accidentally copy the v5 derivation into a runtime path.
  The migration tool code is isolated under `apps/cheetah-migration-tool` and
  its `mappers` module is not re-exported to other crates. Code review and the
  architecture tests must enforce this boundary.
- If the old system contains collisions under `(tenant, external_id, protocol)`,
  the migration tool will map them to the same `DeviceId` and the second record
  in a run will overwrite the first (when `skip_existing=false`) or be counted
  as a conflict (when `skip_existing=true`). This is consistent with the tool
  treating `(tenant, external_id, protocol)` as the stable primary key.

## References

- `apps/cheetah-migration-tool/src/mappers.rs` - `stable_tenant_id`,
  `stable_device_id`, `stable_channel_id` and `MIGRATION_NAMESPACE`.
- `crates/foundation/cheetah-signal-types/src/id.rs` - identity newtypes and
  `Uuid::now_v7()` generation.
- `AGENTS.md` sections 1, 5 and 14.
