# Builder image: the Rust labwired simulator (/run) + the thin Node service that
# proxies /compile to the compile service. No PlatformIO here. Build context is
# the repo root so the `core` submodule is available.
#
# It also bakes ONE curated, VERIFIED example end-to-end: the ESP32-C3 + MLX90640
# thermal-fingerprint IO-Link device. The example firmware is cross-compiled in a
# throwaway `firmware-build` stage (GCC 14 riscv32-esp-elf, ~280 MB) and ONLY the
# small ELF + the example/config YAMLs land in the final image, so /run-example
# can verify it INSIDE the container without shipping a toolchain.

# ── Stage 1: build the Rust CLI ──────────────────────────────────────────────
# Pin to bookworm so the CLI links against glibc 2.36, matching the bookworm
# final stage. (`rust:slim` now tracks Debian trixie/glibc 2.39, which the
# bookworm runtime cannot load — the binary would fail with a GLIBC_2.39 error.)
FROM rust:slim-bookworm AS rust-build
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config libssl-dev gcc \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src/core
COPY core/ ./
RUN cargo build -p labwired-cli --release

# ── Stage 2: cross-compile the baked example firmware (throwaway) ─────────────
# riscv32-esp-elf GCC 14.2 — the example Makefile needs GCC 11+ for
# -march=rv32imc_zicsr_zifencei. We pull the official Espressif standalone
# toolchain (unpacks to /opt/riscv32-esp-elf). Nothing from this stage except the
# resulting ELF is copied forward, so the toolchain never bloats the final image.
FROM debian:bookworm-slim AS firmware-build
ARG RV_TOOLCHAIN_URL=https://github.com/espressif/crosstool-NG/releases/download/esp-14.2.0_20260121/riscv32-esp-elf-14.2.0_20260121-x86_64-linux-gnu.tar.xz
RUN apt-get update && apt-get install -y --no-install-recommends \
      make ca-certificates xz-utils curl \
 && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL "$RV_TOOLCHAIN_URL" -o /tmp/rv.tar.xz \
 && mkdir -p /opt \
 && tar -xJf /tmp/rv.tar.xz -C /opt \
 && rm /tmp/rv.tar.xz
ENV PATH="/opt/riscv32-esp-elf/bin:${PATH}"
WORKDIR /src/core
# core/ carries the example sources, the vendored Melexis driver
# (third_party/mlx90640-library, committed) and the iolinki submodule contents
# (third_party/iolinki — checked out by the workflow's submodules: recursive).
COPY core/ ./
RUN riscv32-esp-elf-gcc --version \
 && make -C examples/esp32c3-mlx90640-thermal/firmware \
 && test -f examples/esp32c3-mlx90640-thermal/firmware/thermal_fingerprint.elf

# ── Stage 3: the Node service ────────────────────────────────────────────────
FROM node:22-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY services/labwired-builder/package.json services/labwired-builder/package-lock.json ./
RUN npm ci --omit=dev
COPY services/labwired-builder/tsconfig.json ./
COPY services/labwired-builder/src ./src
# run.ts + server.ts import the curated chip catalog from board-config via a
# relative path that resolves to /packages/board-config/src/chip-yamls (tsx maps
# the .js specifier to the .ts source). The file is self-contained (no imports),
# so copying just it is enough. Without this the service exits 1 at startup with
# ERR_MODULE_NOT_FOUND, even though the image builds (tsx transpiles at runtime,
# so nothing typechecks the import at build time).
COPY packages/board-config/src/chip-yamls.ts /packages/board-config/src/chip-yamls.ts
COPY --from=rust-build /src/core/target/release/labwired /usr/local/bin/labwired

# Bake the curated example so /run-example can verify it in-container. The
# service runs with LABWIRED_REPO_ROOT=/app/repo; the example's test scripts read
# ./firmware/thermal_fingerprint.elf + ./system-iolink*.yaml, which reference
# ../../configs/chips/esp32c3.yaml (+ its peripheral descriptors) — so we need
# the whole core/configs tree and the example dir, plus the built ELF.
COPY core/configs /app/repo/core/configs
COPY core/examples/esp32c3-mlx90640-thermal /app/repo/core/examples/esp32c3-mlx90640-thermal
COPY --from=firmware-build \
  /src/core/examples/esp32c3-mlx90640-thermal/firmware/thermal_fingerprint.elf \
  /app/repo/core/examples/esp32c3-mlx90640-thermal/firmware/thermal_fingerprint.elf

RUN useradd -m -u 10001 builder && chown -R builder /app
USER builder

ENV BUILDER_ENTRY=1 PORT=18080 LABWIRED_BIN=/usr/local/bin/labwired LABWIRED_REPO_ROOT=/app/repo
EXPOSE 18080
HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD curl -fsS http://127.0.0.1:18080/healthz || exit 1
CMD ["node", "--import", "tsx", "src/server.ts"]
