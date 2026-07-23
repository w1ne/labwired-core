# SPI L3 declarative device primitive — design

**Status:** approved design, pre-implementation
**Date:** 2026-07-23
**Branch:** `feat/spi-l3-declarative` (worktree `../labwired-core-spi-l3`, off latest `main` @ `92d8a8cb`)

## Why

Per `FIDELITY.md`, the product value is the **hardware oracle** on the
peripheral-facing bus surface. The I²C L3 fleet just landed as a *declarative*
primitive: a datasheet-shaped `behavior.i2c` descriptor + a generic
`GenericI2cDevice` engine + `DeclarativeI2cKit`, so new sensors are **data, not
code**, and they turn green as L3 cells in the Tier-1 matrix across the fleet
(`d258575d` → `9684d689` → `4ff81b13` → `4c60f62d` → `2c55eaf7`).

This is the ranked **#1 next bet**: mirror that win for SPI — the biggest
product mirror of what was just closed. Today SPI devices are all hand-written
`.rs` per part (`max7219`, `ili9341`, `adxl`-none, `nrf24l01`, …). We want the
same declarative path for SPI register sensors.

## Scope

In: a declarative `spi_device` primitive covering **register-style SPI sensors**
(the L3 class), the generic engine, a kit, a test fixture, two real parts, and
the fleet matrix cells.

Out (explicitly not this spec): display/framebuffer SPI devices (e-paper,
ILI9341) — they already have honest hand-written oracles and use a D/C GPIO line
rather than a register-pointer protocol. DMA and IRQ-timing work (bets #2/#4)
are separate specs.

## The framing model (the one real design question)

I²C addresses a slave on a shared bus; SPI selects the chip via **CS** and the
first MOSI byte(s) carry a **device-specific command**. The near-universal
register-sensor shape (ADXL345, BMP280/BME280-SPI, LIS3DH, MAX31855-read-only)
is:

```
CS↓  byte0 = [R/W bit | register address]   byteN… = streamed register data   CS↑
```

- On **read**: MISO returns the addressed register's word (MSB-first typically);
  MOSI data bytes are don't-care.
- On **write**: MOSI bytes accumulate into the addressed register.
- The address commonly **auto-increments** across multi-byte bursts.
- **Read-only** parts (MAX31855) have no command byte at all: CS↓ then the
  master clocks out a fixed-width word from register 0.

So `SpiSpec` = a small **framing** block + the **same `registers` list** the I²C
primitive already defines. The measurement→raw-word machinery (`endian`,
`source`, `encode`, `scale_from`) is protocol-agnostic and reused verbatim.

### `SpiFraming` fields

| Field | Meaning | Default |
|-------|---------|---------|
| `command_bytes` | Width of the leading command word (0 = read-only, no command; 1 = ADXL345-style). | 1 |
| `rw_bit` | Bit position in byte0 that selects read(1)/write(0) — ADXL345 convention. `None` ⇒ direction fixed by register `access`. | 7 |
| `rw_read_high` | If true, `rw_bit`=1 means read (ADXL345). | true |
| `addr_mask` / `addr_shift` | Extract the register address from byte0. | 0x3F / 0 |
| `auto_increment` | Address advances by `width`-aware step across a burst (MB bit). | true |
| `word_msb_first` | Bit/byte order of the returned word beyond per-register `endian`. | true |

A read-only part sets `command_bytes: 0` and a single register at `addr: 0`.

## Components (mirror the I²C trio)

### 1. Config schema — `crates/config/src/lib.rs`
- Add `DeviceBehavior::Spi(SpiSpec)` variant (`behavior.spi`).
- `SpiSpec { framing: SpiFraming, registers: Vec<RegisterSpec> }`.
- **Struct reuse decision:** rename the shared `I2cRegister` / `Encode` /
  `ScaleFrom` to protocol-neutral `RegisterSpec` / `Encode` / `ScaleFrom`, and
  keep `pub type I2cRegister = RegisterSpec;` aliases so the I²C engine and any
  external YAML/tests keep compiling unchanged. `Endian` and `I2cAccess`→
  `RegisterAccess` (aliased) likewise. Both primitives now share one definition.

### 2. Engine — `crates/core/src/peripherals/components/declarative_spi.rs`
- `GenericSpiDevice` implementing `SpiDevice` + `SimInput`:
  - `cs_select` resets the frame; `transfer(mosi)` runs the byte state machine
    (accumulate command → then stream register words per framing); `cs_release`
    flushes partials.
  - `SimInput` serves the same named `metadata.inputs` measurement slots the
    I²C engine does, sharing the encode path.
- `DeclarativeSpiKit` implementing `PeripheralKit` (`Transport::Spi`,
  `Category::Spi`), attaching via `ctx.attach_spi_device(...)`. Config key
  `cs_pin` (like `max7219`).
- `from_yaml` / `from_descriptor` constructors paralleling `declarative_i2c.rs`.

### 3. Test fixture — `declarative_spi_fixture.yaml`
Test-only fictional part exercising every framing field (rw-bit read/write,
auto-increment burst, a sourced+encoded measurement, a read-only single
register). Not registered in `KITS` → peripherals manifest unchanged, exactly
like the I²C fixture.

## Phasing (each its own PR)

1. **Schema + engine + fixture, unit-tested only** (= `d258575d`).
   Struct rename+aliases, `SpiSpec`, `GenericSpiDevice`, `DeclarativeSpiKit`,
   fixture YAML, engine unit tests. Manifest untouched. Gate: `cargo test`.
2. **Register two real SPI sensor kits** (= `9684d689`):
   - **ADXL345** accelerometer — canonical R/W + auto-increment register device.
   - **MAX31855** thermocouple — read-only (`command_bytes: 0`) shape.
   Both have common Arduino + Zephyr drivers usable in the fleet matrix. Gate:
   `cargo test` + manifest snapshot.
3. **Fleet matrix — green SPI L3 cells** (= `4ff81b13`→`2c55eaf7`):
   L3 SPI sensor-sample firmware (read register, print value) across Arduino +
   Zephyr systems on F1 / nRF / RP2040 / C3, wired into the Tier-1 matrix the
   same way the I²C L3 cells are. Gate: the Tier-1 matrix CI cell.

## Verification / oracle

- Engine phases 1–2: Rust unit tests assert byte-exact MISO words for known
  register/measurement inputs, and that auto-increment + rw-bit decode match the
  datasheet framing.
- Phase 3: a booting firmware drives the real bus peripheral (SPI1/SPIM/GPSPI)
  → the generic device → asserts the read value at the app level, exactly the
  I²C L3 fleet contract ("does any firmware actually run against it?"). This is
  the `proven-by-fw` classification in `FIRMWARE_EXERCISE_MATRIX.md`.

## Non-goals / risks

- **Not** a display/DMA path. If a target sensor's driver is interrupt-driven
  and depends on completion timing, that is the temporal-fidelity work (bet #2/
  #4) and out of scope here — L3 sensor reads are poll-shaped.
- Struct rename touches the I²C schema types: mitigated by `type` aliases so no
  I²C-side code or YAML changes. Verified by the existing I²C engine tests
  staying green.

## Deferred findings carried into Phase 2 (from the Phase-1 final review)

- **N1 — auto-increment assumes a contiguous register block.** `build_read_buf`
  concatenates every register with `addr ≥ start` back-to-back with no padding
  for address gaps — byte-correct only for a contiguous-by-width data block
  (ADXL345 DATAX0..DATAZ1 is). **Phase 2 must restrict declarative SPI parts to
  contiguous data blocks**, or the engine needs address-stride awareness.
- **N2 — `width ≤ 4` not validated** in the shared `declarative_regs` helpers
  (`width_max` shift, BE `unpack` into `u32`). Pre-existing in the I²C engine via
  the same shared code; reachable only from malformed config, not firmware. Add a
  `width ≤ 4` check to the shared validator when convenient.
- **M4** — `DeclarativeSpiKit`'s `cs_pin` ConfigKey.doc should state the `PA4`
  default (fold in when real parts land).
- **M5** — `from_yaml` empty-register-list check (cosmetic; `from_descriptor`
  already bails, so `attach` rejects it).
