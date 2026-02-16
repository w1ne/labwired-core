# NUCLEO-H563ZI Voiceover Script

Use this with:

```bash
cd core
examples/nucleo-h563zi/scripts/run_video_demo.sh --mode all --port /dev/ttyACM0 --keep-artifacts
```

## Segment 1: Setup (15-20s)

On screen:
1. `core/examples/nucleo-h563zi/system.yaml`
2. VS Code Command Center panel

Say:
1. "This demo runs the same STM32H563 behavior in simulation and on real hardware."
2. "Our board wiring is declarative in `system.yaml`: PB0, PF4, PG4 LEDs and PC13 button."
3. "The UI reflects GPIO register truth from emulation telemetry, not hardcoded LED animation."

## Segment 2: Emulator Proof (60-90s)

On screen:
1. Run `run_video_demo.sh` and show terminal output.

Say:
1. "First we run deterministic emulator checks."
2. "Flash boot alias test passes, then all three demo firmware targets build."
3. "Now we verify behavior contracts: `PASS: uart-smoke` with `OK`."
4. "Then `PASS: io-smoke` with LED transitions."
5. "Then `PASS: fullchip-smoke` with `RCC=1 SYSTICK=1 UART=1` and `ALL=1`."
6. "This is reproducible and CI-friendly."

## Segment 3: Hardware Parity (60-90s)

On screen:
1. Board in frame, LEDs visible.
2. Hardware phase output from the same command.

Say:
1. "Now the same story runs on a physical NUCLEO-H563ZI board."
2. "Firmware is flashed with OpenOCD, UART is captured from ST-Link VCP."
3. "We confirm banner and blink evidence for all three LEDs: PB0, PF4, PG4."
4. "So simulation and board behavior align on observable outputs."

## Segment 4: Command Center Tie-In (20-30s)

On screen:
1. Command Center board IO rows changing while firmware runs.

Say:
1. "Command Center reads board IO from YAML and overlays live states from DAP telemetry."
2. "The source of truth stays in firmware and registers."

## Segment 5: Close (10-15s)

Say:
1. "LabWired gives fast deterministic validation before hardware, then seamless board confirmation."
2. "Next milestone is onboard LAN8742A Ethernet PHY for a visible network demo."
