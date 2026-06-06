# Moat Refinement — Simulation Incumbents Audit + the Validation-Corpus Pillar

**Date:** 2026-06-06
**Extends:** `competitive_analysis_2026-06-03.md` (embedder / bootloop / blueprint.am). That doc settled the agent-loop landscape and the call "your moat is the verification oracle, not the LLM." This doc adds the axis it didn't cover — the **simulation incumbents** (Wokwi, Renode, Espressif QEMU) — and names a moat pillar created by the ESP32-S3 proper-model work merged to main today (labwired-core PR #176): **published silicon validation**.

**Decision context:** "Should labwired-core go closed source?" Answer below: **no** — the code is the least defensible layer; the moat is validation evidence + workflow position + the corpus. This doc specifies what stays public, what stays private, and why.

---

## 1. Simulation-incumbent audit (researched 2026-06-06, sources verified)

### Wokwi — the only commercial simulator comp
- Pricing: free 50 CI min/mo → Hobby $7 → Hobby+ $12 → **Pro $25/seat/mo with 2,000 CI min/mo** (wokwi.com/pricing). VS Code extension is licensed commercial; 273K installs. wokwi-cli has 51★ — CI adoption is early.
- Open/closed split: toy cores open (avr8js, rp2040js), **flagship ESP32 engine closed**; community extends only via WASM Chips API.
- **ESP32-S3 fidelity gaps:** no MCPWM, no I2S, no BLE, no USB-OTG (CDC only), I2C master-only, RMT TX-only, CPU timing explicitly approximated. **No silicon-validation claim anywhere.**
- CI is **cloud-only and metered**; firmware binaries upload to their cloud (IP-sensitive teams blocked); no self-hosted runner. Metered minutes are antithetical to agent-scale parallel iteration.
- Agent story: one **experimental** MCP flag in wokwi-cli (docs.wokwi.com/wokwi-ci/mcp-support).

### Renode (Antmicro)
- Business = consulting only (no SKUs, no cloud product). Precedent worth copying: **Google paid Antmicro** to add Xtensa support — funded model development is a real revenue lane with an open core.
- **No ESP32-S3 SoC support at all** (core translation only; issue #262 open since 2018; zero Espressif boards on the supported list).
- Validation story: "Zephyr samples pass on 470 boards" — regression-by-proxy, never model-vs-silicon evidence.

### Espressif QEMU
- S3 boots (dual-core, flash, crypto, GDMA, systimer) but **stubs exactly what labwired now models faithfully: I2C (open bug), GP-SPI, RMT, I2S, MCPWM, GPIO-matrix fidelity, WiFi/BT/USB**. Espressif disclaims support. `idf.py qemu` + pytest-embedded-qemu = the integration surface to ride (see §4).
- ESP-IDF v6.0 ships an **MCP server at the toolchain layer** (build/flash/target) — vendor appetite for agent workflows confirmed; **the simulation layer of that stack is unclaimed**.

### Demand-side evidence (for the agent-substrate thesis)
- Practitioner discourse names our exact wedge: "hallucinated registers compile fine and only fail when you flash"; firmware has no hot-reload loop (HN embedder thread, gocodeo, dev.to). HIL benches: "$500K–$10M, wait days or weeks" (ministryoftesting).
- Memfault = observability only; **field-crash → deterministic simulated repro is an open white space** (their own Interrupt blog teaches Renode CI as community content, no product).

**Net:** nobody — commercial or open — offers validated-fidelity ESP32-S3 simulation, self-hosted unmetered parallel CI, or an agent-native simulation substrate. All three positions are open simultaneously.

---

## 2. The moat, ranked honestly

The 2026-06-05/06 S3 sessions proved an uncomfortable fact: **an agent + SVD + the coverage probe builds a faithful peripheral register file in hours** (SYSTEM, I2C1, GP-SPI, MCPWM, SENS each landed in one session). Model *code* is therefore a collapsing-cost asset — for us and for any competitor with the same tools. Closing the source protects the cheapest-to-replicate layer. What compounds instead:

1. **Silicon-validation corpus** (NEW pillar, created this week). JTAG reset-halt dumps, register-trace diffs vs physical boards, "unmodified firmware rom-boots bit-identical through the 2nd-stage bootloader," the un-gameable behavioral coverage probe + ratchet. Requires bench hardware + method discipline; a code fork gets the models but **not the evidence**. This directly answers the 06-03 doc's erosion risk ("catalog advantage shrinks if competitors open-source chip libraries") — the catalog can be copied, the validation provenance cannot.
2. **Verification-oracle position in the agent loop** (the 06-03 thesis, unchanged): deterministic, parallel, MCP-native. Embedder/bootloop iterate on stochastic real hardware; Wokwi is metered cloud. We are the only ones who can run 50 deterministic instances per PR locally.
3. **Workflow embedment:** GH Action (shipped), `pytest-embedded-labwired` (§4), VS Code DAP (shipped), MCP (shipped, 11 tools).
4. **Regression corpus:** every real firmware that boots becomes a fixture with golden traces; accrues to whoever has users, unforkable.
5. Model code: table stakes. Keep it open — it buys trust + distribution against a closed Wokwi.

---

## 3. Public / private boundary rule

| Layer | Disposition | Rationale |
|---|---|---|
| Engine (bus, cores, scheduler, snapshots, gdbstub, probe, ratchet) | **Public, MIT** | Distribution + trust vs closed Wokwi; already irrevocably published |
| ESP32-S3 model **to completion** | **Public** | Already out; it's the credibility demo; inconsistent to half-close it |
| Community/new chip models (code) | **Public by default** | Collapsing-cost asset; openness invites the Renode-style contribution tail |
| **Raw HW-oracle data + rigs tooling** (JTAG dumps, trace-diff lane, board farm scripts) | **Private** | The moat. Publish *digests*: per-peripheral "HW-validated ✓" badges + methodology paper, keep raw traces + automation private |
| Hosted/parallel CI orchestration, fleet runner | **Private** (this repo / packages) | The paid product |
| Field-crash → sim-repro bridge (future, Memfault white space) | **Private** | Premium product |
| Strategy/specs like this doc | **Private** (this repo) | Never in labwired-core/docs (public) |

**Standing rule for the agents doing model work:** model code → public core; validation *evidence pipeline* → private; every faithful slice publishes its badge, not its raw traces.

---

## 4. Productization deltas (ordered; only what the 06-03 doc doesn't already cover)

1. **Validation trust surface (cheap, unique, do first):** per-peripheral validation badges in the public README/coverage matrix (`HW-validated ✓` vs `SVD-sourced`), a methodology page, and a published example oracle trace. No one in the field can copy this page truthfully. Today's S3 work is the launch content.
2. **Self-hosted unmetered CI runner as the anti-Wokwi pitch:** "your firmware never leaves your repo; unlimited parallel minutes." Their $25/seat + 2,000 min + binary-upload model is the foil. Price our hosted tier against it ($19/seat already set).
3. **`pytest-embedded-labwired`:** drop-in for pytest-embedded-qemu so ESP-IDF teams swap `idf.py qemu` for labwired with one config line — rides Espressif's own test framework into their QEMU fidelity gap, and composes with their new toolchain MCP.
4. **MCP v2 = fleet primitives:** the 11 tools are single-session; agents need spawn-N-instances, deterministic seeds, coverage/trace artifacts as tool outputs. (Counter to Wokwi's experimental flag while it's still experimental.)
5. Funded-model-development lane (Antmicro precedent): a customer pays for chip X bring-up; code lands public, their fixtures/validation stay private to them. Revenue without closing anything.

## 5. Wedge v2 (post-roast, same day)

The §4 wedge ("agent execution substrate for firmware teams") failed the roast on four counts: (a) the agent-firmware workload barely exists yet — the only entities running it in anger are embedder/bootloop, i.e. competitors; (b) the fidelity lead (MCPWM/I2C/SPI/LEDC/SENS) is orthogonal to the modal S3 firmware's critical path (WiFi/BLE/USB — unsimulated here, WiFi full-stack at Wokwi); (c) "self-hosted unmetered MIT" un-monetizes itself — nothing excludable was named; (d) **zero documented cases of labwired catching a real bug in real user firmware** — the MCPWM stride find was a bug in our own model. Instant-completion semantics (SPI in 0 cycles, ADC constant 0x800) can *hide* the timing bugs buyers fear; the honest catch-class today is driver bring-up / register misuse / init ordering — real, narrow, undemonstrated.

**Revised wedge — prove it on ourselves, then sell the story:**

1. **Dogfood-proof:** "An agent developed, tested, and shipped firmware for a real, shipping product (SpiceDispenser) with the simulator as its only hardware." The dispense path is I2C/PCA9685/servo — squarely inside the fidelity lead, no WiFi dependency. Deliverables: labwired gate in the SpiceDispenser firmware CI (rom-boot + dispense-path assertion on every PR); first real regression caught in CI → write-up; demo video of the loop.
2. **Named beachhead, not "firmware teams":** driver-bringup CI for the S3 peripherals QEMU stubs and Wokwi lacks — motor control (MCPWM), sensors (I2C/SENS), addressable-LED/RMT. Findable buyers (ESP32 robotics/mechatronics), demos that don't die at `esp_wifi_init()`.
3. **The excludable thing, in one sentence:** *the workbench is free; the worker and the proof are paid.* Self-host the simulator forever; pay for (a) the closed-loop firmware agent product on top (06-03 doc expansion #1), (b) hosted fleet + golden-trace/regression-corpus service, (c) funded chip bring-up, (d) support SLA. $19/seat attaches to (a)/(b), never to the MIT engine.
4. **WiFi/BLE is the acknowledged ceiling**, scoped out of the wedge explicitly; S3 radio twin (reusing the classic-ESP32 SimNet work) is the post-wedge expansion that unlocks the modal IoT segment.
5. **Proof-artifact bar for all marketing:** no claim ships without its artifact — the badge page launches only when ≥1 real-firmware-bug write-up exists, so the trust surface never outruns the evidence (the same truth-discipline as the coverage probe).

## 6. Decision record

- **labwired-core stays public MIT.** Closing it protects the wrong layer, kills the trust wedge against Wokwi's closed core, and can't retract what's published (everything through PR #176 is out, 2 forks exist).
- Moat = §2 ranking; go-to-market = §5 wedge v2 (dogfood-proof first, named beachhead, excludable = agent product + hosted proof services); §4 items re-ordered beneath it.
- Revisit trigger: a competitor publishes silicon-validation evidence, or a paying customer demands a private chip model (then use §4.5 lane, not relicensing).
- Next concrete action: labwired gate in SpiceDispenser firmware CI (wedge deliverable 1).
