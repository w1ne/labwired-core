// LabWired API Worker — KV record types and environment bindings

export type WorkspaceStatus = 'active' | 'canceled' | 'payment_failed';
export type KeyStatus = 'active' | 'canceled' | 'payment_failed';
export type Plan = 'free' | 'pro' | 'enterprise';

/** Stored in KV_KEYS under key = the raw API key string (e.g. "lwk_live_...") */
export interface KeyRecord {
  workspace_id: string;
  status: KeyStatus;
  created_at: string; // ISO 8601
  last_used_at: string | null;
}

/** Stored in KV_WORKSPACES under key = workspace_id (e.g. "ws_...") */
export interface WorkspaceRecord {
  stripe_customer_id: string;
  stripe_subscription_id: string;
  customer_email: string;
  plan: Plan;
  cycles_quota_per_month: number;
  cycles_used_mtd: number;
  period_start_date: string; // ISO 8601 date, reset monthly
  status: WorkspaceStatus;
  created_at: string; // ISO 8601
  /** Most recently issued API key. Stored here for /workspaces/me convenience. */
  api_key: string;
}

/** Stored in KV_STRIPE_SUBS under key = Stripe subscription ID ("sub_...") */
export type StripeSubRecord = string; // just the workspace_id

/** Stored in KV_SESSIONS under key = opaque session token (random hex). */
export interface SessionRecord {
  github_id: number;
  login: string;
  avatar_url: string;
  email: string | null;
  created_at: string;
}

/** Cloudflare Worker environment bindings */
export interface Env {
  // KV namespaces
  KV_KEYS: KVNamespace;
  KV_WORKSPACES: KVNamespace;
  KV_STRIPE_SUBS: KVNamespace;
  KV_SESSIONS: KVNamespace;

  // Secrets (set via `wrangler secret put`)
  STRIPE_SECRET_KEY: string;
  STRIPE_WEBHOOK_SECRET: string;
  RESEND_API_KEY: string;
  GITHUB_CLIENT_SECRET: string;

  // Env vars (from wrangler.toml [vars])
  FROM_EMAIL: string;
  PRO_CYCLES_QUOTA: string;
  ENVIRONMENT: string;
  GITHUB_CLIENT_ID: string;
  PLAYGROUND_ORIGIN: string;
}

/** Response body for POST /v1/keys/validate */
export interface ValidateKeyResponse {
  valid: true;
  workspace_id: string;
  plan: Plan;
  cycles_used_mtd: number;
  cycles_quota: number;
  status: WorkspaceStatus;
}

/** Request body for POST /v1/runs */
export interface RunRequest {
  api_key: string;
  firmware_hash: string;
  cycles: number;
  duration_ms: number;
  exit_status: number;
}
