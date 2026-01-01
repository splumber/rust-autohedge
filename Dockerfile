# Build stage
FROM rust:1.92-slim as builder

WORKDIR /usr/src/rust-autohedge

# Install dependencies for building
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src
COPY tests ./tests

# Build the application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /usr/src/rust-autohedge/target/release/rust_autohedge /app/rust_autohedge

# Copy configuration files
COPY config.example.yaml /app/config.example.yaml
COPY .env.example /app/.env.example

# Create a non-root user
RUN useradd -m -u 1000 appuser && \
    chown -R appuser:appuser /app

USER appuser

# Expose any necessary ports (adjust if needed)
EXPOSE 8080

CMD ["/app/rust_autohedge"]
