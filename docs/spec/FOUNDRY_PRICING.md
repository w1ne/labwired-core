# LabWired Foundry — Pricing Model

> **Product**: LabWired Foundry — Agent-Native API for synthesizing and formally verifying hardware digital twins.
> **Primary clients**: AI coding agents, embedded software teams, CI/CD pipelines.
> See also: [Foundry Product Spec](./FOUNDRY_SPEC.md) | [Asset Foundry Technical Spec](../asset_foundry.md)

---

## Core Concept: What is a Simulation Run?

A **simulation run** is a single call to the LabWired Core verification pipeline. It:
1. Accepts a peripheral schema (YAML or raw JSON IR)
2. Compiles and loads it into a deterministic emulator
3. Executes N instruction cycles against it
4. Returns a formally verified `result.json` + clock-accurate `.vcd` waveform

Every run consumes real compute on the Foundry VPS. Pricing reflects this.

### Hosted vs. Local Execution

The LabWired Foundry provides a unique "Synthesis & Proof" service. Users can:
- **Generate & Verify**: Pay for the initial AI synthesis and formal "Solid Proof" run on the Foundry.
- **Download & Run Locally**: Once an asset is generated and verified, you can download the `.json` (Strict IR) and run it **for free** on your own machines using the open-source `labwired` CLI.
- **Ongoing Hosted CI**: Alternatively, you can use the Foundry API for every CI run to guarantee a fresh, formally verified environment without managing your own runners.

---

## Pricing Tiers

| Tier | Price | Runs/Month | Agent Seats | Queue Priority |
| :--- | :---: | :---: | :---: | :---: |
| **Dev (Free)** | €0 | 50 | 1 | Standard |
| **Builder** | €49/mo | 1,000 | 1 | Standard |
| **Team** | €199/mo | 5,000 | 10 | Priority |
| **Enterprise** | Custom | Unlimited | Unlimited | Dedicated |

> **Asset Generation**: Synthesizing a *new* peripheral from a prompt/datasheet counts as **10 simulation runs** (to cover LLM costs + multiple verification passes).
> **Burst usage** beyond your monthly quota: **€0.05 / run**.
> All prices are monthly, billed in advance. Annual discount available: 2 months free.

---

## Feature Comparison

| Feature | Free | Builder | Team | Enterprise |
| :--- | :---: | :---: | :---: | :---: |
| Public catalog read & download | ✅ | ✅ | ✅ | ✅ |
| Strict IR (`.json`) download | ✅ | ✅ | ✅ | ✅ |
| Solid Proof VCD (`.vcd`) download | ✅ | ✅ | ✅ | ✅ |
| REST API key access | ✅ | ✅ | ✅ | ✅ |
| Custom AI synthesis (new peripherals) | ❌ | ✅ | ✅ | ✅ |
| Formal Solid Proof assertions | ❌ | ✅ | ✅ | ✅ |
| Priority simulation queue | ❌ | ❌ | ✅ | ✅ |
| Team API key management | ❌ | ❌ | ✅ | ✅ |
| Usage dashboard & audit logs | ❌ | ❌ | ✅ | ✅ |
| Private model catalog | ❌ | ❌ | ❌ | ✅ |
| On-premise / private VPS deployment | ❌ | ❌ | ❌ | ✅ |
| SLA (99.9% uptime) + dedicated support | ❌ | ❌ | ❌ | ✅ |

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

#### `POST /v1/twins/simulate`
Run the LabWired Core formal verification pipeline against a peripheral schema.

```json
{
  "peripheral_id": "ADXL345",
  "chip_yaml": "<inline YAML or omit if using catalog>",
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
  "status": "pass",
  "assertions_passed": 49,
  "assertions_total": 49,
  "cycles_executed": 2000,
  "artifacts": {
    "ir_url": "https://foundry.labwired.dev/artifacts/run_abc123/adxl345.json",
    "vcd_url": "https://foundry.labwired.dev/artifacts/run_abc123/proof.vcd",
    "result_url": "https://foundry.labwired.dev/artifacts/run_abc123/result.json"
  },
  "usage": {
    "runs_used_this_month": 12,
    "runs_remaining": 988
  }
}
```

#### `GET /v1/catalog`
List all pre-verified peripherals in the public catalog.

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
    "https://foundry.labwired.dev/v1/twins/simulate",
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

## MVP Billing Approach

> [!IMPORTANT]
> For the first 10 paying customers, billing is handled **manually via direct invoice**. Stripe metered billing is planned for **v1.1**. This avoids weeks of integration engineering during the critical early traction phase.

---

## Cost Structure (Internal Reference)

| Cost Item | Monthly |
| :--- | :--- |
| Hetzner VPS (CX21, 2 vCPU / 4 GB RAM) | €7 |
| LLM API (OpenAI / Anthropic for synthesis) | €20–50 |
| Domain + TLS | €1 |
| **Total Operating Cost** | **~€30–60/mo** |

**Break-even**: 2 Builder subscribers (2 × €49 = €98) cover all operating costs.
**Path to €1K MRR**: ~5 Team + 5 Builder subscribers.
