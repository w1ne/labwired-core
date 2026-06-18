# Europa cutover (containerize the live builder)

Europa (`admin@157.180.86.12`) currently serves `builder.labwired.com` via the
**old systemd** `labwired-builder.service` on `127.0.0.1:18080`, fronted by a
**host cloudflared** tunnel. This directory upgrades that to the containerized
stack **without changing the tunnel or hostname** — only the process behind
`127.0.0.1:18080` changes.

## Files
- `docker-compose.yml` — copied from `services/labwired-builder/docker-compose.yml`.
- `docker-compose.europa.yml` — cutover override: binds builder to `127.0.0.1:18080`, no cloudflared sidecar (reuse host cloudflared).
- `docker-compose.validate.yml` — validation override: spare port `18081`, runs alongside the live service.
- `.env` — `BUILDER_SECRET` (reused from `/etc/labwired-builder.env`), `IMAGE_TAG=dev`, `TUNNEL_TOKEN=unused`.
- `cutover.sh` / `rollback.sh` — the switch and its undo.

## Prepared state (no live change yet)
1. Images `ghcr.io/w1ne/labwired-{compile,builder}:dev` loaded into Europa's docker.
2. Validated on spare port `18081` (alongside the live service): `/healthz` + an offline proxied `/compile` returning an ELF.
3. Old systemd service still active and serving; nothing on the live path changed.

## To cut over (when approved)
```bash
cd /opt/labwired-builder-compose && ./cutover.sh
```
Rollback at any time:
```bash
cd /opt/labwired-builder-compose && ./rollback.sh
```

## Risks / guards
- Brief downtime (~seconds) while 18080 hands off from systemd to the container.
- `cutover.sh` stops the systemd unit *first* to free 18080; `rollback.sh` restarts it.
- The compile container is internal/egress-denied and keeps no host port, so it does not collide with proto.cat's compile service on `127.0.0.1:8080`.
- `BUILDER_SECRET` is reused verbatim, so the `labwired-api` Worker keeps matching.
