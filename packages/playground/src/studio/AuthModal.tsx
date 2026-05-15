import { useEffect, useRef, useState } from 'react';
import { SignInButton, useUser } from '@clerk/clerk-react';
import type { UseAuthResult } from './useAuth';

export interface AuthModalProps {
  open: boolean;
  onClose: () => void;
  auth: UseAuthResult;
  onOpenAccount?: () => void;
}

export function AuthModal({ open, onClose, auth, onOpenAccount }: AuthModalProps) {
  const [draft, setDraft] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  const { isSignedIn, user } = useUser();

  useEffect(() => {
    if (open) {
      setDraft('');
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    const ok = await auth.save(draft);
    if (ok) onClose();
  };

  const handleDisconnect = () => {
    auth.clear();
    onClose();
  };

  const connected = auth.status === 'ok' && auth.workspace !== null;
  const loading = auth.status === 'loading';

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="LabWired account"
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
    >
      <button
        type="button"
        aria-label="Close"
        onClick={onClose}
        className="absolute inset-0 bg-black/60 backdrop-blur-sm border-0 outline-none"
      />
      <div className="relative w-full max-w-md bg-bg-base border border-border rounded-xl p-6 shadow-2xl">
        <h2 className="text-fg-primary text-lg font-semibold mb-1">
          {connected || isSignedIn ? 'Connected to LabWired' : 'Sign in to LabWired'}
        </h2>
        <p className="text-fg-tertiary text-sm mb-5">
          {isSignedIn
            ? `Signed in as ${user?.primaryEmailAddress?.emailAddress ?? user?.username ?? 'user'}`
            : connected
              ? `Plan: ${auth.workspace?.plan} · Workspace ${auth.workspace?.workspace_id}`
              : 'Sign in with GitHub, Google, or email — or paste an API key from your Pro welcome email.'}
        </p>

        {!connected && !isSignedIn && (
          <>
            <SignInButton mode="modal">
              <button
                type="button"
                className="w-full h-10 flex items-center justify-center gap-2 rounded-md text-sm font-medium bg-accent text-bg-base hover:bg-accent-hover transition-colors border-0"
              >
                Sign in
              </button>
            </SignInButton>
            <div className="flex items-center gap-3 my-4">
              <div className="flex-1 h-px bg-border" />
              <span className="text-fg-tertiary text-xs uppercase tracking-wider">or</span>
              <div className="flex-1 h-px bg-border" />
            </div>
          </>
        )}

        {!connected && !isSignedIn && (
          <form onSubmit={handleSave}>
            <label htmlFor="lw-api-key" className="block text-fg-secondary text-xs mb-1.5">
              API key
            </label>
            <input
              ref={inputRef}
              id="lw-api-key"
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="lwk_live_…"
              className="w-full h-10 px-3 bg-white/[0.04] border border-border rounded-md text-fg-primary text-sm font-mono outline-none focus:border-accent focus:ring-1 focus:ring-accent/40"
              disabled={loading}
            />
            {auth.error && (
              <p className="mt-2 text-red-400 text-xs" role="alert">
                {auth.error}
              </p>
            )}
            <div className="mt-5 flex gap-2 justify-end">
              <button
                type="button"
                onClick={onClose}
                className="h-9 px-4 rounded-md text-sm text-fg-secondary hover:text-fg-primary bg-transparent border border-border hover:bg-white/[0.04]"
                disabled={loading}
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={loading || draft.trim().length === 0}
                className="h-9 px-4 rounded-md text-sm font-medium bg-accent text-bg-base hover:bg-accent/90 disabled:opacity-50"
              >
                {loading ? 'Verifying…' : 'Connect'}
              </button>
            </div>
            <p className="mt-4 text-fg-tertiary text-xs">
              Don't have a key?{' '}
              <a
                href="https://buy.stripe.com/bJeaEW56u3H16Tc3Gz5AQ03"
                target="_blank"
                rel="noopener noreferrer"
                className="text-accent hover:underline"
              >
                Upgrade to Pro →
              </a>
            </p>
          </form>
        )}

        {isSignedIn && !connected && (
          <>
            <p className="text-fg-tertiary text-xs mb-4">
              Need higher cycle quotas for CI? Paste a Pro API key, or upgrade and use both side-by-side.
            </p>
            <div className="mt-2 flex gap-2 justify-end">
              <button
                type="button"
                onClick={() => {
                  onOpenAccount?.();
                  onClose();
                }}
                className="h-9 px-4 rounded-md text-sm text-fg-secondary hover:text-fg-primary bg-transparent border border-border hover:bg-white/[0.04]"
              >
                Manage account
              </button>
              <button
                type="button"
                onClick={onClose}
                className="h-9 px-4 rounded-md text-sm font-medium bg-accent text-bg-base hover:bg-accent/90"
              >
                Done
              </button>
            </div>
          </>
        )}

        {connected && auth.workspace && (
          <>
            <dl className="grid grid-cols-2 gap-3 text-sm">
              <div>
                <dt className="text-fg-tertiary text-xs">Cycles this month</dt>
                <dd className="text-fg-primary font-mono">
                  {auth.workspace.cycles_used_mtd.toLocaleString()} /{' '}
                  {auth.workspace.cycles_quota.toLocaleString()}
                </dd>
              </div>
              <div>
                <dt className="text-fg-tertiary text-xs">Status</dt>
                <dd className="text-fg-primary capitalize">{auth.workspace.status}</dd>
              </div>
            </dl>
            <div className="mt-5 flex gap-2 justify-end">
              <button
                type="button"
                onClick={handleDisconnect}
                className="h-9 px-4 rounded-md text-sm text-red-400 hover:text-red-300 bg-transparent border border-red-400/40 hover:bg-red-400/10"
              >
                Disconnect
              </button>
              <button
                type="button"
                onClick={onClose}
                className="h-9 px-4 rounded-md text-sm font-medium bg-accent text-bg-base hover:bg-accent/90"
              >
                Done
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
