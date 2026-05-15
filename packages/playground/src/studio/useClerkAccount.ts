// Fetches /v1/auth/me using the active Clerk session token. Returns the
// LabWired account snapshot (plan, workspace_id, api_key, quota) for use by
// the cabinet panel.
import { useCallback, useEffect, useState } from 'react';
import { useAuth as useClerkAuth, useUser } from '@clerk/clerk-react';

const API_BASE =
  (import.meta.env.VITE_LABWIRED_API_BASE as string | undefined) ?? 'https://api.labwired.com';

export interface ClerkAccount {
  user_id: string;
  email: string | null;
  plan: 'free' | 'pro' | 'enterprise';
  workspace_id?: string;
  api_key?: string;
  cycles_used_mtd?: number;
  cycles_quota?: number;
  status?: 'active' | 'canceled' | 'payment_failed';
}

export type ClerkAccountStatus = 'idle' | 'loading' | 'ok' | 'error';

export interface UseClerkAccountResult {
  account: ClerkAccount | null;
  status: ClerkAccountStatus;
  error: string | null;
  refresh: () => Promise<void>;
  rotateKey: () => Promise<string | null>;
}

export function useClerkAccount(enabled: boolean): UseClerkAccountResult {
  const { isLoaded, isSignedIn, getToken } = useClerkAuth();
  const { user } = useUser();
  const [account, setAccount] = useState<ClerkAccount | null>(null);
  const [status, setStatus] = useState<ClerkAccountStatus>('idle');
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!isLoaded || !isSignedIn) {
      setAccount(null);
      setStatus('idle');
      return;
    }
    setStatus('loading');
    setError(null);
    try {
      const token = await getToken();
      const resp = await fetch(`${API_BASE}/v1/auth/me`, {
        headers: token ? { Authorization: `Bearer ${token}` } : {},
      });
      if (!resp.ok) {
        const body = (await resp.json().catch(() => ({}))) as { error?: string };
        throw new Error(body.error ?? `Request failed: ${resp.status}`);
      }
      const data = (await resp.json()) as ClerkAccount;
      setAccount(data);
      setStatus('ok');
    } catch (err) {
      setAccount(null);
      setStatus('error');
      setError(err instanceof Error ? err.message : 'Unknown error');
    }
  }, [getToken, isLoaded, isSignedIn]);

  const rotateKey = useCallback(async (): Promise<string | null> => {
    if (!isSignedIn) return null;
    try {
      const token = await getToken();
      const resp = await fetch(`${API_BASE}/v1/keys/rotate`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        body: '{}',
      });
      if (!resp.ok) {
        const body = (await resp.json().catch(() => ({}))) as { error?: string };
        throw new Error(body.error ?? `Rotate failed: ${resp.status}`);
      }
      const data = (await resp.json()) as { api_key: string; workspace_id: string };
      await refresh();
      return data.api_key;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Unknown error');
      return null;
    }
  }, [getToken, isSignedIn, refresh]);

  // Refresh whenever the panel opens, the user identity changes, or load completes.
  useEffect(() => {
    if (enabled) void refresh();
    // user.id included so a session swap re-fetches the account snapshot
  }, [enabled, refresh, user?.id]);

  return { account, status, error, refresh, rotateKey };
}
