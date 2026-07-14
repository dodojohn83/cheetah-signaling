# cheetah-http-api

Northbound HTTP API for Cheetah Signaling.

## Scope

This crate implements the public `/api/v1` REST API using Axum. It is a
transport adapter that delegates to `cheetah-signal-application` services and is
kept independent of concrete storage backends.

## Responsibilities

- Versioned REST routing and middleware (request ID, trace, timeout, CORS,
  compression, body limit, access logging).
- Authentication and RBAC (static API key, JWT).
- Resource endpoints for tenants, devices, channels, operations, media
  sessions, nodes, webhooks and deliveries.
- Server-Sent Events (`/api/v1/events/stream`) for event streaming.
- Webhook dispatch, signing, retry and dead-letter orchestration.
- RFC 9457 Problem Details error responses.
- OpenAPI specification and consistency checks.

## Allowed dependencies

- `axum`, `tower`, `tower-http` for HTTP handling.
- `jsonwebtoken` for JWT validation.
- `reqwest` for outbound webhook delivery.
- `tokio-stream` for SSE streams.
- Workspace `cheetah-signal-application`, `cheetah-signal-types`,
  `cheetah-domain`, `cheetah-storage-api`, `cheetah-message-api`.

## Forbidden dependencies

- Direct SQLx, PostgreSQL, SQLite, NATS or media client usage. Storage and
  messaging go through the corresponding port crates.
