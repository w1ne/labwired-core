## 🚀 Highlights

**Documentation Overhaul & Architecture Unification**
This release introduces a completely restructured documentation site based on the Diataxis framework, alongside critical architectural improvements in the SVD transformation pipeline and CPU core instruction fidelity.

## ✨ New Features

- **Documentation Overhaul**: Migrated to MkDocs with Material theme, reorganized into Tutorials, How-To, Reference, and Explanation sections.
- **Architecture Unification**: Native ingestion of **Strict IR** (JSON) in the simulation core, bridging `labwired-ir` and `labwired-config`.
- **Asset Foundry Hardening**: Enhanced SVD transformation with flattened inheritance, register array unrolling, and cluster flattening.
- **Timeline View**: Professional visualization of instruction trace data in the VS Code extension.

## 🐛 Bug Fixes

- **Critical Instruction Regression**: Fixed `io-smoke` failure by implementing proper **Thumb-2 `IT` (If-Then) block** support in the `CortexM` core.
- **Instruction Coverage**: Expanded modular decoder and executor for `MOVW`, `MOVT`, `LDR.W`, `STR.W`, and `UXTB.W`.
- **Structural Stability**: Refactored CPU `step` loop for improved variable scoping and exception handling consistency.

## 🛠 Improvements

- **Support Strategy**: Defined **Tier 1 Device Support** (STM32F4, RP2040, nRF52) in `SUPPORTED_DEVICES.md`.
- **Core Guides**: Added comprehensive `architecture_guide.md` and `board_onboarding_playbook.md`.

## 📦 Dependency Updates
- Verified workspace-wide compatibility and version alignment for the 0.12.0 milestone.

## 🤝 Contributors
- @w1ne
- @antigravity

---
_Full Changelog: https://github.com/w1ne/labwired-core/blob/v0.12.0/CHANGELOG.md_
