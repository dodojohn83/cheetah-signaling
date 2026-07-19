# BAS-005: Storage and Migration Baseline

- SQLite/PostgreSQL contract suite return code: 0
- SQLite/PostgreSQL contract suite elapsed: 4.85s
- Storage adapter tests return code: 0
- Storage adapter tests elapsed: 1.28s
- Migration files scanned: 0
- Migration append-only errors: 0

## Commands run

```bash
cargo test -p cheetah-storage-tests --test sqlite --test postgres
cargo test -p cheetah-storage-sqlite -p cheetah-storage-postgres
```

## Contract suite submodules

The shared contract suite in `crates/testing/cheetah-storage-tests/src/contract.rs` runs the same repository port tests against both the SQLite and PostgreSQL adapters. It covers: device, channel, operation, media, list, outbox, outbox retry, transaction, processed message, owner, ownership, node, webhook, step, and unicode.


## Raw test output

```text

running 1 test
test postgres_contract_suite ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.57s


running 1 test
test sqlite_contract_suite ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```
