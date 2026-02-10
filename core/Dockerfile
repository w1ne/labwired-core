# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

# Use official Rust image
FROM rust:latest

# Install build dependencies (if needed, e.g. for linking)
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    gcc \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy dependency files first for caching
COPY Cargo.toml Cargo.lock rustfmt.toml clippy.toml ./
COPY crates ./crates
COPY tests ./tests
COPY tests ./tests
# COPY docs ./docs -- Docs are now in root, skipping to keep image lean
COPY examples ./examples
COPY configs ./configs
COPY system.yaml ./system.yaml

# Install the CI runner binary into PATH (allows: `docker run ... labwired test ...`)
RUN cargo install --locked --path crates/cli

# We assume user might want to mount source, but for a "test runner" image, copying is safer for reproducibility.
# However, to be purely "infrastructure", we might just want the tools.
# Let's stick to "Copy Project & Run Tests" model for CI-like behavior.

# Build and Test
CMD ["cargo", "test"]
