# LabWired Platform Documentation

This directory contains **platform-level** documentation for the LabWired monorepo.

## Platform Strategy & Planning

- **[Demos & Examples](../DEMOS.md)** - Platform-level directory of all demos
- **[Implementation Plan](./plan.md)** - Overall platform roadmap and milestones
- **[Vision Completion Gaps](./vision/VISION_COMPLETION_GAPS.md)** - What is still missing to finish the Agent-First vision
- **[Release Checklist](./RELEASE_CHECKLIST.md)** - Platform release gate for versioning, quality, docs, and publication
- **[Refactor and Optimization Audit (2026-02-13)](./REFACTOR_OPTIMIZATION_AUDIT_2026-02-13.md)** - Runtime code health findings and applied fixes
- **[Postmortems](./postmortems/README.md)** - Incident analyses, root causes, and prevention actions
- **[Demo Dry Run](./DEMO_DRY_RUN.md)** - Release-gate checklist and commands for external demos
- **[NUCLEO-H563ZI Demo Story](./NUCLEO_H563ZI_DEMO.md)** - Marketing narrative and live showcase script
- **[HIL Displacement Showcase](./HIL_DISPLACEMENT_SHOWCASE.md)** - ROI analysis and technical results of displacing physical HIL with LabWired
- **[NUCLEO-H563ZI Video Runbook](./NUCLEO_H563ZI_VIDEO_RUNBOOK.md)** - Shot-by-shot recording procedure with exact commands
- **[NUCLEO-H563ZI Voiceover Script](./NUCLEO_H563ZI_VOICEOVER_SCRIPT.md)** - Ready-to-read narration mapped to terminal/UI moments
- **[VS Code UI Demo Checklist](./VS_CODE_UI_DEMO_CHECKLIST.md)** - Manual UI validation (breakpoints, step, registers, memory, Docker mode)
- **[Business Strategy](./spec/)** - Market analysis, business roadmaps, and strategic vision
- **[Foundry Pricing Model](./spec/FOUNDRY_PRICING.md)** - Pricing tiers, API usage model, and cost structure for LabWired Foundry
- **[Foundry Product Spec](./spec/FOUNDRY_SPEC.md)** - Architecture, API surface, onboarding UX, and dashboard visual spec
- **[Digital Twin Simulator Comparison (2026-02-22)](./spec/DIGITAL_TWIN_SIMULATOR_COMPARISON_2026-02-22.md)** - Deep competitive analysis of LabWired vs direct and adjacent simulator platforms
- **[Top-20 Coverage Matrix](./spec/TOP20_COVERAGE_MATRIX.md)** - Priority target matrix for closing simulation coverage gaps

## Component Documentation

For component-specific technical documentation, see:

- **[Core Emulator](../core/docs/)** - Architecture, CI integration, debugging guides, peripheral development
- **[Board Onboarding Playbook](../core/docs/board_onboarding_playbook.md)** - Config-first runbook for adding new MCU/board targets
- **[VS Code Extension](../vscode/README.md)** - Extension features and usage
- **[AI Tools](../ai/README.md)** - Asset generation documentation

## Repository Structure

```
labwired/
├── core/          # Emulator engine (self-contained, releasable)
├── vscode/        # VS Code extension (self-contained)
├── ai/            # AI tools (self-contained)
└── docs/          # Platform-level strategy docs (this directory)
```
