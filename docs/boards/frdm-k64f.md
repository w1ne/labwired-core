# FRDM-K64F

The **FRDM-K64F** with an **MK64FN1M0VLL12** (Cortex-M4F, 120 MHz max):
1 MiB of program flash at `0x00000000`, 64 KiB SRAM_L at `0x1FFF0000` plus
192 KiB SRAM_U at `0x20000000`, an active-low RGB LED (red `PTB22`, green
`PTE26`, blue `PTB21`), two active-low user buttons (`PTC6`, `PTA4`) and the
OpenSDA Virtual COM Port on **UART0** `PTB16`/`PTB17` (ALT3).

This is the **UART-only L1 slice**. Boot, the UART0 console and the GPIO RGB
LED path (with `PDOR` read-back) are proven by a committed smoke fixture and an
`io-smoke.yaml`. Unmodified upstream **Zephyr v3.7.0** also runs.
`hello_world` boots through Zephyr's own K6x bring-up (SYSMPU, PMC, OSC, MCG
PEE on PLL0). `tests/drivers/uart/uart_basic_api`, the suite Renode's
`NXP-K64F.robot` "Should Run Zephyr Tests for UART" runs, passes 7/7 with the
same typed input, and `samples/synchronization` runs its two-thread semaphore
hand-off on SysTick timeouts and PendSV context switches. The Ethernet/TCP
milestone is explicitly deferred.

!!! warning "Not claimed yet"
    SIM clock gating and PORT pad mux/pulls are **not modelled**. The MCG
    PLL0 reports lock immediately and its output frequency is not derived. The
    SYSMPU, PMC and OSC are register files: no MPU protection, LVD or
    oscillator is modelled. The Ethernet MAC + KSZ8081 PHY, RNGA and FTFE are declared
    stubs. eDMA, PIT/FTM, I²C/SPI, ADC/DAC, USB, CAN and SDHC are **not
    mapped**: firmware that touches them faults with a memory violation. There
    is no silicon capture: **SIM-DERIVED**.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/mk64f12.yaml`](../../configs/chips/mk64f12.yaml) |
| Example system | [`configs/systems/frdm-k64f.yaml`](../../configs/systems/frdm-k64f.yaml) |
| Reference firmware | [`crates/firmware-frdm-k64f-smoke/`](../../crates/firmware-frdm-k64f-smoke/) |
| Example | [`examples/frdm-k64f/`](../../examples/frdm-k64f/) |
| Committed artifact | `tests/fixtures/frdm-k64f-smoke.elf` (`FRDM-K64F SMOKE`) |
| Build target | `thumbv7em-none-eabi` |
| Tier (snapshot) | **L1** — smoke (CPU + memory + UART0 + GPIO); SIM-DERIVED |

---

## Memory map and board wiring

| Region / signal | Base / pin | State |
|--------|------|-------|
| Flash | `0x0000_0000`, 1 MiB | modelled — the smoke links and runs here |
| SRAM_L / SRAM_U | `0x1FFF_0000` 64 KiB / `0x2000_0000` 192 KiB | modelled — stack in SRAM_U |
| SIM | `0x4004_7000` | `type: stub` — clock gates are not modelled |
| MCG | `0x4006_4000` | modelled (`nxp_mcg`, `profile: k6x`) — FEI reset; PLL0 `PLLST`/`LOCK0`, PEE `CLKST=11` (lock immediate) |
| OSC0 / PMC | `0x4006_5000` / `0x4007_D000` | register files from the NXP SVD — Zephyr writes `OSC_CR` and `PMC_REGSC.ACKISO` |
| SYSMPU | `0x4000_D000` | register file from the NXP SVD (`CESR 0x0081_5101`, 12 RGDs) — no protection enforced |
| UART0 (console) | `0x4006_A000` | modelled — classic Kinetis 8-bit UART, IRQ 31; C2 `TIE`/`TCIE`/`RIE` interrupts |
| UART1–4 | `0x4006_B000`… / `0x400E_A000` | modelled — same layout, unexercised |
| GPIOA–E | `0x400F_F000` + `0x40`×n | modelled — PDOR/PSOR/PCOR/PDIR/PDDR |
| PORTA–E | `0x4004_9000` + `0x1000`×n | `type: stub` — pad mux/pulls not modelled |
| WDOG | `0x4005_2000` | `type: stub` — no timeout/reset; the smoke disables it as silicon requires |
| RGB LED | red PTB22, green PTE26, blue PTB21 | modelled GPIO — active low |
| Buttons | SW2 PTC6, SW3 PTA4 | declared `board_io` — not read by the smoke |
| UART0 VCP | PTB16 / PTB17, ALT3 | simulator captures the UART0 stream; OpenSDA/USB not modelled |
| ENET + KSZ8081 PHY | `0x400C_0000` | `type: stub` — TCP test deferred |
| RNGA / FTFE / eDMA / PIT / FTM / I²C / SPI / ADC / DAC / USB / CAN / SDHC | see descriptor | not modelled |

## Boot path

**Fast boot.** The engine loads the ELF and enters it at its entry point
(vector table at `0x0000_0000`) with SP seeded at the top of SRAM_U
(`0x2003_0000`). The Kinetis ROM bootloader and the OpenSDA debug bridge are
not modelled.

## Validation

```bash
# from the repository root
cargo build -p firmware-frdm-k64f-smoke --release --target thumbv7em-none-eabi
./target/release/labwired test --script examples/frdm-k64f/io-smoke.yaml --no-uart-stdout
# stock Zephyr (committed fixtures, rebuilt by crates/firmware-frdm-k64f-zephyr/build.sh)
./target/release/labwired test --script examples/frdm-k64f/zephyr-hello.yaml
./target/release/labwired test --script examples/frdm-k64f/zephyr-uart-basic-api.yaml
./target/release/labwired test --script examples/frdm-k64f/zephyr-sync.yaml
```

See [`examples/frdm-k64f/VALIDATION.md`](../../examples/frdm-k64f/VALIDATION.md)
for the claims, runtime evidence and explicit non-claims.

## Sources

The K64 Sub-Family Reference Manual (SIM/MCG/UART/GPIO/PORT/FTFE), the
MK64F12 datasheet (memory and pins), the NXP FRDM-K64F user manual and the
Zephyr `frdm_k64f` board dts for the LED/button/console wiring — detailed in
[`examples/frdm-k64f/REQUIRED_DOCS.md`](../../examples/frdm-k64f/REQUIRED_DOCS.md).
