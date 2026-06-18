# Europa ops — what's on the box, disk budget, deploy

Europa (`admin@157.180.86.12`, 75G disk) is **shared** by several systems. This
doc is the map so the next deploy doesn't hit a 98%-full surprise.

## What runs on Europa
| System | How | Port(s) | Notes |
|---|---|---|---|
| labwired builder | docker compose (`/opt/labwired-builder-compose`) | `127.0.0.1:18080` | `builder.labwired.com` via host `cloudflared` |
| labwired compile | same compose: `compile` (sealed) + `compile-net` (egress lane) | none (internal) | 15GB→~8GB after the slim rebuild |
| proto.cat | Next.js via systemd `protocat` (`pnpm start`) | `127.0.0.1:3001` | deploy = rsync from a Mac + `pnpm build` |
| proto.cat compile | docker compose (`~/protocat/deploy`) `protocat-compile` | `127.0.0.1:8080` | **to be retired** once labwired's egress lane serves its lib_deps |
| postgres / redis | proto.cat compose | `127.0.0.1:5432/6379` | |
| kernelcad-server | docker | `127.0.0.1:3000` | |
| cloud-dictation-proxy | docker | `8787` | |
| GitHub runner | container (`ubuntu:24.04`) | — | label `deploy`; build target for future on-box deploys |

## Disk budget (the recurring pain)
- 75G total. The labwired compile image is the elephant. Two 15GB images do **not**
  both fit alongside everything else — that's why a compile swap stages a tar +
  rollback tar, removes the old image, then loads the new (brief compile-only window).
- Reclaim safely with `docker builder prune -f` (cache only). Do **NOT**
  `docker image prune -a` — it deletes not-yet-running labwired images and others.
- `localhost` resolves to `::1`; the builder binds IPv4 `127.0.0.1` only. **Always
  use `127.0.0.1`** in env/URLs (else `ECONNREFUSED`).

## Deploy labwired — `deploy.sh` (build on Europa, no uplink transfer)
**Primary path:** from your laptop, `bash services/labwired-builder/deploy/europa/deploy.sh`.
It rsyncs only the compile *source* (a few MB) to `/home/admin/labwired-builder-src`,
builds the ~8GB image **on Europa** (datacenter bandwidth pulls the PlatformIO
frameworks; a persistent BuildKit cache makes catalog-only edits rebuild in
seconds), recreates both compile lanes + builder, health-checks, and keeps the
previous image as `:prev` for **instant rollback** (auto-dropped on success,
auto-restored on failure). No 8–15GB image ever crosses the home uplink.

The 575MB **builder** image changes rarely; when `src/server.ts` etc. change,
rebuild it and `docker save | ssh | docker load` it (small) or build it on the box
(needs the `core` submodule in the context).

**Legacy (image transfer):** `docker save … | gzip | ssh | docker load` + `cutover.sh`
— only if a from-laptop build is needed. Rollback: `rollback.sh` or
`docker load < /home/admin/lwc-rollback.tar.gz`.

Note: the BuildKit cache (~9GB) lives on the box for fast rebuilds; `docker
builder prune -f` reclaims it (next build re-downloads).

## Two compile lanes
- `compile` — SEALED (egress-denied `backend`): untrusted/public source.
- `compile-net` — egress lane (`netlane` bridge): only requests **with `lib_deps`**
  (the builder routes by that), reachable only via the secret-gated builder. Lets
  `pio` download libraries (e.g. proto.cat's e-paper GxEPD2) that the sealed lane
  cannot.
