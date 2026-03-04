# Foundry User Flows

This document details the core user flows for the LabWired Foundry API. It is designed to be a reference for both human developers and AI agents (like Devin, Cursor, or OpenHands) integrating with the platform.

## API Authentication
All protected endpoints require an API Key passed in the `Authorization` header.

```bash
Authorization: Bearer <your_api_key>
```

---

## Flow A: Public Discovery
**Cost: Free**

Before authenticating, you can discover the engine's capabilities and available assets.

### 1. Engine Information
Get the current version and supported hardware features.

**Request:**
```bash
curl -s http://api.labwired.com/v1/info
```

**Example Response:**
```json
{
  "engine": "Foundry",
  "version": "v0.1.0-mvp",
  "capabilities": ["synthesis", "verification", "system-simulation"],
  "status": "online"
}
```

### 2. Browse the Catalog
List pre-verified peripherals that can be downloaded or used in system integration.

**Request:**
```bash
curl -s http://api.labwired.com/v1/catalog
```

---

## Flow B: Usage & Quota Management
**Cost: Free**

Monitor your credit balance and monthly limits.

**Request:**
```bash
curl -s -H "Authorization: Bearer <key>" http://api.labwired.com/v1/usage
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

Always check the cost before initiating a high-level synthesis job.

**Request:**
```bash
curl -s -X POST -H "Authorization: Bearer <key>" \
     -H "Content-Type: application/json" \
     -d '{
       "component_name": "ADXL345",
       "requirements": "I2C interface required."
     }' http://api.labwired.com/v1/estimate
```

**Example Response:**
```json
{
  "component_name": "ADXL345",
  "estimated_cost_runs": 15,
  "message": "Synthesizing ADXL345 will cost approximately 15 runs."
}
```

---

## Flow D: Autonomous Synthesis
**Cost: Dynamic (based on complexity)**

Request the Foundry to write and formally verify a digital twin for you.

**Request:**
```bash
curl -s -X POST -H "Authorization: Bearer <key>" \
     -H "Content-Type: application/json" \
     -d '{
       "component_name": "ADXL345",
       "requirements": "I2C interface required."
     }' http://api.labwired.com/v1/synthesize
```

**Example Response:**
```json
{
  "job_id": "synth-1772623321917987812",
  "status": "processing",
  "message": "Synthesis job started. The internal engine is drafting and formally verifying the model."
}
```

---

## Flow E: Model Verification (VaaS)
**Cost: 1 Run**

Submit your own hardware descriptions (YAML/JSON) for formal proof and cycle-accurate simulation.

**Request:**
```bash
curl -s -X POST -H "Authorization: Bearer <key>" \
     -H "Content-Type: application/json" \
     -d @chip_spec.yaml http://api.labwired.com/v1/models/verify
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

## Flow F: Polling for Results
**Cost: Free**

Retrieve the status and artifacts of a queued verification or synthesis job.

**Request:**
```bash
curl -s -H "Authorization: Bearer <key>" http://api.labwired.com/v1/runs/run-model-1772624589213
```

**Example Response:**
```json
{
  "run_id": "run-model-1772624589213",
  "status": "pass",
  "assertions_passed": 49,
  "assertions_total": 49,
  "artifacts": {
    "ir_url": "/artifacts/run-model-1772624589213/output.json",
    "vcd_url": "/artifacts/run-model-1772624589213/proof.vcd",
    "result_url": "/artifacts/run-model-1772624589213/result.json"
  }
}
```
