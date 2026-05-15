import { useEffect, useState } from 'react';
import { UserProfile, useUser } from '@clerk/clerk-react';
import { useClerkAccount } from './useClerkAccount';
import { buildStripeUpgradeUrl } from './stripeUpgrade';

export interface AccountPanelProps {
  open: boolean;
  onClose: () => void;
}

function maskKey(key: string): string {
  if (key.length <= 12) return key;
  return `${key.slice(0, 12)}${'•'.repeat(Math.max(0, key.length - 12 - 4))}${key.slice(-4)}`;
}

export function AccountPanel({ open, onClose }: AccountPanelProps) {
  const { user } = useUser();
  const { account, status, error, rotateKey, refresh } = useClerkAccount(open);
  const [revealed, setRevealed] = useState(false);
  const [copyState, setCopyState] = useState<'idle' | 'copied'>('idle');
  const [rotateState, setRotateState] = useState<'idle' | 'rotating' | 'rotated' | 'error'>('idle');
  const [confirmRotate, setConfirmRotate] = useState(false);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  useEffect(() => {
    if (!open) {
      setRevealed(false);
      setRotateState('idle');
      setConfirmRotate(false);
    }
  }, [open]);

  if (!open) return null;

  const email = user?.primaryEmailAddress?.emailAddress ?? null;
  const upgradeUrl = buildStripeUpgradeUrl({ clerkUserId: user?.id, email });

  const handleCopy = async () => {
    if (!account?.api_key) return;
    try {
      await navigator.clipboard.writeText(account.api_key);
      setCopyState('copied');
      setTimeout(() => setCopyState('idle'), 2000);
    } catch {
      // clipboard may be unavailable (insecure context, permissions); no-op
    }
  };

  const handleRotate = async () => {
    setRotateState('rotating');
    const newKey = await rotateKey();
    if (newKey) {
      setRotateState('rotated');
      setRevealed(true);
      setConfirmRotate(false);
      setTimeout(() => setRotateState('idle'), 3000);
    } else {
      setRotateState('error');
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Account"
      className="fixed inset-0 z-50 flex items-start justify-center p-4 sm:p-8 overflow-auto"
    >
      <button
        type="button"
        aria-label="Close account panel"
        onClick={onClose}
        className="absolute inset-0 bg-black/70 backdrop-blur-sm border-0 outline-none"
      />
      <div className="relative flex flex-col items-stretch gap-3 w-full max-w-[880px]">
        <div className="flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="h-8 px-3 rounded-md text-xs font-medium bg-white/[0.08] text-fg-secondary hover:text-fg-primary hover:bg-white/[0.12] border-0"
          >
            Close
          </button>
        </div>

        {/* LabWired API key card */}
        <section
          aria-label="LabWired API key"
          className="rounded-xl border border-border bg-bg-elevated/60 backdrop-blur p-5"
        >
          <div className="flex items-center justify-between mb-3">
            <h3 className="text-fg-primary text-[15px] font-semibold">Your LabWired API key</h3>
            {account?.plan && (
              <span className="text-[11px] uppercase tracking-[0.1em] text-fg-tertiary font-semibold">
                Plan · {account.plan}
              </span>
            )}
          </div>

          {status === 'loading' && (
            <p className="text-fg-tertiary text-sm">Loading…</p>
          )}

          {status === 'error' && (
            <div className="text-sm text-red-400">
              {error ?? 'Failed to load account.'}
              <button
                type="button"
                onClick={() => void refresh()}
                className="ml-3 text-accent hover:underline"
              >
                Retry
              </button>
            </div>
          )}

          {status === 'ok' && account && !account.api_key && (
            <div>
              <p className="text-fg-secondary text-sm mb-4">
                You're on the Free tier. Upgrade to Pro to get an API key for CI and agent workflows —
                100M cycles/month, private projects, VCD trace retention.
              </p>
              <a
                href={upgradeUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex h-9 px-4 items-center rounded-pill bg-accent text-bg-base text-sm font-semibold hover:bg-accent-hover transition-colors"
              >
                Upgrade to Pro — $19/mo
              </a>
            </div>
          )}

          {status === 'ok' && account?.api_key && (
            <div>
              <p className="text-fg-tertiary text-xs mb-2">
                Use this key as <code className="text-fg-secondary">LABWIRED_API_KEY</code> in CI or as
                a Bearer token against <code className="text-fg-secondary">api.labwired.com</code>.
                Treat it like a password.
              </p>
              <div className="flex items-stretch gap-2 mb-3">
                <code className="flex-1 px-3 h-9 leading-9 rounded-md bg-white/[0.04] border border-border text-accent text-[13px] font-mono truncate">
                  {revealed ? account.api_key : maskKey(account.api_key)}
                </code>
                <button
                  type="button"
                  onClick={() => setRevealed((v) => !v)}
                  className="h-9 px-3 rounded-md text-xs font-medium bg-white/[0.06] text-fg-secondary hover:text-fg-primary hover:bg-white/[0.10] border-0"
                >
                  {revealed ? 'Hide' : 'Show'}
                </button>
                <button
                  type="button"
                  onClick={handleCopy}
                  className="h-9 px-3 rounded-md text-xs font-medium bg-white/[0.06] text-fg-secondary hover:text-fg-primary hover:bg-white/[0.10] border-0"
                >
                  {copyState === 'copied' ? 'Copied' : 'Copy'}
                </button>
              </div>

              {typeof account.cycles_used_mtd === 'number' && typeof account.cycles_quota === 'number' && (
                <div className="text-fg-tertiary text-[12px] mb-4 font-mono">
                  {account.cycles_used_mtd.toLocaleString()} /{' '}
                  {account.cycles_quota.toLocaleString()} cycles this month
                </div>
              )}

              <div className="flex flex-wrap items-center gap-2">
                {confirmRotate ? (
                  <>
                    <span className="text-fg-secondary text-xs">
                      Rotating invalidates the old key everywhere.
                    </span>
                    <button
                      type="button"
                      onClick={handleRotate}
                      disabled={rotateState === 'rotating'}
                      className="h-8 px-3 rounded-md text-xs font-medium bg-red-500/15 text-red-300 hover:bg-red-500/25 border-0 disabled:opacity-50"
                    >
                      {rotateState === 'rotating' ? 'Rotating…' : 'Confirm rotate'}
                    </button>
                    <button
                      type="button"
                      onClick={() => setConfirmRotate(false)}
                      className="h-8 px-3 rounded-md text-xs font-medium bg-transparent text-fg-tertiary hover:text-fg-primary border border-border"
                    >
                      Cancel
                    </button>
                  </>
                ) : (
                  <button
                    type="button"
                    onClick={() => setConfirmRotate(true)}
                    className="h-8 px-3 rounded-md text-xs font-medium bg-white/[0.06] text-fg-secondary hover:text-fg-primary hover:bg-white/[0.10] border-0"
                  >
                    Rotate key
                  </button>
                )}
                {rotateState === 'rotated' && (
                  <span role="status" className="text-ok text-xs">
                    New key issued. The old key no longer works.
                  </span>
                )}
                {rotateState === 'error' && (
                  <span role="alert" className="text-red-400 text-xs">
                    Rotate failed — try again.
                  </span>
                )}
              </div>
            </div>
          )}
        </section>

        <UserProfile routing="hash" />
      </div>
    </div>
  );
}
