# Stage 1: Builder
FROM rust:1.88 AS builder

# Create a new empty shell project
WORKDIR /usr/src/helios-light-client
RUN apt-get update && apt-get install -y --no-install-recommends musl-tools && rm -rf /var/lib/apt/lists/*
RUN cargo init --bin .

# Copy over the manifests
COPY ./Cargo.toml ./Cargo.lock ./

# Build dependencies for MUSL target (cache layer)
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo build --release --locked --target x86_64-unknown-linux-musl
RUN rm -f target/x86_64-unknown-linux-musl/release/deps/helios_light_client*

# Copy over the source code
COPY ./src ./src

# Build the application (MUSL static)
RUN cargo build --release --locked --target x86_64-unknown-linux-musl

# Stage 2: Final image
FROM debian:bullseye-slim

# Install CA certificates for TLS (Rustls)
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*

# Copy the MUSL static binary from the builder stage
COPY --from=builder /usr/src/helios-light-client/target/x86_64-unknown-linux-musl/release/helios-light-client /usr/local/bin/helios-light-client

# Set the entrypoint
CMD ["helios-light-client"]
