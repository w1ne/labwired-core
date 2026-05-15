import { useCallback, useEffect, useState } from 'react';

const STORAGE_KEY = 'labwired.apiKey';
const KEY_PREFIX = 'lwk_live_';

const API_BASE =
  (import.meta.env.VITE_LABWIRED_API_BASE as string | undefined) ?? 'https://api.labwired.com';

export type AuthStatus = 'idle' | 'loading' | 'ok' | 'error';

export interface Workspace {
  workspace_id: string;
  plan: 'free' | 'pro' | 'enterprise';
  status: 'active' | 'canceled' | 'payment_failed';
  cycles_used_mtd: number;
  cycles_quota: number;
  period_start_date: string;
  created_at: string;
}

export interface UseAuthResult {
  apiKey: string | null;
  workspace: Workspace | null;
  status: AuthStatus;
  error: string | null;
  save: (key: string) => Promise<boolean>;
  clear: () => void;
  refresh: () => Promise<void>;
}

function readStoredKey(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeStoredKey(key: string | null): void {
  try {
    if (key) localStorage.setItem(STORAGE_KEY, key);
    else localStorage.removeItem(STORAGE_KEY);
  } catch {
    // localStorage may be unavailable (private mode, etc.) — fall back to in-memory only
  }
}

async function fetchWorkspace(key: string): Promise<Workspace> {
  const resp = await fetch(`${API_BASE}/v1/workspaces/me`, {
    headers: { Authorization: `Bearer ${key}` },
  });
  if (!resp.ok) {
    const body = (await resp.json().catch(() => ({}))) as { error?: string };
    throw new Error(body.error ?? `Request failed: ${resp.status}`);
  }
  return (await resp.json()) as Workspace;
}

export function useAuth(): UseAuthResult {
  const [apiKey, setApiKey] = useState<string | null>(() => readStoredKey());
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [status, setStatus] = useState<AuthStatus>('idle');
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const key = readStoredKey();
    if (!key) {
      setApiKey(null);
      setWorkspace(null);
      setStatus('idle');
      return;
    }
    setStatus('loading');
    try {
      const ws = await fetchWorkspace(key);
      setWorkspace(ws);
      setStatus('ok');
      setError(null);
    } catch (err) {
      setWorkspace(null);
      setStatus('error');
      setError(err instanceof Error ? err.message : 'Unknown error');
    }
  }, []);

  useEffect(() => {
    if (apiKey) void refresh();
  }, [apiKey, refresh]);

  const save = useCallback(async (rawKey: string): Promise<boolean> => {
    const key = rawKey.trim();
    if (!key.startsWith(KEY_PREFIX)) {
      setError(`Key must start with "${KEY_PREFIX}"`);
      setStatus('error');
      return false;
    }
    setStatus('loading');
    try {
      const ws = await fetchWorkspace(key);
      writeStoredKey(key);
      setApiKey(key);
      setWorkspace(ws);
      setStatus('ok');
      setError(null);
      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Unknown error');
      setStatus('error');
      return false;
    }
  }, []);

  const clear = useCallback(() => {
    writeStoredKey(null);
    setApiKey(null);
    setWorkspace(null);
    setStatus('idle');
    setError(null);
  }, []);

  return { apiKey, workspace, status, error, save, clear, refresh };
}
