// LabWired API Worker — API key generation and KV access

import type { Env, KeyRecord, WorkspaceRecord } from './types.js';

const KEY_PREFIX = 'lwk_live_';
// 32 characters of base32 (uppercase, no padding): 32 * 5 = 160 bits of entropy
const BASE32_CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
const KEY_BODY_LENGTH = 32;

/** Generate a cryptographically random API key. */
export function generateApiKey(): string {
  const bytes = new Uint8Array(KEY_BODY_LENGTH);
  crypto.getRandomValues(bytes);
  let body = '';
  for (let i = 0; i < KEY_BODY_LENGTH; i++) {
    body += BASE32_CHARS[bytes[i] % 32];
  }
  return KEY_PREFIX + body;
}

/** Generate a workspace ID like "ws_<16 hex chars>". */
export function generateWorkspaceId(): string {
  const bytes = new Uint8Array(8);
  crypto.getRandomValues(bytes);
  const hex = Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
  return 'ws_' + hex;
}

/** Write a new API key record to KV. */
export async function writeKeyRecord(
  env: Env,
  apiKey: string,
  workspaceId: string,
): Promise<void> {
  const record: KeyRecord = {
    workspace_id: workspaceId,
    status: 'active',
    created_at: new Date().toISOString(),
    last_used_at: null,
  };
  await env.KV_KEYS.put(apiKey, JSON.stringify(record));
}

/** Read and parse a key record. Returns null if not found or JSON parse error. */
export async function getKeyRecord(env: Env, apiKey: string): Promise<KeyRecord | null> {
  try {
    const raw = await env.KV_KEYS.get(apiKey);
    if (raw === null) return null;
    return JSON.parse(raw) as KeyRecord;
  } catch {
    return null;
  }
}

/** Update key status (e.g. on subscription cancel / payment failure). */
export async function updateKeyStatus(
  env: Env,
  apiKey: string,
  status: KeyRecord['status'],
): Promise<void> {
  const record = await getKeyRecord(env, apiKey);
  if (!record) return;
  record.status = status;
  await env.KV_KEYS.put(apiKey, JSON.stringify(record));
}

/** Touch last_used_at timestamp on a key record (best-effort, no error on fail). */
export async function touchKeyLastUsed(env: Env, apiKey: string): Promise<void> {
  try {
    const record = await getKeyRecord(env, apiKey);
    if (!record) return;
    record.last_used_at = new Date().toISOString();
    await env.KV_KEYS.put(apiKey, JSON.stringify(record));
  } catch {
    // best-effort
  }
}

/** Write a workspace record to KV. */
export async function writeWorkspaceRecord(
  env: Env,
  workspaceId: string,
  record: WorkspaceRecord,
): Promise<void> {
  await env.KV_WORKSPACES.put(workspaceId, JSON.stringify(record));
}

/** Read and parse a workspace record. Returns null if missing or parse error. */
export async function getWorkspaceRecord(
  env: Env,
  workspaceId: string,
): Promise<WorkspaceRecord | null> {
  try {
    const raw = await env.KV_WORKSPACES.get(workspaceId);
    if (raw === null) return null;
    return JSON.parse(raw) as WorkspaceRecord;
  } catch {
    return null;
  }
}

/** Write a Stripe subscription → workspace_id mapping. */
export async function writeSubMapping(
  env: Env,
  stripeSubId: string,
  workspaceId: string,
): Promise<void> {
  await env.KV_STRIPE_SUBS.put(stripeSubId, workspaceId);
}

/** Read a Stripe subscription → workspace_id mapping. */
export async function getSubMapping(env: Env, stripeSubId: string): Promise<string | null> {
  return env.KV_STRIPE_SUBS.get(stripeSubId);
}

/** Write a Clerk user_id → workspace_id mapping. */
export async function writeClerkMapping(
  env: Env,
  clerkUserId: string,
  workspaceId: string,
): Promise<void> {
  await env.KV_CLERK_TO_WORKSPACE.put(clerkUserId, workspaceId);
}

/** Read the workspace_id mapped to a Clerk user. */
export async function getWorkspaceIdByClerkUserId(
  env: Env,
  clerkUserId: string,
): Promise<string | null> {
  return env.KV_CLERK_TO_WORKSPACE.get(clerkUserId);
}

/** Delete a key record (used when rotating). */
export async function deleteKeyRecord(env: Env, apiKey: string): Promise<void> {
  await env.KV_KEYS.delete(apiKey);
}

/**
 * Check if the billing period has rolled over and reset cycles_used_mtd if so.
 * Returns the (potentially updated) workspace record.
 */
export async function maybeResetMtdCycles(
  env: Env,
  workspaceId: string,
  workspace: WorkspaceRecord,
): Promise<WorkspaceRecord> {
  const now = new Date();
  const periodStart = new Date(workspace.period_start_date);
  // Roll over if we're in a new calendar month
  if (
    now.getFullYear() !== periodStart.getFullYear() ||
    now.getMonth() !== periodStart.getMonth()
  ) {
    workspace.cycles_used_mtd = 0;
    workspace.period_start_date = new Date(now.getFullYear(), now.getMonth(), 1).toISOString();
    await writeWorkspaceRecord(env, workspaceId, workspace);
  }
  return workspace;
}
