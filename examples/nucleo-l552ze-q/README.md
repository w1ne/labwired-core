# NUCLEO-L552ZE-Q — LabWired example

The [NUCLEO-L552ZE-Q](https://www.st.com/en/evaluation-tools/nucleo-l552ze-q.html)
board with an **STM32L552ZET6Q**: Cortex-M33 (Armv8-M), 512 KiB flash,
192 KiB SRAM1 (+64 KiB SRAM2), 110 MHz max. This package is the promoted,
runnable path for the board; the staged descriptor under
`configs/chips/onboarding/` remains inventory only.

## What runs here

A bare-register smoke firmware (`crates/firmware-l552ze-smoke`) that:

- enables the L5 clock gates (`RCC_AHB2ENR` for GPIOA/B/C/G,
  `RCC_APB1ENR2.LPUART1EN` for the console),
- routes **PG7/PG8** to **LPUART1** AF8,
- walks the three user LEDs **LD1 PC7**, **LD2 PB7**, **LD3 PA9** and prints
  the state **read back from each port's `ODR`**,
- reads the **B1 PC13** user button,
- prints a heartbeat on **LPUART1** at 115200 8N1 from the 4 MHz MSI reset
  clock.

## Build and run

```bash
# from the repository root
cargo build -p firmware-l552ze-smoke --release --target thumbv8m.main-none-eabi

./target/release/labwired test \
  --script examples/nucleo-l552ze-q/io-smoke.yaml \
  --output-dir out/nucleo-l552ze-q-smoke --no-uart-stdout
```

Interactive run (UART to stdout):

```bash
./target/release/labwired \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-l552ze-smoke \
  --system configs/systems/nucleo-l552ze-q.yaml \
  --max-steps 400000
```

Expected output (first beats):

```text
NUCLEO-L552ZE-Q SMOKE
cpu=cortex-m33 uart=lpuart1
led=ld1 odr=00000080
led=ld1_off odr=00000000
led=ld2 odr=00000080
led=ld2_off odr=00000000
led=ld3 odr=00000200
led=ld3_off odr=00000000
btn=0
hb=0 odr=00000080 btn=0
...
```

## Board facts used

| Item | Value | Source |
| :--- | :--- | :--- |
| MCU | STM32L552ZET6Q, LQFP144 | UM2581; DS12737 |
| Core / clock | Cortex-M33, 110 MHz max (MSI 4 MHz at reset) | DS12737; RM0438 RCC chapter |
| Flash / RAM | 512 KiB @ 0x08000000 / 192 KiB @ 0x20000000 (+64 KiB @ 0x20030000) | DS12737 device summary; Zephyr `stm32l552Xe.dtsi` |
| User LEDs | LD1 PC7, LD2 PB7, LD3 PA9, active high | UM2581; Zephyr `nucleo_l552ze_q` |
| User button | B1 PC13, active high | UM2581; Zephyr `nucleo_l552ze_q` |
| Console UART | LPUART1 PG7 (TX) / PG8 (RX), AF8, 115200 8N1 | UM2581; Zephyr board dts |

## Explicitly not claimed

TrustZone / SAU / GTZC and the secure/non-secure split are **not
modelled**. The PMSAv8 MPU is enforced (MemManage on denied data accesses and
fetches). FP state is stacked eagerly, not lazily. Stock Zephyr
`hello_world`, `synchronization`, `tests/kernel/fpu_sharing/generic` and
`tests/kernel/mem_protect/protection` and `.../userspace` run from `zephyr-*.yaml` (see `VALIDATION.md`). The L5 non-secure FLASH controller, EXTI,
ADC1, FDCAN1 and USB are declared stubs. SPI/I2C/timers/DMA/RNG/RTC are
modelled but unexercised in this L1 slice. See `VALIDATION.md` for the tier
table.
