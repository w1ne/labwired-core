# NUCLEO-L552ZE-Q

The **NUCLEO-L552ZE-Q** with an **STM32L552ZET6Q** (Cortex-M33, Armv8-M,
110 MHz max): 512 KiB of flash at `0x08000000`, 192 KiB SRAM1 at `0x20000000`
plus 64 KiB SRAM2 at `0x20030000`, three user LEDs on `PC7` (LD1, green),
`PB7` (LD2, blue) and `PA9` (LD3, red), a user button on `PC13`, and the
ST-LINK/V2-1 Virtual COM Port on **LPUART1** `PG7`/`PG8` (AF8).

This is the **first STM32L5 part in the engine**. It is an **L1 (smoke)**
target: boot, the new L5 RCC clock gates, the LPUART1 console and the
GPIO LED/button path (with `ODR` read-back) are proven by a committed smoke
fixture and an `io-smoke.yaml`. Unmodified upstream **Zephyr v3.7.0
`hello_world`** (the sample Renode's `ARMv8M.robot` boots) also runs through
Zephyr's own L5 clock/ICACHE/DBGMCU bring-up and prints on LPUART1, and
stock `synchronization` runs its two-thread semaphore hand-off on SysTick
timeouts and PendSV context switches. Stock **`tests/kernel/fpu_sharing/generic`**
(Renode's `ARMv8M.robot` FPU sharing case) passes 2/2 on the modelled FP
context (`CONTROL.FPCA`, extended exception frames) and the PMSAv8 MPU
region file that `TT` reports from. Stock
**`tests/kernel/mem_protect/protection`** (Renode's `ARMv8M.robot` protection
case) passes 5/5: the MPU is enforced, so a store to the read-only flash
region and a fetch from execute-never SRAM raise MemManage. Stock
**`tests/kernel/mem_protect/userspace`** (Renode's userspace case) passes all
40 cases in its three suites: user threads run unprivileged, kernel memory and
the PPB are closed to them, MSR cannot clear `CONTROL.nPRIV`, and `TTT`
reports the unprivileged view that syscall buffer checks use. All four
Renode `ARMv8M.robot` cases now run. There is
no silicon capture: **SIM-DERIVED**, no executing-fidelity differential.

!!! warning "Not claimed yet"
    TrustZone / SAU / GTZC and the secure/non-secure split are **not
    modelled**. The PMSAv8 MPU is enforced for data accesses and instruction
    fetches (MemManage DACCVIOL/IACCVIOL), but MAIR memory types, faults on
    exception stacking (MSTKERR/MUNSTKERR) and lazy FP preservation (MLSPERR)
    are not ([ISA coverage](../isa_coverage.md)). FP state is stacked
    eagerly, not lazily (`FPCCR.LSPACT` reads 0). The L5 non-secure FLASH controller,
    EXTI, ADC, FDCAN and USB are declared stubs.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/stm32l552.yaml`](../../configs/chips/stm32l552.yaml) |
| Example system | [`configs/systems/nucleo-l552ze-q.yaml`](../../configs/systems/nucleo-l552ze-q.yaml) |
| Reference firmware | [`crates/firmware-l552ze-smoke/`](../../crates/firmware-l552ze-smoke/) |
| Example | [`examples/nucleo-l552ze-q/`](../../examples/nucleo-l552ze-q/) |
| Committed artifact | `tests/fixtures/nucleo-l552ze-q-smoke.elf` (`NUCLEO-L552ZE-Q SMOKE`) |
| Zephyr fixtures | `tests/fixtures/stm32l552-zephyr-hello.elf` / `stm32l552-zephyr-sync.elf` — stock Zephyr v3.7.0 `hello_world` / `synchronization`, rebuilt by [`crates/firmware-l552ze-zephyr/build.sh`](../../crates/firmware-l552ze-zephyr/build.sh) |
| Build target | `thumbv8m.main-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + LPUART1 + GPIO); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| Flash | `0x0800_0000`, 512 KiB | modelled — the smoke links and runs here |
| SRAM1 / SRAM2 | `0x2000_0000` 192 KiB / `0x2003_0000` 64 KiB | modelled — data/stack |
| RCC | `0x4002_1000` | modelled (`stm32l5` profile) — AHB1ENR/AHB2ENR/AHB3ENR/APB1ENR1/APB1ENR2/APB2ENR, MSI boot, PLL handshake |
| LPUART1 (console) | `0x4000_8000` | modelled — USART v2 layout, `APB1ENR2` bit 0 |
| USART1–3 / UART4–5 | `0x4001_3800` / `0x4000_4400`… | modelled — USART v2 layout |
| GPIOA–H | `0x4202_0000` + `0x400`×n | modelled — `stm32v2` MODER/ODR/BSRR/AFR |
| LD1 / LD2 / LD3 | PC7 / PB7 / PA9 | modelled GPIO — active high |
| B1 user button | PC13 | modelled GPIO — active high |
| VCP console | PG7 (TX) / PG8 (RX), AF8 | simulator captures the LPUART1 stream; USB bridge not modelled |
| SPI1–3 | `0x4001_3000` / `0x4000_3800` / `0x4000_3C00` | modelled — SPI v2 FIFO, unexercised |
| I2C1–2 | `0x4000_5400` / `0x4000_5800` | modelled — v2 I2C, unexercised |
| TIM1–8, TIM15–17 | see descriptor | modelled — generic TIMx, unexercised |
| DMA1 / DMA2 / DMAMUX1 | `0x4002_0000` / `0x4002_0400` / `0x4002_0800` | modelled — unexercised |
| RNG / CRC / RTC / IWDG / WWDG / PWR | see descriptor | modelled — unexercised |
| ICACHE | `0x4003_0400` | modelled — the U5 `icache` model (same IP); Zephyr enables it |
| DBGMCU | `0xE004_4000` | modelled — `IDCODE 0x2001_6472` (DEV_ID 0x472, rev Z) |
| FLASH controller, EXTI, ADC1, FDCAN1, USB | see descriptor | `type: stub` — zeros (so `FLASH_ACR.LATENCY` does not read back) |
| FP extension (FPCCR/FPCAR/FPDSCR/MVFR) | `0xE000_EF34` | modelled — `cortex_m_fpu`: `CONTROL.FPCA`, extended frames (eager) |
| MPU (PMSAv8, 8 regions) | `0xE000_ED90` | modelled + enforced — `armv8m_mpu`: AP/XN/PRIVDEFENA/HFNMIENA, MemManage with MMFAR; reported by `TT` |
| TrustZone / SAU / GTZC | — | **not modelled** |

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point
(vector table at the flash base, `0x0800_0000`). The initial SP is word 0 of
the firmware's vector table — `0x2003_0000`, the top of SRAM1 in the smoke's
`memory.x` — not an engine default. The STM32 system-memory bootloader and TrustZone
secure-boot path are not modelled.

## Validation

```bash
# from the repository root
cargo build -p firmware-l552ze-smoke --release --target thumbv8m.main-none-eabi
./target/release/labwired test --script examples/nucleo-l552ze-q/io-smoke.yaml --no-uart-stdout
# stock Zephyr hello_world + synchronization (committed fixtures)
./target/release/labwired test --script examples/nucleo-l552ze-q/zephyr-hello.yaml
./target/release/labwired test --script examples/nucleo-l552ze-q/zephyr-sync.yaml
```

See [`examples/nucleo-l552ze-q/VALIDATION.md`](../../examples/nucleo-l552ze-q/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

RM0438 (STM32L552xx/L562xx reference manual) for the RCC/PWR/GPIO/USART/
SPI/I2C register maps; DS12737 (STM32L552xx datasheet) for the memory
boundaries and pin map; UM2581 (STM32L5 Nucleo-144 board MB1361 user
manual) and the Zephyr
`nucleo_l552ze_q` board dts for the LED/button/console wiring — detailed in
[`examples/nucleo-l552ze-q/REQUIRED_DOCS.md`](../../examples/nucleo-l552ze-q/REQUIRED_DOCS.md).
