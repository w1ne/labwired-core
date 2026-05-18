/**
 * SessionDO — Durable Object that backs `/v1/sessions/*`.
 *
 * Each session holds a snapshot of what an MCP-driven agent is doing:
 *   { board_id?, diagram?, source?, elf_b64?, last_sim_state?, ... }
 * plus the metadata to garbage-collect stale ones.
 *
 * State is delivered to watchers via WebSocket. The agent writes via HTTP PUT
 * and the DO broadcasts to all connected sockets.
 *
 * TTL: 1h idle (no activity), 24h hard cap. Owner-token gated for writes;
 * read-only watch needs no auth (anyone with sessionId can subscribe).
 *
 * Auth model (hybrid, per design doc):
 *  - Anonymous: createSession returns { sessionId, ownerToken }. Writes require
 *    Authorization: Bearer <ownerToken>.
 *  - Clerk-attached (optional): if createSession was called with a Clerk JWT,
 *    that user_id is stored. Subsequent writes can authenticate either with
 *    the ownerToken OR a Clerk JWT for the same user_id.
 */

const IDLE_TTL_MS = 60 * 60 * 1000; // 1h
const HARD_TTL_MS = 24 * 60 * 60 * 1000; // 24h
const MAX_DIAGRAM_BYTES = 256 * 1024;
const MAX_SOURCE_BYTES = 1024 * 1024;
const MAX_ELF_BYTES = 4 * 1024 * 1024;
const MAX_SIM_STATE_BYTES = 512 * 1024;

interface SessionRecord {
  session_id: string;
  owner_token: string;
  clerk_user_id?: string;
  created_at: number;
  last_touched: number;
  board_id?: string;
  diagram?: unknown;
  source?: string;
  elf_b64?: string;
  last_sim_state?: unknown;
  status: 'idle' | 'running' | 'completed' | 'failed';
}

interface StateUpdate {
  type: 'state';
  session_id: string;
  diff: Partial<SessionRecord>;
  full?: SessionRecord;
}

function randomToken(bytes = 16): string {
  const buf = new Uint8Array(bytes);
  crypto.getRandomValues(buf);
  return Array.from(buf, (b) => b.toString(16).padStart(2, '0')).join('');
}

function jsonByteLength(value: unknown): number {
  if (value === undefined || value === null) return 0;
  if (typeof value === 'string') return new TextEncoder().encode(value).length;
  return new TextEncoder().encode(JSON.stringify(value)).length;
}

export class SessionDO {
  private state: DurableObjectState;
  private record: SessionRecord | null = null;
  private sockets = new Set<WebSocket>();
  // env not currently needed but kept for future Clerk verification inside the DO.

  constructor(state: DurableObjectState, _env: unknown) {
    this.state = state;
    void _env;
    this.state.blockConcurrencyWhile(async () => {
      this.record = (await this.state.storage.get<SessionRecord>('record')) ?? null;
    });
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const pathname = url.pathname;
    const method = request.method.toUpperCase();

    // ── POST /__init  (called by the public route handler when creating a session)
    if (method === 'POST' && pathname.endsWith('/__init')) {
      const body = (await request.json().catch(() => null)) as {
        clerk_user_id?: string;
        session_id: string;
      } | null;
      if (!body?.session_id) return json({ error: 'session_id required' }, 400);
      if (this.record) return json({ error: 'session already initialized' }, 409);
      const owner_token = randomToken();
      this.record = {
        session_id: body.session_id,
        owner_token,
        clerk_user_id: body.clerk_user_id,
        created_at: Date.now(),
        last_touched: Date.now(),
        status: 'idle',
      };
      await this.state.storage.put('record', this.record);
      // Schedule the hard-TTL alarm so the DO self-destructs.
      await this.state.storage.setAlarm(Date.now() + HARD_TTL_MS);
      return json({ owner_token, session_id: body.session_id, created_at: this.record.created_at });
    }

    // Everything below requires an existing record.
    if (!this.record) return json({ error: 'session not found' }, 404);
    if (this.isExpired()) {
      await this.destroy();
      return json({ error: 'session expired' }, 410);
    }

    // ── GET /                              public read of current state
    if (method === 'GET' && (pathname.endsWith('/state') || /\/sessions\/[^/]+$/.test(pathname))) {
      return json(this.publicView());
    }

    // ── GET .../ws                          WebSocket subscribe (watcher)
    if (method === 'GET' && pathname.endsWith('/ws')) {
      if (request.headers.get('Upgrade') !== 'websocket') {
        return new Response('Expected WebSocket upgrade', { status: 426 });
      }
      const pair = new WebSocketPair();
      const [client, server] = Object.values(pair) as [WebSocket, WebSocket];
      this.attachSocket(server);
      return new Response(null, { status: 101, webSocket: client });
    }

    // ── PUT /                              owner write of full or partial state
    if (method === 'PUT' && pathname.endsWith('/state')) {
      const authErr = this.assertOwner(request);
      if (authErr) return authErr;
      const body = (await request.json().catch(() => null)) as Partial<SessionRecord> | null;
      if (!body) return json({ error: 'invalid body' }, 400);
      const sizeErr = this.validateSizes(body);
      if (sizeErr) return json({ error: sizeErr }, 413);
      // Whitelist mutable fields.
      const diff: Partial<SessionRecord> = {};
      const mutable: (keyof SessionRecord)[] = [
        'board_id',
        'diagram',
        'source',
        'elf_b64',
        'last_sim_state',
        'status',
      ];
      for (const k of mutable) {
        if (k in body) (diff as Record<string, unknown>)[k] = body[k] as unknown;
      }
      Object.assign(this.record, diff, { last_touched: Date.now() });
      await this.state.storage.put('record', this.record);
      this.broadcast({ type: 'state', session_id: this.record.session_id, diff });
      return json({ ok: true, last_touched: this.record.last_touched });
    }

    // ── DELETE /                          end session early (owner)
    if (method === 'DELETE' && /\/sessions\/[^/]+$/.test(pathname)) {
      const authErr = this.assertOwner(request);
      if (authErr) return authErr;
      await this.destroy();
      return json({ ok: true });
    }

    return json({ error: 'not found' }, 404);
  }

  private attachSocket(socket: WebSocket): void {
    socket.accept();
    this.sockets.add(socket);
    // Send initial snapshot so the watcher sees current state immediately.
    socket.send(
      JSON.stringify({
        type: 'snapshot',
        session_id: this.record?.session_id,
        full: this.publicView(),
      }),
    );
    socket.addEventListener('close', () => this.sockets.delete(socket));
    socket.addEventListener('error', () => this.sockets.delete(socket));
  }

  private broadcast(update: StateUpdate): void {
    const payload = JSON.stringify(update);
    for (const sock of this.sockets) {
      try {
        sock.send(payload);
      } catch {
        this.sockets.delete(sock);
      }
    }
  }

  private assertOwner(request: Request): Response | null {
    const auth = request.headers.get('Authorization');
    if (!auth) return json({ error: 'Authorization required' }, 401);
    const m = auth.match(/^Bearer\s+(.+)$/i);
    if (!m) return json({ error: 'invalid Authorization header' }, 401);
    const token = m[1];
    if (this.record && token === this.record.owner_token) return null;
    // (Clerk JWT path could be added here — for v0.3 anonymous-only is fine.)
    return json({ error: 'invalid owner_token' }, 403);
  }

  private isExpired(): boolean {
    if (!this.record) return true;
    const now = Date.now();
    if (now - this.record.created_at > HARD_TTL_MS) return true;
    if (now - this.record.last_touched > IDLE_TTL_MS) return true;
    return false;
  }

  private async destroy(): Promise<void> {
    for (const sock of this.sockets) {
      try {
        sock.close(1001, 'session ended');
      } catch { /* ignore */ }
    }
    this.sockets.clear();
    await this.state.storage.deleteAll();
    await this.state.storage.deleteAlarm();
    this.record = null;
  }

  /** Public-view subset — excludes owner_token. */
  private publicView(): Omit<SessionRecord, 'owner_token'> | null {
    if (!this.record) return null;
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    const { owner_token, ...rest } = this.record;
    return rest;
  }

  private validateSizes(body: Partial<SessionRecord>): string | null {
    if (body.diagram !== undefined && jsonByteLength(body.diagram) > MAX_DIAGRAM_BYTES) {
      return `diagram too large (>${MAX_DIAGRAM_BYTES} bytes)`;
    }
    if (body.source !== undefined && jsonByteLength(body.source) > MAX_SOURCE_BYTES) {
      return `source too large (>${MAX_SOURCE_BYTES} bytes)`;
    }
    if (body.elf_b64 !== undefined && jsonByteLength(body.elf_b64) > MAX_ELF_BYTES) {
      return `elf_b64 too large (>${MAX_ELF_BYTES} bytes)`;
    }
    if (
      body.last_sim_state !== undefined &&
      jsonByteLength(body.last_sim_state) > MAX_SIM_STATE_BYTES
    ) {
      return `last_sim_state too large (>${MAX_SIM_STATE_BYTES} bytes)`;
    }
    return null;
  }

  /** Cloudflare alarm fires when hard TTL passes — auto-destroy. */
  async alarm(): Promise<void> {
    if (!this.record) return;
    if (this.isExpired()) await this.destroy();
    else await this.state.storage.setAlarm(Date.now() + IDLE_TTL_MS);
  }
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}
