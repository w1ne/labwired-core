import { useCallback, useEffect, useState } from 'react';

const STORAGE_KEY = 'labwired.session';

const API_BASE =
  (import.meta.env.VITE_LABWIRED_API_BASE as string | undefined) ?? 'https://api.labwired.com';

export type SessionStatus = 'idle' | 'loading' | 'ok' | 'error';

export interface SessionUser {
  github_id: number;
  login: string;
  avatar_url: string;
  email: string | null;
  plan: 'free' | 'pro';
}

export interface UseSessionResult {
  token: string | null;
  user: SessionUser | null;
  status: SessionStatus;
  error: string | null;
  signInUrl: string;
  signOut: () => Promise<void>;
  refresh: () => Promise<void>;
}

function readStoredToken(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeStoredToken(token: string | null): void {
  try {
    if (token) localStorage.setItem(STORAGE_KEY, token);
    else localStorage.removeItem(STORAGE_KEY);
  } catch {
    // ignore
  }
}

function consumeFragmentToken(): string | null {
  if (typeof window === 'undefined') return null;
  const hash = window.location.hash;
  if (!hash) return null;
  const match = /(?:^|&)session=([^&]+)/.exec(hash.startsWith('#') ? hash.slice(1) : hash);
  if (!match) return null;
  const token = decodeURIComponent(match[1]);
  try {
    const remaining = (hash.startsWith('#') ? hash.slice(1) : hash)
      .split('&')
      .filter((p) => !p.startsWith('session='))
      .join('&');
    const newHash = remaining ? `#${remaining}` : '';
    const newUrl = window.location.pathname + window.location.search + newHash;
    window.history.replaceState(null, '', newUrl);
  } catch {
    // ignore
  }
  return token;
}

async function fetchSessionUser(token: string): Promise<SessionUser> {
  const resp = await fetch(`${API_BASE}/v1/auth/me`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!resp.ok) {
    const body = (await resp.json().catch(() => ({}))) as { error?: string };
    throw new Error(body.error ?? `Request failed: ${resp.status}`);
  }
  return (await resp.json()) as SessionUser;
}

export function useSession(): UseSessionResult {
  const [token, setToken] = useState<string | null>(() => {
    const fromFragment = consumeFragmentToken();
    if (fromFragment) {
      writeStoredToken(fromFragment);
      return fromFragment;
    }
    return readStoredToken();
  });
  const [user, setUser] = useState<SessionUser | null>(null);
  const [status, setStatus] = useState<SessionStatus>('idle');
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const current = readStoredToken();
    if (!current) {
      setToken(null);
      setUser(null);
      setStatus('idle');
      return;
    }
    setStatus('loading');
    try {
      const u = await fetchSessionUser(current);
      setUser(u);
      setStatus('ok');
      setError(null);
    } catch (err) {
      writeStoredToken(null);
      setToken(null);
      setUser(null);
      setStatus('error');
      setError(err instanceof Error ? err.message : 'Unknown error');
    }
  }, []);

  useEffect(() => {
    if (token) void refresh();
  }, [token, refresh]);

  const signOut = useCallback(async () => {
    const current = readStoredToken();
    if (current) {
      try {
        await fetch(`${API_BASE}/v1/auth/logout`, {
          method: 'POST',
          headers: { Authorization: `Bearer ${current}` },
        });
      } catch {
        // best-effort
      }
    }
    writeStoredToken(null);
    setToken(null);
    setUser(null);
    setStatus('idle');
    setError(null);
  }, []);

  return {
    token,
    user,
    status,
    error,
    signInUrl: `${API_BASE}/v1/auth/github/start`,
    signOut,
    refresh,
  };
}
