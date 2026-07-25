FROM rust:1.88-bookworm AS builder
WORKDIR /workspace
COPY . .
ARG BINARY
RUN cargo build --locked --release --bin ${BINARY}

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
ARG BINARY
COPY --from=builder /workspace/target/release/${BINARY} /usr/local/bin/app
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/app"]
