// Anonymous, cookie-less product usage beacons.
//
// Fire-and-forget POSTs to the API worker's /v1/events endpoint (allowlisted
// event names, no PII, no ids). Respects Do Not Track / Global Privacy
// Control and is a silent no-op when the beacon fails — instrumentation must
// never affect the app.

const API_BASE =
  (import.meta.env.VITE_LABWIRED_API_BASE as string | undefined) ?? 'https://api.labwired.com';

export type UsageEventName = 'app_loaded' | 'board_selected' | 'run_clicked' | 'lab_opened';

function optedOut(): boolean {
  if (typeof navigator === 'undefined') return true;
  const nav = navigator as Navigator & { globalPrivacyControl?: boolean };
  return nav.doNotTrack === '1' || nav.globalPrivacyControl === true;
}

/** Report one usage event. Never throws, never blocks. */
export function trackUsage(event: UsageEventName, props?: { board?: string; tool?: string }): void {
  if (optedOut()) return;
  try {
    void fetch(`${API_BASE}/v1/events`, {
      method: 'POST',
      keepalive: true,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ event, ...props }),
    }).catch(() => {});
  } catch {
    // Tracking must never surface in the UI.
  }
}
