# syntax=docker/dockerfile:1

# Pin builder image digest for reproducibility (linux/amd64).
FROM rust:1.96.1-slim-bookworm@sha256:39decc9f9f4a87e03db5069b2424c0ee13c3d7e4116010980d4f96849e823bc4 AS builder

WORKDIR /build
COPY . .

# Build the signaling binary. SQLx query macros use offline mode by default in
# this repository; if metadata needs to be regenerated, use `cargo sqlx prepare`.
# The container image defaults to the `cluster` feature so multi-replica
# Postgres+NATS deployments continue to work. Override with `--build-arg
# CARGO_FEATURES=edge` to produce a smaller edge-only binary.
ARG CARGO_FEATURES=cluster
RUN cargo build --release --locked --bin cheetah-signaling --features "$CARGO_FEATURES"

# Pin runtime base image digest for reproducibility (linux/amd64).
FROM debian:bookworm-slim@sha256:63a496b5d3b99214b39f5ed70eb71a61e590a77979c79cbee4faf991f8c0783e AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root service account.
RUN groupadd -r cheetah -g 1000 \
    && useradd -r -g cheetah -u 1000 -d /var/lib/cheetah -s /usr/sbin/nologin cheetah \
    && mkdir -p /var/lib/cheetah /etc/cheetah \
    && chown -R cheetah:cheetah /var/lib/cheetah

COPY --from=builder /build/target/release/cheetah-signaling /usr/local/bin/cheetah-signaling

USER 1000:1000
EXPOSE 8080 9090

# Writable directories when the container is run with readOnlyRootFilesystem.
VOLUME ["/var/lib/cheetah", "/tmp"]

ENTRYPOINT ["/usr/local/bin/cheetah-signaling"]
CMD []
