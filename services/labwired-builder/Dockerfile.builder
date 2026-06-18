# Builder image: the Rust labwired simulator (/run) + the thin Node service that
# proxies /compile to the compile service. No PlatformIO here. Build context is
# the repo root so the `core` submodule is available.

FROM rust:slim AS rust-build
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config libssl-dev gcc \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src/core
COPY core/ ./
RUN cargo build -p labwired-cli --release

FROM node:22-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY services/labwired-builder/package.json services/labwired-builder/package-lock.json ./
RUN npm ci --omit=dev
COPY services/labwired-builder/tsconfig.json ./
COPY services/labwired-builder/src ./src
COPY --from=rust-build /src/core/target/release/labwired /usr/local/bin/labwired

RUN useradd -m -u 10001 builder && chown -R builder /app
USER builder

ENV BUILDER_ENTRY=1 PORT=18080 LABWIRED_BIN=/usr/local/bin/labwired
EXPOSE 18080
HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD curl -fsS http://127.0.0.1:18080/healthz || exit 1
CMD ["node", "--import", "tsx", "src/server.ts"]
