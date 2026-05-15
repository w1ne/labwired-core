import { useEffect, useRef, useState } from 'react';
import type { UseAuthResult } from './useAuth';
import type { UseSessionResult } from './useSession';

export interface AuthModalProps {
  open: boolean;
  onClose: () => void;
  auth: UseAuthResult;
  session?: UseSessionResult;
}

export function AuthModal({ open, onClose, auth, session }: AuthModalProps) {
  const [draft, setDraft] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

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

  const handleSignOutGithub = async () => {
    if (session) await session.signOut();
    onClose();
  };

  const connected = auth.status === 'ok' && auth.workspace !== null;
  const loading = auth.status === 'loading';
  const githubSignedIn = session?.status === 'ok' && session.user !== null;

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
          {connected || githubSignedIn ? 'Connected to LabWired' : 'Sign in to LabWired'}
        </h2>
        <p className="text-fg-tertiary text-sm mb-5">
          {githubSignedIn
            ? `Signed in as ${session?.user?.login}`
            : connected
              ? `Plan: ${auth.workspace?.plan} · Workspace ${auth.workspace?.workspace_id}`
              : 'Sign in with GitHub, or paste an API key from your Pro welcome email.'}
        </p>

        {!connected && !githubSignedIn && session && (
          <>
            <a
              href={session.signInUrl}
              className="w-full h-10 flex items-center justify-center gap-2 rounded-md text-sm font-medium bg-white text-black hover:bg-white/90 no-underline"
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
                <path d="M8 0C3.58 0 0 3.58 0 8a8 8 0 0 0 5.47 7.59c.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
              </svg>
              Sign in with GitHub
            </a>
            <div className="flex items-center gap-3 my-4">
              <div className="flex-1 h-px bg-border" />
              <span className="text-fg-tertiary text-xs uppercase tracking-wider">or</span>
              <div className="flex-1 h-px bg-border" />
            </div>
          </>
        )}

        {!connected && !githubSignedIn && (
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

        {githubSignedIn && session?.user && (
          <>
            <div className="flex items-center gap-3 mb-4">
              <img
                src={session.user.avatar_url}
                alt=""
                aria-hidden
                className="w-12 h-12 rounded-full"
                referrerPolicy="no-referrer"
              />
              <div>
                <div className="text-fg-primary font-medium">{session.user.login}</div>
                <div className="text-fg-tertiary text-xs">
                  Plan: {session.user.plan}
                  {session.user.email ? ` · ${session.user.email}` : ''}
                </div>
              </div>
            </div>
            {!connected && (
              <p className="text-fg-tertiary text-xs mb-4">
                Need higher cycle quotas for CI? Paste a Pro API key after signing out, or
                upgrade and use both side-by-side.
              </p>
            )}
            <div className="mt-2 flex gap-2 justify-end">
              <button
                type="button"
                onClick={handleSignOutGithub}
                className="h-9 px-4 rounded-md text-sm text-red-400 hover:text-red-300 bg-transparent border border-red-400/40 hover:bg-red-400/10"
              >
                Sign out
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
