# cheetah-cluster-ownership

Device owner lease, resolver, and routing for Cheetah Signaling clusters.

This crate implements the cluster-side ownership semantics required by Phase 09:

- `OwnerLeaseService`: acquire, renew, batch-renew, and release device leases
  through the storage `OwnerRepository`.
- `CachingDeviceOwnerResolver`: a `DeviceOwnerResolver` implementation that
  caches the result of `OwnerRepository::get` with a short TTL and respects the
  `lease_until` deadline.

All time-dependent logic uses the injected `Clock` so tests can run with a
`FakeClock`.
