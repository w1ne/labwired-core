# Browser Egress WS Relay — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a fixed-target WebSocket relay so a firmware image running in the in-browser playground can stream its UART output to a live demo backend, powering a "simulated device → dashboard" homepage demo.

**Architecture:** New sync (thread-per-connection) `tungstenite` relay bin crate in this workspace. Each connection: read a JSON *hello*, validate `transport`+`url`+`topic` against a **fixed allowlist**, build the existing native egress transport (`TcpSink`/`MqttPublisher`/`HttpPoster`), then forward each binary WS frame to it. The browser side needs no new WASM code — it already exposes `drain_uart_output() -> Vec<u8>` (`crates/wasm/src/traces.rs`); JS forwards those bytes over the WS.

**Tech Stack:** Rust, `tungstenite` (sync WebSocket), `serde`/`serde_json`, `anyhow`, reused `labwired-core` egress transports. No tokio (matches the existing std-thread egress worker style).

## Global Constraints

- No `Claude`/`AI`/`assistant` references in commit messages or PR bodies; no `Co-Authored-By` AI footer.
- Commit identity: `user.name = w1ne`, email `14119286+w1ne@users.noreply.github.com` (already set in the worktree).
- Reuse the native transports verbatim — do **not** reimplement TCP/MQTT/HTTP.
- MVP forwards to a **fixed allowlist only**; arbitrary user targets are out of scope (SSRF). Never widen the allowlist to accept caller-supplied hosts in this plan.
- Net-touching tests gate behind the existing `net-tests` cargo feature pattern.

---

### Task 1: Scaffold `crates/egress-relay` + hello parsing & allowlist

**Files:**
- Create: `crates/egress-relay/Cargo.toml`
- Create: `crates/egress-relay/src/hello.rs`
- Create: `crates/egress-relay/src/lib.rs`
- Modify: `Cargo.toml` (workspace `members`, add `"crates/egress-relay"`)

**Interfaces:**
- Produces: `Hello { transport: String, url: String, topic: Option<String>, encoding: String }` (serde `Deserialize`); `Allowlist { entries: Vec<AllowEntry> }` with `fn permits(&self, h: &Hello) -> bool`; `fn parse_hello(text: &str) -> anyhow::Result<Hello>`.

- [ ] **Step 1: Write the failing test**

```rust
// crates/egress-relay/src/hello.rs  (bottom)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_valid_mqtt_hello() {
        let h = parse_hello(r#"{"v":1,"transport":"mqtt","url":"mqtt://demo.internal:1883","topic":"demo/temp","encoding":"raw"}"#).unwrap();
        assert_eq!(h.transport, "mqtt");
        assert_eq!(h.topic.as_deref(), Some("demo/temp"));
    }

    #[test]
    fn allowlist_permits_only_listed_target() {
        let allow = Allowlist { entries: vec![AllowEntry {
            transport: "mqtt".into(), url: "mqtt://demo.internal:1883".into(),
        }] };
        let ok = parse_hello(r#"{"transport":"mqtt","url":"mqtt://demo.internal:1883","topic":"t","encoding":"raw"}"#).unwrap();
        let bad = parse_hello(r#"{"transport":"mqtt","url":"mqtt://evil.example:1883","topic":"t","encoding":"raw"}"#).unwrap();
        assert!(allow.permits(&ok));
        assert!(!allow.permits(&bad));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-egress-relay hello`
Expected: FAIL to compile (`parse_hello`/`Hello`/`Allowlist` not defined).

- [ ] **Step 3: Write minimal implementation**

`crates/egress-relay/Cargo.toml`:
```toml
[package]
name = "labwired-egress-relay"
version.workspace = true
edition = "2021"
license.workspace = true
publish = false

[dependencies]
labwired-core = { path = "../core" }
tungstenite = "0.24"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
tracing = "0.1"

[features]
net-tests = []

[[bin]]
name = "labwired-egress-relay"
path = "src/main.rs"
```

`crates/egress-relay/src/hello.rs`:
```rust
use serde::Deserialize;

/// The first frame a browser sends: what backend it wants to reach.
#[derive(Debug, Clone, Deserialize)]
pub struct Hello {
    pub transport: String,
    pub url: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

fn default_encoding() -> String {
    "raw".to_string()
}

pub fn parse_hello(text: &str) -> anyhow::Result<Hello> {
    Ok(serde_json::from_str(text)?)
}

/// One permitted (transport, url) target. Fixed at deploy time; never derived
/// from caller input.
#[derive(Debug, Clone)]
pub struct AllowEntry {
    pub transport: String,
    pub url: String,
}

#[derive(Debug, Clone, Default)]
pub struct Allowlist {
    pub entries: Vec<AllowEntry>,
}

impl Allowlist {
    pub fn permits(&self, h: &Hello) -> bool {
        self.entries
            .iter()
            .any(|e| e.transport == h.transport && e.url == h.url)
    }
}
```

`crates/egress-relay/src/lib.rs`:
```rust
//! Fixed-target WebSocket egress relay. See
//! `docs/superpowers/specs/2026-07-04-browser-egress-ws-relay-design.md`.
pub mod hello;
```

Add `"crates/egress-relay",` to the workspace `members` list in the root `Cargo.toml`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-egress-relay hello`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/egress-relay Cargo.toml
git commit -m "feat(egress-relay): scaffold relay crate with hello parse + allowlist"
```

---

### Task 2: Build a native transport from a validated hello

**Files:**
- Create: `crates/egress-relay/src/build.rs`
- Modify: `crates/egress-relay/src/lib.rs` (add `pub mod build;`)

**Interfaces:**
- Consumes: `Hello` (Task 1); `labwired_core::network::egress::transport::{EgressTransport, TcpSink, MqttPublisher, HttpPoster}`.
- Produces: `fn build_transport(h: &Hello) -> anyhow::Result<Box<dyn EgressTransport>>`.

- [ ] **Step 1: Write the failing test**

```rust
// crates/egress-relay/src/build.rs (bottom)
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hello::parse_hello;

    #[test]
    fn builds_tcp_transport() {
        let h = parse_hello(r#"{"transport":"tcp","url":"127.0.0.1:9000","encoding":"raw"}"#).unwrap();
        assert!(build_transport(&h).is_ok());
    }

    #[test]
    fn mqtt_requires_topic() {
        let h = parse_hello(r#"{"transport":"mqtt","url":"mqtt://h:1883","encoding":"raw"}"#).unwrap();
        assert!(build_transport(&h).is_err());
    }

    #[test]
    fn rejects_unknown_transport() {
        let h = parse_hello(r#"{"transport":"carrier-pigeon","url":"x","encoding":"raw"}"#).unwrap();
        assert!(build_transport(&h).is_err());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-egress-relay build`
Expected: FAIL (`build_transport` not defined).

- [ ] **Step 3: Write minimal implementation**

```rust
// crates/egress-relay/src/build.rs
use crate::hello::Hello;
use anyhow::Context;
use labwired_core::network::egress::transport::{
    EgressTransport, HttpPoster, MqttPublisher, TcpSink,
};

/// Map a validated hello to a native (blocking) egress transport. Transports
/// connect lazily on first send, so this never touches the network.
pub fn build_transport(h: &Hello) -> anyhow::Result<Box<dyn EgressTransport>> {
    match h.transport.as_str() {
        "tcp" => Ok(Box::new(TcpSink::new(h.url.clone()))),
        "mqtt" => {
            let (host, port) = parse_mqtt_url(&h.url)?;
            let topic = h.topic.clone().context("mqtt hello needs 'topic'")?;
            Ok(Box::new(MqttPublisher::lazy(host, port, topic)))
        }
        "http" => Ok(Box::new(HttpPoster::new(h.url.clone())?)),
        other => anyhow::bail!("unknown transport '{other}'"),
    }
}

/// `mqtt://host:port` -> (host, port). Duplicated from world.rs deliberately —
/// the relay must not depend on the World wiring.
fn parse_mqtt_url(url: &str) -> anyhow::Result<(String, u16)> {
    let rest = url.strip_prefix("mqtt://").unwrap_or(url);
    let (host, port) = rest
        .rsplit_once(':')
        .context("mqtt url must be mqtt://host:port")?;
    Ok((host.to_string(), port.parse()?))
}
```

Add `pub mod build;` to `lib.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-egress-relay build`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/egress-relay/src
git commit -m "feat(egress-relay): build native transport from validated hello"
```

---

### Task 3: Per-connection handler (hello → validate → forward frames)

**Files:**
- Create: `crates/egress-relay/src/conn.rs`
- Modify: `crates/egress-relay/src/lib.rs` (add `pub mod conn;`)

**Interfaces:**
- Consumes: `Allowlist` (Task 1), `build_transport` (Task 2), `tungstenite::WebSocket`.
- Produces: `fn serve_connection<S: Read + Write>(ws: &mut WebSocket<S>, allow: &Allowlist) -> anyhow::Result<()>`.

- [ ] **Step 1: Write the failing test (net-gated: real localhost WS + fake TCP backend)**

```rust
// crates/egress-relay/src/conn.rs (bottom)
#[cfg(all(test, feature = "net-tests"))]
mod tests {
    use super::*;
    use crate::hello::{AllowEntry, Allowlist};
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn forwards_ws_payload_to_allowed_tcp_backend() {
        // Fake customer backend.
        let backend = TcpListener::bind("127.0.0.1:0").unwrap();
        let backend_addr = backend.local_addr().unwrap().to_string();
        let backend_reader = std::thread::spawn(move || {
            let (mut s, _) = backend.accept().unwrap();
            let mut buf = [0u8; 5];
            s.read_exact(&mut buf).unwrap();
            buf
        });

        // Relay WS listener.
        let relay = TcpListener::bind("127.0.0.1:0").unwrap();
        let relay_addr = relay.local_addr().unwrap();
        let allow = Allowlist { entries: vec![AllowEntry {
            transport: "tcp".into(), url: backend_addr.clone(),
        }] };
        let relay_thread = std::thread::spawn(move || {
            let (stream, _) = relay.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let _ = serve_connection(&mut ws, &allow);
        });

        // Browser side.
        let (mut ws, _) =
            tungstenite::client::connect(format!("ws://{relay_addr}/")).unwrap();
        ws.send(tungstenite::Message::Text(
            format!(r#"{{"transport":"tcp","url":"{backend_addr}","encoding":"raw"}}"#),
        )).unwrap();
        ws.send(tungstenite::Message::Binary(b"hello".to_vec())).unwrap();
        ws.close(None).ok();

        assert_eq!(&backend_reader.join().unwrap(), b"hello");
        relay_thread.join().unwrap();
    }
}
```
(Client `connect` needs a `TcpStream`; if the tungstenite version in use wants an explicit stream, adapt to `tungstenite::client(url, TcpStream::connect(relay_addr)?)` — keep the assertion identical.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-egress-relay --features net-tests conn`
Expected: FAIL (`serve_connection` not defined).

- [ ] **Step 3: Write minimal implementation**

```rust
// crates/egress-relay/src/conn.rs
use crate::build::build_transport;
use crate::hello::{parse_hello, Allowlist};
use std::io::{Read, Write};
use tungstenite::{Message, WebSocket};

/// Handle one browser connection: first Text frame is the hello (validated
/// against the fixed allowlist), every subsequent Binary frame is forwarded to
/// the backend transport. A rejected hello closes the socket.
pub fn serve_connection<S: Read + Write>(
    ws: &mut WebSocket<S>,
    allow: &Allowlist,
) -> anyhow::Result<()> {
    // 1. Hello.
    let hello = loop {
        match ws.read()? {
            Message::Text(t) => break parse_hello(&t)?,
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => return Ok(()),
            _ => anyhow::bail!("expected hello text frame first"),
        }
    };
    if !allow.permits(&hello) {
        tracing::warn!(target = %hello.url, "rejected egress target (not allowlisted)");
        let _ = ws.close(None);
        anyhow::bail!("target not allowlisted");
    }
    let mut transport = build_transport(&hello)?;

    // 2. Forward payload frames.
    loop {
        match ws.read()? {
            Message::Binary(b) => {
                if let Err(e) = transport.send(&b) {
                    tracing::warn!("backend send failed: {e:?}");
                    break;
                }
            }
            Message::Text(_) => { /* ignore post-hello text */ }
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => break,
            Message::Frame(_) => {}
        }
    }
    Ok(())
}
```

Add `pub mod conn;` to `lib.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p labwired-egress-relay --features net-tests conn`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add crates/egress-relay/src
git commit -m "feat(egress-relay): per-connection hello validation + frame forwarding"
```

---

### Task 4: Server binary (bind, thread-per-connection, config)

**Files:**
- Create: `crates/egress-relay/src/main.rs`
- Create: `crates/egress-relay/relay.example.toml`

**Interfaces:**
- Consumes: `serve_connection` (Task 3), `Allowlist`/`AllowEntry` (Task 1).
- Produces: `fn allow_from_env(get: impl Fn(&str) -> Option<String>) -> Allowlist` in `main.rs` (injectable env lookup so it is unit-testable without touching process env).

- [ ] **Step 1: Write the failing test (env → allowlist builder)**

```rust
// crates/egress-relay/src/main.rs (bottom) — note: bin crates can host #[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn env_builds_single_allowlist_entry() {
        let env: HashMap<&str, &str> = [
            ("RELAY_ALLOW_TRANSPORT", "mqtt"),
            ("RELAY_ALLOW_URL", "mqtt://demo.internal:1883"),
        ].into_iter().collect();
        let allow = allow_from_env(|k| env.get(k).map(|s| s.to_string()));
        assert_eq!(allow.entries.len(), 1);
        assert_eq!(allow.entries[0].transport, "mqtt");
        assert_eq!(allow.entries[0].url, "mqtt://demo.internal:1883");
    }

    #[test]
    fn env_falls_back_to_local_defaults() {
        let allow = allow_from_env(|_| None);
        assert_eq!(allow.entries.len(), 1);
        assert!(allow.entries[0].url.starts_with("mqtt://127.0.0.1"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p labwired-egress-relay env_`
Expected: FAIL (`allow_from_env` not defined).

- [ ] **Step 3: Write the binary**

```rust
// crates/egress-relay/src/main.rs
use labwired_egress_relay::hello::{AllowEntry, Allowlist};
use labwired_egress_relay::conn::serve_connection;
use std::net::TcpListener;

/// Build the fixed allowlist from environment (injectable `get` for tests).
/// MVP: a single demo target; extend the vec to allowlist more fixed backends.
fn allow_from_env(get: impl Fn(&str) -> Option<String>) -> Allowlist {
    Allowlist {
        entries: vec![AllowEntry {
            transport: get("RELAY_ALLOW_TRANSPORT").unwrap_or_else(|| "mqtt".into()),
            url: get("RELAY_ALLOW_URL").unwrap_or_else(|| "mqtt://127.0.0.1:1883".into()),
        }],
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber_init();
    // MVP: fixed allowlist from env. Point at the demo broker only.
    let allow = allow_from_env(|k| std::env::var(k).ok());
    let bind = std::env::var("RELAY_BIND").unwrap_or_else(|_| "127.0.0.1:8090".into());
    let listener = TcpListener::bind(&bind)?;
    tracing::info!(%bind, "egress relay listening");

    for stream in listener.incoming() {
        let stream = match stream { Ok(s) => s, Err(_) => continue };
        let allow = allow.clone();
        std::thread::spawn(move || {
            let mut ws = match tungstenite::accept(stream) { Ok(w) => w, Err(_) => return };
            if let Err(e) = serve_connection(&mut ws, &allow) {
                tracing::debug!("connection ended: {e:?}");
            }
        });
    }
    Ok(())
}

fn tracing_subscriber_init() {
    // Best-effort; ignore if a global subscriber already exists.
    let _ = tracing::subscriber::set_global_default(
        tracing::subscriber::NoSubscriber::default(),
    );
}
```
(If the workspace already has a standard `tracing_subscriber` setup, use it instead of the `NoSubscriber` stub — match sibling bins in `crates/cli`.)

`crates/egress-relay/relay.example.toml`:
```toml
# Fixed egress allowlist. The relay forwards ONLY to these targets.
# Never add caller-supplied hosts — that turns the relay into an open proxy.
bind = "0.0.0.0:8090"

[[allow]]
transport = "mqtt"
url = "mqtt://demo-broker.labwired.internal:1883"
```

- [ ] **Step 4: Build + run the suite**

Run: `cargo build -p labwired-egress-relay && cargo test -p labwired-egress-relay --features net-tests`
Expected: binary builds; all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/egress-relay
git commit -m "feat(egress-relay): server binary with fixed allowlist + example config"
```

---

### Task 5: Browser JS glue (CROSS-REPO — playground/landing repo)

> **Repo boundary:** This task lives in the playground/embed frontend (`app.labwired.com` / the landing repo's playground), NOT in this workspace. No Rust code or test here. The WASM contract already exists: `WasmSimulator.drain_uart_output(): Uint8Array` (`crates/wasm/src/traces.rs`). Ship it as a separate PR in that repo.

**Contract to implement in JS:**
- On demo start, open `const ws = new WebSocket(RELAY_URL)` (RELAY_URL = the deployed relay).
- On `ws.onopen`, send the hello (must exactly match an allowlist entry):
  `ws.send(JSON.stringify({v:1, transport:"mqtt", url:DEMO_URL, topic:DEMO_TOPIC, encoding:"raw"}))`.
- Each animation frame (alongside the existing `drain_uart_output()` poll that feeds the console), if `ws.readyState === WebSocket.OPEN`, forward the same bytes: `const b = sim.drain_uart_output(); if (b.length) ws.send(b)`.
  - **Do not double-drain:** `drain_uart_output` clears the buffer, so read once per frame and fan out to both the console and the WS.
- On demo stop / unload: `ws.close()`.
- Surface `ws.onerror`/`onclose` in the embed status line (reuse the existing affordance).

- [ ] **Step 1:** Implement the above in the playground's sim-driver module.
- [ ] **Step 2:** Manually verify against a locally-run relay (`RELAY_BIND=127.0.0.1:8090`, allowlist pointing at a local mosquitto) that bytes reach the broker (`mosquitto_sub`).
- [ ] **Step 3:** Commit in the frontend repo (same identity/no-AI-footer rules).

---

### Task 6: Live demo lab + chart page (CROSS-REPO — landing repo)

> **Repo boundary:** landing/playground repo + a deployed relay + demo broker. Depends on Tasks 1–5. This is the payoff artifact.

**Deliverable:** A homepage/embed demo where a real firmware image boots in-browser and its UART readings render on a live chart, end to end.

- [ ] **Step 1:** Stand up the demo backend: a mosquitto (or existing dashboard) + a small chart page subscribing to `DEMO_TOPIC` over MQTT-over-WebSocket.
- [ ] **Step 2:** Deploy the relay (Task 4 binary) with the allowlist fixed to that broker; set `RELAY_URL` in the frontend.
- [ ] **Step 3:** Pick/point a demo firmware that emits periodic `KEY=VALUE\n` UART readings; wire the embed to stream via Task 5.
- [ ] **Step 4:** Verify end to end in a browser: boot → chart moves. Capture the promo-grade recording per the repo's demo-video convention.
- [ ] **Step 5:** Update the egress blog post ("Where it's going" → "shipped") once live. (User will handle blog copy.)

---

## Deferred (explicitly not in this plan)
- Arbitrary user backend / self-hosted relay with per-user allowlist + auth (the SSRF-bearing dev path) — separate spec.
- MQTT auth/TLS/reconnect, keep-alive (inherited native-bridge gap).
- Per-connection bounded drop-oldest queue in the relay (MVP forwards synchronously; a slow backend backpressures that one WS).
- Browser CAN/other-source egress; ingress/bidirectional.
- Per-IP connection caps / idle timeout hardening (add before any public deploy).
