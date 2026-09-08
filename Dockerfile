FROM rust:1.88-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0 AS builder
WORKDIR /workspace
ARG BINARY
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY apps ./apps
COPY crates ./crates
RUN cargo build --locked --release --bin ${BINARY}

FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171 AS runtime
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
ARG BINARY
COPY --from=builder /workspace/target/release/${BINARY} /usr/local/bin/app
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/app"]
