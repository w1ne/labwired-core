// LabWired API Worker — main entry point
// Routes:
//   POST /v1/webhooks/stripe   — Stripe webhook handler
//   POST /v1/keys/validate     — Validate an API key; returns quota info
//   POST /v1/runs              — Record a completed simulation run (meters cycles)
//   GET  /v1/workspaces/me     — Return workspace info for the authenticated key

import Stripe from 'stripe';
import type { Env, WorkspaceRecord, RunRequest } from './types.js';
import {
  generateApiKey,
  generateWorkspaceId,
  writeKeyRecord,
  getKeyRecord,
  updateKeyStatus,
  touchKeyLastUsed,
  writeWorkspaceRecord,
  getWorkspaceRecord,
  writeSubMapping,
  getSubMapping,
  writeClerkMapping,
  getWorkspaceIdByClerkUserId,
  deleteKeyRecord,
  maybeResetMtdCycles,
} from './keys.js';
import { verifyStripeWebhook, getStripeClient } from './stripe.js';
import { verifyClerkRequest } from './clerk.js';
import {
  handleListProjects,
  handleCreateProject,
  handleGetProject,
  handleUpdateProject,
  handleDeleteProject,
} from './projects.js';
import {
  handleCreateSession,
  handleGetSession,
  handleUpdateSession,
  handleDeleteSession,
  handleSessionWebSocket,
} from './sessions.js';
export { SessionDO } from './SessionDO.js';

// ── CORS headers for browser-facing endpoints ──────────────────────────────
const CORS_HEADERS: Record<string, string> = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, PUT, DELETE, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
};

function corsResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...CORS_HEADERS },
  });
}

function errorResponse(message: string, status = 400): Response {
  return corsResponse({ error: message }, status);
}

// ── Main fetch handler ─────────────────────────────────────────────────────
export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const { pathname } = url;
    const method = request.method.toUpperCase();

    // Handle preflight
    if (method === 'OPTIONS') {
      return new Response(null, { status: 204, headers: CORS_HEADERS });
    }

    try {
      if (method === 'POST' && pathname === '/v1/webhooks/stripe') {
        return handleStripeWebhook(request, env);
      }
      if (method === 'POST' && pathname === '/v1/keys/validate') {
        return handleValidateKey(request, env);
      }
      if (method === 'POST' && pathname === '/v1/runs') {
        return handleRecordRun(request, env);
      }
      if (method === 'GET' && pathname === '/v1/workspaces/me') {
        return handleGetWorkspace(request, env);
      }
      if (method === 'GET' && pathname === '/v1/auth/me') {
        return handleAuthMe(request, env);
      }
      if (method === 'POST' && pathname === '/v1/keys/rotate') {
        return handleRotateKey(request, env);
      }

      // ── /v1/projects[/(:id)] — user-owned project storage ────────────────
      if (pathname === '/v1/projects') {
        if (method === 'GET') return handleListProjects(request, env);
        if (method === 'POST') return handleCreateProject(request, env);
      }
      const projectMatch = pathname.match(/^\/v1\/projects\/([A-Za-z0-9_-]+)$/);
      if (projectMatch) {
        const projectId = projectMatch[1];
        if (method === 'GET') return handleGetProject(request, env, projectId);
        if (method === 'PUT') return handleUpdateProject(request, env, projectId);
        if (method === 'DELETE') return handleDeleteProject(request, env, projectId);
      }

      // ── /v1/sessions[/(:id)[/ws]] — live agent-driven session state ──────
      if (pathname === '/v1/sessions') {
        if (method === 'POST') return handleCreateSession(request, env);
      }
      const sessionWsMatch = pathname.match(/^\/v1\/sessions\/([A-Za-z0-9_-]+)\/ws$/);
      if (sessionWsMatch && method === 'GET') {
        return handleSessionWebSocket(request, env, sessionWsMatch[1]);
      }
      const sessionMatch = pathname.match(/^\/v1\/sessions\/([A-Za-z0-9_-]+)$/);
      if (sessionMatch) {
        const sessionId = sessionMatch[1];
        if (method === 'GET') return handleGetSession(request, env, sessionId);
        if (method === 'PUT') return handleUpdateSession(request, env, sessionId);
        if (method === 'DELETE') return handleDeleteSession(request, env, sessionId);
      }

      return errorResponse('Not found', 404);
    } catch (err) {
      console.error('Unhandled error:', err);
      return errorResponse('Internal server error', 500);
    }
  },
};

// ── POST /v1/webhooks/stripe ───────────────────────────────────────────────
async function handleStripeWebhook(request: Request, env: Env): Promise<Response> {
  let event: Stripe.Event;
  try {
    event = await verifyStripeWebhook(request, env);
  } catch (err) {
    console.error('Webhook signature verification failed:', err);
    return new Response(JSON.stringify({ error: 'Invalid signature' }), {
      status: 400,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  console.log(`Stripe event received: ${event.type} id=${event.id}`);

  try {
    switch (event.type) {
      case 'checkout.session.completed':
        await handleCheckoutCompleted(event.data.object as Stripe.Checkout.Session, env);
        break;
      case 'customer.subscription.deleted':
        await handleSubscriptionDeleted(event.data.object as Stripe.Subscription, env);
        break;
      case 'customer.subscription.updated':
        await handleSubscriptionUpdated(event.data.object as Stripe.Subscription, env);
        break;
      case 'invoice.payment_failed':
        await handleInvoicePaymentFailed(event.data.object as Stripe.Invoice, env);
        break;
      default:
        console.log(`Unhandled event type: ${event.type}`);
    }
  } catch (err) {
    console.error(`Error handling event ${event.type}:`, err);
    // Return 200 so Stripe doesn't retry indefinitely for application-level errors
    return new Response(JSON.stringify({ received: true, error: String(err) }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  return new Response(JSON.stringify({ received: true }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

// ── Stripe price IDs ───────────────────────────────────────────────────────
// These map a Stripe price to a LabWired plan. Update when new SKUs are added.
//
// To add a new tier:
//   1. Create the product + recurring price in Stripe Dashboard.
//   2. Copy the price_* ID into the table below.
//   3. Add the plan key to the `Plan` union in ./types.ts.
export const STRIPE_PRICE_PRO = 'price_1TXODOC1n7clsM1CaRfa5EV7';
// TODO(billing): replace with the real Designer price_* after creating the
// "LabWired Designer" $5/seat/mo product in Stripe Dashboard.
export const STRIPE_PRICE_DESIGNER = 'price_REPLACE_DESIGNER';

type PaidPlan = 'designer' | 'pro';

/**
 * Resolve the LabWired plan a checkout session paid for from its line items.
 * Falls back to fetching the session via the Stripe API when line items are
 * not expanded in the webhook payload (the default for `checkout.session.completed`).
 *
 * Default fallback is 'pro' so that an unrecognized price never accidentally
 * downgrades a paying customer to a smaller quota.
 */
async function resolvePaidPlanFromSession(
  session: Stripe.Checkout.Session,
  env: Env,
): Promise<PaidPlan> {
  let priceId: string | null = null;

  const inlineItem = session.line_items?.data?.[0];
  if (inlineItem?.price?.id) {
    priceId = inlineItem.price.id;
  } else if (session.id) {
    try {
      const stripe = getStripeClient(env);
      const items = await stripe.checkout.sessions.listLineItems(session.id, { limit: 1 });
      priceId = items.data[0]?.price?.id ?? null;
    } catch (err) {
      console.error('listLineItems failed; defaulting to pro plan', err);
    }
  }

  if (priceId === STRIPE_PRICE_DESIGNER) return 'designer';
  if (priceId === STRIPE_PRICE_PRO) return 'pro';
  // Unknown price ID — default to 'pro' to avoid accidentally giving paying
  // customers a smaller quota. Log so we notice.
  console.warn(
    `checkout.session.completed: unrecognized price_id "${priceId}" on session ${session.id}; defaulting to pro`,
  );
  return 'pro';
}

async function handleCheckoutCompleted(
  session: Stripe.Checkout.Session,
  env: Env,
): Promise<void> {
  const customerEmail =
    session.customer_details?.email ?? (session.customer_email as string | null) ?? '';
  const stripeCustomerId = typeof session.customer === 'string' ? session.customer : session.customer?.id ?? '';
  const stripeSubId =
    typeof session.subscription === 'string'
      ? session.subscription
      : session.subscription?.id ?? '';
  const clerkUserId =
    typeof session.client_reference_id === 'string' && session.client_reference_id
      ? session.client_reference_id
      : undefined;

  if (!customerEmail) {
    console.error('checkout.session.completed: no customer email in session', session.id);
    return;
  }

  const workspaceId = generateWorkspaceId();
  const apiKey = generateApiKey();
  const proQuota = parseInt(env.PRO_CYCLES_QUOTA || '100000000', 10);
  const designerQuota = parseInt(env.DESIGNER_CYCLES_QUOTA || '10000000', 10);

  const plan = await resolvePaidPlanFromSession(session, env);
  const cyclesQuota = plan === 'designer' ? designerQuota : proQuota;

  const workspace: WorkspaceRecord = {
    stripe_customer_id: stripeCustomerId,
    stripe_subscription_id: stripeSubId,
    customer_email: customerEmail,
    plan,
    cycles_quota_per_month: cyclesQuota,
    cycles_used_mtd: 0,
    period_start_date: new Date(new Date().getFullYear(), new Date().getMonth(), 1).toISOString(),
    status: 'active',
    created_at: new Date().toISOString(),
    api_key: apiKey,
    clerk_user_id: clerkUserId,
  };

  await writeWorkspaceRecord(env, workspaceId, workspace);
  await writeKeyRecord(env, apiKey, workspaceId);
  if (stripeSubId) {
    await writeSubMapping(env, stripeSubId, workspaceId);
  }
  if (clerkUserId) {
    await writeClerkMapping(env, clerkUserId, workspaceId);
  }

  console.log(
    `Workspace created: ${workspaceId} for ${customerEmail} (plan=${plan}, quota=${cyclesQuota})` +
      (clerkUserId ? ` (clerk_user_id=${clerkUserId})` : ' (no clerk_user_id; legacy path)'),
  );
}

async function handleSubscriptionDeleted(
  subscription: Stripe.Subscription,
  env: Env,
): Promise<void> {
  const workspaceId = await getSubMapping(env, subscription.id);
  if (!workspaceId) {
    console.warn(`No workspace found for subscription ${subscription.id}`);
    return;
  }
  const workspace = await getWorkspaceRecord(env, workspaceId);
  if (!workspace) return;

  workspace.status = 'canceled';
  await writeWorkspaceRecord(env, workspaceId, workspace);
  await updateKeyStatus(env, workspace.api_key, 'canceled');
  console.log(`Workspace ${workspaceId} marked canceled`);
}

async function handleSubscriptionUpdated(
  subscription: Stripe.Subscription,
  env: Env,
): Promise<void> {
  const workspaceId = await getSubMapping(env, subscription.id);
  if (!workspaceId) {
    console.warn(`No workspace found for subscription ${subscription.id}`);
    return;
  }
  const workspace = await getWorkspaceRecord(env, workspaceId);
  if (!workspace) return;

  // Sync status: Stripe active/trialing → active, else leave as-is
  if (subscription.status === 'active' || subscription.status === 'trialing') {
    workspace.status = 'active';
    await updateKeyStatus(env, workspace.api_key, 'active');
  } else if (subscription.status === 'canceled') {
    workspace.status = 'canceled';
    await updateKeyStatus(env, workspace.api_key, 'canceled');
  }
  await writeWorkspaceRecord(env, workspaceId, workspace);
  console.log(`Workspace ${workspaceId} status synced to ${workspace.status}`);
}

async function handleInvoicePaymentFailed(invoice: Stripe.Invoice, env: Env): Promise<void> {
  const stripeSubId =
    typeof invoice.subscription === 'string'
      ? invoice.subscription
      : (invoice.subscription as Stripe.Subscription | null)?.id ?? null;
  if (!stripeSubId) return;

  const workspaceId = await getSubMapping(env, stripeSubId);
  if (!workspaceId) return;
  const workspace = await getWorkspaceRecord(env, workspaceId);
  if (!workspace) return;

  workspace.status = 'payment_failed';
  await writeWorkspaceRecord(env, workspaceId, workspace);
  await updateKeyStatus(env, workspace.api_key, 'payment_failed');
  console.log(`Workspace ${workspaceId} marked payment_failed`);
}

// ── POST /v1/keys/validate ─────────────────────────────────────────────────
async function handleValidateKey(request: Request, env: Env): Promise<Response> {
  let body: { api_key?: string };
  try {
    body = (await request.json()) as { api_key?: string };
  } catch {
    return errorResponse('Invalid JSON body');
  }

  const apiKey = body.api_key?.trim();
  if (!apiKey || !apiKey.startsWith('lwk_live_')) {
    return errorResponse('Invalid API key format', 401);
  }

  const keyRecord = await getKeyRecord(env, apiKey);
  if (!keyRecord) {
    return errorResponse('API key not found', 401);
  }
  if (keyRecord.status !== 'active') {
    return errorResponse(`Workspace ${keyRecord.status}`, 403);
  }

  const workspace = await getWorkspaceRecord(env, keyRecord.workspace_id);
  if (!workspace) {
    return errorResponse('Workspace not found', 500);
  }

  const updatedWorkspace = await maybeResetMtdCycles(env, keyRecord.workspace_id, workspace);

  if (updatedWorkspace.cycles_used_mtd >= updatedWorkspace.cycles_quota_per_month) {
    return corsResponse(
      {
        valid: false,
        error: 'Monthly cycle quota exceeded',
        cycles_used_mtd: updatedWorkspace.cycles_used_mtd,
        cycles_quota: updatedWorkspace.cycles_quota_per_month,
      },
      403,
    );
  }

  // best-effort last_used_at update (don't await to keep latency low)
  touchKeyLastUsed(env, apiKey).catch(() => {});

  return corsResponse({
    valid: true,
    workspace_id: keyRecord.workspace_id,
    plan: updatedWorkspace.plan,
    cycles_used_mtd: updatedWorkspace.cycles_used_mtd,
    cycles_quota: updatedWorkspace.cycles_quota_per_month,
    status: updatedWorkspace.status,
  });
}

// ── POST /v1/runs ──────────────────────────────────────────────────────────
async function handleRecordRun(request: Request, env: Env): Promise<Response> {
  let body: RunRequest;
  try {
    body = (await request.json()) as RunRequest;
  } catch {
    return errorResponse('Invalid JSON body');
  }

  const apiKey = body.api_key?.trim();
  if (!apiKey || !apiKey.startsWith('lwk_live_')) {
    return errorResponse('Invalid API key format', 401);
  }

  const keyRecord = await getKeyRecord(env, apiKey);
  if (!keyRecord) {
    return errorResponse('API key not found', 401);
  }
  if (keyRecord.status !== 'active') {
    return errorResponse(`Workspace ${keyRecord.status}`, 403);
  }

  const workspaceId = keyRecord.workspace_id;
  const workspace = await getWorkspaceRecord(env, workspaceId);
  if (!workspace) {
    return errorResponse('Workspace not found', 500);
  }

  const updatedWorkspace = await maybeResetMtdCycles(env, workspaceId, workspace);
  const newCyclesUsed = updatedWorkspace.cycles_used_mtd + (body.cycles || 0);

  if (newCyclesUsed > updatedWorkspace.cycles_quota_per_month) {
    return corsResponse(
      {
        error: 'Monthly cycle quota exceeded',
        cycles_used_mtd: updatedWorkspace.cycles_used_mtd,
        cycles_quota: updatedWorkspace.cycles_quota_per_month,
      },
      429,
    );
  }

  updatedWorkspace.cycles_used_mtd = newCyclesUsed;
  await writeWorkspaceRecord(env, workspaceId, updatedWorkspace);

  console.log(
    `Run recorded: workspace=${workspaceId} firmware=${body.firmware_hash} ` +
      `cycles=${body.cycles} duration=${body.duration_ms}ms exit=${body.exit_status} ` +
      `mtd=${newCyclesUsed}/${updatedWorkspace.cycles_quota_per_month}`,
  );

  return corsResponse({
    ok: true,
    cycles_used_mtd: newCyclesUsed,
    cycles_quota: updatedWorkspace.cycles_quota_per_month,
  });
}

// ── GET /v1/workspaces/me ──────────────────────────────────────────────────
async function handleGetWorkspace(request: Request, env: Env): Promise<Response> {
  // Accept key via Authorization: Bearer <key> or ?api_key=<key>
  const url = new URL(request.url);
  const authHeader = request.headers.get('Authorization') ?? '';
  let apiKey = '';

  if (authHeader.startsWith('Bearer ')) {
    apiKey = authHeader.slice(7).trim();
  } else {
    apiKey = url.searchParams.get('api_key') ?? '';
  }

  if (!apiKey || !apiKey.startsWith('lwk_live_')) {
    return errorResponse('Missing or invalid API key', 401);
  }

  const keyRecord = await getKeyRecord(env, apiKey);
  if (!keyRecord) {
    return errorResponse('API key not found', 401);
  }

  const workspace = await getWorkspaceRecord(env, keyRecord.workspace_id);
  if (!workspace) {
    return errorResponse('Workspace not found', 500);
  }

  const updatedWorkspace = await maybeResetMtdCycles(env, keyRecord.workspace_id, workspace);

  // Don't expose the API key or Stripe IDs in this response
  return corsResponse({
    workspace_id: keyRecord.workspace_id,
    plan: updatedWorkspace.plan,
    status: updatedWorkspace.status,
    cycles_used_mtd: updatedWorkspace.cycles_used_mtd,
    cycles_quota: updatedWorkspace.cycles_quota_per_month,
    period_start_date: updatedWorkspace.period_start_date,
    created_at: updatedWorkspace.created_at,
  });
}

// ── GET /v1/auth/me ────────────────────────────────────────────────────────
// Verifies the request's Clerk session JWT (networkless via CLERK_JWT_KEY) and
// returns the user's id, email, plan, and — if the Clerk user is mapped to a
// paid workspace — the workspace's API key + quota info. This is what powers
// the playground cabinet's "Your LabWired API key" panel.
async function handleAuthMe(request: Request, env: Env): Promise<Response> {
  const verified = await verifyClerkRequest(request, env);
  if (!verified) return errorResponse('Not authenticated', 401);

  const claims = verified.claims;
  const email =
    (typeof claims.email === 'string' && claims.email) ||
    (typeof claims.primary_email_address === 'string' && claims.primary_email_address) ||
    null;

  const workspaceId = await getWorkspaceIdByClerkUserId(env, verified.userId);
  if (workspaceId) {
    const workspace = await getWorkspaceRecord(env, workspaceId);
    if (workspace && workspace.status === 'active') {
      const updated = await maybeResetMtdCycles(env, workspaceId, workspace);
      return corsResponse({
        user_id: verified.userId,
        session_id: verified.sessionId,
        email,
        plan: updated.plan,
        workspace_id: workspaceId,
        api_key: updated.api_key,
        cycles_used_mtd: updated.cycles_used_mtd,
        cycles_quota: updated.cycles_quota_per_month,
        status: updated.status,
      });
    }
  }

  return corsResponse({
    user_id: verified.userId,
    session_id: verified.sessionId,
    email,
    plan: 'free' as const,
  });
}

// ── POST /v1/keys/rotate ───────────────────────────────────────────────────
// Authenticated via Clerk JWT. Generates a new lwk_live_* for the caller's
// workspace, swaps it into KV_KEYS + KV_WORKSPACES, and deletes the old key.
async function handleRotateKey(request: Request, env: Env): Promise<Response> {
  const verified = await verifyClerkRequest(request, env);
  if (!verified) return errorResponse('Not authenticated', 401);

  const workspaceId = await getWorkspaceIdByClerkUserId(env, verified.userId);
  if (!workspaceId) return errorResponse('No workspace for this user', 404);

  const workspace = await getWorkspaceRecord(env, workspaceId);
  if (!workspace) return errorResponse('Workspace not found', 500);
  if (workspace.status !== 'active') {
    return errorResponse(`Workspace ${workspace.status}`, 403);
  }

  const oldKey = workspace.api_key;
  const newKey = generateApiKey();

  await writeKeyRecord(env, newKey, workspaceId);
  workspace.api_key = newKey;
  await writeWorkspaceRecord(env, workspaceId, workspace);
  if (oldKey && oldKey !== newKey) {
    await deleteKeyRecord(env, oldKey);
  }

  console.log(`Key rotated for workspace ${workspaceId} (clerk_user_id=${verified.userId})`);

  return corsResponse({
    api_key: newKey,
    workspace_id: workspaceId,
  });
}
