// LabWired API Worker — KV record types and environment bindings

export type WorkspaceStatus = 'active' | 'canceled' | 'payment_failed';
export type KeyStatus = 'active' | 'canceled' | 'payment_failed';
export type Plan = 'free' | 'designer' | 'pro' | 'enterprise';

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
  /** Clerk user id captured from Stripe checkout's client_reference_id, if present. */
  clerk_user_id?: string;
}

/** Stored in KV_STRIPE_SUBS under key = Stripe subscription ID ("sub_...") */
export type StripeSubRecord = string; // just the workspace_id

/** Cloudflare Worker environment bindings */
export interface Env {
  // KV namespaces
  KV_KEYS: KVNamespace;
  KV_WORKSPACES: KVNamespace;
  KV_STRIPE_SUBS: KVNamespace;
  /** clerk_user_id → workspace_id reverse index. */
  KV_CLERK_TO_WORKSPACE: KVNamespace;
  /** User-owned project storage. Keyed `project:<clerkUserId>:<projectId>`. */
  KV_PROJECTS: KVNamespace;

  /** Live agent-driven sessions for the playground watch mode (v0.3 MCP bridge). */
  SESSIONS: DurableObjectNamespace;

  // Secrets (set via `wrangler secret put`)
  STRIPE_SECRET_KEY: string;
  STRIPE_WEBHOOK_SECRET: string;
  CLERK_SECRET_KEY: string;

  // Env vars (from wrangler.toml [vars])
  PRO_CYCLES_QUOTA: string;
  /** Monthly cycle quota for the Designer ($5/mo) tier. */
  DESIGNER_CYCLES_QUOTA: string;
  ENVIRONMENT: string;
  /** Clerk JWT verification key (PEM). Public; safe to commit. */
  CLERK_JWT_KEY: string;
  /** Clerk publishable key. Required by @clerk/backend v2 to resolve the Frontend API. */
  CLERK_PUBLISHABLE_KEY: string;
  /** OAuth 2.0 authorization server issuer advertised in the MCP protected-resource
   *  metadata (RFC 9728). Must be set (= the Clerk Frontend API origin, e.g.
   *  https://clerk.labwired.com) or agents can't discover where to log in. */
  MCP_AUTHORIZATION_SERVER?: string;
  /** Base URL for the labwired-builder service (e.g. https://builder.labwired.com). */
  BUILDER_URL: string;
  /** Shared secret for authenticating requests to the builder service. */
  BUILDER_SECRET: string;
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
