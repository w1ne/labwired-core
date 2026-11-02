# ESP32-P4 / Guition JC4880P443 — known limitations

This is an **L1 (smoke)** target. The UART0 console path is a real engine; the
list below is what is deliberately *not* modelled, so nothing here reads as
working when it is not.

## Core

- **The LX7-class low-power core is not modelled.** The P4 silicon is
  dual-architecture (HP RISC-V + LP), but this descriptor is the **HP core only**
  — the same scope `esp32c6.yaml` takes for its own LP core. The official
  `esp32p4.svd` names the CPU `RV32IMAFC`.
- **The F (float) and A (atomics) extensions are not decoded.** The interpreter
  (`crates/core/src/decoder/riscv.rs`) implements RV32IMC plus a subset of M; the
  L1 image links the `riscv32imc` subset. A firmware that executes an F or A
  instruction will not run.
- **SMP is out of scope.** The P4 has two HP harts; the twin runs one.

## Memory

- The XIP window (`0x4000_0000`, 64 MiB) is **plain memory** — there is no SPI
  flash controller, MSPI MMU or cache behind it, so an IDF image's mask-ROM boot
  cannot run end-to-end.
- PSRAM (`0x4800_0000`, 16 MiB) is a plain byte-accessible window — no MSPI/DDR
  PHY, no cache.
- The mask ROM (`0x4FC0_0000`) is zero unless `LABWIRED_ESP32P4_ROM` is set; no
  ROM code executes on the bare-ELF path.
- The P4's non-cacheable aliases and the `MAP_DRAM_TO_IRAM` view are not modelled.

## Peripherals

- **GPIO is a declarative register file with no engine.** The P4 GPIO is a
  *two-bank* block (`OUT1`/`ENABLE1`/`IN1`/`STATUS1`), which is not the C3/C6
  single-bank head, so the shared C3 model does not fit. `board_io` LED/button
  entries are declared but not electrically driven.
- **IO_MUX pad routing is recorded, not enforced.** The console shifts bytes
  regardless of which pad function is selected.
- **There is no `clock:` gate.** The P4 clock/reset block is `HP_SYS_CLKRST`
  (different layout from the C6 `PCR`); its tree is not modelled and no
  peripheral is gated on `CLK_EN0`. A firmware that relies on a clock gate runs
  against an ungated twin.
- **The interrupt matrix is not routed.** `INTERRUPT_CORE0` is declarative
  storage; a peripheral source never reaches `mcause`. The L1 smoke is polled.
- **TIMG MWDT is not armed.** The P4 TIMG head matches the C3/C6 and reuses the
  shared `Timg` model as a plain GP timer; the P4 watchdog register block has not
  been verified against a P4 capture, so the MWDT path is off.
- The shared UART model's tail registers past `CONF1` are the C3/S3 offsets, not
  the P4's.

## Not declared at all (unmapped windows fault loudly)

MIPI-DSI host + bridge, MIPI-CSI host + bridge, USB 2.0 OTG + PHY, USB
Serial/JTAG, GDMA, DMA2D, AXI DMA, AHB DMA, H.264, JPEG, PPA, ISP, EMAC, SD/MMC,
LEDC, RMT, PCNT, I2S, SPI0/1/2/3, I2C0/1, TWAI0/1/2, I3C, eFuse, and every radio.

## The panel (this module's whole point)

- **There is no MIPI-DSI controller model.** `crates/core/src/peripherals/` has
  no DSI host, and the declarative display primitive's transports are `gpio`,
  `i2c`, `spi`, `uart`, `analog` and `robotics` — **no `mipi-dsi`**. Every panel
  descriptor the engine ships (`ili9341`, `st7789`, `st7735`, `ssd1306`,
  `sh1107`, `rm67162`, `pcd8544`, `uc8151d`, `ssd1680`, …) is SPI/I²C/GPIO.
- **The capacitive touch controller is not modelled** either.
- Consequence: the `JC4880P443` module is **not** added to the catalog as a
  paintable target. Declaring it would be declaring hardware with no model behind
  it, which this project's `board-compile-coverage` / `board-onboarding` guards
  and the "a chip counts as onboarded when it boots firmware and passes tests"
  rule refuse. The missing capability is named here so the block is actionable:
  **a MIPI-DSI host controller model plus a `mipi-dsi` display transport.**
