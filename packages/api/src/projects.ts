// LabWired API Worker — user-owned projects (Clerk-authenticated)
//
// Stored in KV_PROJECTS, key shape:
//   project:<clerkUserId>:<projectId>  → JSON ProjectRecord
//
// KV's list({prefix}) gives us per-user enumeration without a secondary index.
// All endpoints require a valid Clerk session token in the Authorization
// header (Bearer …) — projects are owned by the authenticated user.

import type { Env } from './types.js';
import { verifyClerkRequest } from './clerk.js';

const PROJECT_ID_LENGTH = 16;
const PROJECT_NAME_MAX = 200;
const DIAGRAM_JSON_MAX = 256 * 1024; // 256 KB — leaves room for richly-wired diagrams
const SOURCE_MAX = 1024 * 1024; // 1 MB

/** Persisted project record. `id` is the project id segment only (not the full key). */
export interface ProjectRecord {
  id: string;
  name: string;
  board_id: string;
  diagram_json: string;
  source_code: string | null;
  created_at: number; // unix ms
  updated_at: number; // unix ms
}

/** What clients see — same fields as ProjectRecord. */
export type ProjectResponse = ProjectRecord;

/** Summary returned by list — no diagram/source body, just metadata. */
export interface ProjectSummary {
  id: string;
  name: string;
  board_id: string;
  created_at: number;
  updated_at: number;
}

const CORS_HEADERS: Record<string, string> = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, PUT, DELETE, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...CORS_HEADERS },
  });
}

function err(message: string, status = 400): Response {
  return json({ error: message }, status);
}

function generateProjectId(): string {
  const bytes = new Uint8Array(PROJECT_ID_LENGTH);
  crypto.getRandomValues(bytes);
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

function kvKey(userId: string, projectId: string): string {
  return `project:${userId}:${projectId}`;
}

function kvPrefix(userId: string): string {
  return `project:${userId}:`;
}

/** Validate input fields shared by POST and PUT. Returns error message or null. */
function validateInput(body: {
  name?: unknown;
  board_id?: unknown;
  diagram_json?: unknown;
  source_code?: unknown;
}): string | null {
  if (typeof body.name !== 'string' || body.name.trim() === '') {
    return 'name is required';
  }
  if (body.name.length > PROJECT_NAME_MAX) {
    return `name exceeds ${PROJECT_NAME_MAX} chars`;
  }
  if (typeof body.board_id !== 'string' || body.board_id.trim() === '') {
    return 'board_id is required';
  }
  if (typeof body.diagram_json !== 'string') {
    return 'diagram_json must be a string';
  }
  if (body.diagram_json.length > DIAGRAM_JSON_MAX) {
    return `diagram_json exceeds ${DIAGRAM_JSON_MAX} bytes`;
  }
  if (body.source_code !== undefined && body.source_code !== null) {
    if (typeof body.source_code !== 'string') {
      return 'source_code must be a string';
    }
    if (body.source_code.length > SOURCE_MAX) {
      return `source_code exceeds ${SOURCE_MAX} bytes`;
    }
  }
  return null;
}

// ── GET /v1/projects ─────────────────────────────────────────────────────────
// List the authenticated user's projects (metadata only, no body).
export async function handleListProjects(request: Request, env: Env): Promise<Response> {
  const auth = await verifyClerkRequest(request, env);
  if (!auth) return err('Unauthorized', 401);

  const list = await env.KV_PROJECTS.list({ prefix: kvPrefix(auth.userId), limit: 1000 });
  const summaries: ProjectSummary[] = [];
  for (const entry of list.keys) {
    const raw = await env.KV_PROJECTS.get(entry.name);
    if (!raw) continue;
    try {
      const rec = JSON.parse(raw) as ProjectRecord;
      summaries.push({
        id: rec.id,
        name: rec.name,
        board_id: rec.board_id,
        created_at: rec.created_at,
        updated_at: rec.updated_at,
      });
    } catch {
      // Skip corrupt entries.
    }
  }

  // Most-recently-updated first — what users expect.
  summaries.sort((a, b) => b.updated_at - a.updated_at);

  return json({ projects: summaries });
}

// ── POST /v1/projects ────────────────────────────────────────────────────────
// Create a new project. Returns the full record (so the client gets the id back).
export async function handleCreateProject(request: Request, env: Env): Promise<Response> {
  const auth = await verifyClerkRequest(request, env);
  if (!auth) return err('Unauthorized', 401);

  let body: Record<string, unknown>;
  try {
    body = (await request.json()) as Record<string, unknown>;
  } catch {
    return err('Invalid JSON body');
  }

  const validation = validateInput(body);
  if (validation) return err(validation);

  const now = Date.now();
  const record: ProjectRecord = {
    id: generateProjectId(),
    name: (body.name as string).trim(),
    board_id: (body.board_id as string).trim(),
    diagram_json: body.diagram_json as string,
    source_code: (body.source_code as string | null | undefined) ?? null,
    created_at: now,
    updated_at: now,
  };

  await env.KV_PROJECTS.put(kvKey(auth.userId, record.id), JSON.stringify(record));
  return json({ project: record }, 201);
}

// ── GET /v1/projects/:id ─────────────────────────────────────────────────────
export async function handleGetProject(
  request: Request,
  env: Env,
  projectId: string,
): Promise<Response> {
  const auth = await verifyClerkRequest(request, env);
  if (!auth) return err('Unauthorized', 401);

  const raw = await env.KV_PROJECTS.get(kvKey(auth.userId, projectId));
  if (!raw) return err('Project not found', 404);

  try {
    const record = JSON.parse(raw) as ProjectRecord;
    return json({ project: record });
  } catch {
    return err('Stored project is corrupt', 500);
  }
}

// ── PUT /v1/projects/:id ─────────────────────────────────────────────────────
export async function handleUpdateProject(
  request: Request,
  env: Env,
  projectId: string,
): Promise<Response> {
  const auth = await verifyClerkRequest(request, env);
  if (!auth) return err('Unauthorized', 401);

  const key = kvKey(auth.userId, projectId);
  const raw = await env.KV_PROJECTS.get(key);
  if (!raw) return err('Project not found', 404);

  let existing: ProjectRecord;
  try {
    existing = JSON.parse(raw) as ProjectRecord;
  } catch {
    return err('Stored project is corrupt', 500);
  }

  let body: Record<string, unknown>;
  try {
    body = (await request.json()) as Record<string, unknown>;
  } catch {
    return err('Invalid JSON body');
  }

  const validation = validateInput(body);
  if (validation) return err(validation);

  const updated: ProjectRecord = {
    ...existing,
    name: (body.name as string).trim(),
    board_id: (body.board_id as string).trim(),
    diagram_json: body.diagram_json as string,
    source_code: (body.source_code as string | null | undefined) ?? null,
    updated_at: Date.now(),
  };

  await env.KV_PROJECTS.put(key, JSON.stringify(updated));
  return json({ project: updated });
}

// ── DELETE /v1/projects/:id ──────────────────────────────────────────────────
export async function handleDeleteProject(
  request: Request,
  env: Env,
  projectId: string,
): Promise<Response> {
  const auth = await verifyClerkRequest(request, env);
  if (!auth) return err('Unauthorized', 401);

  const key = kvKey(auth.userId, projectId);
  // Cheaper to delete unconditionally, but we return 404 for missing IDs so
  // the UI can tell the difference between "deleted" and "never existed".
  const raw = await env.KV_PROJECTS.get(key);
  if (!raw) return err('Project not found', 404);

  await env.KV_PROJECTS.delete(key);
  return json({ ok: true });
}
