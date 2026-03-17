# Foundry User Flows

This document describes the current LabWired Foundry workflow from the user perspective.

Status:
- Hosted Foundry is still a beta or secondary workflow.
- The primary LabWired launch path remains local deterministic simulation and VS Code debugging.
- Use Foundry when you specifically want hosted verification or catalog access.

Base URL:

```text
https://<your-foundry-host>
```

## Authentication

Protected API endpoints require an API key passed in the `Authorization` header.

```bash
Authorization: Bearer <your_api_key>
```

Notes:
- Dashboard routes may also use Clerk session authentication for account management.
- API-submitted runs, run polling, and artifact download use API key auth.

---

## Flow A: Public Discovery
**Cost: Free**

Before authenticating, you can discover the service status and browse the public catalog.

### 1. Engine Information

**Request:**
```bash
curl -s https://<your-foundry-host>/v1/info
```

**Example Response:**
```json
{
  "engine": "Foundry",
  "version": "v1.x",
  "capabilities": ["synthesis", "verification", "system-simulation"],
  "status": "online"
}
```

### 2. Browse the Catalog

**Request:**
```bash
curl -s https://<your-foundry-host>/v1/catalog
```

---

## Flow B: Usage and Quota
**Cost: Free**

Monitor your current usage and remaining quota.

**Request:**
```bash
curl -s -H "Authorization: Bearer <key>" https://<your-foundry-host>/v1/usage
```

**Example Response:**
```json
{
  "workspace_id": "labwired-team",
  "tier": "builder",
  "runs_used_this_month": 15,
  "quota": 1000,
  "runs_remaining": 985
}
```

---

## Flow C: Pricing Estimation
**Cost: Free**

Check the estimated cost before submitting a synthesis request.

**Request:**
```bash
curl -s -X POST -H "Authorization: Bearer <key>" \
     -H "Content-Type: application/json" \
     -d '{
       "component_name": "ADXL345",
       "requirements": "I2C interface required."
     }' https://<your-foundry-host>/v1/estimate
```

---

## Flow D: Submit a Synthesis Run
**Cost: Dynamic**

Request the Foundry to synthesize and verify a digital twin.

**Request:**
```bash
curl -s -X POST -H "Authorization: Bearer <key>" \
     -H "Content-Type: application/json" \
     -d '{
       "component_name": "ADXL345",
       "requirements": "I2C interface required."
     }' https://<your-foundry-host>/v1/synthesize
```

**Example Response:**
```json
{
  "run_id": "run-synth-1772623321917987812",
  "status": "queued",
  "poll_url": "/v1/runs/run-synth-1772623321917987812"
}
```

---

## Flow E: Submit a Verification Run
**Cost: 1 Run**

Submit your own model description for hosted verification and artifact generation.

**Request:**
```bash
curl -s -X POST -H "Authorization: Bearer <key>" \
     -H "Content-Type: application/json" \
     -d @chip_spec.json https://<your-foundry-host>/v1/models/verify
```

**Example Response:**
```json
{
  "run_id": "run-model-1772624589213",
  "status": "queued",
  "poll_url": "/v1/runs/run-model-1772624589213"
}
```

---

## Flow F: Poll for Results
**Cost: Free**

Retrieve the status and artifacts of a queued verification or synthesis run.

**Request:**
```bash
curl -s -H "Authorization: Bearer <key>" https://<your-foundry-host>/v1/runs/run-model-1772624589213
```

**Example Response:**
```json
{
  "run_id": "run-model-1772624589213",
  "status": "pass",
  "assertions_passed": 49,
  "assertions_total": 49,
  "artifacts": {
    "ir_url": "/v1/runs/run-model-1772624589213/artifacts/output.json",
    "vcd_url": "/v1/runs/run-model-1772624589213/artifacts/proof.vcd",
    "result_url": "/v1/runs/run-model-1772624589213/artifacts/result.json"
  }
}
```

---

## Current User Caveats

- Foundry should not be the only public onboarding path until dashboard auth, API key flow, and hosted submission UX are fully aligned.
- If you only want to evaluate LabWired today, start with the local CLI and VS Code workflow first.
