// useProjects — CRUD for user-owned projects via /v1/projects/*.
// Mirrors useClerkAccount's auth pattern: uses Clerk session token.
import { useCallback, useEffect, useState } from 'react';
import { useAuth as useClerkAuth } from '@clerk/clerk-react';

const API_BASE =
  (import.meta.env.VITE_LABWIRED_API_BASE as string | undefined) ?? 'https://api.labwired.com';

/** Metadata returned by GET /v1/projects (no diagram/source body). */
export interface ProjectSummary {
  id: string;
  name: string;
  board_id: string;
  created_at: number;
  updated_at: number;
}

/** Full project record returned by GET /v1/projects/:id. */
export interface ProjectRecord extends ProjectSummary {
  diagram_json: string;
  source_code: string | null;
}

export type ProjectsStatus = 'idle' | 'loading' | 'ok' | 'error';

export interface UseProjectsResult {
  list: ProjectSummary[];
  status: ProjectsStatus;
  error: string | null;
  refresh: () => Promise<void>;
  /** Save a new project. Returns the created record or null on failure. */
  create: (input: { name: string; boardId: string; diagramJson: string; sourceCode: string | null }) => Promise<ProjectRecord | null>;
  /** Update an existing project (PUT). */
  update: (id: string, input: { name: string; boardId: string; diagramJson: string; sourceCode: string | null }) => Promise<ProjectRecord | null>;
  /** Load full project (body included). */
  load: (id: string) => Promise<ProjectRecord | null>;
  /** Delete; refreshes the list on success. */
  remove: (id: string) => Promise<boolean>;
}

export function useProjects(enabled: boolean): UseProjectsResult {
  const { isLoaded, isSignedIn, getToken } = useClerkAuth();
  const [list, setList] = useState<ProjectSummary[]>([]);
  const [status, setStatus] = useState<ProjectsStatus>('idle');
  const [error, setError] = useState<string | null>(null);

  const authedFetch = useCallback(
    async (path: string, init?: RequestInit): Promise<Response> => {
      const token = await getToken();
      return fetch(`${API_BASE}${path}`, {
        ...init,
        headers: {
          ...(init?.body ? { 'Content-Type': 'application/json' } : {}),
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
          ...(init?.headers ?? {}),
        },
      });
    },
    [getToken],
  );

  const refresh = useCallback(async () => {
    if (!isLoaded || !isSignedIn) {
      setList([]);
      setStatus('idle');
      return;
    }
    setStatus('loading');
    setError(null);
    try {
      const resp = await authedFetch('/v1/projects');
      if (!resp.ok) {
        const body = (await resp.json().catch(() => ({}))) as { error?: string };
        throw new Error(body.error ?? `Request failed: ${resp.status}`);
      }
      const data = (await resp.json()) as { projects: ProjectSummary[] };
      setList(data.projects);
      setStatus('ok');
    } catch (err) {
      setStatus('error');
      setError(err instanceof Error ? err.message : 'Unknown error');
    }
  }, [authedFetch, isLoaded, isSignedIn]);

  const create = useCallback<UseProjectsResult['create']>(
    async (input) => {
      try {
        const resp = await authedFetch('/v1/projects', {
          method: 'POST',
          body: JSON.stringify({
            name: input.name,
            board_id: input.boardId,
            diagram_json: input.diagramJson,
            source_code: input.sourceCode,
          }),
        });
        if (!resp.ok) {
          const body = (await resp.json().catch(() => ({}))) as { error?: string };
          throw new Error(body.error ?? `Create failed: ${resp.status}`);
        }
        const data = (await resp.json()) as { project: ProjectRecord };
        await refresh();
        return data.project;
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Unknown error');
        return null;
      }
    },
    [authedFetch, refresh],
  );

  const update = useCallback<UseProjectsResult['update']>(
    async (id, input) => {
      try {
        const resp = await authedFetch(`/v1/projects/${encodeURIComponent(id)}`, {
          method: 'PUT',
          body: JSON.stringify({
            name: input.name,
            board_id: input.boardId,
            diagram_json: input.diagramJson,
            source_code: input.sourceCode,
          }),
        });
        if (!resp.ok) {
          const body = (await resp.json().catch(() => ({}))) as { error?: string };
          throw new Error(body.error ?? `Update failed: ${resp.status}`);
        }
        const data = (await resp.json()) as { project: ProjectRecord };
        await refresh();
        return data.project;
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Unknown error');
        return null;
      }
    },
    [authedFetch, refresh],
  );

  const load = useCallback<UseProjectsResult['load']>(
    async (id) => {
      try {
        const resp = await authedFetch(`/v1/projects/${encodeURIComponent(id)}`);
        if (!resp.ok) {
          const body = (await resp.json().catch(() => ({}))) as { error?: string };
          throw new Error(body.error ?? `Load failed: ${resp.status}`);
        }
        const data = (await resp.json()) as { project: ProjectRecord };
        return data.project;
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Unknown error');
        return null;
      }
    },
    [authedFetch],
  );

  const remove = useCallback<UseProjectsResult['remove']>(
    async (id) => {
      try {
        const resp = await authedFetch(`/v1/projects/${encodeURIComponent(id)}`, {
          method: 'DELETE',
        });
        if (!resp.ok) {
          const body = (await resp.json().catch(() => ({}))) as { error?: string };
          throw new Error(body.error ?? `Delete failed: ${resp.status}`);
        }
        await refresh();
        return true;
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Unknown error');
        return false;
      }
    },
    [authedFetch, refresh],
  );

  // Refresh when enabled (panel opens) or sign-in state changes.
  useEffect(() => {
    if (enabled) void refresh();
  }, [enabled, refresh]);

  return { list, status, error, refresh, create, update, load, remove };
}
