# LabWired Builder — Hetzner Deploy Runbook

Target: a small Hetzner VPS (cx22 / 2 vCPU, 4 GB RAM, Ubuntu 24.04 is fine).
The service is accessed by the Cloudflare Worker via the `builder.labwired.com`
tunnel — never exposed directly to the internet.

The service is **run-only**: it receives a compiled ELF from the agent and runs
it in the `labwired` digital-twin simulator. There is no hosted compiler; agents
compile in their own sandboxes and upload the ELF (see `docs/firmware-scaffolds`
for the exact toolchain flags).

---

## 1. Build and install the native `labwired` simulator binary

On your **dev machine** (where the Rust toolchain lives):

```bash
cd ~/Projects/labwired/core
cargo build --release
scp target/release/labwired hetzner:/usr/local/bin/labwired
```

On the **Hetzner box**, verify:

```bash
sudo chmod +x /usr/local/bin/labwired
labwired --version
```

---

## 2. Deploy the builder service

```bash
sudo mkdir -p /opt/labwired-builder
# Copy the whole labwired-builder package (or git clone and cd into it)
sudo rsync -a --exclude node_modules --exclude .git \
  ~/Projects/labwired/.worktrees/mcp-build-run-loop/services/labwired-builder/ \
  hetzner:/opt/labwired-builder/

# On the Hetzner box:
cd /opt/labwired-builder
npm ci --omit=dev
```

---

## 3. Generate and store the shared secret

```bash
# On any machine with openssl:
openssl rand -hex 32
```

Create `/etc/labwired-builder.env` on the Hetzner box (mode 600, owned by root):

```bash
sudo tee /etc/labwired-builder.env > /dev/null <<'EOF'
BUILDER_SECRET=<paste the 64-char hex string from above>
EOF
sudo chmod 600 /etc/labwired-builder.env
```

---

## 4. Install and enable the systemd unit

```bash
sudo cp /opt/labwired-builder/deploy/labwired-builder.service \
        /etc/systemd/system/labwired-builder.service
sudo systemctl daemon-reload
sudo systemctl enable --now labwired-builder

# Verify it started cleanly:
sudo systemctl status labwired-builder
sudo journalctl -u labwired-builder -n 30
```

Key `Environment=` lines in the unit (see `labwired-builder.service`):
- `BUILDER_ENTRY=1` — without this the server.ts process exits immediately
  (the entrypoint guard at the bottom of server.ts checks for this var).
- `PORT=18080` — listens on the host only (cloudflared proxies it). `europa`
  already uses `127.0.0.1:8080` for another service.
- `MAX_CONCURRENT=2` — limits parallel run jobs.
- `LABWIRED_BIN=/usr/local/bin/labwired` — path to the simulator binary.
- `EnvironmentFile=/etc/labwired-builder.env` — loads `BUILDER_SECRET`.

**Note on `PrivateNetwork`:** The unit does NOT set `PrivateNetwork=yes`. With no
hosted compiler there is no untrusted C/C++ code executing on the server, so
service-level network isolation is not needed. More importantly, `PrivateNetwork=yes`
puts the service into a network namespace with only loopback, which prevents
`cloudflared`/caddy from reaching the listening port.

---

## 5. Create and route the Cloudflare tunnel

```bash
# Authenticate cloudflared (browser pop-up on first run):
cloudflared tunnel login

# Create the named tunnel:
cloudflared tunnel create labwired-builder
# Note the tunnel UUID printed; it will also appear in ~/.cloudflared/<uuid>.json

# Copy this repo's config to the cloudflared config directory:
sudo mkdir -p /home/labwired/.cloudflared
sudo cp /opt/labwired-builder/deploy/cloudflared-config.yml \
        /home/labwired/.cloudflared/config.yml
# Update credentials-file path in config.yml to match the actual JSON path:
#   /home/labwired/.cloudflared/<tunnel-uuid>.json

# Create the DNS CNAME (points builder.labwired.com → the tunnel):
cloudflared tunnel route dns labwired-builder builder.labwired.com

# If `cloudflared tunnel route dns` is unavailable locally, create the same
# proxied CNAME in Cloudflare DNS:
#   builder.labwired.com → <tunnel-id>.cfargotunnel.com

# Install as a system service:
sudo cloudflared service install
sudo systemctl enable --now cloudflared
```

---

## 6. Set the Worker secret (OAuth login workaround)

The shell may have a stale `CLOUDFLARE_API_TOKEN` that returns error 9109.
Unset both CF env vars before calling wrangler so it falls back to the
OAuth login credentials stored in `~/.config/wrangler/config.toml`:

```bash
env -u CLOUDFLARE_API_TOKEN -u CLOUDFLARE_ACCOUNT_ID \
  npx wrangler secret put BUILDER_SECRET --name labwired-api
# When prompted, paste the same 64-char hex string you put in
# /etc/labwired-builder.env.
```

---

## 7. Smoke-test

```bash
# Health check (no auth required):
curl https://builder.labwired.com/healthz
# Expected: {"ok":true}

# Run smoke (requires the correct BUILDER_SECRET + a compiled ELF):
# See docs/firmware-scaffolds/README.md for how to compile a test ELF.
# Then:
# ELF_B64=$(base64 -w 0 firmware.elf)
# SYSTEM_YAML=$(cat blink-l476.system.yaml)
# curl -s -X POST https://builder.labwired.com/run \
#   -H 'Content-Type: application/json' \
#   -H "X-Builder-Secret: $(sudo grep BUILDER_SECRET /etc/labwired-builder.env | cut -d= -f2)" \
#   -d "{\"elfBase64\":\"$ELF_B64\",\"systemYaml\":\"$SYSTEM_YAML\",\"maxSteps\":10000}" | jq .stopReason
```

---

## Security notes

- **`safeEnv()`** — `src/safe-env.ts` strips `BUILDER_SECRET` (and other
  sensitive vars) from the environment passed to every run subprocess.
  The subprocess never sees the secret even on the same process tree.
- `ProtectSystem=strict` + `DynamicUser=yes` prevent the service from writing
  outside `/tmp` and ensure it runs as a transient non-privileged UID.
- `PrivateTmp=yes` — each service restart gets a fresh `/tmp` namespace.
- `NoNewPrivileges=yes` — the process and all children cannot gain new
  privileges via setuid/setcap.
