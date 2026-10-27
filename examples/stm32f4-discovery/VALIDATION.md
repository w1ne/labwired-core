# Validation — STM32F4DISCOVERY (MB997, STM32F407VGT6)

**Status: simulator smoke passed 2026-10-27.** The run uses the exact
`stm32f407vg` chip descriptor and board system below. It is not a physical
board test.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| ELF boot and Cortex-M4F execution | smoke-tested | `labwired test` passed 3/3 assertions at 4,000,000 steps |
| RCC gates for GPIOA/GPIOD and USART2 | smoke-tested | firmware bring-up and UART output |
| User LEDs PD12–PD15 | smoke-tested | UART reports each LED on/off with `GPIOD_ODR` readback |
| USART2 PA2 TX from 16 MHz HSI | smoke-tested | smoke banner and heartbeat lines captured |
| Repeatability | smoke-tested | two runs produced identical UART logs |
| Instruction support for this firmware | audited | 200,000 instructions, 0 unsupported |
| USER button PA0 input | declared only | system manifest maps it; firmware does not read it |
| I²C/SPI/timers/ADC/DMA/USB/Ethernet/CAN | **not modelled here** | see `EXTERNAL_COMPONENTS.md` |

## Commands and evidence

Tool: `labwired-cli 0.25.0`

Firmware target: `thumbv7em-none-eabi`

Firmware SHA-256: `e539bf8f3c22cdc8bb3504341cf1afeada3269655ec92c5a3fc5ac8f8a7907cf`

System chip: `configs/chips/stm32f407vg.yaml`

```sh
cargo build -p firmware-f4-discovery-smoke --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f4-discovery/io-smoke.yaml \
  --output-dir out/f4-discovery-smoke-final --no-uart-stdout
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f4-discovery/io-smoke.yaml \
  --output-dir out/f4-discovery-smoke-repeat --no-uart-stdout
diff out/f4-discovery-smoke-final/uart.log out/f4-discovery-smoke-repeat/uart.log
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-f4-discovery-smoke \
  --system configs/systems/stm32f4-discovery.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/stm32f4-discovery-final \
  --fail-on-unsupported
```

Results: build completed; both smoke runs passed **3/3 assertions** at
4,000,000 steps; the UART logs matched. Captured UART showed `F4-DISCO SMOKE`
and on/off readbacks for LD4/PD12, LD3/PD13, LD5/PD14, and LD6/PD15. The
unsupported-instruction audit exited 0 with **200,000 instructions executed,
0 unsupported**. Detailed run output is under the `out/` paths above.

The heartbeat's `odr=` value is read back from `GPIOD_ODR` after each write.
The board's ST-LINK VCP is not wired to the MCU UART (UM1472); the console
pins are PA2/PA3. The example does not claim a USB console path.
