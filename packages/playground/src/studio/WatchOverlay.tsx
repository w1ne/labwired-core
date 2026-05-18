/**
 * Watch overlay — renders when the URL has ?watch=<sessionId>. Connects to the
 * Worker's SessionDO via WebSocket and reflects what an agent is doing live.
 *
 * Layout:
 *   ┌─────────────────────────────────────────────────────────────────────┐
 *   │  Banner: 👀 Watching agent session <id>   ● status   Exit watch     │
 *   ├──────────────────────────────────┬──────────────────────────────────┤
 *   │ Agent activity                   │                                  │
 *   │  - board                         │       <iframe>                   │
 *   │  - source (read-only)            │       /?lab=<board>&run=1        │
 *   │  - last sim state                │       (real WASM simulator       │
 *   │  - serial tail                   │        renders here)             │
 *   └──────────────────────────────────┴──────────────────────────────────┘
 *
 * The iframe is the cool part: it loads the regular playground at the board
 * the agent picked + auto-Runs. Because LabWired's sim is deterministic,
 * the watcher's browser produces byte-identical output to what the agent saw.
 */
import { useEffect, useMemo, useState } from 'react';

const API_BASE = (import.meta.env.VITE_LABWIRED_API_BASE as string | undefined) ?? 'https://api.labwired.com';

interface SessionState {
  session_id: string;
  status: 'idle' | 'running' | 'completed' | 'failed';
  board_id?: string;
  source?: string;
  diagram?: unknown;
  last_sim_state?: {
    exit_reason?: string;
    final_cycles?: number;
    final_pc_hex?: string;
    serial_tail?: string;
  };
  last_touched?: number;
}

export function watchSessionIdFromUrl(): string | null {
  if (typeof window === 'undefined') return null;
  const params = new URLSearchParams(window.location.search);
  const id = params.get('watch');
  if (!id || !/^[A-Za-z0-9_-]{4,64}$/.test(id)) return null;
  return id;
}

export function WatchOverlay({ sessionId }: { sessionId: string }) {
  const [state, setState] = useState<SessionState | null>(null);
  const [status, setStatus] = useState<'connecting' | 'live' | 'closed' | 'error'>('connecting');

  useEffect(() => {
    let cancelled = false;
    let ws: WebSocket | null = null;

    const wsUrl =
      API_BASE.replace(/^https?/, (m) => (m === 'https' ? 'wss' : 'ws')) +
      `/v1/sessions/${sessionId}/ws`;

    try {
      ws = new WebSocket(wsUrl);
    } catch {
      setStatus('error');
      return;
    }

    ws.addEventListener('open', () => {
      if (!cancelled) setStatus('live');
    });
    ws.addEventListener('message', (e) => {
      if (cancelled) return;
      try {
        const msg = JSON.parse(e.data);
        if (msg.type === 'snapshot' && msg.full) {
          setState(msg.full as SessionState);
        } else if (msg.type === 'state' && msg.diff) {
          setState((prev) => ({ ...(prev ?? { session_id: sessionId, status: 'idle' }), ...msg.diff }));
        }
      } catch { /* ignore malformed */ }
    });
    ws.addEventListener('close', () => !cancelled && setStatus('closed'));
    ws.addEventListener('error', () => !cancelled && setStatus('error'));

    return () => {
      cancelled = true;
      ws?.close();
    };
  }, [sessionId]);

  const statusDot =
    state?.status === 'running'
      ? { color: '#F062B8', label: 'Running' }
      : state?.status === 'completed'
        ? { color: '#3DD68C', label: 'Completed' }
        : state?.status === 'failed'
          ? { color: '#F2545B', label: 'Failed' }
          : { color: '#9098A8', label: 'Idle' };

  // Iframe src changes when the agent picks a new board — iframe reloads,
  // playground auto-runs the new lab. Keying the iframe by board_id makes
  // React unmount/remount it cleanly.
  const iframeSrc = useMemo(() => {
    if (!state?.board_id) return null;
    return `/?lab=${encodeURIComponent(state.board_id)}&run=1`;
  }, [state?.board_id]);

  return (
    <div className="fixed inset-0 z-[1000] flex flex-col bg-bg-base text-fg-primary">
      {/* Banner */}
      <div className="flex items-center gap-3 px-4 h-11 bg-accent/10 border-b border-accent/40 shrink-0">
        <span aria-hidden className="w-2 h-2 rounded-full bg-accent animate-pulse" />
        <div className="text-[13px] font-medium text-accent">👀 Watching agent session</div>
        <code className="text-[11px] font-mono text-fg-secondary bg-bg-surface/60 px-2 py-0.5 rounded truncate max-w-[28ch]">
          {sessionId}
        </code>
        <span className="text-fg-tertiary text-[12px] hidden lg:inline">
          — sim runs locally (deterministic), agent drives state
        </span>
        <div className="flex-1" />
        <span className="text-[11px] flex items-center gap-1.5">
          <span aria-hidden className="w-1.5 h-1.5 rounded-full" style={{ background: statusDot.color }} />
          <span className="text-fg-secondary">{statusDot.label}</span>
        </span>
        <a
          href="/"
          className="text-[11px] px-2 py-1 rounded-pill text-fg-secondary hover:text-fg-primary hover:bg-white/[0.06]"
        >
          Exit watch
        </a>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-hidden grid grid-cols-1 lg:grid-cols-[320px_1fr] gap-px bg-border">
        {/* Agent activity sidebar */}
        <div className="bg-bg-base overflow-auto p-4 text-[12px]">
          <div className="text-fg-tertiary text-[10px] uppercase tracking-wider mb-1">Board</div>
          <div className="text-fg-primary text-base font-semibold">
            {state?.board_id ?? <span className="text-fg-tertiary font-normal text-[12px]">(waiting for agent)</span>}
          </div>

          {state?.last_sim_state && (
            <div className="mt-5 space-y-1.5">
              <div className="text-fg-tertiary text-[10px] uppercase tracking-wider">
                Last simulation
              </div>
              {state.last_sim_state.exit_reason && (
                <div className="font-mono text-[11px]">
                  <span className="text-fg-tertiary">exit_reason:</span>{' '}
                  <span className="text-fg-primary">{state.last_sim_state.exit_reason}</span>
                </div>
              )}
              {state.last_sim_state.final_cycles !== undefined && (
                <div className="font-mono text-[11px]">
                  <span className="text-fg-tertiary">cycles:</span>{' '}
                  <span className="text-fg-primary">{state.last_sim_state.final_cycles.toLocaleString()}</span>
                </div>
              )}
              {state.last_sim_state.final_pc_hex && (
                <div className="font-mono text-[11px]">
                  <span className="text-fg-tertiary">final_pc:</span>{' '}
                  <span className="text-fg-primary">{state.last_sim_state.final_pc_hex}</span>
                </div>
              )}
            </div>
          )}

          {state?.last_sim_state?.serial_tail && (
            <div className="mt-4">
              <div className="text-fg-tertiary text-[10px] uppercase tracking-wider mb-1">Serial</div>
              <pre className="text-[10.5px] font-mono bg-bg-surface/40 p-2 rounded whitespace-pre-wrap overflow-auto max-h-40">
                {state.last_sim_state.serial_tail}
              </pre>
            </div>
          )}

          {state?.source && (
            <div className="mt-4">
              <div className="text-fg-tertiary text-[10px] uppercase tracking-wider mb-1">Source</div>
              <pre className="text-[10.5px] font-mono bg-bg-surface/40 p-2 rounded whitespace-pre overflow-auto max-h-64">
                {state.source}
              </pre>
            </div>
          )}

          {status !== 'live' && (
            <div className="mt-6 text-[12px] text-fg-tertiary">
              {status === 'connecting' && 'Connecting to session…'}
              {status === 'closed' && 'Watch closed. Refresh to retry.'}
              {status === 'error' && 'Watch connection failed.'}
            </div>
          )}
        </div>

        {/* Live simulation iframe */}
        <div className="bg-bg-canvas relative">
          {iframeSrc ? (
            <iframe
              key={state?.board_id}
              src={iframeSrc}
              title="Agent's lab — live simulation"
              className="absolute inset-0 w-full h-full border-0"
              sandbox="allow-scripts allow-same-origin"
            />
          ) : (
            <div className="absolute inset-0 flex items-center justify-center text-fg-tertiary text-sm">
              Waiting for agent to pick a board…
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
