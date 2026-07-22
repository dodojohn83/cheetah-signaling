//! GB28181 end-to-end system, chaos, capacity and soak scenario tests.
//!
//! This crate carries no production code. The scenarios live under `tests/`
//! and drive the GB28181 access/media state machines together with real
//! storage backends and fake media adapters, keeping the control-plane
//! boundary intact (no RTP/RTCP/PS/TS/ES media payloads are produced).
//!
//! See `dev-docs/004_gb28181-improve/reports/gb4-sys-00{1,2,6,7,8}.md`.
