# First Five AI Lab Outreach — 2026-05-16

For: Andrii Shylenko, solo founder, LabWired.

Five cold emails targeting agent-tooling decision-makers at Cognition (Devin),
Anysphere (Cursor), Continue.dev, Replit (Agent team), plus one slot for an
embedded engineering contact Andrii already knows. Goal: land one or two
design partners for the deterministic-oracle MCP surface before the API shape
calcifies. Not raising, not selling procurement.

Send window: this week (2026-05-18 through 2026-05-22), Tuesday–Thursday
mornings recipient-local. Success = two replies that say "send a calendar
link." Tone matches `docs/strategy/pitch/labwired-as-agent-oracle.md`.

---

## Email 1 — Cognition (Devin)

**Recipient:** The Devin engineering team (no named individual; see notes).
**Channel:** `careers@cognition.ai` is the only published address. Pair with a
LinkedIn DM to whoever currently lists "Devin agent capabilities" or "tool use"
in their title, and if there is an active GitHub issue thread on Devin's tool
surface, a comment there is fair game.

**Subject:** Deterministic firmware oracle for Devin via MCP

**Body:**

Hi — writing to the Devin engineering team because I could not identify the
right named owner from public info, and would rather not guess.

I am building LabWired: a cycle-accurate, deterministic firmware simulator for
ARM Cortex-M, RISC-V, and Xtensa MCUs. Open-source Rust core, MIT. Determinism
is enforced by a CI gate that SHA-256-compares five runs of the same firmware
plus manifest plus script — identical `result.json`, or the release fails.

When Devin touches embedded code, it has no fast, reproducible way to verify a
fix. QEMU is non-deterministic, HIL is slow and flaky, "I'm confident this
fixes it" is not an oracle. LabWired is meant to be that oracle inside the
agent loop.

The MCP server is shipped — `npx -y @labwired/mcp` exposes three tools
(`labwired_catalog`, `labwired_simulate`, `labwired_validate_system`) over
stdio. The concrete proof point is a GPDMA stress case on STM32H563ZI where
logic-analyzer HIL hides the bug under sampling jitter and a deterministic
simulator surfaces it on every run — happy to walk through it on a call.
Rust core is open at `github.com/w1ne/labwired-core`.

Ask: 30 minutes with whoever owns Devin's tool surface. Design-partner terms
if useful — free Pro for the team, founder-led integration support, priority
on tools you want exposed.

Andrii

**Send notes:**
- Primary send: `careers@cognition.ai` with subject above and the note
  "for the engineering team — please forward to whoever owns agent tool
  integrations."
- Parallel LinkedIn DM to one engineer whose profile mentions Devin's tooling
  layer, with the body trimmed to the first two paragraphs plus the ask.
- If no reply by 2026-05-26 (5 business days): send the follow-up template
  below as a reply to the original thread. After that, drop it for 60 days.

---

## Email 2 — Anysphere (Cursor)

**Recipient:** Aman Sanger, co-founder, Cursor.
**Channel:** X DM to `@amanrsanger`. Aman has historically replied to
technical, short pitches on X. Founder-direct is appropriate here because
Cursor has no published agent-team email and the company is still small enough
that the founders own the agent roadmap.

**Subject:** Cycle-accurate firmware oracle for Cursor's agent

**Body:**

Aman — short pitch on a primitive I think Cursor's agent is missing.

LabWired: cycle-accurate, deterministic firmware simulator for Cortex-M,
RISC-V, Xtensa. Open-source Rust core, MIT. Determinism is CI-enforced — five
runs of the same inputs SHA-256 to the same `result.json`, or the release
fails.

Cursor specifically: the agent edits firmware fluently, but the verify step is
broken. User has to run on hardware, the agent never closes the loop, and the
"did this actually fix the DMA underrun" answer arrives hours later from a
human. A deterministic simulator wired to MCP gets the agent a verdict it can
trust in seconds.

`npx -y @labwired/mcp` exposes three tools: catalog, simulate, validate. Stdio
MCP, one-line config in `~/.cursor/mcp.json`. The `determinism-proof` gate in
`core-ci.yml` is the receipt that the verdict is trustworthy.

Ask: 30 minutes with you or whoever owns Cursor's agent tool layer. Design
partner terms if useful — free Pro for the team, direct line on integration,
priority on the next tools you want.

Andrii

**Send notes:**
- Primary: X DM to Aman Sanger. If the DM is restricted, a reply to a recent
  technical post of his with "would love to send you a short pitch — DMs open?"
- Backup: same body to Michael Truell on X, but only if Aman does not respond
  in 5 business days. Do not double-tap founders simultaneously.
- If no reply by 2026-05-26: follow-up template below, single reply, then drop
  for 60 days.

---

## Email 3 — Continue.dev

**Recipient:** Nate Sesti and Ty Dunn, co-founders, Continue.
**Channel:** Email via `continue.dev/contact` form, addressed to both by name.
Continue is open source and the founders are known to engage on GitHub
discussions and Discord, so the GitHub Discussions page on `continuedev/continue`
is a parallel surface.

**Subject:** MCP server for deterministic firmware sim — Continue fit?

**Body:**

Nate and Ty — Continue is the most natural first integration target for what
I am building, so leading with you.

LabWired: cycle-accurate, deterministic firmware simulator for Cortex-M,
RISC-V, Xtensa. Open-source Rust core, MIT. Determinism is enforced — five
runs of the same firmware plus manifest plus script produce identical
`result.json`, verified by the `determinism-proof` gate in `core-ci.yml`.

Fit for Continue: your users on embedded codebases hit the same wall agents
hit. Edit step is good, verify step is hardware-or-bust. A deterministic
simulator over MCP closes the loop inside the IDE, no bench required.

`npx -y @labwired/mcp` — stdio, three tools (catalog, simulate, validate),
one-line config in any MCP-aware client including Continue. Rust core open at
`github.com/w1ne/labwired-core`. Happy to send the longer pitch on reply.

Ask: 30-minute technical call. Design-partner terms if useful — free Pro for
the team, founder integration support, priority on the tools Continue users
ask for. Solo, pre-revenue, not raising. Looking for one or two design
partners before the API shape locks.

Andrii

**Send notes:**
- Primary: `continue.dev/contact` form, addressed "Hi Nate and Ty".
- Parallel: open a Discussion on `github.com/continuedev/continue` titled
  "MCP server for deterministic firmware simulation — would Continue users
  use this?" with the first three paragraphs and a link to the repo. Public
  discussion is itself useful even if no founder replies.
- If no email reply by 2026-05-26: follow-up template, single bump, then drop
  for 60 days.

---

## Email 4 — Replit (Agent team)

**Recipient:** The Replit Agent product team (no named individual; see notes).
**Channel:** `developers@replit.com` for the team-level send. Parallel
LinkedIn DM to whoever currently lists "Replit Agent" in their title at the PM
or eng-lead level. Amjad is reachable but too senior for a first cold ping; he
will route or ignore.

**Subject:** Deterministic firmware sim as a Replit Agent tool

**Body:**

Hi — writing to the Replit Agent team. Could not identify the current product
owner from public info, so addressing the team rather than guessing a name.

LabWired: cycle-accurate, deterministic firmware simulator for ARM Cortex-M,
RISC-V, and Xtensa MCUs. Open-source Rust core, MIT. Determinism enforced by
a CI gate that SHA-256-compares five runs of identical inputs — same
`result.json`, every time, on any host.

Fit: Replit Agent already runs in a hosted environment where "agent verifies
its own work before handing back" is most of the product surface. Hardware
and firmware projects are the one domain where the verify step currently
breaks, because there is no determinism oracle the agent can call. LabWired
fills that slot.

MCP server: `npx -y @labwired/mcp`. Three tools, stdio, drops into any MCP
host. Concrete proof point is an STM32H563 GPDMA stress case where HIL hides
the bug under sampling jitter and the simulator catches it every run — happy
to walk through it on a call. Rust core open at
`github.com/w1ne/labwired-core`.

Ask: 30 minutes with whoever owns Agent's tool surface. Design-partner terms
if useful — free Pro for the team, founder integration support, priority on
the tools you want next.

Andrii

**Send notes:**
- Primary send: `developers@replit.com` with the subject above. If Replit no
  longer staffs that alias, fall back to a LinkedIn DM to the most senior IC
  with "Replit Agent" in the headline.
- Parallel: light-touch post on Replit's community forum titled "Has anyone
  tried wiring a deterministic firmware simulator into Replit Agent?" — keeps
  the topic visible even if the email vanishes.
- If no reply by 2026-05-26: single follow-up bump, then drop for 60 days. Do
  not escalate to Amjad on a cold path.

---

## Email 5 — Personal embedded engineering contact

**Fill-in checklist before sending:**

- `<NAME>` — first name of the person. They already know who Andrii is, so no
  bio paragraph.
- `<COMPANY>` — the team or product. Used twice.
- `<SHARED CONTEXT LINE>` — one sentence that anchors the relationship: a
  project you worked on together, a thread on a specific bug, a time you
  helped them, a conference you both attended. This replaces the cold-intro
  framing entirely.
- `<THEIR DOMAIN PAIN>` — one concrete thing you remember them complaining
  about that LabWired touches (HIL flakiness on a specific peripheral, CI
  times, a regression that ate a sprint, a flaky DMA bug, etc).
- Channel: whatever you used last — Signal, WhatsApp, personal email, Slack
  DM. Do not switch channels for this; you are leveraging existing trust.
- If they are at a company whose agent or CI team might also benefit, mention
  it as a soft second ask ("if this is interesting and the agent / platform
  team at `<COMPANY>` would benefit, an intro is welcome — no pressure").

**Subject:** LabWired MCP is shipped — want to be design partner #1?

**Body:**

`<NAME>` — `<SHARED CONTEXT LINE>`.

Quick update on the thing I have been building. LabWired is now an actual
working product: cycle-accurate, deterministic firmware simulator for
Cortex-M, RISC-V, Xtensa. Rust core, MIT. The piece that finally made it
useful to other people shipped this week — `@labwired/mcp`, a Node.js MCP
server that drops into Claude Code, Cursor, or Continue with one config line.
Three tools: catalog, simulate, validate. `npx -y @labwired/mcp`.

Writing you specifically: you have been dealing with `<THEIR DOMAIN PAIN>` for
as long as I have known you, and a deterministic oracle is the actual fix.
Same firmware plus same system plus same script gives byte-identical
`result.json` — enforced by a CI gate that SHA-256-compares five runs.

What I want: 30-minute call where you try the MCP server against a real bug
from `<COMPANY>` and tell me what is missing. In return: free Pro for the
partnership, founder-led integration help, your name on the tools we add next.

If the timing is bad, say so and I will check back in three months.

Andrii

**Send notes:**
- Channel: whichever one you use with `<NAME>` already. Do not introduce a new
  one.
- If they reply "sure, let's talk," send a Cal.com link inside 24 hours. Lag
  here kills warm leads faster than cold ones.
- If they reply "interesting but not now," ask one question: "what would have
  to change for it to be relevant?" Their answer is product feedback even if
  the call never happens.

---

## Follow-up template (5 business days later, single bump)

Send as a reply to the original thread, not a new email. Keep it shorter than
the original.

**Subject:** (Re: original subject — same thread)

**Body:**

Bumping this in case it got buried. Short version: deterministic firmware
simulator with an MCP surface, `npx -y @labwired/mcp`, three tools, drops into
any MCP-aware client. Looking for one or two design partners — not raising.

If the fit is not there, a one-line no is genuinely useful and I will not
ping again. If the right person to talk to is somewhere else on the team, an
intro is welcome.

Andrii

**Rules:**
- One bump only. No third email on a cold thread.
- If they bounce or hard-no, drop for 60 days minimum.
- If they reply but defer, ask for the specific blocker and log it. That is
  the feedback the pitch deck is missing.

---

## What to watch for in replies

**Signals an interested prospect:**

- They ask a specific technical question — "does it model the STM32U5 LPTIM
  glitch?" or "how do you handle non-deterministic peripherals like a real
  radio?" Technical specificity means they are mapping it onto a real bug.
- They forward you to a named IC — "talk to <person>, they own our agent's
  tool layer." A name plus a routing is the strongest possible cold-reply
  signal short of "send a calendar link."
- They ask for the repo, the README, or the determinism-proof CI artifact.
  Engineers checking the receipts are engineers about to take the call.
- They reply outside business hours. Weekend or late-night replies from senior
  people are almost always genuine interest, not delegation.

**Signals a polite brush-off:**

- "Interesting, we will keep this on file." There is no file. Drop 60 days,
  re-approach with a new artifact.
- "Please send more info / a deck." Send the pitch link once. Silence after
  that means the deck was the brush-off.
- Reply from a generic intake address. Treat as bounce, find a human path.
- 10+ business day lag followed by a polite no-action reply. Do not push.

**Signals to walk away entirely:**

- "We are building this internally." Believe them, log it, check back in 6
  months — most internal builds stall.
- "Send us a proposal / SOW / pricing." Procurement path, not design-partner
  path. Decline politely; the wrong customer now is more expensive than none.
- Silence after the follow-up bump. Move on.
