import { useUser, UserButton } from '@clerk/clerk-react';
import type { UseAuthResult } from './useAuth';

export interface AuthPillProps {
  auth: UseAuthResult;
  onOpen: () => void;
}

function quotaPct(used: number, quota: number): number {
  if (quota <= 0) return 0;
  return Math.min(100, Math.round((used / quota) * 100));
}

export function AuthPill({ auth, onOpen }: AuthPillProps) {
  const { isLoaded, isSignedIn } = useUser();
  const apiKeyConnected = auth.status === 'ok' && auth.workspace !== null;
  const loading = auth.status === 'loading' || !isLoaded;

  if (isSignedIn) {
    return (
      <div className="flex items-center shrink-0">
        <UserButton
          afterSignOutUrl="/"
          appearance={{
            elements: {
              avatarBox: 'w-7 h-7',
            },
          }}
        />
      </div>
    );
  }

  if (apiKeyConnected && auth.workspace) {
    const pct = quotaPct(auth.workspace.cycles_used_mtd, auth.workspace.cycles_quota);
    const planLabel = auth.workspace.plan.charAt(0).toUpperCase() + auth.workspace.plan.slice(1);
    return (
      <button
        type="button"
        onClick={onOpen}
        aria-label={`Account: ${planLabel}, ${pct}% of cycle quota used`}
        title={`${auth.workspace.cycles_used_mtd.toLocaleString()} / ${auth.workspace.cycles_quota.toLocaleString()} cycles`}
        className="h-7 px-3 rounded-pill text-xs font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors duration-micro border-0 outline-none focus-visible:ring-2 focus-visible:ring-accent/50 flex items-center gap-1.5 shrink-0"
      >
        <span className="w-1.5 h-1.5 rounded-full bg-accent" aria-hidden />
        {planLabel} · {pct}%
      </button>
    );
  }

  return (
    <button
      type="button"
      onClick={onOpen}
      aria-label="Connect LabWired account"
      disabled={loading}
      className="h-7 px-3 rounded-pill text-xs font-medium bg-white/[0.05] text-fg-secondary hover:bg-white/[0.10] hover:text-fg-primary transition-colors duration-micro border-0 outline-none focus-visible:ring-2 focus-visible:ring-accent/50 flex items-center gap-1.5 shrink-0 disabled:opacity-50"
    >
      {loading ? 'Connecting…' : 'Connect'}
    </button>
  );
}
