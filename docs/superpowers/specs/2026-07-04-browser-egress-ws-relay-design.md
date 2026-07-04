# Browser Egress via WebSocket Relay — Design / Scope

**Status:** SCOPE (not yet planned/implemented). Origin: post-roast tightening of
the egress bridge — the browser is LabWired's primary product surface, and the
native egress path can't run there. See `2026-07-03-egress-bridge-design.md`.

**Goal:** Let a simulated device running in the *in-browser* playground (WASM)
stream its output to a real backend, since WASM has no raw sockets. Ship the
lowest-risk, highest-payoff slice first: a **fixed-target hosted relay** that
powers a live "simulated device → dashboard" demo on the homepage.

---

## Why a relay at all

WASM in the browser cannot open TCP/MQTT/HTTP sockets and has no worker thread
doing blocking I/O. The native architecture (sim enqueues → worker thread →
socket) therefore stops at the sandbox boundary. Something outside the browser
must own the real socket. The browser reaches it over the one transport WASM
*does* have: a WebSocket.

## Architecture

```
WASM sim → EgressBus (enqueue) → WsRelayTransport → [JS WebSocket] → relay → backend
          (deterministic core)   (payload → JS)      (browser)       (real socket)
```

The determinism boundary is unchanged: the sim core still only *enqueues*, and
the bounded drop-oldest buffer still absorbs a slow/dead link. Only the last hop
is new. `EgressBus`, `EgressTap`, `encoding`, and `BufferPolicy` are reused
verbatim — the transport seam already isolates exactly this.

### Component 1 — WASM side: **already done**
The browser build already captures every UART TX byte into a shared
`uart_sink: Arc<Mutex<Vec<u8>>>` and exposes `drain_uart_output() -> Vec<u8>`
(`crates/wasm/src/traces.rs`), which the playground already polls. So browser
egress needs **no new WASM code and no `EgressBus`/worker/`EgressTransport` in
WASM at all** — the determinism boundary is trivially preserved (the sim writes
an in-memory Vec; JS drains it and owns the WebSocket). Component 1 collapses to
"JS forwards the bytes it already drains." (CAN/other-source egress from the
browser is a later concern; MVP is UART, matching the native bridge's manifest
wiring today.)

### Component 2 — JS glue (playground)
- Opens one WebSocket to the relay URL (compile-time/config constant for the
  hosted demo).
- Sends a **hello frame** (JSON) first: `{v:1, transport, url, topic, encoding}`.
- Then streams **payload frames** (binary WS messages) from `egress_drain()`.
- Surfaces relay errors/drops in the playground UI (reuses the existing embed
  status affordance).

### Component 3 — relay server (the only genuinely new service)
- Small async server (Rust/`tokio` + `tokio-tungstenite`, or reuse whatever the
  egress bridge / app backend already runs — TBD in planning).
- Per connection: read hello → **validate against a fixed allowlist** → open the
  real backend socket (reusing the *native* `TcpSink`/`MqttPublisher`/
  `HttpPoster` transports directly — same code, server-side) → forward each
  payload frame.
- Backpressure: bounded per-connection queue, drop-oldest, same semantics as the
  in-sim bus; report drop count back over the WS for UI display.
- Lifecycle: close backend socket on WS close; idle timeout; per-IP connection
  cap.

## MVP scope — Fixed demo target (hosted)

The relay forwards **only** to a LabWired-controlled demo broker/endpoint. The
`url`/`topic` in the hello are validated against a hardcoded allowlist; anything
else is rejected. This removes the entire SSRF / open-proxy risk class (see
Non-goals) and is the exact shape the homepage demo needs.

Deliverable: a homepage/embed demo where a real firmware image boots in the
browser and its UART output appears on a live chart, end to end, no hardware.

**In scope:**
- `WsRelayTransport` + WASM boundary drain API.
- JS WebSocket glue in the playground/embed.
- Relay server with a **fixed allowlist** (demo target only), reusing native
  transports server-side.
- One live demo lab wired end to end (real firmware → chart).
- Wire protocol: `{v, transport, url, topic, encoding}` hello + binary payload
  frames; versioned (`v:1`).

**Explicitly deferred (Phase 2+):**
- **Arbitrary user backend / self-hosted relay** (`labwired-egress-relay
  --allow …`) — the real dev use case, but carries the SSRF liability and needs
  auth; separate spec.
- MQTT auth/TLS/reconnect (inherited gap from the native bridge).
- Ingress / bidirectional (RX injection from the backend).
- Manifest CAN wiring for browser egress.

## Non-goals / risks

- **Open-proxy / SSRF (the load-bearing risk):** a hosted relay that forwards
  arbitrary user bytes to an arbitrary `host:port` is an open proxy into
  internal networks. The fixed-allowlist MVP sidesteps this entirely; the
  Phase-2 arbitrary-target path MUST NOT ship hosted without egress allowlisting
  + auth + rate limits (or ship self-host-only so the user owns the blast
  radius).
- **Cost/abuse:** even fixed-target, a public WS endpoint needs per-IP caps and
  idle timeouts. In MVP scope.
- **Determinism:** unchanged and preserved — the WS hop is off the deterministic
  path exactly like the native socket. The egress *stream* remains non-replayable
  (already stated honestly in the tightened blog copy).

## Open questions for planning
1. Relay hosting: standalone service vs. a route on the existing app backend?
2. WASM boundary: wasm-bindgen exports vs. the playground's current JS↔WASM
   bridge — match whatever `app.labwired.com` already uses.
3. Demo target: dedicated mosquitto + a tiny chart page, or an existing
   dashboard we already run?
