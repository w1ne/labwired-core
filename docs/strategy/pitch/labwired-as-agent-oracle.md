# LabWired as a Deterministic Oracle for Coding Agents

A design-partner proposal for AI lab tech leads.

Andrii Shylenko · andrii@shylenko.com · <https://github.com/w1ne/labwired-core>

---

## The problem your agent has on embedded code

An autonomous coding agent can read a firmware diff, hypothesise a fix for an
ADC race or a DMA underrun, and produce a patch that compiles cleanly and
*looks* correct. Then it stops, because it has no fast way to know whether the
patch actually works.

The current options for the verification step are all bad:

- **HIL bench.** Slow to wire up, slow to run, flaky under cable jitter and
  power noise. Bugs that depend on cycle-exact peripheral timing disappear under
  the logic analyzer. You cannot put one in a CI loop.
- **Best-effort simulators (QEMU, generic emulators).** Non-deterministic
  scheduling. Re-running the same firmware gives different results. An assertion
  passing in run #1 and failing in run #2 is not useful feedback for an agent.
- **LLM self-evaluation.** "I'm confident this fixes it" is not an oracle.

The result is that agents either ship patches without verifying them (and the
human on the other end has to do the HIL loop manually), or they stall. Both
outcomes cap how far an agent can go on embedded, IoT, and robotics work.

## What LabWired is

A cycle-accurate, deterministic firmware simulator for ARM Cortex-M, RISC-V,
and Xtensa MCUs — STM32, RP2040, ESP32, nRF52 families. Open-source Rust core,
MIT licensed.

Determinism is not a marketing claim. It is enforced by a CI gate in
`core-ci.yml` called `determinism-proof` that runs the same firmware + manifest
+ script five times and SHA-256-compares the result artifacts and traces. The
gate has to pass for a release to ship. Same inputs, identical `result.json`,
every time, on any host.

This makes LabWired a usable oracle: the agent gets a verdict it can trust,
fast enough to live inside the inner loop of an iterate-and-verify cycle. The
concrete shape we keep hitting in practice is a GPDMA stress case on
STM32H563ZI where logic-analyzer-driven HIL debugging hides the bug under
sampling jitter and LabWired surfaces it on every run.

## How the agent integration looks today

`@labwired/mcp` ships as a Node.js MCP server over stdio. It works with Claude
Code, Cursor, Continue, and any client that speaks Model Context Protocol. The
README shows the install one-liner. It exposes three tools:

- `labwired_catalog` — list supported chips/boards (filter by name). Free, no
  key.
- `labwired_simulate` — run a base64 ELF against a System Manifest YAML and a
  test script YAML. Returns `result.json` (assertions, exit status, cycles
  consumed) plus the captured UART log.
- `labwired_validate_system` — schema-check a manifest before you spend cycles
  simulating with it.

Tool surface is published as `@labwired/mcp` on npm — install with
`npx -y @labwired/mcp` to inspect the schemas. The cycle-metering and billing
Worker lives behind `api.labwired.com`; we'll share the Worker source under
NDA on request. Stripe checkout issues the API key and surfaces it in the
customer's private cabinet on return — no email handoff. Pro is
$19/seat/month, 100M cycles/month included. Local CLI runs are free.

### The loop, concretely

Bug report: *"On STM32F103, our SPI DMA RX callback is dropping the last byte
intermittently."* The agent:

1. Calls `labwired_catalog` with filter `"stm32f1"` to confirm the target is
   supported.
2. Reads the firmware, hypothesises that the DMA `TCIF` ISR is being cleared
   too late.
3. Calls `labwired_validate_system` with the project's `system.yaml` to make
   sure the bench description still parses.
4. Builds the patched ELF, base64-encodes it, calls `labwired_simulate` with a
   script that asserts the full RX buffer matches a known pattern over 10k
   iterations.
5. Reads `result.json`. If `exit_code == 0` and all assertions pass, the patch
   ships. If not, the UART log and the failed assertion tell it where to look
   next.

The whole loop is mechanical from the agent's side. No human in it. No HIL
queue. No "I think this is right."

## Why the playground matters for agent-generated work

`foundry.labwired.com/playground/` is a browser-based React playground —
nine working firmware labs across eight boards, no install. Today it is a
human-facing surface. The direction we are taking it: every board + firmware +
system manifest the agent produces via MCP becomes a shareable playground URL.
Wokwi did this for prototyping; we want it for the agentic verification path.

The flow we are building toward:

- Headless: agent runs the MCP loop, verifies the fix, attaches `result.json`
  and a VCD trace to the PR.
- Human: reviewer opens the playground link from the PR description, sees the
  exact same board, firmware, and script the agent saw, hits Run, watches it
  execute identically. No "works on my bench" gap.

This part is forward-looking. The headless MCP loop works now; the
share-as-URL surface is the next milestone.

## What we are asking

A design-partner agreement. Specifically:

- A 30-minute technical call to walk your team through the MCP integration on a
  firmware bug from your domain.
- If it is useful, a joint demo and a named quote we can put on the site.
- A feedback channel into what tools and assertions your agent actually wants
  exposed (memory inspection? cycle budgets per peripheral? structured trace
  queries?). We will prioritise those.

In exchange:

- Free Pro access for your team for the duration of the partnership.
- Founder-led integration support — direct line to me, not a ticket queue.
- Priority on the tools you ask for. The current three-tool surface is the
  minimum useful set; expanding it is the next quarter of work and you get to
  shape it.

We are not raising. We are not pitching for procurement. We are looking for one
or two AI labs to validate that the deterministic-oracle shape is the right
primitive for autonomous embedded coding, before locking the API.

## Why now

Agentic coding is starting to touch hardware. Embedded, IoT, robotics, and
device firmware are the next surface after web and backend. Whoever owns the
deterministic verification layer for that surface becomes infrastructure —
the way Vercel became infrastructure for the frontend agent layer.

The piece that has to exist for any of this to work is a fast, reproducible
oracle the agent can call inside its loop. That piece does not exist in QEMU,
does not exist in HIL, and cannot exist in LLM self-evaluation. It exists in
LabWired today, in MVP form, with the MCP surface already shipped.

## Honest status

Solo founder. Pre-revenue. v0.1 of the MCP server. Nine working labs across
eight boards. Stripe and the API worker are live; the first paying customer
has not happened yet. The Rust core has a real test suite and a real
determinism gate, but the simulator's chip coverage is narrower than it will
need to be a year from now.

This is exactly the stage where a design partner shapes the product. Six
months from now the API surface will be harder to move. Today it is not.

If the problem framing above matches something your agent team is hitting,
reply and I will send a calendar link.

— Andrii
