# A static musl build in a small busybox image: no libc drift between the
# build and runtime images, and the container runs as a numeric non-root user.

FROM rust:1.100-bookworm AS build
WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/*

ARG TARGETARCH
RUN case "$TARGETARCH" in \
      arm64) echo aarch64-unknown-linux-musl > /rust-target ;; \
      *)     echo x86_64-unknown-linux-musl > /rust-target ;; \
    esac \
    && rustup target add "$(cat /rust-target)"

# Cache the dependency graph before the sources change.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/oxide-core/Cargo.toml crates/oxide-core/Cargo.toml
COPY crates/oxide-sim/Cargo.toml crates/oxide-sim/Cargo.toml
COPY crates/oxide-secs/Cargo.toml crates/oxide-secs/Cargo.toml
COPY crates/oxide-store/Cargo.toml crates/oxide-store/Cargo.toml
COPY crates/oxide-api/Cargo.toml crates/oxide-api/Cargo.toml
COPY bins/oxide-server/Cargo.toml bins/oxide-server/Cargo.toml
RUN mkdir -p crates/oxide-core/src crates/oxide-sim/src crates/oxide-secs/src \
             crates/oxide-store/src crates/oxide-api/src bins/oxide-server/src \
    && for c in crates/oxide-core crates/oxide-sim crates/oxide-secs crates/oxide-store crates/oxide-api; do \
         echo "" > "$c/src/lib.rs"; \
       done \
    && echo "fn main() {}" > bins/oxide-server/src/main.rs \
    && cargo build --release --target "$(cat /rust-target)" -p oxide-server || true

COPY crates ./crates
COPY bins ./bins
COPY migrations ./migrations
RUN touch crates/*/src/lib.rs bins/oxide-server/src/main.rs \
    && cargo build --release --target "$(cat /rust-target)" -p oxide-server \
    && cp "target/$(cat /rust-target)/release/oxide" /oxide

FROM busybox:1.38
COPY --from=build /oxide /oxide
COPY config /app/config
COPY migrations /app/migrations

WORKDIR /app
USER 10001:10001

ENV OXIDE_HTTP_ADDR=0.0.0.0:18080 \
    SCENARIO_PATH=/app/config/scenario.json \
    OXIDE_MIGRATIONS_DIR=/app/migrations \
    GREPTIME_DB=public \
    SECS_BIND=127.0.0.1:50010 \
    RUST_LOG=info,oxide=debug

EXPOSE 18080
ENTRYPOINT ["/oxide"]
