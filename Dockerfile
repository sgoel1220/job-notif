# Build the Rust service in a separate stage so the runtime image stays small.
FROM rust:1.89-slim-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY migrations ./migrations
COPY companies.json ./companies.json
RUN cargo build --release --locked --bin job-notif

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home app
WORKDIR /app
COPY --from=builder /app/target/release/job-notif /usr/local/bin/job-notif
USER app
EXPOSE 10000
CMD ["/usr/local/bin/job-notif"]
