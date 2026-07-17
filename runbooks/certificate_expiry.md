# Runbook: Certificate Expiry

## Meaning
A TLS/mTLS certificate used by the HTTP API, gRPC inter-node, media control or NATS client is expiring or has expired. Connections fail with certificate validation errors.

## Possible Causes
- Certificate not renewed before expiry.
- Secret provider serving an old certificate version.
- Intermediate CA rotation not reflected in trust store.
- Certificate identity does not match the configured node ID.

## Diagnostic Commands
```bash
# Check certificate expiry for HTTP endpoint
openssl s_client -connect localhost:8443 -servername signaling.example.com -showcerts < /dev/null \
  | openssl x509 -noout -dates -subject -issuer

# Inspect secret references
grep -E "tls|cert|secret" /etc/cheetah-signaling/config.toml

# Review TLS handshake errors
grep -E "tls|certificate|handshake" /var/log/cheetah-signaling/*.log
```

## Mitigation
1. Rotate the expiring certificate via the secret provider (e.g., Vault, file mount, Kubernetes cert-manager).
2. Ensure the new certificate's `subject`/`SAN` matches the expected node identity.
3. Update `CHEETAH_SECURITY__TLS_CERT_REF` and `CHEETAH_SECURITY__TLS_KEY_REF`.
4. Restart the node to load the new certificate; for zero-downtime, use the rolling-upgrade compatibility matrix (`HA-005`).

## Recovery Confirmation
- TLS handshake succeeds without errors.
- `openssl` output shows a future `notAfter` date.
- mTLS peer identity matches the expected node/plugin ID.
- Internal gRPC and media control traffic resumes.
