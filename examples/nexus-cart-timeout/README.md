# Nexus Design AI × LabWired — robotic cart sensor timeout

**Non-confidential feasibility test**, based on the 14 November 2026 handoff.
Nexus supplied a pin-level contract and test sequence, not firmware. LabWired
authored the actual Cortex-M3 firmware here. This is not safety-certified
software or a validated physical cart.

## Interfaces

- MCU: STM32F103C8T6 (64 KiB flash / 20 KiB SRAM), HSI core clock 8 MHz.
- Timer: SysTick COUNTFLAG, 1 ms polling.
- Sensor: PA10 / USART1_RX, 115200 8N1; line S,sequence,distance_mm + LF.
  Require a complete valid frame and strictly increasing positive sequence.
  Old, repeated, malformed and overflowed frames cannot refresh timestamp.
- Operator: PA0 START, PA1 RESET, **test-only** simulated buttons.
  Physical use needs separately engineered operator inputs.
- Actuator permission: PB0 MOTOR_ENABLE; HIGH permits, LOW requests disable.
  LOW on reset; this output never directly powers the motor.
- Qualification: three fresh valid frames with distance >=500 mm, then START.
- Fault: 100 ms stale threshold, PB0 LOW, latched until RESET plus three
  NEW fresh frames and another START. New sensor frames alone cannot restart.

## Reproduce from the labwired-core root

First build: cargo build -p labwired-cli --bin labwired

Then run, in a shell:

    LABWIRED_CLI="$PWD/target/debug/labwired" examples/nexus-cart-timeout/tests/run_cases.sh

The script builds the **real** Cortex-M3 ELF with arm-none-eabi-gcc. The
LabWired CLI injects UART RX bytes at exact virtual-cycle offsets and drives
START/RESET using the existing generic stimuli. The test verifier reads the
real emulator-generated PB0 transitions and UART records, not a mock firmware
state machine. Evidence under ignored out/CASE includes result.json,
uart.log, and run-manifest.json.

| Case | Expected behavior |
| --- | --- |
| stale | RUNNING; repeated seq 6 after 100 ms; PB0 LOW by 205 ms, latched |
| missing | No new frames after 100 ms; PB0 LOW by 205 ms |
| malformed | Invalid line with valid suffix must not refresh freshness |
| healthy | Fresh frames continue, motor remains HIGH, no fault |
| no-start | Fresh frames alone cannot enable motor |
| recovery | Fault, manual RESET, three new frames, then explicit START |

## Safety, coverage and tooling limits

Existing STM32F103 RCC, GPIO, UART1 and SysTick simulation models are reused.
No Nexus-specific hardware model, MCP command, or parallel simulator is added.

The fixture has no routed VDD supply. LabWired emits the fidelity gap
unpowered_rail_assumed: the MCU receives an ideal assumed 3.3 V supply.
Power-on and brownout safety are NOT tested. UART injections exercise the
modeled peripheral byte path, not the analog or wire-level bit timing at
115200 baud. The cart sensor, motor driver, power-stage faults, actual motor
torque, stopping distance and safety certification remain outside this
simulation. This is a firmware reaction-time feasibility demonstration only.
