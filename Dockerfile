FROM rust:1-slim-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY wit ./wit
COPY src ./src

RUN cargo build --release && strip target/release/ferropod

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data

COPY --from=builder /app/target/release/ferropod /usr/local/bin/ferropod

WORKDIR /data
EXPOSE 80 443 8080
VOLUME ["/data"]

ENTRYPOINT ["ferropod"]
CMD ["serve", "--profile", "low-mem", "--bind", "0.0.0.0:8080", "--data-dir", "/data"]
