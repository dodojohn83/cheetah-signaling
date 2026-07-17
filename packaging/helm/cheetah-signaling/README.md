# Cheetah Signaling Helm Chart

Deploys the Cheetah Signaling control plane on Kubernetes.

## Prerequisites

- Kubernetes 1.28+
- Helm 3.12+

## Install

```bash
helm install cheetah-signaling ./packaging/helm/cheetah-signaling \
  --namespace cheetah --create-namespace \
  --set image.tag=v0.1.0 \
  --set existingSecret=cheetah-signaling-secrets
```

## Secret values

Do not put passwords in `values.yaml`. Create a Secret and reference it via `existingSecret`:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: cheetah-signaling-secrets
type: Opaque
stringData:
  CHEETAH_SECRET_DB_PASSWORD: "..."
```

## Container image

The chart defaults to `ghcr.io/dodojohn83/cheetah-signaling`. Build and push with:

```bash
docker build -f packaging/container/Dockerfile -t ghcr.io/dodojohn83/cheetah-signaling:v0.1.0 .
```
