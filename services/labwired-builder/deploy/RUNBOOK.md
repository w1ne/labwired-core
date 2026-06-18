# LabWired Builder — Hetzner Deploy Runbook

The builder runs as a Docker Compose stack on a small Hetzner VPS (cx22 /
2 vCPU / 4 GB, Ubuntu 24.04). Steady-state deploy is two commands; the box needs
only Docker (no Rust, Node, or PlatformIO installed on the host).

```
compile      ghcr.io/w1ne/labwired-compile   PlatformIO build service  (internal only)
builder      ghcr.io/w1ne/labwired-builder    /run sim + /compile proxy (internal only)
cloudflared  cloudflare/cloudflared           tunnel → builder.labwired.com
```

Both app images are **private GHCR packages**. The compile container compiles
untrusted source, so it lives on an egress-denied internal network and is never
published to the host. All PlatformIO frameworks are baked into the compile
image at build time, so it compiles every catalog board with **no network**.

## Exposure invariants (do not break)

1. **Never add a `ports:` mapping to `builder` or `compile`.** Docker's iptables
   rules bypass the host firewall; a published port is internet-reachable even
   with UFW denying it. cloudflared reaches the builder over the compose network.
2. **The Cloudflare tunnel has exactly one ingress rule:**
   `builder.labwired.com → http://builder:18080`, then a `404` catch-all. Never
   point an ingress rule at the compile service.
3. **The compile service stays on the `backend` (internal) network** so it has no
   internet egress at runtime (its toolchain caches are baked into the image).

---

## One-time bootstrap

### 1. Install Docker

```bash
curl -fsSL https://get.docker.com | sudo sh
```

### 2. Authenticate to private GHCR (read-only)

Create a fine-grained PAT with **`read:packages`** only, then:

```bash
echo "<PAT>" | docker login ghcr.io -u <github-username> --password-stdin
```

### 3. Create the deploy directory and config

```bash
sudo mkdir -p /opt/labwired-builder && cd /opt/labwired-builder
# Copy docker-compose.yml, .env.example, deploy.sh from
# services/labwired-builder/ (scp, or curl the raw files from the repo).
cp .env.example .env && chmod 600 .env
# Edit .env: set BUILDER_SECRET (openssl rand -hex 32 — must match the
# labwired-api Worker secret), TUNNEL_TOKEN, and IMAGE_TAG.
```

Set the Worker side of the shared secret (once, from a machine with wrangler):

```bash
env -u CLOUDFLARE_API_TOKEN -u CLOUDFLARE_ACCOUNT_ID \
  npx wrangler secret put BUILDER_SECRET --name labwired-api
# paste the same value as BUILDER_SECRET in .env
```

### 4. Create the Cloudflare named tunnel (token-based)

In the Cloudflare dashboard → Zero Trust → Networks → Tunnels:
1. Create a tunnel named `labwired-builder`; copy its **token** into `.env`
   (`TUNNEL_TOKEN`).
2. Add a **public hostname**: `builder.labwired.com` → service
   `http://builder:18080`. (This is the single ingress rule from invariant #2.)

### 5. Bring it up

```bash
./deploy.sh
```

---

## Steady-state deploy

```bash
cd /opt/labwired-builder
./deploy.sh        # docker compose pull && up -d
```

Pin a specific build by setting `IMAGE_TAG=<commit-sha>` in `.env` instead of
`latest`.

---

## Adding a board

The board catalog is a single file: `services/labwired-builder/src/boards.ts`.
Add one entry and that is the whole job — the compile service supports it,
`/boards` lists it, and CI's image build auto-bakes its PlatformIO framework
(`src/warm-cache.ts` derives the bake from the catalog; there is no second list).
The Dockerfile's BuildKit cache mount means a new board only downloads its own
framework, not all of them. Deploy the new board by pulling the new image tag.

---

## Smoke test

```bash
# Public health (through the tunnel):
curl https://builder.labwired.com/healthz          # {"ok":true}

# Proxied compile (through the tunnel; needs BUILDER_SECRET):
SECRET=$(grep BUILDER_SECRET .env | cut -d= -f2)
curl -s -X POST https://builder.labwired.com/compile \
  -H 'Content-Type: application/json' -H "X-Builder-Secret: $SECRET" \
  -d '{"board":"stm32l476","language":"cpp","source":"int main(void){while(1){}}"}' \
  | python3 -c 'import sys,json;print("ok=",json.load(sys.stdin)["ok"])'
```

## Logs & ops

```bash
docker compose logs -f builder
docker compose logs -f compile
docker compose logs -f cloudflared
docker compose ps
```
