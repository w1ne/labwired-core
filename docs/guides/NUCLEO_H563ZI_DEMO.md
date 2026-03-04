[← Back to Hub](../README.md)

# NUCLEO-H563ZI Demo Story

## Positioning

LabWired turns embedded validation into a repeatable software workflow.
This demo shows a single STM32H563 scenario working in both emulator and real hardware, with the same observable behavior.

## What Audience Sees

1. Deterministic emulator pass for boot, UART, GPIO, and core peripherals.
2. Physical NUCLEO-H563ZI board flashing and running blink + UART.
3. Matching behavior signatures across both environments.
4. **Golden Reference Proof**: Instruction-level lockstep determinism verified between hardware and simulation.
## Why It Matters

1. Faster firmware validation cycles without always waiting for hardware benches.
2. Reproducible CI-style checks before hardware-in-the-loop.
3. Clear transition from simulation to board proof, reducing integration risk.

## Demo Scope (Current)

1. MCU: STM32H563ZI.
2. Board IO: LEDs `PB0`, `PF4`, `PG4`; button `PC13`.
3. VCP logs through `USART3`.
4. Full-chip smoke for modeled peripherals (`RCC`, `SYSTICK`, `UART`, `GPIOA..GPIOG`).
5. **Determinism Proof**: Cycle-accurate (instruction-level) comparative analysis via [AetherDebugger](https://github.com/w1ne/AetherDebugger).
## Live Demo Commands

From `core/`:

1. Emulator capability showcase:
```bash
examples/nucleo-h563zi/scripts/run_full_example.sh
```

2. Hardware blink + UART:
```bash
examples/nucleo-h563zi/scripts/run_blink_uart_hardware.sh --port /dev/ttyACM0
```

3. End-to-end dual run:
```bash
examples/nucleo-h563zi/scripts/run_blink_uart_dual.sh --port /dev/ttyACM0
```

4. Video-friendly combined run (concise proof output):
```bash
examples/nucleo-h563zi/scripts/run_video_demo.sh --mode all --port /dev/ttyACM0 --keep-artifacts
```

## Proof Points to Call Out

1. Emulator output includes `OK`, `H563-IO`, and `ALL=1`.
2. Board output includes `H563-BLINK-UART` and alternating `PB0=1` / `PB0=0`.
3. Same board mappings are validated in both runs.
4. VS Code Command Center reads `board_io` from `system.yaml` and reflects live GPIO-driven states.

## Suggested Talk Track

1. "We start in deterministic simulation to validate behavior quickly."
2. "Then we run the same capability story on a physical NUCLEO-H563ZI board."
3. "The observable outputs match, so simulation is a trustworthy pre-hardware gate."
4. "We've formalized this trust with a **Golden Reference Report**, proving that the first 50 instructions of the boot sequence are identical between the emulated CPU and the physical board."
## Next Marketing Step

Extend this demo with onboard Ethernet PHY (`LAN8742A`) to show a network scenario (ping/echo throughput) as the next visible milestone.

## Golden Reference & Verification
- **Determinism Report**: [determinism_report_h563.json](../core/examples/nucleo-h563zi/golden-reference/determinism_report_h563.json)
- **Hardware Trace**: [hw_trace.json](../core/examples/nucleo-h563zi/golden-reference/hw_trace.json)
- **Simulation Trace**: [sim_trace.json](../core/examples/nucleo-h563zi/golden-reference/sim_trace.json)

## Technical Entry Point

- Human runbook: `../core/examples/nucleo-h563zi/VALIDATION.md`
- Example root: `../core/examples/nucleo-h563zi/README.md`
- Video runbook: `./NUCLEO_H563ZI_VIDEO_RUNBOOK.md`
- Voiceover script: `./NUCLEO_H563ZI_VOICEOVER_SCRIPT.md`
