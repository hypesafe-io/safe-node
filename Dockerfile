# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.94
ARG DEBIAN_VERSION=bookworm

FROM rust:${RUST_VERSION}-${DEBIAN_VERSION} AS builder

WORKDIR /usr/src/safe-node

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libsqlite3-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked

FROM debian:${DEBIAN_VERSION}-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 --user-group safe-node \
    && mkdir -p /app/config /app/data \
    && chown -R safe-node:safe-node /app

WORKDIR /app

COPY --from=builder /usr/src/safe-node/target/release/safe-node /usr/local/bin/safe-node

USER safe-node

ENV RUST_LOG=info,safe_node=debug

EXPOSE 9909

ENTRYPOINT ["safe-node"]
CMD ["run"]
