# Containerized Hetzner Deploy (Slice A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Repo conventions (from project memory):** commits must contain NO "Claude"/AI/assistant references; commit author email is `14119286+w1ne@users.noreply.github.com`. Integrate branches with `git merge`, never `git rebase`.

**Goal:** Replace the 7-step manual Hetzner builder deploy with a single `docker compose pull && up -d`, by splitting the builder into a small sim image + a shared PlatformIO compile image + a cloudflared sidecar, all published privately to GHCR.

**Architecture:** Two CI-built images — `labwired-builder` (Rust sim `/run` + thin Node service that proxies `/compile`) and `labwired-compile` (Node compile service + PlatformIO + warmed platform caches) — run as a 3-service compose stack (builder, compile, cloudflared) on a two-network topology that denies internet egress to the untrusted compile container. The external contract `builder.labwired.com/{run,compile}` is unchanged.

**Tech Stack:** Node 22 / TypeScript (tsx), vitest, Rust (core submodule, `labwired-cli`), PlatformIO, Docker + Compose, GitHub Actions → private GHCR, Cloudflare named tunnel (token).

**Scope note:** This is Slice A. The proto.cat contract unification (superset request/response, merged `chip_families` catalog, retiring the Python `compile-service/`) is **Slice B** — a separate plan written after this ships. Slice A makes no change to labwired's own `/compile` or `/run` behavior.

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `services/labwired-builder/src/compile-server.ts` | Standalone HTTP entrypoint for the compile image: `/compile`, `/boards`, `/healthz` | Create |
| `services/labwired-builder/test/compile-server.test.ts` | Tests for the compile server | Create |
| `services/labwired-builder/src/server.ts` | Builder server — add `/compile` proxy to `COMPILE_URL` | Modify |
| `services/labwired-builder/test/server.test.ts` | Add proxy-path test | Modify |
| `services/labwired-builder/Dockerfile.builder` | Build sim + node runtime image (context = repo root) | Create |
| `services/labwired-builder/Dockerfile.compile` | PlatformIO + node compile image (context = service dir) | Create |
| `services/labwired-builder/.dockerignore` | Keep node_modules/.git out of build context | Create |
| `services/labwired-builder/docker-compose.yml` | 3-service stack, two networks, hardening | Create |
| `services/labwired-builder/.env.example` | Documents required env (`BUILDER_SECRET`, `TUNNEL_TOKEN`, `IMAGE_TAG`) | Create |
| `services/labwired-builder/deploy.sh` | `docker compose pull && up -d` wrapper | Create |
| `services/labwired-builder/deploy/RUNBOOK.md` | Rewrite: one-time bootstrap + steady-state | Modify |
| `.github/workflows/builder-deploy.yml` | Build + push both images to private GHCR on `main` | Create |
| `.gitignore` | Ignore the live `.env` | Modify |

---

## Task 1: Compile HTTP server entrypoint

Wrap the existing pure `compile()` in a minimal HTTP server for the compile image. Reuses `compile.ts` unchanged.

**Files:**
- Create: `services/labwired-builder/src/compile-server.ts`
- Test: `services/labwired-builder/test/compile-server.test.ts`

- [ ] **Step 1: Write the failing test**

Create `services/labwired-builder/test/compile-server.test.ts`:

```ts
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { Server } from 'node:http';
import { makeCompileServer } from '../src/compile-server.js';

let server: Server;
let base: string;

beforeAll(async () => {
  server = makeCompileServer();
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  const addr = server.address();
  if (addr === null || typeof addr === 'string') throw new Error('no port');
  base = `http://127.0.0.1:${addr.port}`;
});

afterAll(() => new Promise<void>((r) => server.close(() => r())));

describe('compile-server', () => {
  it('serves /healthz open', async () => {
    const res = await fetch(`${base}/healthz`);
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true });
  });

  it('aliases /health for proto.cat healthchecks', async () => {
    const res = await fetch(`${base}/health`);
    expect(res.status).toBe(200);
  });

  it('lists boards at /boards', async () => {
    const res = await fetch(`${base}/boards`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { boards: { board: string; runnable: boolean }[] };
    expect(body.boards.some((b) => b.board === 'stm32l476')).toBe(true);
  });

  it('rejects an unknown board with ok:false (no compiler needed)', async () => {
    const res = await fetch(`${base}/compile`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ board: 'not-a-board', source: 'int main(){}' }),
    });
    expect(res.status).toBe(200);
    const body = (await res.json()) as { ok: boolean };
    expect(body.ok).toBe(false);
  });

  it('returns 400 on invalid JSON', async () => {
    const res = await fetch(`${base}/compile`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '{not json',
    });
    expect(res.status).toBe(400);
  });

  it('404s unknown routes', async () => {
    const res = await fetch(`${base}/nope`);
    expect(res.status).toBe(404);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd services/labwired-builder && npx vitest run test/compile-server.test.ts`
Expected: FAIL — `Cannot find module '../src/compile-server.js'`.

- [ ] **Step 3: Write the implementation**

Create `services/labwired-builder/src/compile-server.ts`:

```ts
import { createServer, IncomingMessage, ServerResponse } from 'node:http';
import { compile, supportedCompileBoards, type CompileRequest } from './compile.js';

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    req.on('data', (c: Buffer) => chunks.push(c));
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    req.on('error', reject);
  });
}

function json(res: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body);
  res.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) });
  res.end(payload);
}

/** Compile service: PlatformIO build only. Reachable only on a private compose
 *  network (never published to host/internet), so auth is optional — enable it
 *  by setting COMPILE_SECRET (checked against the X-Builder-Secret header). */
export function makeCompileServer() {
  const secret = process.env.COMPILE_SECRET;

  return createServer(async (req: IncomingMessage, res: ServerResponse) => {
    const url = req.url ?? '/';

    if (url === '/healthz' || url === '/health') {
      json(res, 200, { ok: true });
      return;
    }
    if (url === '/boards' && req.method === 'GET') {
      json(res, 200, { boards: supportedCompileBoards() });
      return;
    }
    if (url === '/compile' && req.method === 'POST') {
      if (secret && req.headers['x-builder-secret'] !== secret) {
        json(res, 401, { ok: false, error: 'unauthorized' });
        return;
      }
      let parsed: unknown;
      try {
        parsed = JSON.parse(await readBody(req));
      } catch {
        json(res, 400, { ok: false, error: 'invalid JSON body' });
        return;
      }
      const result = await compile(parsed as CompileRequest);
      json(res, 200, result);
      return;
    }
    json(res, 404, { ok: false, error: 'not found' });
  });
}

// Entry guard — only listen when run as the compile image entrypoint.
if (process.env.COMPILE_ENTRY === '1') {
  const port = process.env.PORT ? parseInt(process.env.PORT, 10) : 8080;
  makeCompileServer().listen(port, () => {
    process.stdout.write(`labwired-compile listening on port ${port}\n`);
  });
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd services/labwired-builder && npx vitest run test/compile-server.test.ts`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add services/labwired-builder/src/compile-server.ts services/labwired-builder/test/compile-server.test.ts
git commit -m "feat(builder): standalone compile HTTP server entrypoint"
```

---

## Task 2: Builder proxies `/compile` to the compile service

When `COMPILE_URL` is set, the builder forwards `/compile` to the internal compile service instead of running PlatformIO locally. Preserves the public `builder.labwired.com/compile` contract while moving the toolchain into its own container. Local compile remains the fallback (for dev/tests).

**Files:**
- Modify: `services/labwired-builder/src/server.ts`
- Modify: `services/labwired-builder/test/server.test.ts`

- [ ] **Step 1: Write the failing test**

Append a new `describe` block to `services/labwired-builder/test/server.test.ts`. The file already imports from `vitest` and `../src/server.js` — **merge** any missing names (`afterEach`, `createServer`, the `Server` type) into the existing import lines rather than adding duplicate `import` statements (duplicate imports are a compile error):

```ts
import { afterEach, describe, expect, it } from 'vitest';
import type { Server } from 'node:http';
import { createServer } from 'node:http';
import { makeServer } from '../src/server.js';

describe('builder /compile proxy', () => {
  const started: Server[] = [];
  afterEach(async () => {
    delete process.env.COMPILE_URL;
    await Promise.all(started.map((s) => new Promise<void>((r) => s.close(() => r()))));
    started.length = 0;
  });

  async function listen(s: Server): Promise<string> {
    started.push(s);
    await new Promise<void>((r) => s.listen(0, '127.0.0.1', r));
    const addr = s.address();
    if (addr === null || typeof addr === 'string') throw new Error('no port');
    return `http://127.0.0.1:${addr.port}`;
  }

  it('forwards /compile to COMPILE_URL and returns its response', async () => {
    // Fake upstream compile service.
    let received: unknown;
    const upstream = createServer((req, res) => {
      const chunks: Buffer[] = [];
      req.on('data', (c) => chunks.push(c));
      req.on('end', () => {
        received = JSON.parse(Buffer.concat(chunks).toString());
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ ok: true, elfBase64: 'ZmFrZQ==', diagnostics: [] }));
      });
    });
    const upstreamBase = await listen(upstream);
    process.env.COMPILE_URL = upstreamBase;

    const builderBase = await listen(makeServer({ secret: 's3cret' }));
    const res = await fetch(`${builderBase}/compile`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Builder-Secret': 's3cret' },
      body: JSON.stringify({ board: 'stm32l476', source: 'int main(){}' }),
    });

    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, elfBase64: 'ZmFrZQ==', diagnostics: [] });
    expect(received).toEqual({ board: 'stm32l476', source: 'int main(){}' });
  });

  it('returns 502 when the compile service is unreachable', async () => {
    process.env.COMPILE_URL = 'http://127.0.0.1:1'; // nothing listening
    const builderBase = await listen(makeServer({ secret: 's3cret' }));
    const res = await fetch(`${builderBase}/compile`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Builder-Secret': 's3cret' },
      body: JSON.stringify({ board: 'stm32l476', source: 'int main(){}' }),
    });
    expect(res.status).toBe(502);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd services/labwired-builder && npx vitest run test/server.test.ts -t "proxy"`
Expected: FAIL — the builder currently calls local `compile()` (no `COMPILE_URL` branch), so the forwarded-body assertion / 502 path fails.

- [ ] **Step 3: Write the implementation**

In `services/labwired-builder/src/server.ts`, replace the `/compile` branch:

```ts
      } else if (url === '/compile') {
        const result = await compile(parsed as CompileRequest);
        json(res, 200, result);
```

with:

```ts
      } else if (url === '/compile') {
        const compileUrl = process.env.COMPILE_URL;
        if (compileUrl) {
          try {
            const upstream = await fetch(`${compileUrl.replace(/\/$/, '')}/compile`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify(parsed),
            });
            const text = await upstream.text();
            res.writeHead(upstream.status, { 'Content-Type': 'application/json' });
            res.end(text);
          } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            json(res, 502, { ok: false, error: `compile backend unreachable: ${message}` });
          }
        } else {
          const result = await compile(parsed as CompileRequest);
          json(res, 200, result);
        }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd services/labwired-builder && npx vitest run test/server.test.ts`
Expected: PASS (existing tests + the 2 new proxy tests).

- [ ] **Step 5: Run the full suite to confirm no regressions**

Run: `cd services/labwired-builder && LABWIRED_BIN="$(git rev-parse --show-toplevel)/core/target/release/labwired" npx vitest run`
Expected: PASS. (If the `labwired` binary isn't built locally, the `/run` e2e tests skip/fail on the binary — that is pre-existing and unrelated; the compile/server/proxy tests must pass.)

- [ ] **Step 6: Commit**

```bash
git add services/labwired-builder/src/server.ts services/labwired-builder/test/server.test.ts
git commit -m "feat(builder): proxy /compile to COMPILE_URL when set"
```

---

## Task 3: `Dockerfile.compile` — the shared PlatformIO image

**Files:**
- Create: `services/labwired-builder/Dockerfile.compile`
- Create: `services/labwired-builder/.dockerignore`

- [ ] **Step 1: Create `.dockerignore`**

Create `services/labwired-builder/.dockerignore`:

```
node_modules
.git
.env
test/fixtures/*.elf
```

- [ ] **Step 2: Write the Dockerfile**

Create `services/labwired-builder/Dockerfile.compile` (build context = `services/labwired-builder`):

```dockerfile
# Shared PlatformIO compile service. Built + published by labwired; also
# consumed by proto.cat (Slice B). Untrusted source is compiled here, so it runs
# non-root and is deployed on an egress-denied compose network.
FROM node:22-bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
      python3 python3-venv git curl ca-certificates build-essential \
 && rm -rf /var/lib/apt/lists/*

# PlatformIO in an isolated venv (Debian's Python is externally-managed).
RUN python3 -m venv /opt/pio && /opt/pio/bin/pip install --no-cache-dir platformio==6.1.16
ENV PIO_BIN=/opt/pio/bin/pio
ENV PLATFORMIO_CORE_DIR=/opt/platformio

# Warm the toolchain/framework caches by platform (broader than per-board; covers
# every board in the catalog). This is the heavy, cacheable layer.
RUN /opt/pio/bin/pio pkg install -g -p ststm32 \
 && /opt/pio/bin/pio pkg install -g -p espressif32 \
 && /opt/pio/bin/pio pkg install -g -p nordicnrf52 \
 && /opt/pio/bin/pio pkg install -g -p raspberrypi

WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev
COPY tsconfig.json ./
COPY src ./src

# Non-root; give it ownership of the dirs pio/node write to at runtime.
RUN useradd -m -u 10001 builder \
 && chown -R builder /app /opt/platformio
USER builder

ENV COMPILE_ENTRY=1 PORT=8080
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1
CMD ["node", "--import", "tsx", "src/compile-server.ts"]
```

- [ ] **Step 3: Build the image**

Run: `cd services/labwired-builder && docker build -f Dockerfile.compile -t labwired-compile:dev .`
Expected: builds successfully (first build is slow — it downloads 4 PlatformIO platforms).

- [ ] **Step 4: Smoke-test the running container**

```bash
docker run -d --name lwc-smoke -p 127.0.0.1:8080:8080 labwired-compile:dev
sleep 5
curl -fsS http://127.0.0.1:8080/healthz                       # {"ok":true}
curl -fsS http://127.0.0.1:8080/boards | head -c 200          # board list JSON
# Compile a trivial ST blink (proves the warmed toolchain works):
curl -fsS -X POST http://127.0.0.1:8080/compile \
  -H 'Content-Type: application/json' \
  -d '{"board":"stm32l476","language":"cpp","source":"#include <stm32l4xx.h>\nint main(void){while(1){}}"}' \
  | python3 -c 'import sys,json; d=json.load(sys.stdin); print("ok=",d["ok"]); print("elf?",bool(d.get("elfBase64")))'
docker rm -f lwc-smoke
```
Expected: `/healthz` returns `{"ok":true}`; `/boards` lists boards; the compile returns `ok= True` with a non-empty ELF.

- [ ] **Step 5: Commit**

```bash
git add services/labwired-builder/Dockerfile.compile services/labwired-builder/.dockerignore
git commit -m "feat(builder): Dockerfile for shared PlatformIO compile image"
```

---

## Task 4: `Dockerfile.builder` — sim + thin Node service

**Files:**
- Create: `services/labwired-builder/Dockerfile.builder`

- [ ] **Step 1: Write the Dockerfile**

Create `services/labwired-builder/Dockerfile.builder` (build context = **repo root**, because it needs the `core` submodule):

```dockerfile
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
```

- [ ] **Step 2: Ensure the submodule is present, then build**

```bash
cd "$(git rev-parse --show-toplevel)"
git submodule update --init --recursive core
docker build -f services/labwired-builder/Dockerfile.builder -t labwired-builder:dev .
```
Expected: builds successfully (the Rust release build of `labwired-cli` runs in stage 1; several minutes cold).

- [ ] **Step 3: Smoke-test the running container**

```bash
docker run -d --name lwb-smoke -e BUILDER_SECRET=test -p 127.0.0.1:18080:18080 labwired-builder:dev
sleep 3
curl -fsS http://127.0.0.1:18080/healthz     # {"ok":true}
docker rm -f lwb-smoke
```
Expected: `{"ok":true}`.

- [ ] **Step 4: Commit**

```bash
git add services/labwired-builder/Dockerfile.builder
git commit -m "feat(builder): multi-stage Dockerfile for sim + node service"
```

---

## Task 5: `docker-compose.yml` — the 3-service stack

**Files:**
- Create: `services/labwired-builder/docker-compose.yml`
- Create: `services/labwired-builder/.env.example`
- Modify: `.gitignore`

- [ ] **Step 1: Write `.env.example`**

Create `services/labwired-builder/.env.example`:

```dotenv
# Copy to .env on the Hetzner box (chmod 600) and fill in real values.
# Shared secret guarding the public /run + /compile (set the same value as the
# labwired-api Worker's BUILDER_SECRET). Generate with: openssl rand -hex 32
BUILDER_SECRET=replace-me

# Cloudflare named-tunnel token (Zero Trust → Tunnels → your tunnel → token).
TUNNEL_TOKEN=replace-me

# Image tag to deploy (e.g. latest or a specific commit SHA).
IMAGE_TAG=latest
```

- [ ] **Step 2: Write `docker-compose.yml`**

Create `services/labwired-builder/docker-compose.yml`:

```yaml
# LabWired builder stack — pull-to-deploy on Hetzner.
#   docker compose pull && docker compose up -d
#
# Networks:
#   backend (internal: true)  -> compile + builder; NO internet egress.
#   edge    (bridge)          -> builder + cloudflared; cloudflared reaches CF.
# The compile container (runs untrusted source through a compiler) is on the
# egress-denied backend only and is never published to the host/internet.

services:
  compile:
    image: ghcr.io/w1ne/labwired-compile:${IMAGE_TAG:-latest}
    restart: unless-stopped
    networks: [backend]
    security_opt: ["no-new-privileges:true"]
    cap_drop: ["ALL"]
    tmpfs: ["/tmp"]
    mem_limit: 2g
    cpus: 2.0
    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://127.0.0.1:8080/healthz"]
      interval: 30s
      timeout: 5s
      retries: 3

  builder:
    image: ghcr.io/w1ne/labwired-builder:${IMAGE_TAG:-latest}
    restart: unless-stopped
    env_file: [.env]
    environment:
      COMPILE_URL: http://compile:8080
    depends_on:
      compile:
        condition: service_healthy
    networks: [backend, edge]
    security_opt: ["no-new-privileges:true"]
    cap_drop: ["ALL"]
    tmpfs: ["/tmp"]
    mem_limit: 1g
    cpus: 2.0
    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://127.0.0.1:18080/healthz"]
      interval: 30s
      timeout: 5s
      retries: 3

  cloudflared:
    image: cloudflare/cloudflared:latest
    restart: unless-stopped
    command: ["tunnel", "run"]
    environment:
      TUNNEL_TOKEN: ${TUNNEL_TOKEN}
    depends_on:
      builder:
        condition: service_healthy
    networks: [edge]

networks:
  backend:
    internal: true
  edge:
    driver: bridge
```

- [ ] **Step 3: Validate the compose file**

Run: `cd services/labwired-builder && IMAGE_TAG=dev TUNNEL_TOKEN=x docker compose config >/dev/null && echo OK`
Expected: `OK` (compose parses and interpolates without error).

- [ ] **Step 4: End-to-end local stack test (builder ↔ compile, no tunnel)**

```bash
cd services/labwired-builder
# Tag the locally-built images so the compose (which references ghcr.io/...) finds them:
docker tag labwired-compile:dev ghcr.io/w1ne/labwired-compile:dev
docker tag labwired-builder:dev ghcr.io/w1ne/labwired-builder:dev
# Bring up just compile + builder (skip cloudflared — no tunnel locally):
IMAGE_TAG=dev BUILDER_SECRET=test TUNNEL_TOKEN=unused \
  docker compose up -d compile builder
sleep 8
# Exec into builder and prove the proxied /compile reaches the compile service:
docker compose exec -T builder sh -c \
  'curl -fsS -X POST http://127.0.0.1:18080/compile -H "Content-Type: application/json" -H "X-Builder-Secret: test" -d "{\"board\":\"stm32l476\",\"language\":\"cpp\",\"source\":\"int main(void){while(1){}}\"}"' \
  | python3 -c 'import sys,json; print("ok=", json.load(sys.stdin)["ok"])'
IMAGE_TAG=dev docker compose down
```
Expected: `ok= True` — the builder proxied `/compile` over the `backend` network to the compile service and got an ELF back.

- [ ] **Step 5: Ignore the live `.env`**

Add to the repo-root `.gitignore`:

```
services/labwired-builder/.env
```

- [ ] **Step 6: Commit**

```bash
git add services/labwired-builder/docker-compose.yml services/labwired-builder/.env.example .gitignore
git commit -m "feat(builder): compose stack (builder + compile + cloudflared)"
```

---

## Task 6: CI — build + push both images to private GHCR

**Files:**
- Create: `.github/workflows/builder-deploy.yml`

- [ ] **Step 1: Write the workflow**

Create `.github/workflows/builder-deploy.yml`:

```yaml
name: Build & Push Builder Images

on:
  push:
    branches: ["main"]
    paths:
      - "services/labwired-builder/**"
      - "core"
      - ".github/workflows/builder-deploy.yml"
  workflow_dispatch:

permissions:
  contents: read
  packages: write

jobs:
  build-push:
    runs-on: ubuntu-latest
    steps:
      - name: Checkout (with submodules)
        uses: actions/checkout@v4
        with:
          submodules: recursive

      - name: Set up Buildx
        uses: docker/setup-buildx-action@v3

      - name: Log in to GHCR
        uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Build & push labwired-compile
        uses: docker/build-push-action@v6
        with:
          context: services/labwired-builder
          file: services/labwired-builder/Dockerfile.compile
          push: true
          tags: |
            ghcr.io/w1ne/labwired-compile:latest
            ghcr.io/w1ne/labwired-compile:${{ github.sha }}
          cache-from: type=gha,scope=labwired-compile
          cache-to: type=gha,mode=max,scope=labwired-compile

      - name: Build & push labwired-builder
        uses: docker/build-push-action@v6
        with:
          context: .
          file: services/labwired-builder/Dockerfile.builder
          push: true
          tags: |
            ghcr.io/w1ne/labwired-builder:latest
            ghcr.io/w1ne/labwired-builder:${{ github.sha }}
          cache-from: type=gha,scope=labwired-builder
          cache-to: type=gha,mode=max,scope=labwired-builder
```

- [ ] **Step 2: Lint the workflow YAML**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/builder-deploy.yml')); print('OK')"`
Expected: `OK`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/builder-deploy.yml
git commit -m "ci: build and push builder images to private GHCR"
```

- [ ] **Step 4: Post-merge manual verification (do after this lands on `main`)**

After merge, confirm the workflow ran and the packages exist + are **private**:
```bash
gh run list --workflow=builder-deploy.yml -L 3
gh api user/packages/container/labwired-compile | python3 -c 'import sys,json;print(json.load(sys.stdin)["visibility"])'
gh api user/packages/container/labwired-builder  | python3 -c 'import sys,json;print(json.load(sys.stdin)["visibility"])'
```
Expected: latest run `completed/success`; both packages print `private`. If a package shows `public`, set it to private in GitHub → Packages → package → Settings → Change visibility.

---

## Task 7: `deploy.sh` + rewritten RUNBOOK

**Files:**
- Create: `services/labwired-builder/deploy.sh`
- Modify: `services/labwired-builder/deploy/RUNBOOK.md`

- [ ] **Step 1: Write `deploy.sh`**

Create `services/labwired-builder/deploy.sh`:

```bash
#!/usr/bin/env bash
# Steady-state deploy: pull the latest images and (re)start the stack.
# Run from the deploy directory on the Hetzner box (where .env lives).
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -f .env ]; then
  echo "ERROR: .env not found. Copy .env.example to .env and fill it in." >&2
  exit 1
fi

docker compose pull
docker compose up -d
docker compose ps
```

- [ ] **Step 2: Make it executable**

Run: `chmod +x services/labwired-builder/deploy.sh`

- [ ] **Step 3: Rewrite the RUNBOOK**

Replace the entire contents of `services/labwired-builder/deploy/RUNBOOK.md` with:

````markdown
# LabWired Builder — Hetzner Deploy Runbook

The builder runs as a Docker Compose stack on a small Hetzner VPS (cx22 /
2 vCPU / 4 GB, Ubuntu 24.04). Steady-state deploy is two commands; the box needs
only Docker (no Rust, Node, or PlatformIO installed on the host).

```
compile      ghcr.io/w1ne/labwired-compile   PlatformIO build service  (internal only)
builder      ghcr.io/w1ne/labwired-builder    /run sim + /compile proxy (internal only)
cloudflared  cloudflare/cloudflared           tunnel → builder.labwired.com
```

Both app images are **private GHCR packages**. The compile container compiles
untrusted source, so it lives on an egress-denied internal network and is never
published to the host.

## Exposure invariants (do not break)

1. **Never add a `ports:` mapping to `builder` or `compile`.** Docker's iptables
   rules bypass the host firewall; a published port is internet-reachable even
   with UFW denying it. cloudflared reaches the builder over the compose network.
2. **The Cloudflare tunnel has exactly one ingress rule:**
   `builder.labwired.com → http://builder:18080`, then a `404` catch-all. Never
   point an ingress rule at the compile service.
3. **The compile service stays on the `backend` (internal) network** so it has no
   internet egress at runtime (its toolchain caches are baked into the image).

---

## One-time bootstrap

### 1. Install Docker

```bash
curl -fsSL https://get.docker.com | sudo sh
```

### 2. Authenticate to private GHCR (read-only)

Create a fine-grained PAT with **`read:packages`** only, then:

```bash
echo "<PAT>" | docker login ghcr.io -u <github-username> --password-stdin
```

### 3. Create the deploy directory and config

```bash
sudo mkdir -p /opt/labwired-builder && cd /opt/labwired-builder
# Copy docker-compose.yml, .env.example, deploy.sh from
# services/labwired-builder/ (scp, or curl the raw files from the repo).
cp .env.example .env && chmod 600 .env
# Edit .env: set BUILDER_SECRET (openssl rand -hex 32 — must match the
# labwired-api Worker secret), TUNNEL_TOKEN, and IMAGE_TAG.
```

Set the Worker side of the shared secret (once, from a machine with wrangler):

```bash
env -u CLOUDFLARE_API_TOKEN -u CLOUDFLARE_ACCOUNT_ID \
  npx wrangler secret put BUILDER_SECRET --name labwired-api
# paste the same value as BUILDER_SECRET in .env
```

### 4. Create the Cloudflare named tunnel (token-based)

In the Cloudflare dashboard → Zero Trust → Networks → Tunnels:
1. Create a tunnel named `labwired-builder`; copy its **token** into `.env`
   (`TUNNEL_TOKEN`).
2. Add a **public hostname**: `builder.labwired.com` → service
   `http://builder:18080`. (This is the single ingress rule from invariant #2.)

### 5. Bring it up

```bash
./deploy.sh
```

---

## Steady-state deploy

```bash
cd /opt/labwired-builder
./deploy.sh        # docker compose pull && up -d
```

Pin a specific build by setting `IMAGE_TAG=<commit-sha>` in `.env` instead of
`latest`.

---

## Smoke test

```bash
# Public health (through the tunnel):
curl https://builder.labwired.com/healthz          # {"ok":true}

# Proxied compile (through the tunnel; needs BUILDER_SECRET):
SECRET=$(grep BUILDER_SECRET .env | cut -d= -f2)
curl -s -X POST https://builder.labwired.com/compile \
  -H 'Content-Type: application/json' -H "X-Builder-Secret: $SECRET" \
  -d '{"board":"stm32l476","language":"cpp","source":"int main(void){while(1){}}"}' \
  | python3 -c 'import sys,json;print("ok=",json.load(sys.stdin)["ok"])'
```

## Logs & ops

```bash
docker compose logs -f builder
docker compose logs -f compile
docker compose logs -f cloudflared
docker compose ps
```
````

- [ ] **Step 4: Verify the RUNBOOK has no stale systemd references**

Run: `grep -ni -E "systemd|systemctl|rsync|scp .*labwired|cargo build" services/labwired-builder/deploy/RUNBOOK.md || echo "clean"`
Expected: `clean` (the old manual-install instructions are fully replaced).

- [ ] **Step 5: Commit**

```bash
git add services/labwired-builder/deploy.sh services/labwired-builder/deploy/RUNBOOK.md
git commit -m "docs(builder): container deploy runbook + deploy.sh"
```

---

## Task 8: Retire the obsolete systemd unit + cloudflared config

The host-level systemd unit and the credentials-file cloudflared config are superseded by the compose stack (token-based tunnel). Remove them so there is one deploy path, not two.

**Files:**
- Delete: `services/labwired-builder/deploy/labwired-builder.service`
- Delete: `services/labwired-builder/deploy/cloudflared-config.yml`

- [ ] **Step 1: Confirm nothing else references them**

Run: `cd "$(git rev-parse --show-toplevel)" && grep -rn -E "labwired-builder.service|cloudflared-config.yml" --exclude-dir=node_modules --exclude-dir=.git . || echo "no references"`
Expected: `no references` (the rewritten RUNBOOK does not mention them).

- [ ] **Step 2: Delete the files**

```bash
git rm services/labwired-builder/deploy/labwired-builder.service \
       services/labwired-builder/deploy/cloudflared-config.yml
```

- [ ] **Step 3: Commit**

```bash
git commit -m "chore(builder): remove systemd/credentials-file deploy (superseded by compose)"
```

---

## Final verification

- [ ] **Run the builder package test suite:**
  `cd services/labwired-builder && npx vitest run`
  Expected: PASS (compile, compile-server, server + proxy, run unit tests). The `/run` e2e tests require a locally built `labwired` binary; build it first with `git submodule update --init core && (cd core && cargo build -p labwired-cli --release)` and set `LABWIRED_BIN` if you want them green locally.

- [ ] **Typecheck:** `cd services/labwired-builder && npm run typecheck`
  Expected: no errors.

- [ ] **Confirm the deploy is genuinely one command** by reading `deploy.sh` and the RUNBOOK steady-state section: `docker compose pull && docker compose up -d`, box needs only Docker + a read-only GHCR login.

---

## Slice B (separate plan — not in scope here)

After Slice A ships, a follow-up plan covers the proto.cat unification:
- Superset request (`board` | `labwired_board_id` | `chip_family`) and response
  (emit both `elfBase64`/`elf_base64`, `log`/`log_tail`, plus `platformio_board`,
  `framework`, `mapping_source`, `supported_chip_families`) in the compile service.
- Merge proto.cat's `chip_families` + alias board ids into one catalog served at
  `/boards`.
- Repoint proto.cat's `LABWIRED_COMPILE_URL` at `ghcr.io/w1ne/labwired-compile`,
  delete its Python `compile-service/`, and verify its TS client against the
  superset response.
