# Projects & Visibility — Phase B (Designer tier)

Status: design only — pending sign-off
Owner: andrii
Last edited: 2026-05-16

Phase B spec for Designer tier: server-side **projects** (save / load / share /
fork) and a **visibility** model (public / unlisted / private). Phase A shipped
Stripe + Clerk + workspaces + API keys; the playground itself has no
server-side persistence — boards live in `localStorage` per board id (see
`getWorkspaceStorageKey` in `packages/playground/src/App.tsx`) and sharing is
hash-encoded via `generateShareUrl` in `packages/ui/src/editor/sharing.ts`.
This doc defines what we add on top.

---

## 1. Goals & non-goals

**Goals.**
- Authenticated users can save a project to the server, give it a name, and
  reopen it on another device.
- Public projects have a stable, crawlable URL that someone can paste into a
  Reddit thread or a bug report.
- Designer tier ($5/mo) gates the **private** flag. Free users can save
  unlimited public projects only.
- Forking a public project produces a new owned copy attributed to the original.
- The Library page surfaces "Your projects" plus a small curated "Featured" set
  carried over from today's static featured-labs grid.

**Non-goals (Phase B).**
- No project-level collaboration / multi-cursor editing.
- No comments, likes, follows, or any social graph beyond owner+fork-parent.
- No project-level version history beyond the latest saved revision.
- No team workspaces / shared ownership. A project belongs to one workspace.
- No moderation tooling beyond "owner can delete" + a manual admin kill switch.
- No imported firmware blob storage on free tier (see §6).

---

## 2. User stories

Named-persona stories — the use cases this design must serve.

1. **Aleksei, freelance firmware contractor.** Wires an STM32F103 + NTC
   thermistor lab on the train. Has Designer. Saves as `client-acme-ntc-bringup`
   set to private. Opens it again on his desktop, hits Run, keeps iterating.

2. **Maria, embedded TA.** Builds a public project `arm-cortex-m-week3-blink`
   for her students. Pastes the URL into the course LMS. Expects the link to
   keep working at the end of the semester and the page to render without
   login.

3. **Pavlo, hobbyist on the free tier.** Sees Aleksei's *unlisted* shared
   project link in a forum. Forks it. The fork lands in his "Your projects"
   list, defaulted to public (free tier can't go private). The original keeps
   showing `forked 1×` to Aleksei.

4. **Ines, recruiter scouting for embedded work.** Googles `stm32 mpu6050
   labwired`. Hits a crawlable public-project page. Sees a working live demo
   without signing up. Bookmarks LabWired.

5. **Karim, on a flaky cafe wifi.** Was editing an anonymous (logged-out)
   workspace, decides to sign up. The local `labwired-diagram:<boardId>` /
   `labwired-source:<boardId>` workspace gets promoted to a real Project on
   first save. Nothing is lost.

6. **Dmytro, demoing for a procurement call.** Forks `featured/blinky` into his
   workspace to scribble on, but doesn't want it indexed. Picks **unlisted**.

---

## 3. Data model

One new KV namespace, plus one R2 bucket for blobs. Stored value:

```
KV_PROJECTS[<projectId>] = ProjectRecord (JSON)
```

```
ProjectRecord {
  id:              string           // "prj_" + 16 hex chars
  slug:            string           // url-safe; unique per owner; ≤ 48 chars
  owner_workspace_id: string        // ws_…
  owner_clerk_user_id: string       // duplicated for fast list-by-user
  name:            string           // human label, ≤ 80 chars
  description:     string | null    // ≤ 280 chars, nullable
  visibility:      "public" | "unlisted" | "private"
  board_id:        string           // e.g. "stm32f103-blinky"
  diagram:         Diagram          // ~few KB JSON, inline
  source:          string           // firmware source, ≤ 64 KB inline
  firmware_blob_key: string | null  // R2 key, populated when uploaded ELF used
  forked_from:     string | null    // parent projectId, null if original
  fork_count:      number           // denormalized cache
  created_at:      string           // ISO 8601
  updated_at:      string           // ISO 8601
  schema_version:  1
}
```

Plus three secondary indexes (KV keys, no value besides a marker):

```
KV_PROJECT_INDEX[user:<clerk_user_id>:<updated_at_desc>:<projectId>] = ""
KV_PROJECT_INDEX[public:<updated_at_desc>:<projectId>] = ""   // listing
KV_PROJECT_INDEX[slug:<clerk_user_id>:<slug>] = "<projectId>" // unique check
```

The `<updated_at_desc>` segment is `9999999999999 - epoch_ms`, zero-padded — KV
list returns keys in lexical order so this gives newest-first. Cheap and good
enough until we outgrow KV (see §8).

---

## 4. URL scheme

Decision: **`app.labwired.com/p/<projectId>`** for private/unlisted, and
**`app.labwired.com/p/<projectId>/<slug>`** as the canonical form for
public projects (slug is decorative; the id is the source of truth).

Reasoning:
- Username-based URLs (`/<user>/<slug>`) require Clerk-handle ownership and
  username collision handling. Not worth the cost for Phase B.
- A numeric/hex id (Wokwi-style) prevents enumeration of private projects, since
  ids are 64-bit random.
- The trailing slug is for SEO and shareability — the server 301s any wrong
  slug back to the canonical one. If the project is unlisted/private and the
  viewer isn't the owner, the slug is stripped to avoid leaking the title.
- `/p/` keeps things shorter than `/projects/<id>` and frees up `/projects` for
  a "Your projects" dashboard route later.

The current `?lab=<boardId>` deep-link in `BoardPicker` and the `#<hash>` share
mode in `App.tsx` both continue to work unchanged.

---

## 5. Visibility model

Three states: **public**, **unlisted**, **private**.

- **public** — appears in own listing, in `/p/featured`, indexable by Google
  via the static HTML rendered server-side for the project page.
- **unlisted** — accessible to anyone with the link, but never listed
  anywhere, and `robots: noindex`. This is the natural state for "share with a
  reviewer" use cases (Pavlo / Dmytro stories).
- **private** — only the owner workspace can read or fork. Designer-gated.

Two states (public/private only) would be simpler, but unlisted is the
single most useful state when sharing a work-in-progress in chat or a bug
report, and Wokwi users will expect it. Cost to support is one `if` branch in
the read handler and one entry in the visibility enum.

---

## 6. Plan-tier limits

| Tier             | Public | Unlisted | Private | Firmware blob (R2) | Forks of own projects |
|------------------|--------|----------|---------|--------------------|-----------------------|
| Anonymous        | 0      | 0        | 0       | none               | n/a                   |
| Free (signed-in) | 25     | 5        | 0       | none (source only) | unlimited             |
| Designer ($5)    | 100    | 25       | 25      | up to 10 MB/file   | unlimited             |
| Pro ($19)        | 1000   | 250      | 250     | up to 10 MB/file   | unlimited             |
| Enterprise       | custom | custom   | custom  | custom             | unlimited             |

Notes:
- **Anonymous users cannot create projects.** They can still use the playground
  with localStorage and `#<hash>` shares as today. Forcing sign-up at the save
  step is what converts traffic to accounts. Wokwi allows anonymous publish but
  we're optimizing for funnel, not max-volume content.
- **Firmware blob upload** is the cliff to gate. Uploading a real ELF is what
  burns R2 storage cost; we hold the line at the paid tier.
- Limits are server-enforced in `POST /v1/projects` against `workspace.plan`
  and a count computed by listing the user-index prefix. Cheap.
- The free-tier 25-public cap is high enough that a hobbyist never hits it, low
  enough to deter someone scripting an SEO farm.

---

## 7. API surface

All endpoints are on the existing Worker (`packages/api/src/index.ts`), prefix
`/v1/projects`. Auth column meanings: **clerk** = `Authorization: Bearer
<clerk_jwt>` (same path as `/v1/auth/me`); **key** = `Authorization: Bearer
lwk_live_…`; **none** = no auth header required.

| Method | Path                       | Auth   | Plan gate | Returns                                  |
|--------|----------------------------|--------|-----------|------------------------------------------|
| POST   | `/v1/projects`             | clerk  | per-tier limit | `ProjectRecord` (201)               |
| GET    | `/v1/projects/:id`         | none*  | —         | `ProjectRecord` minus blob bytes         |
| GET    | `/v1/projects/:id/blob`    | none*  | —         | `application/octet-stream` (R2 stream)   |
| PUT    | `/v1/projects/:id`         | clerk  | private→Designer | `ProjectRecord`                   |
| DELETE | `/v1/projects/:id`         | clerk  | —         | `204`                                    |
| GET    | `/v1/projects/me`          | clerk  | —         | `{ items: ProjectSummary[], cursor }`    |
| GET    | `/v1/projects/featured`    | none   | —         | `{ items: ProjectSummary[] }`            |
| POST   | `/v1/projects/:id/fork`    | clerk  | per-tier limit | `ProjectRecord` (the new copy)      |

*`GET /v1/projects/:id` returns 404 for `private` projects unless the request
carries a clerk JWT for the owner workspace, and 404 for non-existent ids
(never 403 — that confirms existence).

`ProjectSummary` is `ProjectRecord` without `diagram`, `source`,
`firmware_blob_key`; just enough for the Library card grid.

Rate limits (cheap, enforced via the existing CF rate-limit binding pattern, not
this spec's scope to design):
- Anonymous reads of public projects: 600/min/IP.
- Authed writes: 60/min/workspace.
- Fork: 30/min/workspace.

Featured projects are not a separate type — they're a hard-coded allowlist of
`projectId` values held in a `KV_PROJECTS_FEATURED` namespace (single key
`list` → JSON array). Edit by hand; no admin UI in Phase B.

---

## 8. Storage decision

Recommendation: **KV for metadata + small payloads, R2 only for uploaded
firmware blobs.** Hybrid.

| Candidate         | Pros                                              | Cons                                                              |
|-------------------|---------------------------------------------------|-------------------------------------------------------------------|
| **KV only**       | Zero new bindings; matches Phase A patterns       | 25 MB/value cap; KV listing is slow at scale; no SQL queries      |
| **D1 only**       | Real SQL, indexes, joins; trivial pagination      | New binding, new schema migrations; D1 is still GA-young; cost    |
| **R2 only**       | Cheap for large blobs; bytes are bytes            | No metadata queries; we'd need a manifest file per user — gross   |
| **KV + R2 hybrid (rec.)** | Metadata fast and free-ish; blobs cheap; no new query layer | Two stores to keep consistent; secondary indexes by hand    |

Why hybrid wins for Phase B:
- A `ProjectRecord` minus the firmware blob is ~10-40 KB. Diagrams are small
  JSON; sources are capped at 64 KB. Well under KV's 25 MB.
- The only thing that can blow up is an uploaded ELF (1-10 MB). That goes in R2
  under key `firmware/<projectId>/<sha256>.bin`. Cheap, content-addressed.
- The "list my projects" and "list featured" queries are both
  prefix-scans on KV — fast for the volumes we'll see in year one (<10k
  projects total seems plausible).
- We can migrate to D1 later when we want `WHERE board_id = ?` filters or
  full-text search. KV records are a JSON-shaped row; the migration is a
  one-shot script.

Open trade-off: KV is eventually-consistent. A user who saves and immediately
reloads may see the previous version for a few seconds. Acceptable for
Phase B; we'll add a `If-Match: <updated_at>` check on `PUT` if it becomes a
problem.

---

## 9. UI changes

### 9.1 Save dialog (new)

Triggered by the existing Share button (`ShareIcon` in `App.tsx` line ~1304) —
the button now opens a "Save / Share" sheet. Pure save (no name change) is the
Cmd-S shortcut.

```
┌──────────────────────────────────────────────────────┐
│ Save project                                    ⨯    │
├──────────────────────────────────────────────────────┤
│  Name        [ blinky-pa5-experiment             ]   │
│  Description [ (optional)                        ]   │
│                                                      │
│  Visibility                                          │
│   (●) Public      anyone with link, listed           │
│   ( ) Unlisted    anyone with link, hidden           │
│   ( ) Private     only you   [ Designer ↑ ]         │
│                                                      │
│  Board: STM32F103 Blinky                             │
│  Firmware blob: (none — using compiled source)       │
│                                                      │
│              [ Cancel ]      [ Save project ]        │
└──────────────────────────────────────────────────────┘
```

The Private radio is disabled and shows a small inline upsell link to the
Stripe checkout (existing `buildStripeUpgradeUrl` flow).

### 9.2 "Your projects" in `AccountPanel`

New section in `packages/playground/src/studio/AccountPanel.tsx`, below the
API key card:

```
─────────  Your projects  ───────────────────────────────
  blinky-pa5-experiment        public · 2 days ago     →
  client-acme-ntc-bringup      private · 4 hours ago   →
  forked: maria/week3-blink    public · last week      →

           [ See all 12 → ]   [ + New project ]
─────────────────────────────────────────────────────────
```

Three most-recent, plus an "all" link that opens the Library on the Yours
tab. Each row is a button that loads the project into the playground (same
codepath as `?lab=` deep-links, but with a `?project=<id>` param).

### 9.3 Library page rework

`packages/playground/src/library/Library.tsx` keeps its current content but
moves it under a second tab. Tabs at the top of the page body:

```
   ┌──────────┬──────────┬──────────────────┐
   │  Yours   │ Featured │ All boards       │   ← tab strip
   └──────────┴──────────┴──────────────────┘
```

- **Yours** — list of `GET /v1/projects/me`, paginated. Empty state shows a
  "Make your first project" CTA pointing at the playground.
- **Featured** — current 10 hardcoded `FEATURED_LABS` cards, rendered from
  `GET /v1/projects/featured` (which during Phase B can just be a static
  fallback if KV returns empty).
- **All boards** — current `SUPPORTED_BOARDS` grid, unchanged.

### 9.4 Fork button

When the loaded project's `owner_workspace_id` is not the current user's,
the toolbar shows a new "Fork" button between Share and Export. Clicking
prompts for a new name (default: `<original-name> (fork)`) and visibility,
then calls `POST /v1/projects/:id/fork`. On 200, the URL replaces to the new
project. For anonymous users, the button opens the existing `AuthModal`.

---

## 10. Migration & launch sequencing

Four commits, in order. Each is independently deployable.

1. **Worker: storage + read endpoints, no UI.**
   Add `KV_PROJECTS`, `KV_PROJECT_INDEX`, `KV_PROJECTS_FEATURED`, R2 bucket
   binding `R2_FIRMWARE`. Implement `POST/GET/PUT/DELETE /v1/projects` +
   `/me` + `/featured`. Featured allowlist starts empty. Smoke test via
   curl. No frontend code yet.

2. **Frontend: Save dialog + project URL load.**
   Wire the Share button to the Save sheet. Implement `?project=<id>` load
   on App mount alongside the existing `#<hash>` and `?lab=` paths. On
   first sign-in, if a non-empty workspace exists in
   `labwired-diagram:<currentBoardId>`, prompt once: "Save your current work
   as a project?" — Yes calls `POST /v1/projects`, No clears the prompt
   flag. `localStorage` workspaces stay around as ephemeral session state;
   we don't bulk-migrate.

3. **Library rework + Fork.**
   Tab strip in `Library.tsx`. Yours/Featured tabs wired to the new API.
   Fork button in the playground toolbar. Backfill 6-10 curated featured
   projects by hand (run a script as the founder account, then add their
   ids to `KV_PROJECTS_FEATURED:list`).

4. **AccountPanel "Your projects" + upgrade upsell.**
   Add the three-row recent-projects block. Wire the Private-radio upsell
   to the existing Stripe upgrade URL.

Rollback story: each commit can be reverted independently. KV/R2 data isn't
deleted on revert; old project URLs just 404 cleanly because the routes are
gone.

---

## 11. Effort estimate

T-shirt sizes per sub-task, with rough subagent-hours.

| Sub-task                                                | Size | Hours |
|---------------------------------------------------------|------|-------|
| Worker: new KV/R2 bindings + ProjectRecord codec        | S    | 2     |
| Worker: CRUD endpoints + plan-gating                    | M    | 6     |
| Worker: fork endpoint (atomic copy, fork_count incr.)   | S    | 3     |
| Worker: `/featured` + allowlist plumbing                | S    | 2     |
| Frontend: Save dialog component                         | M    | 4     |
| Frontend: `?project=<id>` load path + URL plumbing      | S    | 3     |
| Frontend: first-sign-in migration prompt                | S    | 2     |
| Frontend: Library tabs + Yours/Featured fetch           | M    | 5     |
| Frontend: Fork button + flow                            | S    | 3     |
| Frontend: AccountPanel projects block                   | S    | 3     |
| Frontend: upgrade upsell for Private radio              | XS   | 1     |
| Tests (unit + a couple e2e save/load/fork)              | M    | 5     |
| Curate the first 6-10 featured projects by hand         | XS   | 2     |
| **Total**                                               |      | **~41 hrs ≈ 1.5 weeks** of subagent time |

Risky items, honestly:
- **Fork atomicity** — KV doesn't have transactions. If a fork copies the
  record but fails to bump the parent's `fork_count`, the count drifts. Plan:
  treat `fork_count` as a soft-cache and don't show it until we have a fix-up
  job.
- **Plan-limit count enforcement** — listing a KV prefix to count is racy. Two
  parallel `POST /v1/projects` from the same user can both pass the check.
  Acceptable; we'd rather over-count than block.
- **Server-side render for SEO** — the page rendering `/p/:id` for Google needs
  HTML, not the SPA. Workers can render a stub HTML shell with `<title>` and
  `<meta>` populated from the record, plus a `<noscript>` fallback. Not free,
  but not gnarly. Tracked under "Frontend: `?project=<id>` load path".

---

## 12. Open questions

- **Slug vs id-only URLs.** I picked `/p/<id>/<slug>`. But for a Reddit paste,
  `/p/<id>` is shorter and matches Wokwi. Want the slug for SEO; not sure it's
  worth the extra UI to choose one when saving. **Need a call.**
- **Anonymous public publish.** I said no. Wokwi says yes. There's a real
  growth argument either way — anonymous publish lowers friction, but every
  abusive SEO-spam project becomes our problem to moderate. Want the founder's
  view before locking.
- **Free-tier public cap of 25** is a guess. No data. Pick a number, watch.
- **Featured curation tooling.** Phase B is "edit KV by hand." That's fine
  until we have 20+ featured projects, at which point we'll want a tiny admin
  page. Out of scope here but worth flagging.
- **What counts as a project under Designer's "25 private" cap when you delete
  one?** Hard-deletes free up the slot; soft-deletes (which we don't currently
  do) would not. Phase B is hard-delete only. Document as such.
- **Firmware-blob retention on deletion.** If a project references a blob and
  is deleted, do we GC the blob? Phase B: yes, in the DELETE handler, since
  blobs are content-addressed and shared across forks. Means a fork must
  bump a refcount or copy the blob. **Open**: I'll default to "copy on
  fork" (simpler, slightly more R2 cost) unless told otherwise.

---

## 13. Out of scope (deferred)

Explicitly Phase C / D:

- **Project history / revisions.** No git-like history; latest revision only.
- **Comments / likes / follows / a social graph.** None of this.
- **Search across public projects.** Not until we have D1 or an index. Today
  the only discovery is the curated `/featured` list.
- **Team workspaces** — a Project belongs to one workspace_id. Multi-tenant
  ownership comes when we have a Team SKU.
- **Embed-iframe view of a saved project.** Today `?embed=true` works on
  `#<hash>` URLs; extending it to `?project=<id>` is a one-liner but I'm
  leaving it for Phase C alongside the embed-only design polish.
- **Project-level analytics** (views, fork counts beyond a denormalized
  cache).
- **Public-project moderation tools.** For Phase B the founder has a kill
  switch by deleting the KV record directly.
- **Migration of existing `labwired-diagram:<boardId>` localStorage state into
  projects en masse.** We do the gentle "prompt once on first sign-in" path
  in §10 step 2 and leave the rest as session state.
