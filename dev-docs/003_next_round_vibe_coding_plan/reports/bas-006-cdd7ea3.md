# BAS-006: Workspace Baseline Report

- Commit: `cdd7ea3f54ec4a7607f21dd0217a1ef2fbc4d16c`
- Generated: 2026-07-22T08:41:03.701769+00:00

## Toolchain

- rustc: rustc 1.96.1 (31fca3adb 2026-06-26)
- cargo: cargo 1.96.1 (356927216 2026-06-26)
- buf: buf not found
- protoc: libprotoc 25.3

## System

- OS: Linux 5.15.200
- Arch: x86_64
- CPU: INTEL(R) XEON(R) PLATINUM 8559C
- Memory: 7.8 GiB

## Commands

| command | exit | duration (s) | passed | failed | ignored | raw |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `fmt` | 0 | 0.77 | - | - | - | `target/reports/baseline/cdd7ea3f54ec4a7607f21dd0217a1ef2fbc4d16c/raw/fmt.txt` |
| `clippy` | 0 | 9.09 | - | - | - | `target/reports/baseline/cdd7ea3f54ec4a7607f21dd0217a1ef2fbc4d16c/raw/clippy.txt` |
| `deny` | 0 | 0.73 | - | - | - | `target/reports/baseline/cdd7ea3f54ec4a7607f21dd0217a1ef2fbc4d16c/raw/deny.txt` |
| `test` | 0 | 66.73 | 1144 | 0 | 11 | `target/reports/baseline/cdd7ea3f54ec4a7607f21dd0217a1ef2fbc4d16c/raw/test.txt` |

## Unrun

- `buf format/lint`: buf not installed in this environment
- `cargo nextest`: cargo-nextest not installed; fell back to cargo test

## Features

`default`, `test-helpers`, `test-support`, `test-util`

## Warnings

- None captured.

## Failure Mapping

- No known failures mapped.
