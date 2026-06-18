# Shared Compile Service & Containerized Hetzner Deploy — Design

- **Date:** 2026-06-17
- **Status:** Approved (brainstorm), pending implementation plan
- **Owner repo:** `w1ne/labwired`
- **Affected repos:** `w1ne/labwired` (primary), `protocat` (final follow-up slice)

## Problem

Two pain points, addressed together:

1. **The Hetzner builder is slow and fragile to deploy.** Today it is a 7-step
   manual `RUNBOOK.md`: build the Rust `labwired` binary on a dev machine → `scp`
   it → `rsync` the Node service → `npm ci` → generate a secret → install a
   systemd unit → set up a `cloudflared` tunnel → set a Worker secret. There is
   no single artifact and no one-command deploy.

2. **The PlatformIO compile path is forked, not shared.** labwired-builder
   (`services/labwired-builder/src/compile.ts`, Node/TS) and proto.cat
   (`compile-service/server.py`, Python/Flask) are two independent
   reimplementations of the same "compile agent C/C++ to an ELF via a generated
   `platformio.ini`" service. Their board catalogs have already drifted
   (`PIO_BOARDS` in code vs `board_map.json`). The in-code comment claiming the
   catalog is "the single source of truth … shared with proto.cat" is
   aspirational and currently false.

## Goals

- Hetzner deploy becomes **`docker compose pull && docker compose up -d`** — the
  box needs only Docker (no Rust, Node, pipx, or PlatformIO installed on host).
- The **PlatformIO compile service is a single shared image**, built and
  published by labwired, consumed by both labwired and proto.cat.
- **No external contract breakage:** `builder.labwired.com/{run,compile}` (called
  by the `labwired-api` Cloudflare Worker) keeps working unchanged.
- proto.cat is **not rewritten** — it keeps its TS app and TS compile client; it
  only drops its Python backend and repoints a URL.

## Non-goals

- No change to the Cloudflare-hosted pieces' *hosting model*: Pages
  (`landing_page`, `playground`) and the `labwired-api` Worker keep deploying
  from CI as they do today.
- No change to the `/run` simulator semantics or the `labwired` Rust binary.
- Not building a registry/orchestrator beyond Docker Compose. Single-box deploy.

## Decisions (settled during brainstorm)

| Decision | Choice |
|---|---|
| Image scope | Multi-service compose of small prebuilt images (not one fat image) |
| Ship mechanism | CI builds → **private** GHCR; boxes pull with a read-only token |
| Tunnel | `cloudflared` as a compose sidecar (token-based) |
| Sharing depth | **Fully unified compile service** |
| Canonical service language | **Node/TS** (labwired's `compile.ts`), Python retired |
| Owner repo | **labwired** publishes the shared image; proto.cat consumes it |
| Builder ↔ compile | Builder **proxies** `/compile` to the internal compile service |

## Architecture

```
labwired repo ──CI──► ghcr.io/w1ne/labwired-compile:<ver>   ← THE shared part (PlatformIO + warm caches + Node compile svc)
              └─CI──► ghcr.io/w1ne/labwired-builder:<ver>    ← Rust sim + thin Node svc (small)

Hetzner  /opt/labwired-builder/docker-compose.yml + .env:
   ├─ compile      (labwired-compile image)                         :8080  internal-only
   ├─ builder      (labwired-builder image)  /run + proxy /compile  :18080 internal-only
   └─ cloudflared  (official image, TUNNEL_TOKEN)  builder.labwired.com → builder:18080

proto.cat  deploy/docker-compose.yml:
   compile-service:  image: ghcr.io/w1ne/labwired-compile:<ver>   ← was `build: ../compile-service`
   (proto.cat TS client: LABWIRED_COMPILE_URL → http://compile-service:8080)
```

### Components

1. **`labwired-compile` image** (the shared artifact)
   - Base: `node:22-bookworm-slim` + python3 + pipx + `platformio`.
   - **Warm caches by platform** (`pio pkg install -g -p ststm32 espressif32
     nordicnrf52 raspberrypi`) — platform installs are broader than board installs
     and cover every board in the catalog for both projects. This is the heavy,
     slow, cacheable layer.
   - Houses the canonical compile service (a standalone Node HTTP server wrapping
     the existing `compile.ts` logic — see "Canonical compile service" below).
   - Runs **non-root**; `HEALTHCHECK` on `/healthz`.
   - Listens on `:8080`.

2. **`labwired-builder` image** (small)
   - Multi-stage: stage 1 `rust:slim` builds `cargo build -p labwired-cli
     --release` → `/labwired` (build context includes the `core` **submodule**);
     stage 2 `node:22-bookworm-slim` with the existing `services/labwired-builder`
     Node service + the copied `labwired` binary. **No PlatformIO** here anymore.
   - Exposes `/run` (sim) and `/healthz` exactly as today.
   - `/compile` becomes a **proxy**: forwards the request to
     `http://compile:8080/compile` over the internal compose network, preserving
     the public `builder.labwired.com/compile` contract.

3. **`services/labwired-builder/docker-compose.yml`**
   - `compile`: `image: ghcr.io/w1ne/labwired-compile:<ver>`, no published host
     port (internal network only), `restart: unless-stopped`, healthcheck.
   - `builder`: `image: ghcr.io/w1ne/labwired-builder:<ver>`, `env_file: .env`
     (`BUILDER_SECRET`), `COMPILE_URL=http://compile:8080`, `depends_on: compile`
     (healthy), no published host port, `restart: unless-stopped`.
   - `cloudflared`: official image, `command: tunnel run`, `TUNNEL_TOKEN` from
     `.env`, `depends_on: builder`. Token-based tunnel → ingress
     (`builder.labwired.com → http://builder:18080`) configured once in the CF
     Zero-Trust dashboard (no credentials JSON / `config.yml` in the repo).
   - **Hardening carried over from the systemd unit:** `security_opt:
     [no-new-privileges:true]`, `cap_drop: [ALL]`, `tmpfs: /tmp`,
     `mem_limit`/`cpus`, dedicated internal network.

4. **CI — `.github/workflows/builder-deploy.yml`**
   - On push to `main` touching `services/labwired-builder/**`, `core`, or the
     workflow file → `docker buildx` build + push **both** images to GHCR
     (`:latest` + `:<sha>` + `:<semver>` when tagged), GHA layer cache,
     `GITHUB_TOKEN` auth, `submodules: recursive` checkout.
   - **Packages are private.** This is internal tooling, not a public artifact.
     CI pushes with the workflow's `GITHUB_TOKEN` (`packages: write`). Each
     deploy box authenticates once with a fine-grained, **read-only**
     (`read:packages`) token via `docker login ghcr.io`; both boxes are under the
     `w1ne` namespace so one token serves both. Nothing secret is baked into the
     images (PlatformIO, board catalog, MIT-licensed compile logic) — private is
     about not advertising internal tooling, not secret protection.
   - Existing `builder-ci.yml` (PR test job) is unchanged.

5. **`deploy.sh` + rewritten `RUNBOOK.md`**
   - One-time bootstrap: install Docker; create `/opt/labwired-builder/` with
     `docker-compose.yml` + `.env` (`BUILDER_SECRET`, `TUNNEL_TOKEN`); create the
     named tunnel + ingress in CF dashboard.
   - Steady state: `deploy.sh` = `docker compose pull && docker compose up -d`.

### Canonical compile service (contract reconciliation)

The shared service is Node/TS, built from labwired's `compile.ts` (richer:
per-board `platformio.ini` generation + diagnostic parsing + `runnable` flag). It
must satisfy **both** callers, so it accepts a **superset request** and emits a
**superset response**.

**Request (accept all of):**
- `board` (labwired `PIO_BOARDS` id) — labwired-builder's current key.
- `labwired_board_id` and/or `chip_family` (fallback) — proto.cat's keys.
- `source`, `language?` (`c`|`cpp`).

**Response (superset; emit both namings):**
- `ok: boolean`
- ELF: emit **both** `elfBase64` (labwired) and `elf_base64` (proto.cat).
- log: emit **both** `log` (labwired) and `log_tail` (proto.cat).
- `diagnostics[]` (labwired), `runnable` (labwired).
- `platformio_board`, `framework`, `mapping_source` (proto.cat).
- On failure: `error`, `supported_board_ids`, `supported_chip_families`
  (proto.cat).

**Endpoints:** `/compile` (POST), `/boards` (GET, proto.cat), `/healthz` (GET).
A `/health` alias is added so proto.cat's existing healthcheck keeps working.

**Auth:** optional, env-gated. The compile service is **never published to the
host or internet** — it is reachable only on a private compose network. The
builder still gates the public `/compile` behind `X-Builder-Secret` (unchanged),
and proto.cat reaches the service only on its own private compose network. So
the public exposure is identical to today.

**Board catalog:** single source of truth lives in labwired, baked into the
`labwired-compile` image and served at `/boards`. proto.cat's `chip_families`
fallback entries are merged into it. This kills the `PIO_BOARDS` vs
`board_map.json` drift.

## Security / exposure surface

The compile endpoint is RCE-by-design (untrusted C/C++ → real compiler), so the
exposure boundary is load-bearing. These are **invariants the implementation must
enforce**, not assumptions:

**Reachable from the internet (via tunnel → `builder.labwired.com` → `builder:18080`):**
- `/healthz` (open, returns only `{ok:true}`), `/run` and `/compile` (both behind
  `X-Builder-Secret`). Nothing else.

**Must stay internal — enforced invariants:**
1. **No `ports:` mapping on `builder` or `compile`.** They communicate over a
   private compose network by service name. Rationale: Docker injects its own
   iptables rules, so a published port is reachable from the internet **even when
   the host firewall (UFW / Hetzner) denies it**. If a port must ever be
   published, bind `127.0.0.1:` explicitly.
2. **The compile service is never in the tunnel ingress.** Exactly one ingress
   rule (`builder.labwired.com → http://builder:18080`) followed by a
   `http_status:404` catch-all. A rule pointed at `compile:8080` would expose the
   auth-optional compiler.
3. **Compile container drops network egress.** PlatformIO needs network only for
   the one-time cache warm at *build* time; at *run* time the caches are baked in,
   so the running container needs no outbound. Dropping egress turns "RCE" into
   "RCE in a sealed sandbox."

**Residual, named:**
- Single shared `X-Builder-Secret` guards the RCE surface — leak = compile+run of
  arbitrary code. Hardening (non-root, `cap_drop: ALL`, `no-new-privileges`,
  mem/cpu limits, tmpfs `/tmp`) limits blast radius.
- `.env` (`BUILDER_SECRET`, `TUNNEL_TOKEN`): `chmod 600`, gitignored, never baked
  into an image layer.
- GHCR images carry **no secrets** — no secret build-args; safe to publish.

## Data flow

```
Agent / ChatGPT App ─► labwired-api Worker ─► builder.labwired.com (tunnel)
                                                    │
                                              builder:18080
                                              ├─ /run     → Rust labwired sim (in-image)
                                              └─ /compile → proxy → compile:8080 → PlatformIO build → ELF
proto.cat app (TS) ─► LABWIRED_COMPILE_URL ─► compile-service:8080 (same labwired-compile image)
```

## Error handling

- Compile service down → builder's `/compile` proxy returns `502` with a clear
  message (distinguish "compile backend unreachable" from "compile failed").
- `depends_on … condition: service_healthy` so builder/cloudflared wait for
  their dependency to pass its healthcheck before accepting traffic.
- `restart: unless-stopped` on all three services; cloudflared reconnects the
  tunnel automatically.
- Compile timeouts / log truncation behavior preserved from current `compile.ts`
  (`COMPILE_TIMEOUT_MS`, `MAX_LOG`).

## Testing

- **Unit/contract:** existing `services/labwired-builder` vitest suite keeps
  passing against the locally built `labwired` binary. Add contract tests
  asserting the superset response shape (both `elfBase64`/`elf_base64`,
  `log`/`log_tail`) for representative boards across both request styles.
- **Image smoke (CI):** after build, `docker run` each image and curl `/healthz`;
  for `labwired-compile`, curl `/boards` and a tiny blink compile for one ST and
  one ESP32 board.
- **Compose smoke (CI, optional):** `docker compose up`, hit builder `/healthz`
  and a proxied `/compile`, assert a non-empty ELF; tear down.
- **Manual:** the rewritten RUNBOOK's smoke section (`/healthz`, an authed
  `/run`, a proxied `/compile`).

## Rollout / slices

1. **Slice A (labwired, self-contained, independently valuable):**
   `labwired-compile` image (canonical Node service + superset contract + merged
   catalog), `labwired-builder` image (sim-only + `/compile` proxy),
   `docker-compose.yml`, `builder-deploy.yml` CI, `deploy.sh`, rewritten RUNBOOK.
   End state: Hetzner is pull-to-deploy; `/compile` and `/run` behave as before.
2. **Slice B (proto.cat follow-up, small):** swap `deploy/docker-compose.yml`
   `compile-service` from `build: ../compile-service` to `image:
   ghcr.io/w1ne/labwired-compile:<ver>`; set `LABWIRED_COMPILE_URL`; delete the
   Python `compile-service/` (`server.py`, `board_map.json`, its Dockerfile);
   verify proto.cat's TS client against the superset response.

## Risks & mitigations

- **Image size** — warming 4 PlatformIO platforms (ST/ESP32/Nordic/RP2040) yields
  a multi-GB `labwired-compile` image. First `docker pull` is slow. Mitigate:
  GHCR + layer caching; pull is effectively one-time per box. Acceptable for the
  one-deploy goal.
- **Cross-repo coupling** — proto.cat now depends on a labwired-published image
  tag. Mitigate: publish **versioned** tags (`:1`, `:1.2.0`); proto.cat pins a
  version, never floats on `:latest`.
- **Auth-optional compile service** runs untrusted source through a compiler.
  Mitigate: never publish its port to host/internet (private compose network
  only); public `/compile` stays behind the builder's secret; resource limits +
  non-root + no-new-privileges on the container.
- **`core` is a submodule** — image build must checkout with `submodules:
  recursive` and use a build context that includes `core/`. Captured in the CI
  step and Dockerfile context.
- **Contract drift returns** — mitigate by deleting proto.cat's Python service in
  Slice B so there is exactly one implementation and one catalog.
