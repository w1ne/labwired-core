# LabWired vs Digital Twin Simulators (Deep Comparison)

Date: February 22, 2026  
Author: Codex research synthesis  
Scope: Embedded firmware/system simulation platforms and adjacent industrial digital twin platforms

Daily operating view (lean): `docs/spec/COMPETITIVE_EXECUTION_ONE_PAGER.md`

## 1) Executive Summary

LabWired is currently best positioned in the **deterministic embedded firmware simulation for CI** segment, not in the broad industrial asset-graph digital twin segment.

The most relevant direct competitors are:
- QEMU
- Renode
- Wokwi
- Intel/Wind River Simics
- Synopsys Virtualizer / VDK
- dSPACE VEOS
- ETAS COSYM

The adjacent (different category) platforms are:
- AWS IoT TwinMaker
- Azure Digital Twins
- Ansys Twin Builder

The key strategic conclusion is:
- **Win zone for LabWired**: config-first board/chip modeling, deterministic replayability, agent-first CI workflows, and fast board onboarding.
- **Parity gap to close first**: richer peripheral library coverage, multi-node/network simulation ergonomics, and enterprise-grade model ecosystem tooling.
- **Avoid scope drift**: cloud ontology/enterprise graph platforms (AWS/Azure class) should be integration targets, not core product scope.

## 2) Methodology

This report uses:
- Primary vendor documentation/product pages for external platforms.
- LabWired repository docs for current internal capability baseline.

External claims are dated "as of February 22, 2026" and should be revalidated before pricing or procurement decisions.

## 3) LabWired Baseline (Current State)

From `README.md`, `core/docs/architecture.md`, and related docs:
- Deterministic CPU+bus simulation architecture with pluggable CPU trait.
- Current architecture support includes Cortex-M (ARMv7-M/Thumb-2 path) and RV32I.
- Config-first modeling (`chip` + `system` YAML manifests).
- CI-oriented execution and artifacts (`labwired test`, YAML script runner).
- GDB RSP and DAP integration for debugging.
- Current documented peripheral focus: `uart`, `gpio`, `timer`, `rcc`, and generic MMIO stubs.

Implication:
- LabWired is already aligned with developer-in-loop and CI-in-loop firmware validation.
- It is not yet a full vehicle/factory/enterprise twin platform, and should not be positioned as one.

## 4) Market Segmentation: Direct vs Adjacent

### 4.1 Direct Comparison Class: Embedded Virtual Platforms

These tools execute unmodified firmware/software binaries against virtualized SoCs/boards and are the closest basis for head-to-head comparison:
- QEMU
- Renode
- Wokwi
- Simics
- Synopsys Virtualizer
- VEOS
- COSYM

### 4.2 Adjacent Class: Enterprise/Industrial Twin Platforms

These tools center on graph/asset/3D operational twins and IoT data fusion, not low-level MCU firmware emulation:
- AWS IoT TwinMaker
- Azure Digital Twins
- Ansys Twin Builder (bridging physical simulation and deployment)

These are more likely **integration channels** than direct substitutes for LabWired core.

## 5) Deep Competitor Analysis (Direct Class)

## 5.1 QEMU

What it is:
- General-purpose emulator with broad architecture coverage and system/user mode emulation.

Strengths:
- Massive ecosystem maturity and very wide target support.
- Strong open-source base (GPLv2).
- Highly scriptable and deeply integrated into many OSS workflows.

Weaknesses vs LabWired target use-case:
- Board/peripheral extension effort can be high for new embedded targets.
- MCU-level deterministic test ergonomics are not its strongest default UX.
- Not opinionated toward firmware CI workflows out of the box.

Strategic read for LabWired:
- Compete on modeling velocity, deterministic firmware test UX, and config-first onboarding.

## 5.2 Renode

What it is:
- Open-source embedded/IoT system simulation framework focused on full-system virtual platforms and multi-node scenarios.

Strengths:
- Strong embedded positioning and unmodified binary workflows.
- Mature Robot Framework-based automated testing and CI story.
- Broad architecture support and complex system simulation orientation.
- MIT license.

Weaknesses vs LabWired opportunity:
- Higher conceptual complexity for teams wanting minimal config-first bring-up.
- Opportunity for LabWired to offer tighter deterministic/agent-first loops and lighter onboarding flow.

Strategic read:
- Renode is the closest open competitor for embedded CI simulation depth.
- If LabWired cannot match core workflow smoothness plus deterministic throughput, adoption will stall.

## 5.3 Wokwi

What it is:
- Browser-first embedded simulator with strong developer UX and broad educational/prototyping adoption.

Strengths:
- Extremely low friction onboarding.
- Strong supported-board catalog for Arduino/ESP32/RP2040/STM32 classes.
- CI and IDE integrations available.
- Custom chip extensibility via Chips API/WASM.

Weaknesses vs LabWired enterprise trajectory:
- Positioned primarily for prototyping/education and maker-to-pro workflows.
- Less oriented to deep pre-silicon or safety-style evidence workflows.

Strategic read:
- Wokwi is a UX benchmark. LabWired should borrow onboarding simplicity while keeping enterprise determinism depth.

## 5.4 Intel/Wind River Simics

What it is:
- Longstanding full-system virtual platform simulator for pre/post-silicon software development and integration.

Strengths:
- Enterprise-grade tooling around platform control, debugging, and checkpointing.
- Mature virtual platform architecture and commercial services ecosystem.
- Strong usage in large-scale silicon and systems organizations.

Weaknesses vs LabWired opportunity:
- Enterprise-heavy stack with higher adoption overhead for smaller teams.
- Commercial model/tooling stack can be costly relative to OSS-first alternatives.

Strategic read:
- Simics defines the high-end reference for virtual platform rigor.
- LabWired can attack from below with faster adoption, transparent config model, and CI-native usage.

## 5.5 Synopsys Virtualizer / VDK

What it is:
- Commercial virtual prototyping suite producing virtual development kits (VDKs), especially strong in automotive and pre-silicon shift-left.

Strengths:
- Strong model ecosystem and unmodified production binary workflows.
- Explicit positioning for development before silicon availability.
- Deep commercial alignment with SoC/automotive programs.

Weaknesses vs LabWired opportunity:
- Access and cost profile are enterprise-centric.
- Integration, model procurement, and vendor coupling can be heavy for smaller product teams.

Strategic read:
- Competing directly requires differentiated time-to-first-value and cost-to-scale advantage.
- LabWired should avoid feature-by-feature imitation; focus on fast deterministic CI and open model portability.

## 5.6 dSPACE VEOS

What it is:
- PC-based simulation platform for automotive SIL with virtual ECUs, vehicle models, and communication buses.

Strengths:
- Strong automotive bus-level simulation (CAN/LIN/Ethernet) and toolchain integration.
- Explicit reproducibility/synchronization messaging.
- Safety-oriented ecosystem positioning.

Weaknesses vs LabWired opportunity:
- Primarily automotive domain concentration.
- Typically part of larger proprietary toolchain investments.

Strategic read:
- VEOS is a target benchmark if LabWired pursues automotive SIL workflows.

## 5.7 ETAS COSYM

What it is:
- Co-simulation/integration platform for ECU development (MiL/SiL, closed-loop support, standards integration).

Strengths:
- Strong co-simulation interoperability with open standards (ASAM XiL, FMI, etc.).
- CI/CD and cloud/on-prem framing.
- Good fit for model/toolchain-heavy automotive environments.

Weaknesses vs LabWired opportunity:
- Integration complexity is non-trivial.
- Strongest fit remains automotive engineering workflows rather than broad embedded developer workflows.

Strategic read:
- COSYM demonstrates how valuable standards-based interoperability is in enterprise contexts.
- LabWired should consider selective interoperability points without adopting full complexity.

## 6) Adjacent Platform Analysis (Not Direct Replacements)

## 6.1 AWS IoT TwinMaker

Core value:
- Operational digital twin service focused on knowledge graph + 3D scene + data connectors.

Why adjacent:
- Focus is plant/building/equipment operations and data fusion, not MCU instruction-level firmware execution.

LabWired implication:
- Treat as downstream integration surface for operational dashboards, not as core emulator competitor.

## 6.2 Azure Digital Twins

Core value:
- PaaS twin graph using DTDL models and event-driven integration across IoT/business systems.

Why adjacent:
- Graph semantics and cloud data orchestration, not low-level firmware/bus emulation.

LabWired implication:
- Potential bridge: export simulation metadata/events to DTDL-compatible twin graphs for enterprise observability.

## 6.3 Ansys Twin Builder

Core value:
- Physics/system modeling with MiL/SiL/HIL style integration and deployment workflows.

Why partially adjacent:
- Closer to engineering simulation than cloud twin graphs, but still typically system/physics/control co-sim rather than MCU board emulation-first workflow.

LabWired implication:
- Could be partner/integration path for teams combining firmware validation and plant/physics models.

## 7) Capability Matrix (As of 2026-02-22)

Legend:
- `High`: strong explicit capability/evidence in primary sources.
- `Medium`: present but not core differentiator in sources.
- `Low`: limited evidence in the sourced material or outside platform focus.

| Platform | Core Focus | Deterministic CI Testing | Board/SoC Modeling Flexibility | Multi-node/System Co-Sim | Cloud/App Twin Graph | Typical Access Model |
|---|---|---:|---:|---:|---:|---|
| LabWired | Embedded firmware simulation | High | Medium (improving) | Medium (emerging) | Low | OSS project / local tooling |
| QEMU | General emulation/virtualization | Medium | High | Medium | Low | OSS (GPLv2) |
| Renode | Embedded/IoT system simulation | High | High | High | Low | OSS (MIT) + services |
| Wokwi | Browser embedded simulation | Medium-High | Medium | Low-Medium | Low | SaaS + paid tiers |
| Simics | Enterprise virtual platforms | High | High | High | Low | Commercial + Intel public release |
| Synopsys Virtualizer | Pre-silicon virtual prototyping | High | High | High | Low | Commercial enterprise |
| dSPACE VEOS | Automotive SIL virtual ECUs | High | Medium-High | High | Low | Commercial enterprise |
| ETAS COSYM | Automotive co-simulation (MiL/SiL) | High | Medium-High | High | Low | Commercial enterprise |
| AWS IoT TwinMaker | Operational digital twin graph/3D | Medium | Low (firmware-level) | Medium | High | Managed cloud service |
| Azure Digital Twins | Twin graph/ontology platform | Medium | Low (firmware-level) | Medium | High | Managed cloud PaaS |
| Ansys Twin Builder | Physics/system digital twins | Medium-High | Medium | High | Medium | Commercial engineering suite |

## 8) Where LabWired Should Attack

## 8.1 Near-Term (0-2 quarters)

1. Double down on deterministic CI as first-class product surface.
2. Expand high-value peripheral/model coverage for top MCU families (practical parity path).
3. Make onboarding and first-pass model authoring dramatically faster than Renode/QEMU.
4. Publish repeatable benchmark suite (time-to-first-pass, model bring-up time, CI throughput).

## 8.2 Mid-Term (2-6 quarters)

1. Introduce stronger multi-node/network simulation ergonomics where customer pull is real.
2. Add model packaging/versioning ecosystem that supports enterprise reuse.
3. Provide optional interoperability adapters into enterprise twin stacks (AWS/Azure/Ansys), not full reimplementation.

## 8.3 Positioning Guardrails

1. Position as "firmware/system virtual platform for deterministic engineering validation".
2. Avoid broad "everything digital twin" messaging that dilutes product category clarity.
3. Sell concrete outcomes: regression velocity, deterministic defect reproduction, and CI cost/time reduction.

## 9) Risks and Blind Spots

1. Overpromising fidelity beyond current peripheral/model depth will damage credibility.
2. Competing head-on with enterprise incumbents on full-stack feature breadth is a capital trap.
3. Underinvesting in ecosystem usability (docs, templates, onboarding, examples) will forfeit adoption even if engine quality is high.

## 10) Recommended Internal KPIs for This Strategy

1. Time-to-first-UART-smoke for a new board (minutes).
2. Board onboarding lead time from public docs to deterministic run (hours/days).
3. CI simulation throughput and variance across runs.
4. Unsupported-instruction/peripheral access rates by firmware suite.
5. Deterministic replay success rate across host environments.

## 11) Merged Scoring Analysis (Baseline + ICP)

Scoring scale:
- `1` = weak fit
- `3` = moderate fit
- `5` = strong fit

Weighted score formula:
- `Total = Σ(weight_i * score_i)` where weights sum to 100.

Note:
- This is a strategic fit model from published capabilities (not a lab benchmark).
- Scores should be recalibrated after hands-on pilots.

### 11.1 Baseline Segment A: Embedded Product Team (CI-first firmware validation)

Criteria and weights:
- Deterministic regression behavior: 25
- Time-to-first-value/onboarding friction: 20
- Peripheral/board coverage: 20
- CI automation ergonomics: 15
- Debug tooling depth: 10
- Cost/accessibility for team scale: 10

Platforms scored: LabWired, Renode, QEMU, Wokwi, Simics, Synopsys Virtualizer

| Platform | Determinism (25) | Onboarding (20) | Coverage (20) | CI UX (15) | Debug (10) | Cost/Access (10) | Weighted Total (/500) |
|---|---:|---:|---:|---:|---:|---:|---:|
| Renode | 5 | 3 | 5 | 5 | 4 | 5 | 445 |
| LabWired | 5 | 4 | 3 | 5 | 4 | 5 | 425 |
| Wokwi | 4 | 5 | 4 | 4 | 3 | 4 | 410 |
| QEMU | 3 | 2 | 5 | 3 | 4 | 5 | 345 |
| Simics | 5 | 2 | 5 | 4 | 5 | 1 | 390 |
| Synopsys Virtualizer | 5 | 2 | 5 | 4 | 5 | 1 | 390 |

Ranked shortlist (Segment A):
1. Renode
2. LabWired
3. Wokwi
4. Simics / Synopsys Virtualizer (tie; enterprise-heavy)
5. QEMU

Strategic interpretation for LabWired:
- LabWired is already top-tier on determinism + CI ergonomics.
- The largest score drag is peripheral/board coverage breadth versus mature incumbents.
- Closing coverage gaps likely yields the highest ranking improvement per engineering effort.

### 11.2 Baseline Segment B: Automotive/Enterprise Virtual Validation Program

Criteria and weights:
- Toolchain interoperability/co-sim ecosystem: 25
- Determinism/reproducibility: 20
- Safety-process fit and evidence workflows: 20
- Multi-node/vehicle-scale simulation fit: 20
- Cost/flexibility: 15

Platforms scored: dSPACE VEOS, ETAS COSYM, Simics, Synopsys Virtualizer, Renode, LabWired

| Platform | Interop (25) | Determinism (20) | Safety Fit (20) | Multi-node (20) | Cost/Flex (15) | Weighted Total (/500) |
|---|---:|---:|---:|---:|---:|---:|
| Synopsys Virtualizer | 5 | 5 | 5 | 5 | 1 | 440 |
| dSPACE VEOS | 5 | 5 | 5 | 5 | 1 | 440 |
| ETAS COSYM | 5 | 5 | 5 | 5 | 1 | 440 |
| Simics | 4 | 5 | 4 | 5 | 1 | 395 |
| Renode | 3 | 5 | 3 | 4 | 5 | 400 |
| LabWired | 2 | 5 | 2 | 3 | 5 | 325 |

Ranked shortlist (Segment B):
1. Synopsys Virtualizer / dSPACE VEOS / ETAS COSYM (tie)
2. Renode
3. Simics
4. LabWired

Strategic interpretation for LabWired:
- LabWired is not yet the best primary choice for full automotive enterprise toolchain replacement.
- Practical entry strategy is targeted wedge adoption: deterministic CI firmware validation layer that coexists with incumbent enterprise stacks.

### 11.3 Baseline Segment C: Cloud/Operational Digital Twin Programs

Criteria and weights:
- Asset graph/ontology capabilities: 30
- Cloud integration/data connectors: 25
- Operational dashboard/3D/context tooling: 20
- Firmware-level simulation depth: 15
- Cost/flexibility: 10

Platforms scored: AWS IoT TwinMaker, Azure Digital Twins, Ansys Twin Builder, LabWired

| Platform | Graph/Ontology (30) | Cloud Connectors (25) | Ops UI/3D (20) | Firmware Sim (15) | Cost/Flex (10) | Weighted Total (/500) |
|---|---:|---:|---:|---:|---:|---:|
| AWS IoT TwinMaker | 5 | 5 | 5 | 1 | 3 | 410 |
| Azure Digital Twins | 5 | 5 | 3 | 1 | 3 | 370 |
| Ansys Twin Builder | 3 | 3 | 3 | 2 | 2 | 270 |
| LabWired | 1 | 2 | 1 | 5 | 5 | 220 |

Ranked shortlist (Segment C):
1. AWS IoT TwinMaker
2. Azure Digital Twins
3. Ansys Twin Builder
4. LabWired (as simulation source, not full platform replacement)

Strategic interpretation for LabWired:
- In operational twin programs, LabWired should be sold as a firmware-truth subsystem feeding higher-level twin platforms.
- Competing directly as a graph/operations platform is low-leverage and off-strategy.

### 11.4 Baseline Consolidated Guidance

1. Primary battleground: Segment A (embedded CI validation), where LabWired can realistically compete and win.
2. Expansion path: selective Segment B entry via integration-first strategy, not full incumbent displacement.
3. Partner path: Segment C integration adapters (event/state export) instead of platform cloning.
4. Product prioritization signal: peripheral/model coverage expansion has the highest near-term strategic ROI.

### 11.5 ICP Overlay (Adjusted Weighting and Ranking)

This subsection tunes scoring for three explicit ICPs, using the same `1-5` scoring scale and weighted model.

#### ICP 1: Startup IoT Product Team

Decision pattern:
- Small team, tight budget, immediate need for fast CI feedback and low setup friction.

Weights:
- Onboarding speed: 30
- CI determinism: 25
- Cost/accessibility: 20
- Peripheral coverage for common IoT MCUs: 15
- Debug productivity: 10

Scores:

| Platform | Onboarding (30) | Determinism (25) | Cost/Access (20) | Coverage (15) | Debug (10) | Weighted Total (/500) |
|---|---:|---:|---:|---:|---:|---:|
| LabWired | 4 | 5 | 5 | 3 | 4 | 425 |
| Renode | 3 | 5 | 5 | 5 | 4 | 420 |
| Wokwi | 5 | 4 | 4 | 4 | 3 | 420 |
| QEMU | 2 | 3 | 5 | 5 | 4 | 335 |
| Simics | 2 | 5 | 1 | 5 | 5 | 315 |

Rank (ICP 1):
1. LabWired
2. Renode / Wokwi (tie)
3. QEMU
4. Simics

Implication:
- LabWired can be the default choice for startup IoT if onboarding stays crisp and coverage expands on top IoT boards.

#### ICP 2: Automotive Tier-1 Validation Organization

Decision pattern:
- Process-heavy environments prioritize standards/toolchain interoperability, repeatability, and safety evidence workflows.

Weights:
- Interoperability/co-simulation standards fit: 30
- Determinism/reproducibility: 20
- Safety evidence/process fit: 20
- Multi-node/vehicle-scale fit: 20
- Cost/flexibility: 10

Scores:

| Platform | Interop (30) | Determinism (20) | Safety Fit (20) | Multi-node (20) | Cost/Flex (10) | Weighted Total (/500) |
|---|---:|---:|---:|---:|---:|---:|
| Synopsys Virtualizer | 5 | 5 | 5 | 5 | 1 | 460 |
| dSPACE VEOS | 5 | 5 | 5 | 5 | 1 | 460 |
| ETAS COSYM | 5 | 5 | 5 | 5 | 1 | 460 |
| Simics | 4 | 5 | 4 | 5 | 1 | 410 |
| Renode | 3 | 5 | 3 | 4 | 5 | 390 |
| LabWired | 2 | 5 | 2 | 3 | 5 | 310 |

Rank (ICP 2):
1. Synopsys Virtualizer / dSPACE VEOS / ETAS COSYM (tie)
2. Simics
3. Renode
4. LabWired

Implication:
- LabWired entry point is a deterministic CI layer and subsystem verification accelerator, not a one-step replacement.

#### ICP 3: Semiconductor Vendor (Pre-silicon + SDK Enablement)

Decision pattern:
- Needs early software enablement before silicon, strong model control, and scalable developer ecosystem support.

Weights:
- Pre-silicon virtual platform depth: 30
- Model extensibility/control: 25
- Determinism and debug depth: 20
- Ecosystem reach/developer adoption: 15
- Cost/flexibility: 10

Scores:

| Platform | Pre-silicon Depth (30) | Extensibility (25) | Determinism+Debug (20) | Ecosystem Reach (15) | Cost/Flex (10) | Weighted Total (/500) |
|---|---:|---:|---:|---:|---:|---:|
| Synopsys Virtualizer | 5 | 5 | 5 | 4 | 1 | 440 |
| Simics | 5 | 4 | 5 | 4 | 1 | 415 |
| QEMU | 3 | 5 | 3 | 5 | 5 | 390 |
| Renode | 3 | 4 | 4 | 4 | 5 | 385 |
| LabWired | 2 | 4 | 4 | 2 | 5 | 325 |

Rank (ICP 3):
1. Synopsys Virtualizer
2. Simics
3. QEMU
4. Renode
5. LabWired

Implication:
- To win semiconductor accounts, LabWired must materially improve pre-silicon depth and ecosystem reach, while keeping its CI determinism edge.

### 11.6 Merged Outcome Across All Analyses

1. Repeated top gap across baseline and ICP overlays: **coverage depth** (board + peripheral fidelity breadth).
2. Repeated differentiator to preserve: **deterministic CI workflow and replayability**.
3. Required market entry logic:
   - Win embedded CI-first teams now.
   - Enter enterprise via integration, not displacement.
   - Integrate into cloud twin ecosystems instead of cloning them.
4. Execution implication: Priority 1 (coverage depth) is the highest-ROI move and should anchor the next two quarters.

## 12) Competitive Gap-Closure Plan (How LabWired Becomes More Competitive)

Lean 80/20 plan: execute only what moves Segment A ranking fastest.

### 12.1 Priority Stack (Only 3 Items)

1. **Coverage depth on Top-5 executable targets** (`stm32f103-bluepill`, `stm32h563-nucleo`, `stm32f401-nucleo`, `firmware-rv32i-ci-fixture`, `firmware-stm32f103-blinky-stm32f103`).
2. **Onboarding speed to first deterministic smoke** (time-to-first-UART-smoke).
3. **Deterministic CI evidence quality** (artifact reproducibility and hard gating).

Everything else is secondary until these three are stable.

### 12.2 6-Week Execution Plan

1. Weeks 1-2:
   - Keep Top-5 matrix green with hard CI gate.
   - Fix highest-frequency peripheral failures only.
2. Weeks 3-4:
   - Improve onboarding path (starter manifests + clearer failure diagnostics).
   - Reduce median onboarding time.
3. Weeks 5-6:
   - Stabilize deterministic replay artifacts and close top flakiness sources.
   - Recompute Segment A weighted score from actual matrix outcomes.

### 12.3 Hard Success Criteria (Quarter Gate)

1. Top-5 deterministic pass rate: `>=80%` sustained.
2. Median time-to-first-UART-smoke for supported families: `<60 minutes`.
3. Deterministic replay success for CI failures: `>=95%`.

If these are not met, do not expand scope.

### 12.4 Immediate Implementation Backlog

1. Keep `core/.github/workflows/core-coverage-matrix-smoke.yml` as a hard gate for Top-5 executable targets.
2. Use `core/scripts/generate_coverage_matrix_scoreboard.py` artifacts as single source of truth.
3. Prioritize bug fixes by matrix failure frequency, not by speculative feature breadth.
4. Update `core/docs/coverage_scoreboard.md` only from CI artifacts.

### 12.5 Current Scoreboard (Implementation Alignment)

Latest validated matrix run on `main`:
- Date: `2026-02-22`
- Run: `https://github.com/w1ne/labwired-core/actions/runs/22281072679`
- Summary: `6/6 pass`, `0 fail`, `0 missing`

Top-5 hard gate targets:
- `stm32f103-bluepill`
- `stm32h563-nucleo`
- `stm32f401-nucleo`
- `firmware-rv32i-ci-fixture`
- `firmware-stm32f103-blinky-stm32f103`

Gate state:
- Required targets passing: `5/5` (`100%`, threshold `>=80%`)

Onboarding KPI instrumentation (new):
- Workflow: `core/.github/workflows/core-onboarding-smoke.yml`
- Per-target artifact: `onboarding-metrics.json` (elapsed seconds, failure stage, first error signature)
- Aggregated artifact: `onboarding-scoreboard.json` / `onboarding-scoreboard.md`
- Current policy: soft threshold tracking (`3600s`) before hard gating

## 13) Sources

### LabWired internal sources
- `README.md`
- `core/docs/architecture.md`
- `docs/DIGITAL_TWIN_SPEC.md`
- `core/docs/ci_test_runner.md`

### External primary sources
1. QEMU emulation docs: https://www.qemu.org/docs/master/about/emulation.html
2. QEMU license docs: https://www.qemu.org/docs/master/about/license.html
3. QEMU system targets docs: https://www.qemu.org/docs/master/system/targets.html
4. QEMU STM32 board docs (example target limitations): https://qemu.eu/doc/8.2/system/arm/stm32.html
5. Renode overview: https://renode.io/about/
6. Renode testing/CI docs: https://renode.readthedocs.io/en/latest/introduction/testing.html
7. Renode architecture support example (RISC-V): https://renode.readthedocs.io/en/latest/basic/configuring-a-risc-v-cpu.html
8. Renode repository/license statement: https://github.com/renode/renode
9. Wokwi supported hardware: https://docs.wokwi.com/getting-started/supported-hardware
10. Wokwi custom chips API: https://docs.wokwi.com/chips-api/chip-json/
11. Wokwi custom chips WASM workflow: https://docs.wokwi.com/guides/custom-chips-to-wasm
12. Wokwi pricing: https://wokwi.com/pricing
13. Wokwi VS Code licensing page: https://wokwi.com/license
14. Intel Simics public release overview: https://www.intel.com/content/www/us/en/developer/articles/tool/simics-simulator.html
15. Simics checkpointing docs: https://intel.github.io/tsffs/simics/simics-user-guide/checkpointing.html
16. Simics command docs (`read-configuration`): https://intel.github.io/tsffs/simics/rm-base/rm-cmd-read-configuration.html
17. Synopsys Virtualizer overview: https://www.synopsys.com/verification/virtual-prototyping/virtualizer.html
18. dSPACE VEOS: https://www.dspace.com/en/pub/home/products/sw/simulation_software/veos.cfm
19. ETAS COSYM: https://www.etas.com/ww/en/products-services/software-testing-solutions/cosym/
20. AWS IoT TwinMaker overview: https://docs.aws.amazon.com/iot-twinmaker/latest/guide/what-is-twinmaker.html
21. AWS IoT TwinMaker docs overview: https://aws.amazon.com/documentation-overview/iot-twinmaker/
22. Azure Digital Twins overview: https://learn.microsoft.com/en-us/azure/digital-twins/overview
23. Azure Digital Twins documentation hub: https://learn.microsoft.com/en-us/azure/digital-twins/
24. DTDL specification portal: https://azure.github.io/opendigitaltwins-dtdl/
25. Ansys Twin Builder: https://www.ansys.com/products/digital-twin/ansys-twin-builder
26. MathWorks SIL/PIL overview: https://www.mathworks.com/help/ecoder/ug/about-sil-and-pil-simulations.html
