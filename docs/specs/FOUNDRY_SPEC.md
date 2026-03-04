[← Back to Hub](../README.md)

# LabWired Foundry — Product Specification

> **Version**: 0.1 (MVP)
> **Status**: Pre-build
> See also: [Pricing Model](./FOUNDRY_PRICING.md) | [Business Model & Risk Storming](../../.gemini/antigravity/brain/5475f5e0-78a6-4696-9a5a-758889e7121e/foundry_business_model.md)

---

## 1. Product Vision

LabWired Foundry is an **Agent-Native simulation API** — the first hosted service that transforms peripheral descriptions into formally verified hardware digital twins. The primary consumer is an AI coding agent, not a human clicking buttons.

> "Generate once on the Foundry. Run forever on your local CI. Zero cost for deterministic local runs."

### The "Pay-for-Verification" Model
Users pay for the expensive, high-fidelity **synthesis and formal proof** process on the Foundry. Once the asset is "Solid Proven," the resulting Strict IR is portable. Users can download the model and integrate it into their local dev environments or internal CI clusters using the open-source LabWired toolchain.

---

---

## 2. Architecture Overview

```mermaid
graph TD
    A["AI Agent / CI Bot"] -->|"POST /v1/tasks/{id}/verify (API Key)"| B[Go Backend]
    B --> C[Job Queue]
    C --> D["LabWired Core (Rust)\nFormal Simulation Engine\n(Docker sandbox)"]
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
| Database | SQLite | Stores API Keys & Run quotas |
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
- **Quota Management**: A dedicated `quotaMiddleware` intercepts expensive simulation tasks. It queries the `simulation_runs` table in SQLite to ensure the authenticated workspace has not exceeded its monthly run limit (e.g., Free Tier: 1000 runs/month). High-usage agents are HTTP 429 rate-limited if they exceed quotas.
- **Revocation**: Keys can be rotated or soft-deleted via the database, instantly blocking further access.

---

### Endpoints (v1)

| Method | Path | Auth | Description |
| :--- | :--- | :---: | :--- |
| `GET` | `/v1/catalog` | Optional | List all public pre-verified peripherals |
| `GET` | `/v1/catalog/{id}` | Optional | Detail view: register map, proof status, artifact URLs |
| `POST` | `/v1/twins/simulate` | Required | Enqueue a simulation run; returns `run_id` |
| `GET` | `/v1/runs/{run_id}` | Required | Poll status and fetch artifact URLs when complete |
| `GET` | `/v1/usage` | Required | Current-period run count, quota, and tier |
| `POST` | `/v1/keys/rotate` | Required | Invalidate current key and issue a new one |

---

### Async Simulation Lifecycle

Simulations are long-running (up to 30s). The API uses an **Asynchronous Polling Pattern**:

1.  **Submission**: `POST /v1/twins/simulate` returns `202 Accepted` with a `run_id`.
2.  **Execution**: The job is pushed to the `asyncio.Queue`. A background worker picks it up, spins up the Docker sandbox, and executes the formal proofs.
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
| `403` | `quota_exceeded` | Monthly run limit reached. | Upgrade tier or wait for reset. |
| `422` | `invalid_schema` | YAML/JSON failed schema validation. | Check input against [Strict IR Spec](../asset_foundry.md). |
| `429` | `rate_limited` | Too many requests per second (concurrency). | Implement exponential backoff. |
| `503` | `queue_full` | System at capacity on the VPS. | Retry after 5-10 seconds. |
| `500` | `internal_error` | Unexpected backend or sandbox crash. | Report to LabWired support. |

---

### Security & Isolation Constraints

To protect the host VPS (€7 Hetzner box), all simulation jobs run inside an **Isolated Docker Sandbox**:

-   **Networking**: `--network none` (Strictly no egress/ingress).
-   **Resources**: `--cpus=1`, `--memory=512m`.
-   **Filesystem**:
    -   Read-only rootfs (`--read-only`).
    -   `/output` mounted as a `tmpfs` (RAM-only) capped at 10MB.
-   **Runtime**: `sysbox` or standard `runc` with a non-root user.
-   **Validation**: User-provided YAML is sanitized and validated against a Pydantic schema **before** reaching the shell/container.

---

## 4. Data Model

### `api_keys` table
| Column | Type | Notes |
| :--- | :--- | :--- |
| `id` | UUID | Primary key |
| `workspace_id` | UUID | FK to workspace |
| `key_hash` | TEXT | bcrypt hash of `lw_sk_live_...` |
| `tier` | ENUM | `free`, `builder`, `team`, `enterprise` |
| `monthly_quota` | INT | Max runs per billing period |
| `revoked` | BOOL | Soft-delete for rotation |

### `simulation_runs` table
| Column | Type | Notes |
| :--- | :--- | :--- |
| `run_id` | UUID | Primary key, exposed to clients |
| `workspace_id` | UUID | Owner |
| `peripheral_id` | TEXT | Catalog ID or `custom` |
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
| **SaaS Analytics** | Indefinite | Anonymized usage trends (runs-per-month). |
| **API Keys** | Until Revoked | Stored as salted bcrypt hashes. |

> **GDPR Compliance**: The Hetzner VPS is hosted in Frankfurt (EU). No personally identifiable information (PII) is included in simulation artifacts.

---

## 6. Deployment Layout (Hetzner VPS)

```bash
/srv/foundry/
├── api/            # FastAPI Python backend
├── web/            # Dashboard frontend (React build)
├── db/             # SQLite (key_store.db)
├── artifacts/      # /artifacts/{run_id} storage
│   └── catalog/    # Pre-verified golden assets
```

**Caddy Proxy**:
- Port 80/443: Main landing and dashboard.
- Path `/v1/*`: Proxied to FastAPI (`uvicorn` on port 8000).
- Path `/artifacts/*`: Directly served static file directory.

---

## 7. Catalog Management

Pre-verified peripherals in the public catalog are added by the LabWired team:

1.  Run the verification pipeline locally (`verify_harness.py + labwired test`)
2.  Confirm 100% assertion pass rate
3.  Copy the IR `.json`, `proof.vcd`, and `result.json` to `/srv/foundry/artifacts/catalog/{id}/`
4.  Insert a row into the `catalog` table
5.  The asset is immediately visible at `GET /v1/catalog/{id}`

> The catalog is **curated and internally maintained** for quality. Users cannot push to the public catalog.
> Enterprise users get a **private catalog** for their own synthesized assets.

---

## 8. Developer Onboarding Experience (60 Seconds)

This is the highest-priority UX moment. A developer (or their agent) must be productive within 60 seconds of landing.

### Step 1 — Signup (10s)
- Email + password only (GitHub OAuth in v1.1)
- API key shown immediately on screen, ready to copy
- No credit card required for Free tier

### Step 2 — First API Call (30s)
Landing page shows a single ready-to-run `curl` command, pre-populated with the user's key:

```bash
curl -X POST https://foundry.labwired.dev/v1/twins/simulate \
  -H "Authorization: Bearer lw_sk_live_YOUR_KEY" \
  -H "Content-Type: application/json" \
  -d '{"peripheral_id": "ADXL345", "limits": {"max_steps": 2000}}'
```

### Step 3 — Use the Result (20s)
- Copy `artifacts.ir_url` from the JSON response
- Download the `.json` and drop it into your LabWired project under `assets/`
- Done — CI now runs formal simulation with a formally-proven peripheral model

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
  - Headline: *"Formally proven hardware simulation. One API call."*
  - Sub-text: Target embedded teams and AI agents
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
- **Quota bar**: e.g., `12 / 50 runs used this month (Free tier)`
- **Recent runs table**: run_id, chip, timestamp, status (with PASS/FAIL badge), artifact link
- **API Key section**: masked key display, "Copy", "Rotate" button
- **Upgrade CTA** for free users

---

## 10. MVP Scope (v0.1 Deliverables)

-   [x] FastAPI skeleton with API Key Authentication.
-   [ ] Async Job Queue (`asyncio.Queue`) for simulation serialization.
-   [ ] Docker-wrapped verification harness.
-   [ ] React-based Catalog & Download Dashboard.
-   [ ] 60-second onboarding: "Signup -> Key -> First Simulation".
-   [ ] WebSocket streaming / Webhooks
-   [ ] Private catalog for Enterprise
