# ADXL345 Sensor Lab Playground Redesign

Date: 2026-05-07

## Summary

Redesign the LabWired playground around one flagship public demo: a guided ADXL345 Sensor Lab. The first release should serve hobbyists first while proving to firmware developers that LabWired is running real MCU and peripheral simulation.

The default experience should be a beautiful, interactive, circuit-first lab rather than a blank workbench. KiCad integration stays on the roadmap, but the data model and UI composition should not block future schematic or netlist import.

## Goals

- Make the playground immediately useful and understandable for hobbyists.
- Show real MCU + peripheral simulation, not scripted visual-only motion.
- Create a polished demo suitable for blog posts, landing pages, embeds, and social clips.
- Preserve developer credibility through advanced simulation observability.
- Keep the first release narrow enough to ship as a real end-to-end lab.

## Non-Goals

- KiCad import or export in the first release.
- Fault injection in the first Sensor Lab.
- A broad project marketplace.
- A full visual builder redesign beyond what the ADXL345 lab requires.
- A fake or scripted sensor stream as the launch path.

## Product Experience

The playground opens directly into a guided ADXL345 Sensor Lab. The main stage shows an MCU board, an ADXL345 breakout, I2C or SPI wiring, run controls, and live sensor output. The circuit canvas is the hero.

The normal path uses a compact step rail:

1. Wire check.
2. Upload and run firmware.
3. Watch acceleration.
4. Share or embed the demo.

The ADXL345 appears as a board-like component with live X/Y/Z state. Users can manipulate simulated sensor orientation or acceleration through an obvious control, such as a tilt pad or axis sliders. Firmware serial output updates live so users can connect sensor input to firmware behavior.

Advanced mode is a drawer rather than the primary layout. It exposes registers, instruction trace, memory, generated YAML/config, and raw simulation details for firmware developers. The default view remains visually clean and beginner-friendly.

The lab also has a shareable demo mode that uses the same real simulation with a cleaner embedded layout for external posts and landing-page placement.

## Technical Architecture

The design follows existing repo boundaries:

- `core/` owns real simulation behavior, including ADXL345 bus behavior and firmware-visible state.
- `packages/ui` owns reusable UI pieces: ADXL345 component rendering, live axis display, chart, guided step rail, advanced drawer, and shared simulator state adapters.
- `packages/playground` owns product composition: default lab selection, guided mode, advanced mode, bundled config/firmware loading, and share/embed URLs.

The launch path must use real simulation:

1. Playground loads the bundled ADXL345 lab config and firmware.
2. The WASM simulator bridge starts from the real system and chip config.
3. UI reads serial output, board IO state, registers, trace, memory, and ADXL345-observable state.
4. User changes simulated sensor orientation or acceleration.
5. Playground sends that input into the simulator through a typed peripheral/input API.
6. Firmware reads the ADXL345 through the simulated bus and emits real output.

The key boundary is that UI may visualize sensor state, but firmware-visible behavior must come from the simulator model.

## MVP Scope

- One flagship lab: ADXL345 Sensor Lab.
- One primary MCU and bus path selected from the fastest reliable existing LabWired support.
- Guided Stage layout as the default playground entry.
- Real firmware plus real simulated ADXL345 behavior.
- Live X/Y/Z values and a simple chart.
- Serial output visible in the guided view.
- Circuit state visible on the main stage.
- Advanced drawer with registers, trace, memory, generated YAML/config, and raw simulation details.
- Share/embed mode suitable for public posts.

The implementation should choose the first MCU and bus route based on what `core` already supports most reliably. The design intentionally does not hard-code a risky target before implementation discovery.

## Validation

- Core test proves firmware reads ADXL345 values through the simulated bus.
- Core or bridge-level test proves user-controlled sensor state affects firmware-visible ADXL345 reads.
- Playground/unit test proves the bundled lab config is loadable and share/embed state is stable.
- Manual browser verification proves:
  - Guided lab loads on desktop and mobile.
  - Simulation starts from the real bundled firmware/config.
  - Sensor input changes affect serial output and chart values.
  - Advanced drawer surfaces registers, trace, memory, and generated config.
  - Embed mode is clean enough for posts and landing pages.

## Roadmap Hooks

KiCad integration is deferred, but the Sensor Lab should keep project data structured enough for future schematic or netlist import. Future work can add KiCad import/export once the real simulation and guided lab experience are proven.

Potential later additions:

- KiCad schematic/netlist import.
- Multiple sensor labs.
- Content gallery or project marketplace.
- Exported waveforms and reports.
- Fault injection as an advanced developer feature.
