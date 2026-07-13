# cheetah-architecture-test

Workspace-level architecture tests that parse `cargo metadata` and enforce dependency rules.

- `domain` crates must not depend on `tokio`, `axum`, `tonic`, `sqlx`, `async-nats`, `quick-xml`.
- `core` protocol crates must not depend on runtime, I/O, or media adapters.
- The workspace crate graph must be acyclic.
