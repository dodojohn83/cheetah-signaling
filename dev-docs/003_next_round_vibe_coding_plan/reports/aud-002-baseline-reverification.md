# AUD-002: Phase 00 Baseline Re-verification

- Date: 2026-07-19T13:38:12.312289+00:00
- Host: devin-box (Linux-5.15.200-x86_64-with-glibc2.35, x86_64, 2 CPUs)

## Commands run

| Check | Command | Exit | Elapsed |
|-------|---------|------|----------|
| fmt | `cargo fmt --all -- --check` | PASS (0) | 0.54s |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | PASS (0) | 17.96s |
| nextest | `cargo nextest run --workspace` | PASS (0) | 45.17s |
| buf_format | `buf format --diff --exit-code` | PASS (0) | 0.03s |
| buf_lint | `buf lint` | PASS (0) | 0.46s |
| deny | `cargo deny check` | PASS (0) | 0.75s |

## Skipped checks

- `registry`: script not present in this branch; see dedicated Phase 01 PR for evidence.
- `architecture`: script not present in this branch; see dedicated Phase 01 PR for evidence.
- `storage`: script not present in this branch; see dedicated Phase 01 PR for evidence.

- Passed: 6/6
- Failed: none

## Notes

- This report re-runs the workspace, Proto, and storage quality gates as part of AUD-002.
- Component-specific audits (registry, architecture, storage baseline) are maintained in their respective PRs.
