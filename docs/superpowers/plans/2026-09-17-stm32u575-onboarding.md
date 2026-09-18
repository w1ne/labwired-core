# STM32U575ZI Onboarding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Boot real STM32CubeU5 HAL and Arduino-core firmware on a modeled STM32U575ZI (NUCLEO-U575ZI-Q) and prove it through LabWired's existing fidelity engine (io-smoke, onboarding smoke, Arduino matrix, audit, validation manifest).

**Architecture:** Datasheet/SVD-derived chip + system YAML reuses the shared V2 STM32 peripheral models (WBA52/H5 siblings); any model gap surfaced by running firmware gets a focused unit test plus the smallest fix. The spec is `docs/superpowers/specs/2026-09-17-stm32u575-onboarding-design.md`.

**Tech Stack:** Rust 1.95.0 (pinned), arm-none-eabi-gcc 13.2, STM32CubeU5 HAL, STM32duino 3.0.0 / PlatformIO ststm32, Python validation scripts.

**Worktree:** `/home/andrii/projects/labwired-u5-onboarding`, branch `feat/onboard-stm32u575`. All paths below are relative to the worktree root (the `core/` repo root) unless absolute. `CARGO_TARGET_DIR` is NOT redirected; the default `target/` is what io-smoke paths promise.

**Commit policy:** do not commit unless the user asks. Each task still ends with a "checkpoint" listing exactly what would be staged.

---

### Task 1: Baseline + SVD ingest

**Files:**
- Create: `tests/fixtures/real_world/stm32u575.svd`
- Create: `configs/peripherals/stm32u575/*.yaml` (generated)

- [ ] **Step 1: Add the M33 Rust target to the pinned toolchain**

Run:
```bash
rustup target add thumbv8m.main-none-eabi
rustup target list --installed | grep thumbv8m
```
Expected: `thumbv8m.main-none-eabi` listed. `rust-toolchain.toml` pins 1.95.0 so rustup installs the target for that toolchain.

- [ ] **Step 2: Baseline green check**

Run:
```bash
cargo test -p labwired-core --lib peripherals::rcc -- --nocapture
```
Expected: all `v2_*` / `f4_*` RCC tests pass on unmodified `main`. If anything fails, stop and report.

- [ ] **Step 3: Vendor the ST SVD**

Run:
```bash
curl -sL -o tests/fixtures/real_world/stm32u575.svd \
  https://raw.githubusercontent.com/cmsis-svd/cmsis-svd-data/main/data/STMicro/STM32U575.svd
sha256sum tests/fixtures/real_world/stm32u575.svd
```
Expected hash: `2e45c70a3387a46a07092b145301e7bee3e95480377e67e252db6bf4ecfd701a` (matches the copy fetched 2026-09-17; note it in `REQUIRED_DOCS.md` later).

- [ ] **Step 4: Generate debug schemas**

Run:
```bash
python3 scripts/gen_debug_schemas.py \
  --svd tests/fixtures/real_world/stm32u575.svd \
  --out configs/peripherals/stm32u575
ls configs/peripherals/stm32u575 | wc -l
```
Expected: one YAML per SVD peripheral (RCC, PWR, GPIOA..I, USART1, LPUART1, I2C1..4, SPI1..3, TIMx, GPDMA1, ADC1, RTC, IWDG, ICACHE, DBGMCU, ...).

- [ ] **Step 5: Checkpoint** — staged set: the `.svd` + the generated directory. No commit.

---

### Task 2: Chip descriptor `configs/chips/stm32u575.yaml`

**Files:**
- Create: `configs/chips/stm32u575.yaml`

Source values: `tests/fixtures/real_world/stm32u575.svd` (bases/IRQs), Zephyr `stm32u575Xi.dtsi` (2 MiB flash, 768 KiB SRAM1+2+3), SVD DBGMCU_IDCODE reset `0x30016482`.

- [ ] **Step 1: Write the chip YAML**

```yaml
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
#
# Source references:
# - ST SVD (bases/IRQs): tests/fixtures/real_world/stm32u575.svd
#   (cmsis-svd-data STMicro/STM32U575.svd, sha256
#   2e45c70a3387a46a07092b145301e7bee3e95480377e67e252db6bf4ecfd701a)
# - Memory sizes: DS13736 + Zephyr dts/arm/st/u5/stm32u575Xi.dtsi
#   (flash 2 MiB @0x08000000; SRAM1+2+3 768 KiB @0x20000000; SRAM4 16 KiB @0x28000000).
# - Board VCP (USART1 PA9/PA10, 115200 8N1): stm32u5xx-nucleo BSP COM1 +
#   Zephyr nucleo_u575zi_q board docs.
#
# No bench part: reset values below are SVD/RM0456-derived unless a comment
# says otherwise. Same-family silicon pins: WBA52 shares this V2 RCC layout.
#
# Known divergences / not modeled (first pass):
# - TrustZone/GTZC/SAU enforcement: not modeled (factory TZEN=0 per UM2883,
#   so non-secure alias 0x08000000 is the boot path).
# - OCTOSPI/HSPI, USB OTG, FDCAN, ethernet, ADC4: not declared.
# - FLASH programming semantics beyond HAL latency/option-lock: not modeled.
# - Clock gating (`clock:` gates) is not declared yet, so the RCC model is
#   permissive about missing clock-enable bits on U5 for now.

name: "stm32u575zi"
arch: "arm"
core: "cortex-m33"
# STM32U575ZI maximum CPU frequency, 160 MHz (DS13736).
cpu_hz: 160_000_000

flash:
  base: 0x08000000
  # Binary units so the buffer is exactly 2 * 0x100000 bytes (two 1 MiB banks on
  # real silicon); SI "2MB" would parse as 2,000,000 and break bank math.
  size: "2MiB"
ram:
  base: 0x20000000
  # SRAM1+SRAM2+SRAM3 = 768 KiB contiguous (Zephyr stm32u575Xi.dtsi).
  size: "768KiB"
memory_regions:
  # SRAM4, 16 KiB (DS13736 + RM0456; SVD SRAM4 base 0x28000000).
  - name: "sram4"
    base: 0x28000000
    size: "16KiB"

peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x46020C00
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/rcc.yaml"
      profile: "stm32v2"
  - id: "pwr"
    type: "pwr"
    base_address: 0x46020800
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/pwr.yaml"
      profile: "wba"
  # FLASH: the "l4" profile is ACR/LATENCY-approximate only; U5 NSKEYR/
  # SECKEYR/OPTKEYR (0x08/0x0C/0x10) fit neither L4 nor H5 map, not modeled.
  - id: "flash"
    type: "flash"
    base_address: 0x40022000
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/flash.yaml"
      profile: "l4"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x42020000
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/gpioa.yaml"
      profile: "stm32v2"
  - id: "gpiob"
    type: "gpio"
    base_address: 0x42020400
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/gpiob.yaml"
      profile: "stm32v2"
  - id: "gpioc"
    type: "gpio"
    base_address: 0x42020800
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/gpioc.yaml"
      profile: "stm32v2"
  - id: "gpiod"
    type: "gpio"
    base_address: 0x42020C00
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/gpiod.yaml"
      profile: "stm32v2"
  - id: "gpioe"
    type: "gpio"
    base_address: 0x42021000
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/gpioe.yaml"
      profile: "stm32v2"
  - id: "gpiof"
    type: "gpio"
    base_address: 0x42021400
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/gpiof.yaml"
      profile: "stm32v2"
  - id: "gpiog"
    type: "gpio"
    base_address: 0x42021800
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/gpiog.yaml"
      profile: "stm32v2"
  - id: "gpioh"
    type: "gpio"
    base_address: 0x42021C00
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/gpioh.yaml"
      profile: "stm32v2"
  - id: "gpioi"
    type: "gpio"
    base_address: 0x42022000
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/gpioi.yaml"
      profile: "stm32v2"
  # USART1 is the NUCLEO-U575ZI-Q VCP console (PA9/PA10). IRQ 61 per SVD.
  - id: "usart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    irq: 61
    config:
      debug_schema: "../peripherals/stm32u575/usart1.yaml"
      profile: "stm32v2"
  - id: "usart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    irq: 62
    config:
      debug_schema: "../peripherals/stm32u575/usart2.yaml"
      profile: "stm32v2"
  - id: "usart3"
    type: "uart"
    base_address: 0x40004800
    size: "1KB"
    irq: 63
    config:
      debug_schema: "../peripherals/stm32u575/usart3.yaml"
      profile: "stm32v2"
  - id: "uart4"
    type: "uart"
    base_address: 0x40004C00
    size: "1KB"
    irq: 64
    config:
      debug_schema: "../peripherals/stm32u575/uart4.yaml"
      profile: "stm32v2"
  - id: "lpuart1"
    type: "uart"
    base_address: 0x46002400
    size: "1KB"
    irq: 66
    config:
      debug_schema: "../peripherals/stm32u575/lpuart1.yaml"
      profile: "stm32v2"
  # I2C1 is the default Arduino Wire bus. Modern L4-class I2C IP (same profile
  # WBA52 wires). Event IRQ 55; error IRQ 56 not declared (single-line model).
  - id: "i2c1"
    type: "i2c"
    base_address: 0x40005400
    size: "1KB"
    irq: 55
    config:
      debug_schema: "../peripherals/stm32u575/i2c1.yaml"
      profile: "stm32l4"
  - id: "i2c2"
    type: "i2c"
    base_address: 0x40005800
    size: "1KB"
    irq: 57
    config:
      debug_schema: "../peripherals/stm32u575/i2c2.yaml"
      profile: "stm32l4"
  - id: "i2c3"
    type: "i2c"
    base_address: 0x46002800
    size: "1KB"
    irq: 88
    config:
      debug_schema: "../peripherals/stm32u575/i2c3.yaml"
      profile: "stm32l4"
  # SPI1 is the default Arduino SPI bus (PA5 SCK / PA6 MISO / PA7 MOSI / PA4 NSS).
  # No pad_map for SPI1-3 on this first pass: per-pad logic-analyzer publishing
  # is off; the H5 AF table is not transcribed yet.
  - id: "spi1"
    type: "spi"
    base_address: 0x40013000
    size: "1KB"
    irq: 59
    config:
      debug_schema: "../peripherals/stm32u575/spi1.yaml"
      profile: "stm32h5"
  - id: "spi2"
    type: "spi"
    base_address: 0x40003800
    size: "1KB"
    irq: 60
    config:
      debug_schema: "../peripherals/stm32u575/spi2.yaml"
      profile: "stm32h5"
  - id: "spi3"
    type: "spi"
    base_address: 0x46002000
    size: "1KB"
    irq: 99
    config:
      debug_schema: "../peripherals/stm32u575/spi3.yaml"
      profile: "stm32h5"
  # TIM2/TIM5 are the 32-bit general-purpose timers on U5.
  - id: "tim2"
    type: "timer"
    base_address: 0x40000000
    size: "1KB"
    irq: 45
    config:
      debug_schema: "../peripherals/stm32u575/tim2.yaml"
      width: 32
  - id: "tim3"
    type: "timer"
    base_address: 0x40000400
    size: "1KB"
    irq: 46
    config:
      debug_schema: "../peripherals/stm32u575/tim3.yaml"
  - id: "tim6"
    type: "timer"
    base_address: 0x40001000
    size: "1KB"
    irq: 49
    config:
      debug_schema: "../peripherals/stm32u575/tim6.yaml"
      basic: true
  - id: "tim7"
    type: "timer"
    base_address: 0x40001400
    size: "1KB"
    irq: 50
    config:
      debug_schema: "../peripherals/stm32u575/tim7.yaml"
      basic: true
  - id: "tim1_pwm"
    type: "timer"
    base_address: 0x40012C00
    size: "1KB"
    irq: 42
    config:
      debug_schema: "../peripherals/stm32u575/tim1.yaml"
      advanced: true
  - id: "gpdma1"
    type: "gpdma"
    base_address: 0x40020000
    # U5 GPDMA1 has 16 channels (SVD registers to ~0x84C); the model implements
    # only 8. 8 x 0x80 + 0x50 base exceeds 1KB, so the window must be 2KB.
    size: "2KB"
    irq: 29
    config:
      debug_schema: "../peripherals/stm32u575/gpdma1.yaml"
      irq_base: 29
  # ADC1 is the analogRead path on the Arduino core, clocked from AHB2ENR.ADCEN.
  - id: "adc1"
    type: "adc"
    base_address: 0x42028000
    size: "1KB"
    irq: 37
    config:
      debug_schema: "../peripherals/stm32u575/adc1.yaml"
      # U5 ADC is NOT the L4/H5 map and NOT exactly the H7 map either. Verified
      # from the vendored SVD: CFGR1.RES is 2 bits at [3:2] (L4: [4:3], H7:
      # 3 bits [4:2]) while PCSEL/LTR1-3/HTR1-3/CALFACT2 are present like the
      # H7. "stm32h7" is the closest existing layout; the RES delta is
      # documented, not modeled. analogRead (matrix L5_adc) stays skipped.
      profile: "stm32h7"
  - id: "rtc"
    type: "rtc_v3"
    base_address: 0x46007800
    size: "1KB"
    irq: 2
    config:
      debug_schema: "../peripherals/stm32u575/rtc.yaml"
  - id: "iwdg"
    type: "iwdg"
    base_address: 0x40003000
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/iwdg.yaml"
  - id: "crc"
    type: "crc"
    base_address: 0x40023000
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/crc.yaml"
      idr_width: 32
  - id: "rng"
    type: "rng"
    base_address: 0x420C0800
    size: "1KB"
    irq: 94
    config:
      debug_schema: "../peripherals/stm32u575/rng.yaml"
  - id: "icache"
    type: "icache"
    base_address: 0x40030400
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/icache.yaml"
  # U5 DBGMCU base is 0xE0044000 (not the M4 0xE0042000). IDCODE reset
  # 0x30016482 per SVD (DEV_ID 0x482, REV_ID 0x3001).
  - id: "dbgmcu"
    type: "dbgmcu"
    base_address: 0xE0044000
    size: "1KB"
    config:
      debug_schema: "../peripherals/stm32u575/dbgmcu.yaml"
      idcode: 0x30016482
  # SysTick is required by every HAL/Arduino delay path; declare it so the
  # scheduler-backed counter is mapped at 0xE000E010. CALIB is left at the
  # model default (NoRef) — no bench silicon capture for a U5 TENMS value yet.
  - id: "systick"
    type: "systick"
    base_address: 0xE000E010
  - id: "nvic"
    type: "nvic"
    base_address: 0xE000E100
    size: "1KB"
```

- [ ] **Step 2: Validate the YAML parses + register coverage**

Run:
```bash
cargo test -p labwired-config 2>&1 | tail -5
cargo test -p labwired-core --test register_coverage stm32u575 -- --nocapture 2>&1 | tail -20
```
Expected: config tests pass. `register_coverage` has no U575 entry yet (added in Task 6), so the second command only proves the YAML parses when referenced; if it aborts on no-match, that is fine at this step.

- [ ] **Step 3: Reject obvious typos**

Run:
```bash
python3 - <<'EOF'
import yaml
d = yaml.safe_load(open('configs/chips/stm32u575.yaml'))
ids = [p['id'] for p in d['peripherals']]
assert len(ids) == len(set(ids)), f"duplicate ids: {ids}"
assert d['core'] == 'cortex-m33' and d['cpu_hz'] == 160_000_000
print(f"{len(ids)} peripherals, RAM {d['ram']['size']}, flash {d['flash']['size']}")
EOF
```
Expected: `Nx peripherals, RAM 768KiB, flash 2MiB`.

- [ ] **Step 4: Regenerate embedded descriptors (wasm lookup table)**

Run:
```bash
python3 scripts/gen_embedded_descriptors.py
git diff --stat crates/core/src/bus/embedded_descriptors.rs
```
Expected: the file gains `stm32u575/*` include_str! entries.

- [ ] **Step 5: Checkpoint** — staged set: chip YAML + embedded_descriptors.rs.

---

### Task 3: System manifest, Rust smoke crate, example smokes

**Files:**
- Create: `configs/systems/nucleo-u575zi.yaml`
- Create: `examples/nucleo-u575zi/system.yaml`
- Create: `examples/nucleo-u575zi/io-smoke.yaml`
- Create: `examples/nucleo-u575zi/uart-smoke.yaml`
- Create: `crates/firmware-stm32u575-demo/{Cargo.toml,build.rs,memory.x,minimal.ld,src/main.rs}`
- Modify: `Cargo.toml` (workspace members + `[profile.release.package.firmware-stm32u575-demo]`)

- [ ] **Step 1: Board system manifest (TOP20 row-19 path)**

`configs/systems/nucleo-u575zi.yaml`:
```yaml
# LabWired - NUCLEO-U575ZI-Q board system (STM32U575ZI).
# Board references: UM2861 (NUCLEO-U575ZI-Q user manual), stm32u5xx-nucleo BSP
# (COM1 = USART1 PA9/PA10), Zephyr nucleo_u575zi_q board docs (LEDs).
name: "nucleo-u575zi"
chip: "../chips/stm32u575.yaml"
debug_uart: "usart1"
external_devices: []
board_io:
  - id: "led_green_ld1"
    kind: "led"
    peripheral: "gpioc"
    pin: 7
    signal: "output"
    active_high: true
  - id: "led_blue_ld2"
    kind: "led"
    peripheral: "gpiob"
    pin: 7
    signal: "output"
    active_high: true
  - id: "led_red_ld3"
    kind: "led"
    peripheral: "gpiog"
    pin: 2
    signal: "output"
    active_high: true
  - id: "button_user_pc13"
    kind: "button"
    peripheral: "gpioc"
    pin: 13
    signal: "input"
    active_high: true
```

- [ ] **Step 2: Example system manifest (used by smokes)**

`examples/nucleo-u575zi/system.yaml`:
```yaml
# LabWired - NUCLEO-U575ZI-Q example system configuration.
# Same board IO as configs/systems/nucleo-u575zi.yaml; kept as its own file so
# the example is self-contained the way examples/nucleo-h563zi is.
name: "nucleo-u575zi-example"
chip: "../../configs/chips/stm32u575.yaml"
debug_uart: "usart1"
external_devices: []
board_io:
  - id: "led_green_ld1"
    kind: "led"
    peripheral: "gpioc"
    pin: 7
    signal: "output"
    active_high: true
  - id: "led_blue_ld2"
    kind: "led"
    peripheral: "gpiob"
    pin: 7
    signal: "output"
    active_high: true
  - id: "led_red_ld3"
    kind: "led"
    peripheral: "gpiog"
    pin: 2
    signal: "output"
    active_high: true
  - id: "button_user_pc13"
    kind: "button"
    peripheral: "gpioc"
    pin: 13
    signal: "input"
    active_high: true
```

- [ ] **Step 3: Rust smoke firmware crate**

`crates/firmware-stm32u575-demo/Cargo.toml`:
```toml
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

[package]
name = "firmware-stm32u575-demo"
version.workspace = true
edition = "2021"

[dependencies]
panic-halt = "0.2"

[[bin]]
name = "firmware-stm32u575-demo"
path = "src/main.rs"
test = false
bench = false
```

`crates/firmware-stm32u575-demo/build.rs` (copy of `crates/firmware-h563-demo/build.rs`, verbatim):
```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    File::create(out.join("minimal.ld"))
        .unwrap()
        .write_all(include_bytes!("minimal.ld"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg=-Tminimal.ld");

    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=minimal.ld");
}
```

`crates/firmware-stm32u575-demo/memory.x`:
```
/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 2M
  RAM : ORIGIN = 0x20000000, LENGTH = 768K
}
```

`crates/firmware-stm32u575-demo/minimal.ld`:
```
/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 2M
  RAM : ORIGIN = 0x20000000, LENGTH = 768K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x200C0000); /* Initial SP: 768 KB SRAM top */
    LONG(Reset | 1);  /* Reset handler (Thumb bit set) */
  } > FLASH

  .text :
  {
    *(.text*)
    *(.rodata*)
  } > FLASH

  /DISCARD/ :
  {
    *(.ARM.exidx*)
    *(.note.gnu.build-id*)
  }
}
```

`crates/firmware-stm32u575-demo/src/main.rs`:
```rust
#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

// NUCLEO-U575ZI-Q VCP maps COM1 to USART1 (PA9/PA10) in stm32u5xx_nucleo.c.
// TDR offset 0x28 per RM0456 (USART v2 register map, same as WBA52/H563).
const USART1_TDR_PTR: *mut u8 = (0x4001_3800 + 0x28) as *mut u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(USART1_TDR_PTR, b'O');
        core::ptr::write_volatile(USART1_TDR_PTR, b'K');
        core::ptr::write_volatile(USART1_TDR_PTR, b'\n');
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
```

- [ ] **Step 4: Register the crate in the workspace**

Edit `Cargo.toml` members list, inserting alphabetically after `"crates/firmware-h563-io-demo"`:
```toml
    "crates/firmware-stm32u575-demo",
```
Also add the matching per-package release profile block (same fields as
`firmware-h563-demo`'s, placed after the `firmware-h563-io-demo` block):
```toml
[profile.release.package.firmware-stm32u575-demo]
codegen-units = 1
opt-level = "s"
```

- [ ] **Step 5: Smoke scripts**

`examples/nucleo-u575zi/uart-smoke.yaml`:
```yaml
# LabWired - NUCLEO-U575ZI-Q UART smoke test
schema_version: "1.0"
inputs:
  firmware: "../../target/thumbv8m.main-none-eabi/release/firmware-stm32u575-demo"
  system: "./system.yaml"
limits:
  max_steps: 64
assertions:
  - uart_contains: "OK"
  - expected_stop_reason: max_steps
```

`examples/nucleo-u575zi/io-smoke.yaml`:
```yaml
# LabWired - NUCLEO-U575ZI-Q IO smoke test
# The Rust demo writes OK over USART1 (VCP) and idles; strict_onboarding.rs
# requires this exact filename for every chip with an example directory.
schema_version: "1.0"
inputs:
  firmware: "../../target/thumbv8m.main-none-eabi/release/firmware-stm32u575-demo"
  system: "./system.yaml"
limits:
  max_steps: 64
assertions:
  - uart_contains: "OK"
  - expected_stop_reason: max_steps
```

- [ ] **Step 6: Build + run the smoke**

Run:
```bash
cargo build -p firmware-stm32u575-demo --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-u575zi/io-smoke.yaml
```
Expected: PASS immediately (`OK` in the UART) — the direct-TDR demo does not exercise RCC/PWR/FLASH, so no model fixes are needed for this step. If the YAML fails to load, fix that before touching models.

- [ ] **Step 7: Checkpoint** — staged set: configs/systems, examples/nucleo-u575zi, new crate, root Cargo.toml.

---

### Task 4: First boot — RCC/PWR model gaps (TDD)

**Files:**
- Modify: `crates/core/src/peripherals/rcc.rs` (`V2Rcc` + tests)
- Modify: `crates/core/src/peripherals/pwr.rs` if the WBA profile does not fit U5
- Modify: `crates/core/src/peripherals/uart.rs` only if PRESC read-back is required

The Task 3 smoke already PASSes as committed: the direct-TDR Rust demo never touches
RCC/PWR/FLASH, and the U5 descriptor declares no `clock:` gates, so USART1 writes land
without any clock-enable and the run stops at `max_steps`. (Verified 2026-09-17:
`cargo run -q -p labwired-cli -- test --script examples/nucleo-u575zi/io-smoke.yaml`
→ `PASS 2/2 checks · io-smoke · 64 steps`; a `--max-steps 200 --trace` run shows no
unmapped access and idles at PC=0x8000020.) The TDD red for this task therefore starts
at the `v2_u5_pll1_registers_round_trip_and_ready` unit test below; the ARM-side run
that actually exercises RCC/PWR/FLASH is Task 5's Cube HAL firmware.

- [ ] **Step 1: Failing unit test — U5 PLL1 registers must round-trip**

Add to the `mod tests` in `crates/core/src/peripherals/rcc.rs`, next to `v2_wb_wba_backup_and_switch_gates`:

```rust
#[test]
fn v2_u5_pll1_registers_round_trip_and_ready() {
    // STM32U5 (RM0456) / WBA put PLL1CFGR at 0x28, PLL1DIVR at 0x34,
    // PLL1FRACR at 0x38. Today 0x28 is a request/ack hack that corrupts
    // PLL1CFGR reads, and 0x34/0x38 read 0.
    let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);

    // PLL1CFGR: source MSIS, M=1, REN|PEN, RGE range 0.
    let pll1cfgr: u32 = 0x0000_1401;
    rcc.write_reg(0x28, pll1cfgr);
    assert_eq!(rcc.read_reg(0x28), pll1cfgr, "PLL1CFGR must read back");

    // PLL1DIVR reset value per SVD is 0x01010280 (N=0x80, P=1, Q=1, R=2).
    rcc.write_reg(0x34, 0x0101_0280);
    assert_eq!(rcc.read_reg(0x34), 0x0101_0280);

    rcc.write_reg(0x38, 0x0000_0000);
    assert_eq!(rcc.read_reg(0x38), 0x0000_0000);

    // CR.PLL1ON (bit 24) must gate CR.PLL1RDY (bit 25) like the classic path.
    rcc.write_reg(0x00, 1 << 24);
    assert_ne!(rcc.read_reg(0x00) & (1 << 25), 0, "PLL1RDY follows PLL1ON");
    rcc.write_reg(0x00, 0);
    assert_eq!(rcc.read_reg(0x00) & (1 << 25), 0, "PLL1RDY follows PLL1ON off");
}
```

- [ ] **Step 2: Run it — expected FAIL**

Run:
```bash
cargo test -p labwired-core --lib v2_u5_pll1_registers_round_trip_and_ready -- --nocapture
```
Expected: fails on the `0x28` read-back (the request/ack arm masks bit 20/22).

- [ ] **Step 3: Implement U5 PLL1 storage**

In `V2Rcc`, add fields:
```rust
    /// WBA/U5 RCC_PLL1CFGR @ 0x28 (RM0493/RM0456). On U5 this is a plain
    /// storage register, NOT the request/ack pair the G4/WB-era model assumed.
    #[serde(default)]
    pll1cfgr: u32, // 0x28
    /// WBA/U5 RCC_PLL1DIVR @ 0x34, reset 0x01010280.
    #[serde(default = "pll1divr_reset")]
    pll1divr: u32, // 0x34
    /// WBA/U5 RCC_PLL1FRACR @ 0x38.
    #[serde(default)]
    pll1fracr: u32, // 0x38
```
with
```rust
fn pll1divr_reset() -> u32 {
    0x0101_0280
}
```
Replace the `0x28 => { request/ack }` read arm with `0x28 => self.pll1cfgr` and add `0x34 => self.pll1divr`, `0x38 => self.pll1fracr`. Replace `0x28 => self.reg28 = value` in `write_reg` with `0x28 => self.pll1cfgr = value`, and add `0x34 => self.pll1divr = value`, `0x38 => self.pll1fracr = value`.

**WBA/G474 regression guard:** before changing behavior, capture current WBA PLL1CFGR read semantics:
```bash
cargo test -p labwired-core --lib v2_wb_wba_backup_and_switch_gates -- --nocapture
```
Expected pass before and after; if any WBA-specific test references `reg28`, update it in the same change and say why in a comment.

**Design addendum (approved 2026-09-17, after a BLOCKED report):** plain storage
at 0x28 is wrong for WBA. RM0493 §7.7.13 defines PLL1RCLKPRE (bit20, RW) and
PLL1RCLKPRERDY (bit22, read-only); Zephyr's `clock_stm32_ll_wba.c` and the
Cube HAL `HAL_RCC_ClockConfig` both clear bit20 and spin on bit22, and the two
committed WBA firmware survival tests fail with empty UART under storage-only.
U5 (RM0456) has no field at bits 19-31, so storage is correct there. Therefore:

- `RccRegisterLayout::Stm32Wba` (`"stm32wba" | "wba"`) maps to
  `V2Rcc::new_wba()`, which sets `wba_rclk_pre_ready: bool`; the 0x28 read arm
  then reproduces the exact old synthetic handshake (`bit22 = !bit20`, other
  bits as stored). `Stm32V2` (U5) never sets the flag and is plain storage.
- `configs/chips/stm32wba52.yaml` rcc entry changes `profile: "stm32v2"` →
  `profile: "stm32wba"`. WB55 (`stm32wb`) and G474 (`stm32g4`) are untouched.
- `v2_wb_wba_backup_and_switch_gates` asserts the backup/CFGR1 gates on both
  V2 layouts and the 0x28 split per family.

**Final-review map correction (2026-09-18): `Stm32V2` is U5-only.** By the
time WBA/WB/G4 got their own profiles, `Stm32V2` was reachable only from
`stm32u575` (`V2Rcc::new_u5`), yet its decode still served the classic G4/WB
map on that instance (CFGR@0x08 with SWS gating + forced bits16-18,
PLLCFGR@0x0C, BDCR@0x90, CSR@0x94, CRRCR@0x98; `rcc_reg_offset` returning
CRRCR=0x98). Corrected against `configs/peripherals/stm32u575/rcc.yaml`:

- U5 clock tree: ICSCR1/2/3 @0x08/0x0C/0x10 (SVD resets 0x44000000 /
  0x084210 / 0x00100000), CRRCR@0x14, CFGR1@0x1C (SW 00 MSIS→MSISRDY bit2,
  01 HSI16→bit10, 10 HSE→bit17, 11 PLL1→bit25), AHB2ENR2@0x90,
  AHB3ENR@0x94. All classic arms stay behind `!u5_cr_ready`.
- **BDCR@0xF0 correction:** the review brief said "LSE handshake at 0xF0,
  LSI handshake at CSR@0xF4". The vendored schema and CubeU5
  `stm32u5xx_hal_rcc.c` (lines 809-992, 1732) show U5 keeps *all three*
  handshakes in BDCR — LSEON(0)→LSERDY(1), LSESYSEN(7)→LSESYSRDY(11),
  LSION(26)→LSIRDY(27) — and CSR@0xF4 holds only reset flags + MSIS/MSIK
  ranges. The LSE-only arm hung the Arduino L0 fixture in
  `HAL_RCC_OscConfig` polling LSESYSRDY(bit11); the board doc and tests now
  assert the correct map.
- `rcc_reg_offset`: CRRCR→0x14 on U5, `cfgr1`→0x1C on U5/WBA; AHB1ENR@0x88,
  AHB2ENR2@0x90, AHB3ENR@0x94, APB3ENR@0xA8 intentionally stay out of
  `V2EnrMap` (no U5 `clock:` gates declared yet).

Commits: `0dd75b2b fix(rcc): make the stm32v2 layout U5-faithful`,
`2a795d09 fix(cpu): implement RRX immediate shifts` (the board doc's RRX gap
entry is gone). Proof: v2_ 20 pass, full lib 3545 pass / 0 fail, WB55/WBA52/
G474/H563 filters green, HAL 20M `U575-HAL OK` + `BLINK 0 LD1=1`, Arduino
matrix 7 pass/2 skip, Zephyr matrix 4/4.

- [ ] **Step 4: Re-run the unit test — expected PASS**

Run:
```bash
cargo test -p labwired-core --lib v2_ -- --nocapture
```
Expected: all V2 tests pass, including the new one.

- [ ] **Step 5: Re-run the smoke and fix the next gap**

Run:
```bash
cargo build -p firmware-stm32u575-demo --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-u575zi/io-smoke.yaml
```
Repeat TDD (failing test → fix → green) for each next gap. Expected candidates, in order:
- `PwrWba` VOSR reads: U5 PWR_VOSR reset is 0x0000_8000 (SVD) and VOS bits are [3:2]; if the run shows a `PWR` census miss, extend `PwrWba` with a U5 arm rather than forking a new struct, and add a unit test `pwr_u5_vosr_ready_and_reset`.
- FLASH ACR latency read-back: if the firmware spins, check the `l4` profile covers `ACR@0x00` read/write for the U5 base; add a test `flash_u5_acr_latency_round_trip` in `crates/core/src/peripherals/flash.rs`.
- UART PRESC@0x2C: only if the HAL polls it back; add storage + a test in `uart.rs` following `v2_brr_read_back`.

- [ ] **Step 6: Checkpoint** — staged set: rcc.rs (+pwr.rs/flash.rs/uart.rs as surfaced), with tests.

---

### Task 5: STM32CubeU5 HAL firmware

**Files:**
- Create: `examples/nucleo-u575zi/board_firmware/main.c`
- Create: `examples/nucleo-u575zi/board_firmware/Makefile`
- Create: `examples/nucleo-u575zi/board_firmware/README.md`
- Create: `examples/nucleo-u575zi/board_firmware/stm32u5xx_hal_conf.h`

- [ ] **Step 1: Check out CubeU5 next to the repo (external, not committed)**

Run:
```bash
git clone --depth 1 https://github.com/STMicroelectronics/STM32CubeU5 /home/andrii/projects/STM32CubeU5
git -C /home/andrii/projects/STM32CubeU5 rev-parse HEAD
```
Record the revision in `board_firmware/README.md` and `VALIDATION.md`.

- [ ] **Step 2: Vendor a minimal `stm32u5xx_hal_conf.h`**

Copy `Projects/NUCLEO-U575ZI-Q/Templates/TrustZoneDisabled/Inc/stm32u5xx_hal_conf.h` into `board_firmware/`, then edit the module list to only what this app uses: `HAL_MODULE_ENABLED`, `HAL_RCC_MODULE_ENABLED`, `HAL_RCC_EX_MODULE_ENABLED`, `HAL_GPIO_MODULE_ENABLED`, `HAL_UART_MODULE_ENABLED`, `HAL_PWR_MODULE_ENABLED`, `HAL_PWR_EX_MODULE_ENABLED`, `HAL_CORTEX_MODULE_ENABLED`, `HAL_ICACHE_MODULE_ENABLED`, `HAL_FLASH_MODULE_ENABLED`, `HAL_FLASH_EX_MODULE_ENABLED`. Keep the ST copyright header. Define `HSE_VALUE 8000000U` and leave `USE_HAL_DRIVER` to the Makefile.

- [ ] **Step 3: Write `main.c`**

```c
/*
 * LabWired NUCLEO-U575ZI-Q Cube HAL smoke.
 * Register flow mirrors STM32CubeU5
 * Projects/NUCLEO-U575ZI-Q/Templates/TrustZoneDisabled (TrustZone factory-
 * disabled per UM2883), trimmed to HAL-core + GPIO + UART (no BSP).
 *
 * Console: USART1 PA9/PA10 @115200 8N1 (board VCP).
 * LED: PC7 (LD1 green, active high).
 * Output: "U575-HAL OK" once, then "BLINK n LD1=<0|1>" every 250 ms.
 */
#include "stm32u5xx_hal.h"

static UART_HandleTypeDef huart1;

static void SystemClock_Config(void);
static void CACHE_Enable(void);
static void GPIO_Init(void);
static void USART1_Init(void);
static void Error_Handler(void);
static void puts_uart(const char *s);

int main(void)
{
  char buf[32];
  uint32_t n = 0;

  HAL_Init();
  CACHE_Enable();
  SystemClock_Config();
  GPIO_Init();
  USART1_Init();

  HAL_UART_Transmit(&huart1, (uint8_t *)"U575-HAL OK\r\n", 13, 1000);

  while (1)
  {
    HAL_GPIO_TogglePin(GPIOC, GPIO_PIN_7);
    int level = (HAL_GPIO_ReadPin(GPIOC, GPIO_PIN_7) == GPIO_PIN_SET) ? 1 : 0;
    int len = 0;
    buf[len++] = 'B'; buf[len++] = 'L'; buf[len++] = 'I'; buf[len++] = 'N'; buf[len++] = 'K';
    buf[len++] = ' ';
    buf[len++] = (char)('0' + (n % 10));
    buf[len++] = ' ';
    buf[len++] = 'L'; buf[len++] = 'D'; buf[len++] = '1'; buf[len++] = '=';
    buf[len++] = (char)('0' + level);
    buf[len++] = '\r'; buf[len++] = '\n';
    HAL_UART_Transmit(&huart1, (uint8_t *)buf, (uint16_t)len, 1000);
    n++;
    HAL_Delay(250);
  }
}

static void SystemClock_Config(void)
{
  RCC_OscInitTypeDef RCC_OscInitStruct = {0};
  RCC_ClkInitTypeDef RCC_ClkInitStruct = {0};

  __HAL_RCC_PWR_CLK_ENABLE();
  HAL_PWREx_ControlVoltageScaling(PWR_REGULATOR_VOLTAGE_SCALE1);
  HAL_PWREx_ConfigSupply(PWR_SMPS_SUPPLY);
  __HAL_RCC_PWR_CLK_DISABLE();

  RCC_OscInitStruct.OscillatorType = RCC_OSCILLATORTYPE_MSI;
  RCC_OscInitStruct.MSIState = RCC_MSI_ON;
  RCC_OscInitStruct.MSIClockRange = RCC_MSIRANGE_4; /* 4 MHz */
  RCC_OscInitStruct.MSICalibrationValue = RCC_MSICALIBRATION_DEFAULT;
  RCC_OscInitStruct.PLL.PLLState = RCC_PLL_ON;
  RCC_OscInitStruct.PLL.PLLSource = RCC_PLLSOURCE_MSI;
  RCC_OscInitStruct.PLL.PLLMBOOST = RCC_PLLMBOOST_DIV1;
  RCC_OscInitStruct.PLL.PLLM = 1;
  RCC_OscInitStruct.PLL.PLLN = 80;
  RCC_OscInitStruct.PLL.PLLR = 2;
  RCC_OscInitStruct.PLL.PLLP = 2;
  RCC_OscInitStruct.PLL.PLLQ = 2;
  RCC_OscInitStruct.PLL.PLLFRACN = 0;
  RCC_OscInitStruct.PLL.PLLRGE = RCC_PLLVCIRANGE_0;
  if (HAL_RCC_OscConfig(&RCC_OscInitStruct) != HAL_OK)
  {
    Error_Handler();
  }

  RCC_ClkInitStruct.ClockType = (RCC_CLOCKTYPE_SYSCLK | RCC_CLOCKTYPE_HCLK |
                                 RCC_CLOCKTYPE_PCLK1 | RCC_CLOCKTYPE_PCLK2 |
                                 RCC_CLOCKTYPE_PCLK3);
  RCC_ClkInitStruct.SYSCLKSource = RCC_SYSCLKSOURCE_PLLCLK;
  RCC_ClkInitStruct.AHBCLKDivider = RCC_SYSCLK_DIV1;
  RCC_ClkInitStruct.APB1CLKDivider = RCC_HCLK_DIV1;
  RCC_ClkInitStruct.APB2CLKDivider = RCC_HCLK_DIV1;
  RCC_ClkInitStruct.APB3CLKDivider = RCC_HCLK_DIV1;
  if (HAL_RCC_ClockConfig(&RCC_ClkInitStruct, FLASH_LATENCY_4) != HAL_OK)
  {
    Error_Handler();
  }
}

static void CACHE_Enable(void)
{
  HAL_ICACHE_ConfigAssociativityMode(ICACHE_1WAY);
  HAL_ICACHE_Enable();
}

static void GPIO_Init(void)
{
  GPIO_InitTypeDef gpio = {0};
  __HAL_RCC_GPIOC_CLK_ENABLE();
  gpio.Pin = GPIO_PIN_7;
  gpio.Mode = GPIO_MODE_OUTPUT_PP;
  gpio.Pull = GPIO_NOPULL;
  gpio.Speed = GPIO_SPEED_FREQ_LOW;
  HAL_GPIO_Init(GPIOC, &gpio);
  HAL_GPIO_WritePin(GPIOC, GPIO_PIN_7, GPIO_PIN_RESET);
}

static void USART1_Init(void)
{
  GPIO_InitTypeDef gpio = {0};
  __HAL_RCC_GPIOA_CLK_ENABLE();
  __HAL_RCC_USART1_CLK_ENABLE();

  gpio.Pin = GPIO_PIN_9 | GPIO_PIN_10;
  gpio.Mode = GPIO_MODE_AF_PP;
  gpio.Pull = GPIO_PULLUP;
  gpio.Speed = GPIO_SPEED_FREQ_HIGH;
  gpio.Alternate = GPIO_AF7_USART1;
  HAL_GPIO_Init(GPIOA, &gpio);

  huart1.Instance = USART1;
  huart1.Init.BaudRate = 115200;
  huart1.Init.WordLength = UART_WORDLENGTH_8B;
  huart1.Init.StopBits = UART_STOPBITS_1;
  huart1.Init.Parity = UART_PARITY_NONE;
  huart1.Init.Mode = UART_MODE_TX_RX;
  huart1.Init.HwFlowCtl = UART_HWCONTROL_NONE;
  huart1.Init.OverSampling = UART_OVERSAMPLING_16;
  huart1.Init.OneBitSampling = UART_ONE_BIT_SAMPLE_DISABLE;
  huart1.Init.ClockPrescaler = UART_PRESCALER_DIV1;
  huart1.AdvancedInit.AdvFeatureInit = UART_ADVFEATURE_NO_INIT;
  if (HAL_UART_Init(&huart1) != HAL_OK)
  {
    Error_Handler();
  }
}

void HAL_UART_MspInit(UART_HandleTypeDef *huart)
{
  (void)huart;
}

static void Error_Handler(void)
{
  __disable_irq();
  while (1)
  {
  }
}

#ifdef USE_FULL_ASSERT
void assert_failed(uint8_t *file, uint32_t line)
{
  (void)file;
  (void)line;
  while (1)
  {
  }
}
#endif
```

- [ ] **Step 4: Makefile (H563 pattern, HAL sources added)**

```make
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.

TARGET := u575_hal_smoke
BUILD_DIR := build

# Same convention as examples/nucleo-h563zi/board_firmware (STM32CubeH5):
# checkout STM32CubeU5 as a sibling of the repo root.
STM32CUBE_U5_DIR ?= $(abspath ../../../../STM32CubeU5)

HAL_DIR := $(STM32CUBE_U5_DIR)/Drivers/STM32U5xx_HAL_Driver
CMSIS_DEVICE := $(STM32CUBE_U5_DIR)/Drivers/CMSIS/Device/ST/STM32U5xx
CMSIS_INCLUDE := $(STM32CUBE_U5_DIR)/Drivers/CMSIS/Include
STARTUP_SRC := $(CMSIS_DEVICE)/Source/Templates/gcc/startup_stm32u575xx.s
SYSTEM_SRC := $(CMSIS_DEVICE)/Source/Templates/system_stm32u5xx.c
LINKER_SCRIPT := $(CMSIS_DEVICE)/Source/Templates/gcc/linker/STM32U575xx_FLASH.ld

CC := arm-none-eabi-gcc
OBJCOPY := arm-none-eabi-objcopy
SIZE := arm-none-eabi-size

HAL_SRCS := \
	$(HAL_DIR)/Src/stm32u5xx_hal.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_cortex.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_rcc.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_rcc_ex.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_pwr.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_pwr_ex.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_gpio.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_uart.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_flash.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_flash_ex.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_icache.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_dma.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_dma_ex.c \
	$(HAL_DIR)/Src/stm32u5xx_hal_exti.c

CFLAGS := \
	-mcpu=cortex-m33 \
	-mthumb \
	-mfloat-abi=hard \
	-mfpu=fpv5-sp-d16 \
	-ffreestanding \
	-fno-builtin \
	-ffunction-sections \
	-fdata-sections \
	-fno-common \
	-Wall \
	-Wextra \
	-Os \
	-g3 \
	-DUSE_HAL_DRIVER \
	-DSTM32U575xx \
	-I. \
	-I$(CMSIS_DEVICE)/Include \
	-I$(CMSIS_INCLUDE) \
	-I$(HAL_DIR)/Inc

LDFLAGS := \
	-mcpu=cortex-m33 \
	-mthumb \
	-mfloat-abi=hard \
	-mfpu=fpv5-sp-d16 \
	-nostdlib \
	-Wl,--gc-sections \
	-Wl,-Map,$(BUILD_DIR)/$(TARGET).map \
	-T$(LINKER_SCRIPT)

MAIN_OBJ := $(BUILD_DIR)/main.o
SYSTEM_OBJ := $(BUILD_DIR)/system_stm32u5xx.o
STARTUP_OBJ := $(BUILD_DIR)/startup_stm32u575xx.o
HAL_OBJS := $(patsubst $(HAL_DIR)/Src/%.c,$(BUILD_DIR)/hal_%.o,$(HAL_SRCS))

ELF := $(BUILD_DIR)/$(TARGET).elf

.PHONY: all clean

all: $(ELF)
	$(SIZE) $(ELF)

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(MAIN_OBJ): main.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(SYSTEM_OBJ): $(SYSTEM_SRC) | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(STARTUP_OBJ): $(STARTUP_SRC) | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD_DIR)/hal_%.o: $(HAL_DIR)/Src/%.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(ELF): $(MAIN_OBJ) $(SYSTEM_OBJ) $(STARTUP_OBJ) $(HAL_OBJS)
	$(CC) $(LDFLAGS) $^ -o $@

clean:
	rm -rf $(BUILD_DIR)
```

- [ ] **Step 5: Build and run the HAL firmware**

Run:
```bash
make -C examples/nucleo-u575zi/board_firmware
cargo run -q -p labwired-cli -- \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000
```
Expected: `U575-HAL OK` then repeating `BLINK n LD1=<0|1>`. Fix model gaps via TDD in Task 4 style (each fix gets a unit test). Determinism: run twice, diff UART bytes (capture with `--uart-log out/hal-1.txt` if supported, else `--json`/stdout redirect) — expects byte-identical output.

- [ ] **Step 6: Checkpoint** — staged set: board_firmware (main.c, Makefile, README.md, stm32u5xx_hal_conf.h).

---

### Task 6: Repo registry wiring

**Files:**
- Modify: `crates/core/tests/chip_conformance.rs` (add `ChipConf`)
- Modify: `crates/core/tests/svd_conformance.rs` (add SVD entry)
- Modify: `crates/core/tests/register_coverage.rs` (add `ChipEntry`)
- Modify: `crates/core/tests/board_coverage_ratchet.rs` (not-shipped list)
- (already regenerated) `crates/core/src/bus/embedded_descriptors.rs`

- [ ] **Step 1: chip_conformance**

Add after the `stm32h563` entry:
```rust
    ChipConf {
        name: "stm32u575",
        yaml: "configs/chips/stm32u575.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
```
(`behavior_gate: None` is honest until a committed ELF fixture + survival case exists; stm32h735 sets the same precedent.)

- [ ] **Step 2: svd_conformance**

Add to the table:
```rust
    ("stm32u575", "tests/fixtures/real_world/stm32u575.svd"),
```

- [ ] **Step 3: register_coverage**

Add (the real type is the tuple alias `(&str, &str, Option<&str>)`, not a struct):
```rust
    (
        "stm32u575",
        "configs/chips/stm32u575.yaml",
        Some("tests/fixtures/real_world/stm32u575.svd"),
    ),
```

- [ ] **Step 4: board_coverage_ratchet**

Add to the not-shipped list with a reason:
```rust
    "stm32u575", // First U5 part; sim-validated, no bench silicon capture yet
```

- [ ] **Step 5: Run the registries**

Run:
```bash
cargo test -p labwired-core --test chip_conformance stm32u575 -- --nocapture
cargo test -p labwired-core --test svd_conformance stm32u575 -- --nocapture
cargo test -p labwired-core --test register_coverage -- --nocapture 2>&1 | tail -30
cargo test -p labwired-core --test board_coverage_ratchet -- --nocapture 2>&1 | tail -20
```
Expected: all pass; register_coverage reports the U575 coverage gap honestly (existing gates allow holes as long as the entry is declared).

- [ ] **Step 6: strict onboarding (io-smoke gate)**

Run:
```bash
cargo test -p labwired-core --test strict_onboarding -- --nocapture 2>&1 | tail -30
```
Expected: `stm32u575 ... is strictly onboarded.` The test builds `firmware-stm32u575-demo` itself via the io-smoke path.

- [ ] **Step 7: Fleet-derived gate sweep** (added 2026-09-18 after review)

The four registries above are not the only gates that enumerate the fleet from
`configs/chips/`, and the per-chip name filter in Step 5 matches no test
function — these all go red the moment the descriptor lands. Run each:

```bash
cargo test -p labwired-core --test bus_visibility -- --nocapture
pytest -q scripts/perf/test_board_perf.py
python3 scripts/ci/chip_coverage.py --report
python3 scripts/generate_validation_status.py --check --drift
bash scripts/ci/docs-runnable-chips.sh        # or scripts/ci/test-docs-runnable-chips.sh
cargo test -p labwired-core --test nvic_masking_config_path -- --nocapture
```

- `bus_visibility`: U575 publishes no SPI pad_map, so SPI1-3 cannot narrate
  edges — add a `("stm32u575", BusKind::Spi, …)` EXCLUSIONS row and regenerate
  `docs/coverage/bus-visibility.{json,md}` with
  `UPDATE_BUS_VISIBILITY_BASELINE=1`. I2C and UART do produce edges.
- `board_perf`: U575 matches the `stm32` fixture (arm/0x08000000/0x20000000);
  record real baselines with
  `cargo build --release -p labwired-cli --features event-scheduler` then
  `python3 scripts/perf/board_perf.py --boards stm32u575 --update`.
- `chip_coverage --report`: U575 needs the workflow matrix entry (Task 8 Step 3
  can land early) before it is declared "covered".
- `generate_validation_status --check --drift`: the drift rows are from the
  Task 4 RCC/WBA model edits, not U575; regenerate the doc only if the diff is
  purely generated, and leave `--drift` itself for the maintainer re-ack.
- `docs-runnable-chips.json`: add U575 as `needs-build` (the io-smoke ELF is
  built from source, not a firmware-demos asset).
- `nvic_masking_config_path.rs`: add `"stm32u575"` to `CHIPS_DECLARING_NVIC`.

- [ ] **Step 8: Checkpoint** — staged set: four test files + fleet sweep.

---

### Task 7: Arduino matrix (fidelity engine)

**Files:**
- Create: `validation/arduino-matrix/systems/stm32u575.yaml`
- Modify: `validation/arduino-matrix/boards.yaml`

- [ ] **Step 1: Matrix system manifest**

`validation/arduino-matrix/systems/stm32u575.yaml`:
```yaml
# Arduino matrix system — INA219 on Wire for L3; L0-L2 ignore it.
name: "arduino-matrix-stm32u575"
chip: "../../../configs/chips/stm32u575.yaml"
external_devices:
  # Matrix L3_i2c_sensor: INA219 at 0x40 on default Arduino Wire bus (i2c1).
  - id: "ina219"
    type: "ina219"
    connection: "i2c1"
    config:
      i2c_address: 0x40
  # Matrix L4_spi_sensor: MAX31855 on default Arduino SPI (spi1).
  - id: "max31855"
    type: "max31855"
    connection: "spi1"
    config:
      cs_pin: "PA4"
board_io: []
```

- [ ] **Step 2: boards.yaml entry**

Append after the `stm32wba52` block:
```yaml
  - id: stm32u575
    chip: stm32u575
    system: systems/stm32u575.yaml
    pio: { platform: ststm32, board: nucleo_u575zi_q, framework: arduino }
    max_steps: 10000000
    budget_reason: "Measured first run: L0~0.17M, L2~0.49M, L3 Wire.begin i2c_computeTiming 7.1M, L6 analogWrite 5.0M — 10M covers L0-L4/L6/L7 with ~40% headroom (was 20M H563/L476 sibling budget)"
    led_watch: "gpioc:7"
    led_min_edges: 2
    family: stm32
    sketches_skip: [L5_adc, L8_can]
    sketches_skip_reason:
      L5_adc: "ADC1 descriptor uses the closest profile (stm32h7); the U5 RES[3:2] delta is documented but analogRead is unproven on U5 yet"
      L8_can: "FDCAN not declared in the U5 chip yaml (first pass)"
```

- [ ] **Step 3: Build the CLI and run the matrix**

Run:
```bash
cargo build -p labwired-cli --release
python3 validation/arduino-matrix/run_matrix.py --boards stm32u575
```
Expected first run: a mix of pass/fail; `out/stm32u575/<sketch>/build.log`, `uart.log`, `result.json` per sketch. For each failure: read `uart.log`, classify (`boot_fail`/`oracle_fail`/`unmodeled`), fix the model gap with a unit test (Task 4 style), re-run. Sketches that fail for *Arduino-core* reasons unrelated to models get a `sketches_skip` entry with the reason — never a silent failure.

- [ ] **Step 4: Re-run until the scoreboard is stable**

Run:
```bash
python3 validation/arduino-matrix/run_matrix.py --boards stm32u575
sed -n '1,40p' docs/coverage/arduino-scoreboard.md
```
Expected: L0-L4, L6, L7 pass (L5/L8 skipped with reasons) — or every deviation documented in `boards.yaml`.

`docs/coverage/arduino-scoreboard.md` stays at the last full 18-board run:
the runner only publishes there on a complete matrix, and a full regen is
deferred to Task 8 / the scheduled CI lane (the per-board
`out/stm32u575/scoreboard.md` is gitignored and carries the same rows).

- [ ] **Step 5: Wire the board into the Arduino CI workflow matrix** (added 2026-09-18 review)

Add `- stm32u575` to the `board:` matrix in
`.github/workflows/core-arduino-matrix-smoke.yml` (alphabetically after
`stm32l476`). `scripts/ci/chip_coverage.py --report` is a disk-derived gate
that cross-checks every board declared in `validation/arduino-matrix/boards.yaml`
against the workflow matrices, and it errors until this entry lands:

```bash
python3 scripts/ci/chip_coverage.py --report   # must exit 0
```

- [ ] **Step 6: Checkpoint** — staged set: matrix system + boards.yaml + workflow matrix entry (+ model fixes from Step 3).

---

### Task 7b: Zephyr matrix (fidelity engine, added 2026-09-18 per user direction)

Same bar as the Arduino matrix, Zephyr side. The workspace at `~/zephyrproject` is Zephyr 3.7.2 and already has `boards/st/nucleo_u575zi_q`; `west` lives in `~/zephyrproject/.venv/bin` (add to PATH or activate). Toolchain is system `arm-none-eabi-gcc` with `ZEPHYR_TOOLCHAIN_VARIANT=gnuarmemb` (no Zephyr SDK needed per `validation/zephyr-matrix/README.md`).

**Files:**
- Create: `validation/zephyr-matrix/systems/stm32u575.yaml`
- Modify: `validation/zephyr-matrix/boards.yaml`

- [ ] **Step 1: Matrix system manifest**

`validation/zephyr-matrix/systems/stm32u575.yaml` (mirror `systems/stm32h563.yaml`):
```yaml
# Zephyr matrix system — INA219 for L3; L0–L2 ignore it.
name: "zephyr-matrix-stm32u575"
chip: "../../../configs/chips/stm32u575.yaml"
external_devices:
  - id: "ina219"
    type: "ina219"
    connection: "i2c1"
    config:
      i2c_address: 0x40
board_io: []
```

- [ ] **Step 2: boards.yaml entry** (keep list order consistent with siblings)

```yaml
  - id: stm32u575
    chip: stm32u575
    system: systems/stm32u575.yaml
    zephyr_board: nucleo_u575zi_q
    family: stm32
```

- [ ] **Step 3: Build the CLI and run the matrix**

```bash
cargo build -p labwired-cli --release
PATH="$HOME/zephyrproject/.venv/bin:$PATH" \
  python3 validation/zephyr-matrix/run_matrix.py --boards stm32u575
```
Expected first run: mixed L0–L3; outputs under `out/stm32u575/<level>/{build.log,zephyr.elf,uart.log,result.json}`. Classify each failure exactly as Task 7 Step 3 (boot_fail / oracle_fail / unmodeled) and fix model gaps with unit tests (Task 4 style). Levels L0–L2 are the required bar; L3 needs the INA219 i2c path (same engine as Arduino L3).

- [ ] **Step 4: Scoreboard + docs**

```bash
python3 validation/zephyr-matrix/run_matrix.py --boards stm32u575 --no-build
sed -n '1,40p' docs/coverage/zephyr-scoreboard.md
```
Expected: L0–L2 green and L3 green or documented in `validation/zephyr-matrix/PROBLEMS.md`/skips. Update `validation/FRAMEWORK_FLEET.md` chip-coverage table with the stm32u575 row (Arduino + Zephyr + L3 kit columns, honestly).

- [ ] **Step 5: Checkpoint** — staged set: zephyr-matrix system + boards.yaml (+ model fixes), FRAMEWORK_FLEET row.

---

### Task 8: Manifest, board docs, workflow, scoreboards

**Files:**
- Modify: `validation/manifest.yaml` (new entry)
- Create: `docs/boards/stm32u575.md`
- Modify: `.github/workflows/core-onboarding-smoke.yml` (matrix + target list)
- Create: `examples/nucleo-u575zi/{README.md,VALIDATION.md,REQUIRED_DOCS.md,EXTERNAL_COMPONENTS.md}`

- [ ] **Step 1: Manifest entry**

Add next to `stm32h735` in `validation/manifest.yaml`, with `models:` covering every path the chip YAML wires:
```yaml
  - id: stm32u575
    doc: docs/boards/stm32u575.md
    chip: configs/chips/stm32u575.yaml
    tier: sim-validated
    note: "STM32U575ZI (NUCLEO-U575ZI-Q), first U5 part. Cortex-M33, 2 MiB flash, 768 KiB SRAM + 16 KiB SRAM4. Reuses the shared V2 RCC/GPIO/UART models plus the U5 PLL1 register block (PLL1CFGR 0x28 / DIVR 0x34 / FRACR 0x38). Validated by: real STM32CubeU5 HAL firmware (160 MHz PLL1 bring-up, USART1 VCP banner + LED loop), Rust io-smoke, Arduino matrix L0-L4/L6/L7, unsupported-instruction audit. NO bench part: every value is SVD/RM0456-derived; Renode has no STM32U5 platform, so no Renode differential is claimed. TrustZone/GTZC, OCTOSPI, USB, FDCAN, ADC4 and flash program/erase are not modeled."
    offline_tests:
      - "strict_onboarding (io-smoke builds + runs firmware-stm32u575-demo)"
      - "chip_conformance (estate OK)"
      - "svd_conformance / register_coverage (SVD pinning)"
      - "arduino matrix L0-L4/L6/L7 (validation/arduino-matrix)"
    models:
      - crates/core/src/peripherals/rcc.rs
      - crates/core/src/peripherals/pwr.rs
      - crates/core/src/peripherals/flash.rs
      - crates/core/src/peripherals/uart.rs
      - crates/core/src/peripherals/gpio.rs
      - crates/core/src/peripherals/i2c.rs
      - crates/core/src/peripherals/spi.rs
      - crates/core/src/peripherals/timer.rs
      - crates/core/src/peripherals/adc.rs
      - crates/core/src/peripherals/rtc_v3.rs
      - crates/core/src/peripherals/iwdg.rs
      - crates/core/src/peripherals/gpdma.rs
      - crates/core/src/peripherals/crc.rs
      - crates/core/src/peripherals/rng.rs
      - crates/core/src/peripherals/dbgmcu.rs
      - configs/chips/stm32u575.yaml
      - configs/peripherals/stm32u575
```
Then run `python3 scripts/generate_validation_status.py --check` and fix any drift/digest complaint by following its message (likely needs `--write` once).

- [ ] **Step 2: Board doc**

Create `docs/boards/stm32u575.md` from `docs/boards/_TEMPLATE.md` (read the template first), stating: sim-validated tier, no silicon, U5 PLL1 addition, VCP USART1, LED PC7, Arduino Serial = USART1, known gaps (TrustZone, OCTOSPI, USB, FDCAN, ADC4, flash programming).

- [ ] **Step 3: Onboarding-smoke workflow**

Edit `.github/workflows/core-onboarding-smoke.yml`:
- toolchain targets line 62: `thumbv6m-none-eabi,thumbv7m-none-eabi,thumbv7em-none-eabi,thumbv8m.main-none-eabi`
- matrix include:
```yaml
          - id: stm32u575-nucleo
            crate: firmware-stm32u575-demo
            target: thumbv8m.main-none-eabi
            script: examples/nucleo-u575zi/io-smoke.yaml
            system: configs/systems/nucleo-u575zi.yaml
```

Fleet-sweep awareness: the *Arduino* workflow matrix
(`core-arduino-matrix-smoke.yml`) is a disk-derived gate too — Task 7 Step 5
adds `stm32u575` there, and `scripts/ci/chip_coverage.py --report` fails if a
board is declared in `validation/arduino-matrix/boards.yaml` and no workflow
matrix names it. When this step touches workflows, re-run `--report` and keep
both matrices in sync.

- [ ] **Step 4: Example docs**

- `REQUIRED_DOCS.md` — SVD sha256 + URL, DS13736, RM0456, UM2861, `stm32u5xx-nucleo-bsp`, Zephyr board doc, and the Renode finding: "Renode master has no STM32U5 platform (checked 2026-09-17, `platforms/cpus/` listing); closest references `stm32wba52.repl`, `stm32l552.repl`".
- `EXTERNAL_COMPONENTS.md` — none (bare board); CubeU5 checkout instructions.
- `README.md` — quick start: build rust smoke, run io-smoke, build HAL firmware, run.
- `VALIDATION.md` — exact commands + captured evidence from Tasks 3-5, 7, 7b, including the CubeU5 git revision and the two-run determinism diff.

- [ ] **Step 5 (added 2026-09-18): PR-gate survival fixtures + bus-proof row**

The `bus_proof_matrix_is_complete_and_not_fabricated` gate derives the chip set from `configs/chips/*.yaml` and currently FAILS because `stm32u575` has no row. Fix it with real evidence, not type-checking:

1. Build the **stock** Zephyr hello_world for the fixture (do NOT copy the matrix L0 ELF — that is the in-tree `LW_Z0_OK` sample, not stock firmware, and every existing `*-zephyr-hello.elf` is stock):
   ```bash
   PATH="$HOME/zephyrproject/.venv/bin:$PATH" ZEPHYR_TOOLCHAIN_VARIANT=gnuarmemb GNUARMEMB_TOOLCHAIN_PATH=/usr \
     west -p auto -d /tmp/zephyr-u575-hello build -b nucleo_u575zi_q "$HOME/zephyrproject/zephyr/samples/hello_world"
   cp /tmp/zephyr-u575-hello/build/zephyr/zephyr.elf tests/fixtures/stm32u575-zephyr-hello.elf
   ```
   Record the Zephyr revision (3.7.2 @ `c66235fb7346`) in the case comment; expected console string is `Hello World! nucleo_u575zi_q`.
2. In `crates/core/tests/firmware_survival.rs`, find the surviving cases table (`case_by_name`) and add a `stm32u575_zephyr` case mirroring `stm32h563_zephyr` but with the U575 ELF path and the stock console string `Hello World! nucleo_u575zi_q`. Add the `#[test] fn test_stm32u575_zephyr_survival()` next to the H563/WBA ones. Test must be a hard `assert!` on the ELF's presence (no self-skip) — the bus-proof gate rejects vacuous evidence.
3. Best-effort Arduino serial survival fixture: if the Task 7 matrix build leaves a usable L0 ELF (`validation/arduino-matrix/out/stm32u575/L0_serial_boot/*.elf` or PlatformIO `.pio/build/*/firmware.elf`), commit it as `tests/fixtures/stm32u575-arduino-serial.elf` and add `test_stm32u575_arduino_serial_survival` mirroring `test_stm32wba52_arduino_serial_survival` (marker string from the sketch). If the artifact is not stable/committable, skip and say so — do not commit a fabricated binary.
4. Add the `stm32u575` row to `validation/bus_proof_matrix.json` (`chips` map, keys `in_boards`, `uart`, `spi`, `i2c`; mirror the H563 entry shape):
   - `uart`: `proven`, citing `crates/core/tests/firmware_survival.rs` / `test_stm32u575_zephyr_survival`, with the real signal description and the lane that runs it (`core-ci` firmware_survival).
   - `i2c`: `proven` citing `validation/arduino-matrix/sketches/L3_i2c_sensor/src/main.ino` / `LW_L3_OK` if Task 7 L3 passes; otherwise `none`/`shallow` with the honest gap (the gate checks the cited test name literally appears in the cited file).
   - `spi`: `proven` citing `.../L4_spi_sensor/src/main.ino` / `LW_L4_OK` only if Task 7 L4 passes; otherwise `shallow`/`none` with a real gap statement. Never overclaim.
   - Update the `surveyed` metadata if the schema expects it (read the `$schema_note` and sibling rows first).
5. Run `cargo test -p labwired-core --lib bus_proof_matrix -- --nocapture` and `cargo test -p labwired-core --test firmware_survival stm32u575 -- --nocapture` — both green.
6. Update `validation/FRAMEWORK_FLEET.md` coverage table with the `stm32u575` row (Arduino matrix ✅/levels, Zephyr matrix ✅/levels, L3 kit column, notes with skips), matching what Tasks 7/7b actually produced.
7. Scoreboards: `docs/coverage/arduino-scoreboard.md` and `docs/coverage/zephyr-scoreboard.md` are published only by FULL matrix runs. Do not publish a single-board run (it would clobber the fleet table). Either run the full matrices and publish, or append an explicit "U575 verified locally 2026-09-18; fleet scoreboard refresh pending next full run" note to `examples/nucleo-u575zi/VALIDATION.md` and leave the scoreboards to CI. Never overwrite a full-run scoreboard with a partial one.
8. Store the L3 negative-control evidence (no `external_devices` → `LW_Z3_FAIL err=-5` vs with INA219 → `LW_Z3_OK`) in `VALIDATION.md` with the exact command used.

- [ ] **Step 6: Checkpoint** — staged set: manifest, board doc, workflow, example docs, survival fixtures + test, bus_proof_matrix.json, FRAMEWORK_FLEET.md.

---

### Task 9: Final validation sweep

- [ ] **Step 1: Targeted + regression tests**

```bash
cargo test -p labwired-core --lib peripherals:: -- --nocapture 2>&1 | tail -20
cargo test -p labwired-core --test chip_conformance --test svd_conformance --test strict_onboarding -- --nocapture 2>&1 | tail -30
cargo test -p labwired-core --test firmware_survival stm32u575 -- --nocapture 2>&1 | tail -10
cargo test -p labwired-core --lib bus_proof_matrix -- --nocapture 2>&1 | tail -10
cargo test -p labwired-core h563 -- --nocapture 2>&1 | tail -10
cargo test -p labwired-core wba52 -- --nocapture 2>&1 | tail -10
```
Expected: all green; no H563/WBA52/L476 regressions.

- [ ] **Step 2: Unsupported-instruction audit**

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system configs/systems/nucleo-u575zi.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo-u575zi
```
Expected: report written; no unsupported instructions on the boot path (or each one documented).

- [ ] **Step 3: Determinism**

Run the HAL firmware twice into files and diff:
```bash
cargo run -q -p labwired-cli -- --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000 > out/u575-run-1.txt
cargo run -q -p labwired-cli -- --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000 > out/u575-run-2.txt
diff out/u575-run-1.txt out/u575-run-2.txt && echo DETERMINISTIC
```
Expected: `DETERMINISTIC`.

- [ ] **Step 4: Docs/gates freshness**

```bash
python3 scripts/generate_validation_status.py --check
python3 validation/arduino-matrix/run_matrix.py --boards stm32u575 --no-build
PATH="$HOME/zephyrproject/.venv/bin:$PATH" python3 validation/zephyr-matrix/run_matrix.py --boards stm32u575 --no-build
cargo fmt --all -- --check
cargo clippy -p labwired-core --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: clean (fix fmt/clippy before claiming done).

- [ ] **Step 5: Write the evidence summary**

Append the exact command outputs to `examples/nucleo-u575zi/VALIDATION.md` (commands + observed result, not summaries), then report to the user: files changed, commands run, evidence, known gaps.

---

## Self-review notes

- Spec coverage: sources (Tasks 1, 8.4), configs (2, 3), HAL (5), fidelity engine + Arduino (7), registries (6), manifest/docs/scoreboards (8), validation/audit/determinism (9). Renode limitation documented (8.4).
- The Cube HAL run may surface the same class of shared-model gaps the spec lists; Task 4's TDD loop is the catch-all, with named candidate tests for RCC/PWR/FLASH/UART.
- WBA52/H563 regression commands are explicit in Tasks 2.2 and 9.1.
