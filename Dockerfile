# Stage 1: Build the Rust application
FROM rust:1.71 as builder

# Create app directory
WORKDIR /usr/src/app

# Copy manifest and source
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build in release mode
RUN cargo build --release

# Stage 2: Create minimal runtime image
FROM debian:bookworm-slim

# Install CA certificates for TLS validation
RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m appuser
USER appuser
WORKDIR /home/appuser

# Copy binary from builder
COPY --from=builder /usr/src/app/target/release/p2p-ws-server ./p2p-ws-server

# Expose port (Render uses $PORT env if provided)
EXPOSE 8080

# Entrypoint
ENTRYPOINT ["./p2p-ws-server"]
