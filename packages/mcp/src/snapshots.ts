/**
 * In-memory snapshot store for `labwired_inspect_run`. Lets the agent fetch
 * detailed state slices (registers, memory, peripheral) without re-shipping
 * full state through `labwired_run_lab`'s return.
 *
 * Lifecycle: store entries when a run completes, evict after TTL or when over
 * MAX_ENTRIES (LRU). The MCP server process holds these; they don't survive a
 * restart. Agents that want durable snapshots must capture full state in their
 * own memory.
 */

const TTL_MS = 10 * 60 * 1000; // 10 minutes
const MAX_ENTRIES = 50;

export interface SimSnapshot {
  /** Final register file (ARM Cortex-M general regs + system regs). */
  registers?: Record<string, number>;
  /** UART output captured during run, full transcript (capped 256 KB). */
  serial_output?: string;
  /** GPIO transition events (cycle, pin, from, to). May be truncated. */
  gpio_events?: Array<{ sim_cycle: number; pin: string; from: 0 | 1; to: 0 | 1 }>;
  gpio_truncated?: boolean;
  /** Final program counter. */
  final_pc_hex?: string;
  /** Final cycle count. */
  final_cycles?: number;
  /** Result.json payload from labwired-cli for full fidelity. */
  raw_result?: unknown;
  /** Board id this run was executed against. */
  board_id?: string;
  /** Timestamp for TTL accounting. */
  created_at: number;
}

interface Entry {
  snapshot: SimSnapshot;
  last_access: number;
}

const store = new Map<string, Entry>();

/** ULID-ish opaque id; cryptographic strength not required. */
function newId(): string {
  return (
    'snap_' +
    Math.random().toString(36).slice(2, 10) +
    Math.random().toString(36).slice(2, 10)
  );
}

function evictExpired(): void {
  const now = Date.now();
  for (const [id, e] of store) {
    if (now - e.snapshot.created_at > TTL_MS) store.delete(id);
  }
}

function evictLruIfNeeded(): void {
  if (store.size <= MAX_ENTRIES) return;
  const sorted = [...store.entries()].sort((a, b) => a[1].last_access - b[1].last_access);
  const toEvict = sorted.length - MAX_ENTRIES;
  for (let i = 0; i < toEvict; i++) store.delete(sorted[i][0]);
}

export function putSnapshot(snapshot: SimSnapshot): string {
  evictExpired();
  const id = newId();
  store.set(id, { snapshot, last_access: Date.now() });
  evictLruIfNeeded();
  return id;
}

export function getSnapshot(id: string): SimSnapshot | null {
  evictExpired();
  const e = store.get(id);
  if (!e) return null;
  e.last_access = Date.now();
  return e.snapshot;
}

export function snapshotCount(): number {
  return store.size;
}
