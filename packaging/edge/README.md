# Cheetah Signaling Edge Packaging

This directory contains the artifacts used to build and install the edge (single-node) package.

## Layout

```text
cheetah-signaling.service   systemd unit with NoNewPrivileges and restart policy
scripts/
  install.sh                first-time install, creates user/directories
  upgrade.sh                binary upgrade preserving data and config
  uninstall.sh              removes binaries/unit/config, keeps data
  cheetah-signaling-preflight  data-dir space/permission check
edge/
  README.md                 this file
  build-edge-package.sh     tarball builder
```

## Build

```bash
./packaging/edge/build-edge-package.sh x86_64-unknown-linux-gnu v0.1.0
```

The script produces:

- `cheetah-signaling-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`
- `cheetah-signaling-v0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256`
- `cheetah-signaling-v0.1.0-x86_64-unknown-linux-gnu.sbom.json` (package list from `cargo metadata`)
- `ThirdPartyLicenses.txt` (aggregated license summary)

## Install

```bash
tar -xzf cheetah-signaling-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
cd cheetah-signaling-v0.1.0-x86_64-unknown-linux-gnu
sudo ./install.sh v0.1.0
```

Edit `/etc/cheetah/config.toml`, then start the service:

```bash
sudo systemctl start cheetah-signaling
```

## Upgrade

```bash
sudo ./upgrade.sh v0.1.1
```

`upgrade.sh` stops the service, runs `install.sh`, and starts the service again. Existing data in `/var/lib/cheetah` and configuration in `/etc/cheetah/config.toml` are preserved.

## Verification

Verify the tarball integrity before install:

```bash
sha256sum -c cheetah-signaling-*.tar.gz.sha256
```

GitHub release builds also include SLSA build-provenance attestations (created by `actions/attest-build-provenance`). You can verify a release artifact's provenance with the `gh attestation verify` command, for example:

```bash
gh attestation verify cheetah-signaling-*.tar.gz --owner dodojohn83
```
