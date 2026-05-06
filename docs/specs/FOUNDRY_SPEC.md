[← Back to Hub](../README.md)

# LabWired Foundry — Product Specification

> **Version**: 0.1 (MVP)
> **Status**: Pre-build
> See also: [Pricing Model](./FOUNDRY_PRICING.md) | [Business Model & Risk Storming](../../.gemini/antigravity/brain/5475f5e0-78a6-4696-9a5a-758889e7121e/foundry_business_model.md)

---

## 1. Product Vision

LabWired Foundry is an **Agent-Native verification API** for running firmware against a curated catalog of pre-verified board and chip assets. The primary consumer is an AI coding agent, CI system, or platform integration, not a human clicking buttons.

> "Verify on the Foundry. Run locally when you want. Use the same deterministic assets in both places."

### The Managed Verification Model
Users pay for trusted hosted verification capacity, artifact retention, queue guarantees, and catalog-backed execution. Supported assets remain portable: users can download Strict IR and integrate it into local development environments or internal CI clusters using the open-source LabWired toolchain.

---

---

## 2. Architecture Overview

```mermaid
graph TD
    A["AI Agent / CI Bot"] -->|"POST /v1/models/verify (API Key)"| B[Go Backend]
    B --> C[Job Queue]
    C --> D["LabWired Core (Rust)\nFormal Simulation Engine\n(Host process + timeout)"]
    D --> E[Artifact Storage\nLocal filesystem]
    E -->|"VCD traces + Compiler Logs"| B
    B -->|"Result + Feedback"| A

    F["Human Dev (Optional)"] -->|"Browser"| G[React Dashboard]
    G -->|"catalog, usage"| B
```

**Infrastructure (Hetzner VPS CX21, €7/mo)**:
| Component | Implementation | Notes |
| :--- | :--- | :--- |
| API Server | Go (`net/http`) | High-concurrency VaaS |
| Job Queue | Go Channels | In-memory job buffering |
| Database | SQLite | Stores API keys, workspaces, and run quotas |
| Artifact Storage | Local filesystem | Ephemeral VCD trace storage |
| Simulation | LabWired Orchestrator | Native Rust execution |
| Reverse Proxy | Caddy (auto TLS) | |
| Frontend | React + Vite | Storefront & Dev Portal |

---

## 3. API Surface

### Authentication & Quotas

```http
Authorization: Bearer lw_sk_live_xxxxxxxxxxxxxxxx
```

- **Authentication**: API keys are generated via an admin CLI (`cmd/addkey`) and stored as bcrypt hashes in SQLite. The Go backend enforces auth middleware on all Agent API routes.
- **Quota Management**: A dedicated `quotaMiddleware` intercepts compute tasks. It queries the `simulation_runs` table in SQLite to ensure the authenticated workspace has not exceeded its monthly run limit for the active package. High-usage agents are HTTP 429 rate-limited if they exceed quotas.
- **Revocation**: Keys can be rotated or soft-deleted via the database, instantly blocking further access.

---

### Endpoints (v1)

| Method | Path | Auth | Description |
| :--- | :--- | :---: | :--- |
| `GET` | `/v1/catalog` | Optional | List all public pre-verified assets |
| `GET` | `/v1/catalog/{id}` | Optional | Detail view: register map, proof status, artifact URLs |
| `POST` | `/v1/models/verify` | Required | Enqueue a verification run for a supported catalog component |
| `POST` | `/v1/systems/verify` | Required | Enqueue an integrated board simulation |
| `GET` | `/v1/usage` | Required | Current-period run count, quota, and tier |

---

### Async Simulation Lifecycle

Simulations are long-running (up to 30s). The API uses an **Asynchronous Polling Pattern**:

1.  **Submission**: `POST /v1/models/verify` returns `202 Accepted` with a `run_id`.
2.  **Execution**: The job is pushed to an in-memory Go channel queue. Background workers execute `labwired test` with bounded timeout and persist artifacts.
3.  **Polling**: The client calls `GET /v1/runs/{run_id}`.
    -   `queued`: Waiting for a worker.
    -   `running`: Simulation is active.
    -   `pass` | `fail`: Simulation completed, artifacts ready.
    -   `error`: Infrastructure failure (timeout, sandbox crash).

> **Future v1.1**: WebSocket streaming for cycle-by-cycle metrics or Webhooks for CI integration.

---

### Error Taxonomy

| HTTP Status | Error Code | Logic | Client Action |
| :---: | :--- | :--- | :--- |
| `401` | `unauthorized` | Missing/Invalid API key. | Check key in dashboard. |
| `429` | `quota_exceeded` | Monthly run limit reached. | Upgrade tier or wait for reset. |
| `422` | `invalid_target` | Unsupported asset or malformed request. | Check the catalog and request payload. |
| `429` | `rate_limited` | Too many requests per second (concurrency). | Implement exponential backoff. |
| `503` | `queue_full` | System at capacity on the VPS. | Retry after 5-10 seconds. |
| `500` | `internal_error` | Unexpected backend or sandbox crash. | Report to LabWired support. |

---

### Security & Isolation Constraints

Current implementation executes verification as a host subprocess (`labwired test`) with:

-   **Input Limits**: request body and field-size caps before persistence.
-   **Quota Gates**: per-workspace quota checks and atomic run reservation.
-   **Timeouts**: bounded execution with `context.WithTimeout`.
-   **Artifact Scoping**: runs are polled by `(run_id, workspace_id)`.

> Planned hardening: container sandboxing (`--network none`, strict CPU/memory caps, read-only rootfs).

---

## 4. Data Model

### `api_keys` table
| Column | Type | Notes |
| :--- | :--- | :--- |
| `id` | UUID | Primary key |
| `workspace_id` | UUID | FK to workspace |
| `key_hash` | TEXT | bcrypt hash of `lw_sk_live_...` |
| `tier` | ENUM | `free`, `pro`, `enterprise` |
| `monthly_quota` | INT | Max runs per billing period |
| `revoked` | BOOL | Soft-delete for rotation |

### `simulation_runs` table
| Column | Type | Notes |
| :--- | :--- | :--- |
| `run_id` | UUID | Primary key, exposed to clients |
| `workspace_id` | UUID | Owner |
| `peripheral_id` | TEXT | Catalog or private asset ID |
| `status` | ENUM | `queued`, `running`, `pass`, `fail`, `error` |
| `artifacts_path` | TEXT | `/srv/artifacts/{run_id}/` |
| `assertions_passed` | INT | |
| `assertions_total` | INT | |
| `created_at` | TIMESTAMP | |

---

## 5. Data Retention & Privacy

| Asset Type | Retention | Notes |
| :--- | :--- | :--- |
| **Simulation Artifacts** | 14 Days | VCD/JSON files deleted from VPS disk after expiry. |
| **Usage Logs** | 90 Days | High-level metadata (run_id, timestamp) for billing. |
| **SaaS Analytics** | Indefinite | Anonymized usage trends (verification runs per month). |
| **API Keys** | Until Revoked | Stored as salted bcrypt hashes. |

> **GDPR Compliance**: The Hetzner VPS is hosted in Frankfurt (EU). No personally identifiable information (PII) is included in simulation artifacts.

Retention is enforced by a periodic backend cleanup sweep:
- `ARTIFACT_RETENTION_DAYS` (default: `14`)
- `ARTIFACT_CLEANUP_INTERVAL_SECONDS` (default: `3600`)
- Cleanup removes expired artifact directories and clears `simulation_runs.artifacts_path`.

---

## 6. Deployment Layout (Hetzner VPS)

```bash
/srv/foundry/
├── api/            # Go backend
├── web/            # Dashboard frontend (React build)
├── db/             # SQLite (key_store.db)
├── artifacts/      # /artifacts/{run_id} storage
│   └── catalog/    # Pre-verified golden assets
```

**Caddy Proxy**:
- Port 80/443: Main landing and dashboard.
- Path `/v1/*`: Proxied to Go API server.
- Artifacts are served by authenticated API routes (`/v1/runs/{run_id}/artifacts/{file}`).

---

## 7. Catalog Management

Pre-verified peripherals in the public catalog are added by the LabWired team:

1.  Run the verification pipeline locally (`verify_harness.py + labwired test`)
2.  Confirm 100% assertion pass rate
3.  Copy the IR `.json`, `proof.vcd`, and `result.json` to `/srv/foundry/artifacts/catalog/{id}/`
4.  Insert a row into the `catalog` table
5.  The asset is immediately visible at `GET /v1/catalog/{id}`

> The catalog is **curated and internally maintained** for quality. Users cannot push to the public catalog.
> Enterprise users can receive a **private catalog** backed by LabWired-managed onboarding and validation.

---

## 8. Developer Onboarding Experience (60 Seconds)

This is the highest-priority UX moment. A developer (or their agent) must be productive within 60 seconds of landing.

### Step 1 — Signup (10s)
- Email + password only (GitHub OAuth in v1.1)
- API key shown immediately on screen, ready to copy
- No credit card required for the Free tier during early rollout

### Step 2 — First API Call (30s)
Landing page shows a single ready-to-run `curl` command, pre-populated with the user's key:

```bash
curl -X POST https://foundry.labwired.dev/v1/models/verify \
  -H "Authorization: Bearer lw_sk_live_YOUR_KEY" \
  -H "Content-Type: application/json" \
  -d '{"peripheral_id": "ADXL345", "limits": {"max_steps": 2000}}'
```

### Step 3 — Use the Result (20s)
- Copy `artifacts.ir_url` from the JSON response
- Download the `.json` and drop it into your LabWired project under `assets/`
- Done — CI now runs formal simulation with the same verified asset used in Foundry

---

## 9. Dashboard UI — Visual Specification

### Design Language
| Token | Value | Usage |
| :--- | :--- | :--- |
| Background | `#0d1117` | Page base |
| Surface | `#161b22` | Cards, panels |
| Border | `#30363d` | Card outlines |
| Accent (active) | `#00d9ff` | CTA buttons, links, highlights |
| Pass green | `#39ff14` | PASS proof badge, success |
| Fail red | `#ff4444` | FAIL status |
| Text primary | `#e6edf3` | Headings, body |
| Text secondary | `#8b949e` | Metadata, labels |
| Font (prose) | `Inter` (Google Fonts) | |
| Font (code) | `JetBrains Mono` | API keys, JSON, commands |

**Effects**: Glassmorphism card borders (`backdrop-filter: blur`). PASS badge has a soft neon glow pulse animation (CSS keyframe). Hover on catalog cards lifts with a subtle glow.

---

### Pages

#### `/` — Landing + Catalog *(unauthenticated)*
- **Hero** (above fold):
  - Headline: *"Deterministic firmware verification. One API call."*
  - Sub-text: For AI agents, CI systems, and embedded platform teams
  - Live `curl` snippet (dark code block, copy button)
  - CTA: "Get your free API key →"
- **Catalog grid** (below fold):
  - Responsive 3-column grid of asset cards
  - Each card: chip name, register count, `PASS` badge, "Download IR" button

#### `/assets/{id}` — Asset Detail *(unauthenticated)*
- **Proof badge** at top: `49/49 Passed · 2000 cycles`
- **Register map table**: offset, name, reset value, access type, description
- **Artifact downloads**: `adxl345.json`, `proof.vcd`, `result.json`
- **"Run New Simulation"** button (redirects to signup if unauthenticated)

#### `/dashboard` — Developer Portal *(auth required)*
- **Quota bar**: e.g., `12 / 100 runs used this month (Free tier)`
- **Recent runs table**: run_id, chip, timestamp, status (with PASS/FAIL badge), artifact link
- **API Key section**: masked key display, "Copy", "Rotate" button
- **Upgrade CTA** for Pro/Enterprise users

---

## 10. MVP Scope (v0.1 Deliverables)

### Backend
-   [x] Go API server with API Key Authentication & quota middleware.
-   [x] `/v1/catalog`, `/v1/usage`, and verification endpoints.
-   [/] **Async Job Queue**: `handleVerifyModel` / `handleVerifySystem` → `202 Accepted` + enqueue; `GET /v1/runs/{run_id}` polling endpoint.
-   [/] **Per-workspace quota**: `monthly_quota` column in `api_keys`; middleware reads DB instead of hardcoded value.
-   [x] **Billing webhook integration**: `POST /v1/webhooks/stripe` signature verification + idempotent subscription event handling.
-   [x] **Artifact serving**: authenticated run-scoped endpoint `/v1/runs/{run_id}/artifacts/{file}`.
-   [x] **Artifact retention cleanup**: periodic TTL sweep removes expired run artifacts and clears DB pointers.

### Frontend
-   [/] **Public landing page** (`/`): hero with live `curl` snippet + CTA; catalog grid from `/v1/catalog`.
-   [ ] **Asset detail page** (`/assets/:id`): proof badge, register map table, download buttons.
-   [/] **Developer dashboard** (`/dashboard`): quota bar, recent runs table, API key copy/rotate, upgrade path for paid plans.

### Infrastructure
-   [ ] Docker-wrapped verification sandbox (network=none, 512 MB, read-only rootfs).
-   [ ] Caddy reverse-proxy config (TLS, `/v1/*` → backend).
-   [ ] Deploy to Hetzner VPS.
