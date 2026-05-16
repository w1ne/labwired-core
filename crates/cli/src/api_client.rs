// LabWired CLI — LabWired API client for key validation and run metering
//
// Activated when the LABWIRED_API_KEY environment variable is set.
// If the variable is absent the CLI runs in free-tier mode (no HTTP calls).

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::warn;

/// Base URL for the LabWired API. Override with LABWIRED_API_BASE for testing.
fn api_base() -> String {
    std::env::var("LABWIRED_API_BASE")
        .unwrap_or_else(|_| "https://api.labwired.com".to_string())
}

// ── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ValidateKeyResponse {
    pub valid: bool,
    pub workspace_id: Option<String>,
    pub plan: Option<String>,
    pub cycles_used_mtd: Option<u64>,
    pub cycles_quota: Option<u64>,
    pub status: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
struct RunRequest<'a> {
    api_key: &'a str,
    firmware_hash: &'a str,
    cycles: u64,
    duration_ms: u64,
    exit_status: i32,
}

/// Outcome of key validation; controls whether the run should proceed.
#[derive(Debug)]
pub enum ValidateOutcome {
    /// Key is valid. Contains the quota info for the end-of-run report.
    Valid {
        workspace_id: String,
        plan: String,
        cycles_quota: u64,
        cycles_used_mtd: u64,
    },
    /// Key is structurally invalid or not found in the API.
    Invalid,
    /// Workspace exists but the monthly cycle quota is exhausted.
    QuotaExceeded,
    /// HTTP or network error — caller should decide whether to proceed or abort.
    NetworkError(String),
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Validate the API key before starting a simulation run.
///
/// Returns `None` if `LABWIRED_API_KEY` is not set (free-tier mode).
/// Returns `Some(ValidateOutcome)` when a key is present.
pub fn validate_key(api_key: &str) -> ValidateOutcome {
    let url = format!("{}/v1/keys/validate", api_base());

    let body = serde_json::json!({ "api_key": api_key });

    let result = ureq::post(&url)
        .timeout(Duration::from_secs(10))
        .send_json(&body);

    match result {
        Ok(resp) => {
            let status = resp.status();
            match resp.into_json::<ValidateKeyResponse>() {
                Ok(data) => {
                    if status == 200 && data.valid {
                        ValidateOutcome::Valid {
                            workspace_id: data.workspace_id.unwrap_or_default(),
                            plan: data.plan.unwrap_or_else(|| "pro".to_string()),
                            cycles_quota: data.cycles_quota.unwrap_or(0),
                            cycles_used_mtd: data.cycles_used_mtd.unwrap_or(0),
                        }
                    } else if status == 403 {
                        ValidateOutcome::QuotaExceeded
                    } else {
                        ValidateOutcome::Invalid
                    }
                }
                Err(e) => ValidateOutcome::NetworkError(format!("JSON parse error: {}", e)),
            }
        }
        Err(ureq::Error::Status(status, resp)) => {
            if status == 403 {
                // Try to parse for a richer error message
                let body_text = resp.into_string().unwrap_or_default();
                if body_text.contains("quota") {
                    return ValidateOutcome::QuotaExceeded;
                }
            }
            ValidateOutcome::Invalid
        }
        Err(e) => ValidateOutcome::NetworkError(format!("Network error: {}", e)),
    }
}

/// Record a completed simulation run for metering purposes.
///
/// This is best-effort: failures are logged but do not abort the process.
/// Call this *after* the simulation completes regardless of pass/fail.
pub fn record_run(
    api_key: &str,
    firmware_hash: &str,
    cycles: u64,
    duration_ms: u64,
    exit_status: i32,
) {
    let url = format!("{}/v1/runs", api_base());

    let body = RunRequest {
        api_key,
        firmware_hash,
        cycles,
        duration_ms,
        exit_status,
    };

    let result = ureq::post(&url)
        .timeout(Duration::from_secs(10))
        .send_json(&body);

    match result {
        Ok(resp) => {
            let status = resp.status();
            if status == 429 {
                warn!("LabWired API: monthly cycle quota exceeded (run still completed locally)");
            } else if status != 200 {
                warn!("LabWired API: run record returned unexpected status {}", status);
            }
        }
        Err(e) => {
            warn!("LabWired API: failed to record run (best-effort): {}", e);
        }
    }
}
