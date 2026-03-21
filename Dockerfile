FROM rust:1-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src/ src/
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/irrigator /irrigator
ENTRYPOINT ["/irrigator"]
