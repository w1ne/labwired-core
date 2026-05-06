[← Back to Hub](../README.md)

# LabWired Foundry — Pricing Model

> **Product**: LabWired Foundry — Agent-Native API for deterministic firmware verification on a curated board catalog.
> **Primary clients**: AI coding agents, platform teams, embedded software teams, CI/CD pipelines.
> See also: [Foundry Product Spec](./FOUNDRY_SPEC.md) | [Asset Foundry Technical Spec](../asset_foundry.md)

---

## Core Concept: What is a Verification Run?

A **verification run** is a single call to the LabWired Core verification pipeline against a supported board or catalog asset. It:
1. Selects a pre-verified catalog target or workspace-scoped private target
2. Compiles and loads it into a deterministic emulator
3. Executes N instruction cycles against it
4. Returns a formally verified `result.json` + clock-accurate `.vcd` waveform

Every run consumes real compute on the Foundry. Pricing reflects managed verification capacity, artifact retention, and queue guarantees rather than raw compute resale.

### Hosted vs. Local Execution

The LabWired Foundry is a managed verification service. Users can:
- **Verify Against the Catalog**: Run firmware against LabWired-maintained, pre-verified board and chip assets.
- **Download & Run Locally**: Export supported assets and run them **for free** on your own machines using the open-source `labwired` CLI when local execution is sufficient.
- **Use Hosted Validation in CI**: Call the Foundry API from CI or agent workflows when you need a trusted, centrally managed, machine-readable execution environment.

Board and chip synthesis remain internal capabilities used to expand the catalog. They are not part of the public pricing model at this stage.

---

## Pricing Tiers

| Tier | Price | Verification Runs / Month | API Keys | Queue / SLA |
| :--- | :---: | :---: | :---: | :---: |
| **Free** | €0 | 100 | 1 | Standard |
| **Pro** | €199/mo | 5,000 | 10 | Priority |
| **Enterprise** | Custom | Contracted | Contracted | Dedicated + SLA |

> **Overage** beyond the monthly quota on `Pro`: **€0.05 / run**.
> `Enterprise` contracts are priced around throughput, retention policy, concurrency, private catalog needs, support response targets, and deployment model.
> All prices are monthly, billed in advance. Annual discount available: 2 months free.

---

## Feature Comparison

| Feature | Free | Pro | Enterprise |
| :--- | :---: | :---: | :---: |
| Public catalog read & download | ✅ | ✅ | ✅ |
| Strict IR (`.json`) download | ✅ | ✅ | ✅ |
| Solid Proof VCD (`.vcd`) download | ✅ | ✅ | ✅ |
| REST API key access | ✅ | ✅ | ✅ |
| Formal Solid Proof assertions | ✅ | ✅ | ✅ |
| Priority verification queue | ❌ | ✅ | ✅ |
| Multiple API keys | ❌ | ✅ | ✅ |
| Usage dashboard & audit logs | ❌ | ✅ | ✅ |
| Private model catalog | ❌ | ❌ | ✅ |
| Longer artifact retention | ❌ | Limited | ✅ |
| On-premise / private VPS deployment | ❌ | ❌ | ✅ |
| SLA (99.9% uptime) + dedicated support | ❌ | ❌ | ✅ |

---

## API Usage Model

### Authentication

Every API call carries a bearer key issued at signup:

```http
Authorization: Bearer lw_sk_live_xxxxxxxxxxxxxxxx
```

Keys are scoped to a workspace and can be rotated or revoked at any time from the developer portal.

---

### Key Endpoints

#### `POST /v1/models/verify`
Run the LabWired Core formal verification pipeline against a supported catalog target.

```json
{
  "peripheral_id": "ADXL345",
  "limits": {
    "max_steps": 2000,
    "wall_time_ms": 10000
  },
  "options": {
    "generate_vcd": true,
    "proof_level": "solid"
  }
}
```

**Response**:
```json
{
  "run_id": "run_abc123",
  "status": "queued",
  "poll_url": "/v1/runs/run_abc123",
  "artifacts": {
    "ir_url": "https://foundry.labwired.dev/v1/runs/run_abc123/artifacts/output.json",
    "vcd_url": "https://foundry.labwired.dev/v1/runs/run_abc123/artifacts/proof.vcd",
    "result_url": "https://foundry.labwired.dev/v1/runs/run_abc123/artifacts/result.json"
  },
  "usage": {
    "runs_used_this_month": 12,
    "runs_remaining": 988
  }
}
```

Artifact files are served through authenticated run-scoped endpoints and retained according to server policy
(default: 14 days).

#### `GET /v1/catalog`
List all pre-verified assets in the public catalog.

#### `GET /v1/catalog/{peripheral_id}`
Get full register map, proof status, and artifact URLs for a specific asset.

#### `GET /v1/usage`
Retrieve current billing period usage and quota remaining.

---

### Agent Integration Example

An AI agent (Claude, Cursor, GitHub Copilot) can call the Foundry as a tool:

```python
import requests

response = requests.post(
    "https://foundry.labwired.dev/v1/models/verify",
    headers={"Authorization": "Bearer lw_sk_live_xxxx"},
    json={
        "peripheral_id": "ADXL345",
        "limits": {"max_steps": 2000},
        "options": {"generate_vcd": True}
    }
)
result = response.json()
# result["status"] == "pass"
# result["artifacts"]["ir_url"] -> drop into your LabWired project
```

---

## Billing Model

Billing is subscription-led, not synthesis-led.

Current state:
- the production checkout URL is not finalized in this repo snapshot
- billing should be treated as beta or manual until the live payment path is verified

For early customers, invoicing can remain manual while pricing and packaging are validated.

---

## MVP Billing Approach

> [!IMPORTANT]
> For the first 10 paying customers, billing is handled **manually via direct invoice**. Self-serve checkout is planned for a later billing iteration. This avoids spending engineering time on billing infrastructure before package validation.

---

## Cost Structure (Internal Reference)

| Cost Item | Monthly |
| :--- | :--- |
| Hetzner VPS (CX21, 2 vCPU / 4 GB RAM) | €7 |
| Catalog onboarding and verification overhead | Variable |
| Domain + TLS | €1 |
| **Total Operating Cost** | **Low fixed cost + variable verification labor** |

**Break-even framing**: fixed infra is not the constraint; trusted catalog coverage and customer support are the actual cost centers.
**Path to €1K MRR**: 5 `Pro` subscribers.
