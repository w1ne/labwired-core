# Plan 2 — Boot Path + USB_SERIAL_JTAG + SYSTIMER (esp-hal hello-world end-to-end)

**Date:** 2026-04-26
**Branch target:** `feature/esp32s3-plan2-boot-uart` (off `feature/esp32s3-plan1-foundation`)
**Predecessor:** Plan 1 (`docs/case_study_esp32s3_plan1.md`)
**Successor:** Plan 3 — GPIO + IO_MUX + Interrupt Matrix + SYSTIMER alarms (M4 demo: blinky)

---

## 1. Goal

Run a real `esp-hal` Rust firmware (`xtensa-esp32s3-none-elf` target) end-to-end in the LabWired simulator. The firmware is the canonical hello-world: print `"Hello world!"` to stdout once per second, indefinitely.

The simulator must:
- Load the ELF (multi-segment: IRAM text, DRAM data, flash-XIP rodata).
- Boot it directly to `main` without a 2nd-stage bootloader, by synthesising the post-bootloader CPU state and pre-populating the EXTMEM cache MMU table.
- Service the handful of ROM functions the firmware calls during init.
- Model `USB_SERIAL_JTAG` well enough that `esp_println::println!` produces visible output on the host's stdout.
- Model `SYSTIMER` well enough that `esp_hal::delay::Delay::delay_millis(1000)` blocks for approximately one second of simulated time.

Optional stretch: a `--diff` CLI flag that runs the same firmware on the connected ESP32-S3-Zero (via Plan 1's OpenOCD harness) and compares the captured `/dev/ttyACM0` byte stream.

This is the M3 milestone from `docs/design/2026-04-24-esp32s3-zero-digital-twin-design.md`.

## 2. Non-goals

- **GPIO, IO_MUX, Interrupt Matrix.** All three land in Plan 3 alongside the blinky demo.
- **2nd-stage bootloader emulation.** Plan 2 uses fast-boot (`--boot=fast` from the design doc); BROM loading + bootloader stage execution (`--boot=full`) is deferred indefinitely.
- **SMP / cpu1.** The simulator runs cpu0 only. esp-hal hello-world does not use the second core.
- **SYSTIMER alarms / IRQs.** `Delay::delay_millis` is polling-based — it busy-reads the counter. Alarms land in Plan 3 alongside the interrupt matrix.
- **Full USB stack.** We model the `USB_SERIAL_JTAG` peripheral's MMIO interface (FIFO + status + control regs) — not USB endpoint emulation. The host sees the bytes the firmware writes; the firmware doesn't see actual USB packets.
- **Dynamic flash-XIP MMU updates.** The page table is populated once at boot from the ELF segment layout and never modified. Real firmware does call `Cache_*` ROM functions to remap pages at runtime; for hello-world the static table suffices.
- **HW oracle test correctness for the 28 leftover Plan 1 failures.** The `--diff` stretch goal compares observable USB-CDC output, not per-instruction state. The Plan 1 oracle bank stays as it is (Plan 1.5 spec exists separately if we want to come back to it).
- **Any ESP32 variant other than ESP32-S3-Zero (FH4R2 variant).** Single chip target.
- **Custom linker scripts or build-system shims for the example firmware.** We use the standard `esp-hal` template unmodified.

## 3. Context

Plan 1 closed with a complete Xtensa LX7 simulator backed by hardware-oracle-validated semantics for every implemented instruction. The HW-oracle harness, OpenOCD bridge, and ELF-loading infrastructure are all in place and reusable.

What's missing for "run a real binary" is the **system layer**: a way to compose the CPU + bus + a realistic peripheral set into a `Machine` that boots an ELF the way the real chip would. The design doc §6 describes this layer at a high level; Plan 2 builds the concrete implementation for the smallest demo that proves it works.

The choice of esp-hal hello-world over esp-idf hello_world is deliberate. esp-hal is a pure Rust no_std stack with no separate 2nd-stage bootloader (the linker produces a single image that boots directly from ROM-second-stage), it links against fewer ROM functions, and it matches the project's existing Rust workflow (`firmware-ci-fixture`, `riscv-ci-fixture` etc.). The cost is "less iconic than esp-idf"; the benefit is "drastically simpler boot path and clean MIT/Apache-2.0 vendoring story".

The choice of USB_SERIAL_JTAG over UART0 is similarly deliberate. The S3-Zero exposes USB_SERIAL_JTAG over the same USB cable that powers the OpenOCD JTAG endpoint. No external USB-UART adapter required, no external wiring. `esp_println` with `feature = "jtag-serial"` outputs through this path.

The choice to include SYSTIMER (vs. deferring it to Plan 3) is pragmatic. The smallest convincing hello-world prints in a loop with a delay between iterations. Stripping the delay would produce "smallest possible scope" but a less recognisable demo. SYSTIMER is ~300 LoC of register modelling; pulling it forward into Plan 2 front-loads the timebase work and lets Plan 3 focus on GPIO + interrupts.

## 4. Architecture

### 4.1 Three-layer split (extends Plan 1's split)

- **Simulator core** (`crates/core/`) — unchanged Cpu / Bus / Peripheral traits; new `boot/` module; new `peripherals/esp32s3/` module group; new `system/xtensa.rs` glue.
- **Firmware fixtures** (`examples/esp32s3-hello-world/`) — a real `esp-hal` Cargo crate, target `xtensa-esp32s3-none-elf`, builds via `cargo +esp build --release`.
- **CLI** (`crates/cli/`) — extended with a `run` subcommand that loads a chip YAML + an ELF and runs the simulation indefinitely.

### 4.2 Boot path: fast-boot

The fast-boot strategy (per design doc §6.4) is:

1. Parse the ELF with `goblin`.
2. Place each `PT_LOAD` segment at its virtual address by writing through the bus. The bus's address-resolution dispatches each write to the correct peripheral (IRAM, DRAM, or `FlashXipPeripheral` depending on `p_vaddr`).
3. For segments whose `p_vaddr` lies in the flash-XIP windows (`0x42000000+` for I-cache, `0x3C000000+` for D-cache), pre-populate the EXTMEM cache MMU table so `FlashXipPeripheral` reads return the correct backing-store bytes.
4. Synthesise post-bootloader CPU state:
   - `PC = elf.entry`
   - `SP = ELF symbol _stack_start_cpu0` (or chip YAML `stack_top` fallback)
   - `PS = 0x0000_001F` (post-reset value, confirmed in Plan 1 against real silicon)
   - `WindowBase = 0`, `WindowStart = 0x0001`
   - `VECBASE = 0x40000000` (ROM vector default — firmware relocates this itself if it cares)
5. Hand control to the simulation step loop.

No BROM execution. No 2nd-stage bootloader. The state we synthesise is the state the chip would be in *after* both have completed.

### 4.3 ROM-thunk dispatch

Real firmware calls a small set of ROM functions for things the chip's ROM is the canonical implementation of (`ets_printf`, cache maintenance, flash access). The simulator services these by:

1. A `RomThunkBank` peripheral mapped at `0x40000000` (size 0x60000, covering BROM extent) holds a `HashMap<u32, RomThunkFn>` of registered thunks at known PC addresses. The set of registered addresses is determined empirically by disassembling the firmware and listing every `BL` / `CALL` whose target lies in the BROM range.
2. The bank pre-populates its memory at each registered thunk address with the 3-byte sequence `BREAK 1, 14` (encoded `0xF0, 0x42, 0x00`). When the CPU fetches from a thunk address, it gets a BREAK instruction back through the normal fetch path.
3. The `Break` exec arm in `crates/core/src/cpu/xtensa_lx7.rs` adds a check for `(level=1, imm=14)` (a unique level/imm pair distinct from oracle-harness BREAK 1,15): if the current PC is registered in the thunk bank, call the registered Rust function and return `Ok(())` (the function is responsible for setting `PC = a0` to return to the caller per Xtensa CALL convention). If the PC is in the BROM range but not registered, raise `SimulationError::NotImplemented(format!("ROM thunk at 0x{pc:08x}"))`. If the PC is anywhere else, fall through to the existing `BreakpointHit` raise.

This avoids:
- Decoder-level changes (BREAK is already decoded and BREAK 1,15 is reserved for the oracle harness).
- Coupling between exec and peripheral state (the BREAK arm gets a `bus.get_rom_thunk(pc)` call; everything else stays in the bank).
- Silent fall-through on missing ROM functions (the explicit `NotImplemented` makes the dev's next step obvious: add the missing thunk).

### 4.4 Peripherals

Six new peripherals under `crates/core/src/peripherals/esp32s3/`:

- `RomThunkBank` — see §4.3.
- `UsbSerialJtag` — MMIO at `0x60038000`. Writes to the FIFO data register append to a sink (a `Vec<u8>` for tests, plus host stdout for live runs). Status register reads always return "ready + space available". Interrupt registers are stubbed.
- `Systimer` — MMIO at `0x60023000`. Two 64-bit counters with a load/update handshake. Counters tick on every `tick()` call by `(cycles_elapsed * 16_000_000 / cpu_clock_hz)` to match the real chip's 16 MHz SYSTIMER clock. No alarms.
- `SystemStub` — MMIO at `0x600C0000`. Read-as-zero, write-accept, except for `SYSCLK_CONF` which reflects whatever the firmware writes (so esp-hal's clock-config code reads back what it set).
- `RtcCntlStub` — MMIO at `0x60008000`. Fully cosmetic for hello-world: read-as-zero, write-accept.
- `EfuseStub` — MMIO at `0x60007000`. Returns canned values for the few fields esp-hal reads at boot: MAC = `02:00:00:00:00:01`, chip-rev = 0.

Plus one shared backing store:

- `FlashXipPeripheral` — backing memory for the flash-XIP windows. Mapped twice on the bus (`0x42000000` for I-cache, `0x3C000000` for D-cache) but shares a single `Vec<u8>` of flash bytes. Reads consult the MMU page table (populated at boot) to translate virtual page → physical page in the backing store.

### 4.5 System glue

`crates/core/src/system/xtensa.rs` (new, the missing module from Plan 1) provides:

```rust
pub struct Esp32s3Opts {
    pub iram_size: usize,         // 512 KiB default
    pub flash_size: usize,        // 4 MiB default for the FH4R2 variant
    pub cpu_clock_hz: u32,        // 80_000_000 default
    pub stack_top: u32,           // fallback if _stack_start_cpu0 missing
}

pub fn configure_xtensa_esp32s3(
    bus: &mut SystemBus,
    opts: &Esp32s3Opts,
) -> XtensaLx7;
```

Registers all peripherals at their canonical addresses, returns a fresh `XtensaLx7` with default reset state.

### 4.6 Chip YAML

`configs/chips/esp32s3-zero.yaml` follows the same schema as the existing `configs/chips/stm32f103.yaml`. The CLI's existing system-loading path can use it; the YAML is also documentation of what's mapped where.

## 5. Components

### 5.1 `crates/core/src/boot/mod.rs` + `boot/esp32s3.rs`

```rust
pub fn fast_boot(
    elf_bytes: &[u8],
    bus: &mut SystemBus,
    cpu: &mut XtensaLx7,
    opts: &BootOpts,
) -> SimResult<BootResult>;

pub struct BootOpts {
    pub stack_top_fallback: u32,
}

pub struct BootResult {
    pub entry: u32,
    pub stack: u32,
    pub xip_pages_loaded: usize,
}
```

Roughly 250 LoC including unit tests.

### 5.2 `crates/core/src/peripherals/esp32s3/mod.rs`

Module root — re-exports the six new submodules. ~10 LoC.

### 5.3 `crates/core/src/peripherals/esp32s3/rom_thunks.rs`

```rust
pub type RomThunkFn = fn(&mut XtensaLx7, &mut dyn Bus) -> SimResult<()>;

pub struct RomThunkBank {
    thunks: HashMap<u32, RomThunkFn>,
    backing: Vec<u8>,        // pre-filled with BREAK 1,14 at each thunk addr
    base: u32,
    size: u32,
}

impl RomThunkBank {
    pub fn new(base: u32, size: u32) -> Self;
    pub fn register(&mut self, pc: u32, thunk: RomThunkFn);
    pub fn get(&self, pc: u32) -> Option<RomThunkFn>;
}

// Initial registered set (the minimum esp-hal hello-world is *expected* to
// call; the actual address-to-symbol map comes from disassembling the built
// firmware via `xtensa-esp32s3-elf-objdump` and reading ESP-IDF's
// `rom/esp32s3.ld` for ROM symbol addresses):
//   ets_printf                          — read fmt + args, write to host stdout
//   Cache_Suspend_DCache                — NOP, return 0
//   Cache_Resume_DCache                 — NOP, return 0
//   esp_rom_spiflash_unlock             — NOP, return 0
//   rom_config_instruction_cache_mode   — NOP
//   ets_set_appcpu_boot_addr            — NOP (cpu1 not modelled)
//
// The actual addresses (e.g. `ets_printf` is at 0x40000xxx — exact value
// determined empirically) are filled in during implementation. Any BROM-range
// PC the firmware jumps to that is *not* in the table raises NotImplemented
// with the PC, so the dev knows exactly which thunk to add next.
```

Roughly 250 LoC including unit tests for each thunk's documented side-effect.

### 5.4 `crates/core/src/peripherals/esp32s3/usb_serial_jtag.rs`

```rust
pub struct UsbSerialJtag {
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
}

impl UsbSerialJtag {
    pub fn new() -> Self;
    pub fn set_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>, echo_stdout: bool);
}

impl Peripheral for UsbSerialJtag {
    // EP1_REG (0x00) write-only: append byte to sink + stdout
    // EP1_CONF_REG (0x04) read-only: returns 0x3 (WR_DONE | EP_DATA_FREE)
    // INT_RAW (0x08), INT_ST (0x0C), INT_ENA (0x10), INT_CLR (0x14): stub
    // …
}
```

Roughly 150 LoC including unit tests.

### 5.5 `crates/core/src/peripherals/esp32s3/systimer.rs`

```rust
pub struct Systimer {
    unit0: SystimerUnit,
    unit1: SystimerUnit,
    conf: u32,                 // 0x00: clock-select + enable bits
    cpu_clock_hz: u32,         // for tick math
}

struct SystimerUnit {
    counter: u64,              // continuously incremented
    snapshot: u64,             // captured on update-handshake write
    pending_load: Option<u64>, // load_lo + load_hi staging
}

impl Peripheral for Systimer {
    // CONF (0x00): bits[0,1] = unit0/unit1 work_en
    // UNIT0_OP (0x04) write: trigger snapshot
    // UNIT0_LOAD_HI (0x18), UNIT0_LOAD_LO (0x1C): pending_load assembly
    // UNIT0_VALUE_HI (0x40), UNIT0_VALUE_LO (0x44): snapshot readback
    // (similar offsets for UNIT1)

    fn tick(&mut self) -> PeripheralTickResult {
        // increment counter by (cycles * 16_000_000 / cpu_clock_hz)
    }
}
```

Roughly 300 LoC including unit tests.

### 5.6 `crates/core/src/peripherals/esp32s3/system_stub.rs`

Three small stubs (`SystemStub`, `RtcCntlStub`, `EfuseStub`) in one file. Total ~150 LoC.

### 5.7 `crates/core/src/peripherals/esp32s3/flash_xip.rs`

```rust
pub struct FlashXipPeripheral {
    backing: Arc<Mutex<Vec<u8>>>,    // shared between I-cache and D-cache mapping
    page_table: [Option<u16>; 64],   // virt page (64 KiB) -> phys page index
    base: u32,                       // 0x42000000 or 0x3C000000
}

impl FlashXipPeripheral {
    pub fn new_shared(backing: Arc<Mutex<Vec<u8>>>, base: u32) -> Self;
    pub fn map_page(&mut self, virt_page: u8, phys_page: u16);
}

impl Peripheral for FlashXipPeripheral {
    // read: page_table[virt_page] -> backing[phys_page * 64KiB + offset]
    // write: SimulationError::MemoryViolation (XIP windows are read-only)
}
```

Roughly 120 LoC including unit tests.

### 5.8 `crates/core/src/system/xtensa.rs`

See §4.5 for the API. Wires all peripherals + returns the CPU. ~120 LoC.

### 5.9 `crates/core/src/lib.rs`

Add `pub mod boot;` (one line).

### 5.10 `crates/core/src/cpu/xtensa_lx7.rs`

Modify the `Break` exec arm to dispatch BREAK 1,14 to ROM thunks (~30 LoC change).

### 5.11 `configs/chips/esp32s3-zero.yaml`

```yaml
name: "esp32s3-zero"
arch: "xtensa-lx7"
iram:
  base: 0x40370000
  size: "512KiB"
dram:
  base: 0x3FC88000
  size: "480KiB"
flash:
  base: 0x42000000
  alias: 0x3C000000
  size: "4MiB"
brom:
  base: 0x40000000
  size: "384KiB"
stack_top: 0x3FCDFFF0
cpu_clock_hz: 80000000
peripherals:
  - id: "rom_thunks"
    type: "rom_thunk_bank"
    base_address: 0x40000000
    size: "384KiB"
  - id: "usb_serial_jtag"
    type: "usb_serial_jtag"
    base_address: 0x60038000
    size: "1KiB"
  - id: "systimer"
    type: "systimer"
    base_address: 0x60023000
    size: "1KiB"
  - id: "system"
    type: "system_stub"
    base_address: 0x600C0000
    size: "4KiB"
  - id: "rtc_cntl"
    type: "rtc_cntl_stub"
    base_address: 0x60008000
    size: "4KiB"
  - id: "efuse"
    type: "efuse_stub"
    base_address: 0x60007000
    size: "1KiB"
```

### 5.12 `examples/esp32s3-hello-world/`

A standard `esp-hal` template:

```
examples/esp32s3-hello-world/
  Cargo.toml
  rust-toolchain.toml      # references the esp toolchain
  .cargo/
    config.toml            # target = "xtensa-esp32s3-none-elf", linker = …
  src/
    main.rs                # canonical hello-world (esp_hal::Config, Delay,
                           # esp_println::println!)
  README.md                # build instructions, toolchain prerequisites
```

`Cargo.toml`:

```toml
[package]
name = "esp32s3-hello-world"
version = "0.1.0"
edition = "2024"

[dependencies]
esp-hal      = { version = "1.0", features = ["esp32s3"] }
esp-println  = { version = "0.13", features = ["esp32s3", "jtag-serial"] }
esp-backtrace = { version = "0.15", features = ["esp32s3", "panic-handler", "println"] }
```

`src/main.rs`:

```rust
#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{delay::Delay, prelude::*};
use esp_println::println;

#[entry]
fn main() -> ! {
    let _peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();
    loop {
        println!("Hello world!");
        delay.delay_millis(1000);
    }
}
```

The exact `esp-hal` / `esp-println` versions pinned during implementation may differ; the implementer pins to the latest stable when writing the crate.

### 5.13 `crates/cli/src/main.rs`

Extend with a `run` subcommand (or extend the existing one — implementer picks the cleaner design). Roughly 60 LoC additional.

```
labwired-cli run \
    --chip configs/chips/esp32s3-zero.yaml \
    --firmware examples/esp32s3-hello-world/target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world \
    [--max-steps N] \
    [--diff /dev/ttyACM0]
```

### 5.14 `docs/case_study_esp32s3_plan2.md` (new)

Closeout document analogous to `docs/case_study_esp32s3_plan1.md`. Records what shipped, what plan corrections were caught, what's next for Plan 3.

## 6. Data flow

**Boot:** ELF bytes → `goblin::Elf::parse` → per-segment `bus.write_u8` writes → segments land in correct peripheral by address resolution → flash-XIP segments also register MMU page mappings → CPU state set → simulation loop starts.

**Step loop:** `cpu.step(&mut bus)` → fetch from PC (may resolve to RAM, flash-XIP, or ROM-thunk peripheral) → decode → execute → if BREAK 1,14 dispatch to thunk → `bus.tick_peripherals()` increments SYSTIMER and any other tick-driven state.

**Output:** firmware writes byte to `0x60038000` → bus routes to `UsbSerialJtag::write` → byte appended to host stdout (for live run) and to capture buffer (for tests).

**Diff (stretch):** parallel HW run via OpenOCD `program` + flash + capture `/dev/ttyACM0` for N seconds → byte-stream comparison against sim capture → pass if byte streams equal (tolerating leading/trailing partial reads).

## 7. Error handling

| Failure mode | Surface |
|---|---|
| Unregistered ROM call | `SimulationError::NotImplemented(format!("ROM thunk at 0x{pc:08x}"))` |
| ELF segment outside any peripheral | `SimulationError::MemoryViolation(addr)` from the bus |
| Missing `_stack_start_cpu0` AND no `stack_top` in YAML | `BootError::NoStackTop` listing both sources |
| EXTMEM page-table overflow (>64 pages) | `BootError::TooManyXipPages` |
| Step loop error during run | logged with PC + last-N-instructions disassembly; CLI exits non-zero |
| Unmapped MMIO read | `SimulationError::MemoryViolation`; the design doc explicitly endorses this strictness |
| YAML schema error | `ConfigError` with the offending field path |

No silent fallbacks. Every failure mode names its cause clearly enough for the dev to act on it.

## 8. Testing strategy

### 8.1 Unit tests (no firmware required)

Per-peripheral `mod tests`:
- `RomThunkBank`: register, lookup, BREAK 1,14 byte-pattern in backing memory at registered addresses.
- `UsbSerialJtag`: write to FIFO appends to sink, status register reads constants, byte-by-byte access works.
- `Systimer`: counter increments on tick, load/update handshake captures snapshot, clock-rate scaling correct for 80 MHz / 240 MHz CPU clocks.
- `SystemStub` / `RtcCntlStub` / `EfuseStub`: register defaults, read-as-zero behaviour, EFUSE MAC readback returns canned value.
- `FlashXipPeripheral`: page-table mapping, read-through, write raises MemoryViolation.
- `boot::esp32s3::fast_boot`: parses a fabricated minimal ELF, places segments correctly, synthesises CPU state correctly.

All in `crates/core` and run with `cargo test --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture`.

### 8.2 Boot-path integration test

`crates/core/tests/boot_esp32s3.rs` (gated on `feature = "esp32s3-fixtures"` so plain `cargo test` without the Xtensa toolchain still works):

1. Build `examples/esp32s3-hello-world/` via `std::process::Command` (`cargo build --release`).
2. Read the resulting ELF.
3. Call `configure_xtensa_esp32s3 + fast_boot`.
4. Assert: `cpu.pc == elf.entry`, `cpu.regs.read_logical(1)` (SP) is in DRAM range, IRAM contains the first 16 bytes of the ELF text segment.

### 8.3 End-to-end demo test (the headline acceptance test)

`crates/core/tests/e2e_hello_world.rs` (also gated on `esp32s3-fixtures`):

1. Build the firmware.
2. Run `configure + fast_boot + step loop` for a bounded number of steps (or simulated cycles equivalent to ~3 seconds wall-clock at 80 MHz).
3. Capture the USB_SERIAL_JTAG sink.
4. Assert: captured bytes contain `"Hello world!\nHello world!\nHello world!\n"` (three iterations).

Failure-mode for this test prints the captured bytes verbatim and the simulator's last 100 instructions.

### 8.4 CLI smoke test

`crates/cli/tests/run_smoke.rs` (also gated on `esp32s3-fixtures`):

Spawn the CLI as a subprocess with the chip YAML + firmware ELF, kill it after 3 wall-clock seconds, parse stdout, assert it contains the expected output.

### 8.5 HW diff (stretch goal — optional for Plan 2 acceptance)

Add a `--diff /dev/ttyACM0` flag to the CLI. When set, the CLI:

1. Flashes the firmware to the live S3-Zero via OpenOCD `program` (Plan 1's existing path).
2. In parallel, runs the simulator and reads from `/dev/ttyACM0`.
3. Compares the two byte streams over a 3-second window.
4. Exits 0 if they match, non-zero with a diff if not.

If this works on the first try, ship it in Plan 2. If it needs investigation (cable enumeration timing, baud-rate quirks), defer to Plan 2.5.

### 8.6 Sequencing

| Order | Task | Why |
|---|---|---|
| 1 | Boot module skeleton + ELF segment loader + unit test | Spine of everything else; prove fast_boot is sound on a fabricated ELF before touching real firmware. |
| 2 | RomThunkBank + initial thunk set + BREAK 1,14 dispatch hook | Required before any real firmware can fetch from the BROM range. |
| 3 | UsbSerialJtag peripheral + unit tests | Required for output to be observable. |
| 4 | Systimer peripheral + unit tests | Required for `delay_millis` to terminate in finite time. |
| 5 | System / RtcCntl / Efuse stubs + FlashXip peripheral | Required for `esp_hal::init` to complete without crashing. |
| 6 | `system/xtensa.rs` glue + chip YAML | Wires everything together. |
| 7 | Build the example firmware crate | First time we touch real esp-hal. |
| 8 | First e2e attempt; iterate on missing ROM thunks | The Risk #1 mitigation cycle — adds whatever thunks the firmware turns out to need. |
| 9 | CLI `run` subcommand wiring | User-facing surface. |
| 10 | E2E test in CI; case study | Closeout. |
| 11 (optional) | `--diff` HW comparison | Stretch goal. |

## 9. Exit criteria

| # | Criterion | Verification |
|---|---|---|
| 1 | Sim suite stays green | `cargo test --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture` ≥ baseline + new unit tests |
| 2 | `esp-hal` hello-world builds | `cd examples/esp32s3-hello-world && cargo build --release` exits 0 |
| 3 | Fast-boot synthesises correct entry state | `cargo test -p labwired-core --features esp32s3-fixtures boot_esp32s3` passes |
| 4 | E2E demo prints expected output | `cargo test -p labwired-core --features esp32s3-fixtures e2e_hello_world` passes |
| 5 | CLI runs the firmware end-to-end | manually: `labwired-cli run --chip configs/chips/esp32s3-zero.yaml --firmware …/esp32s3-hello-world` prints "Hello world!" once per second to terminal stdout for at least 5 iterations |
| 6 | No silent ROM calls | every BROM-range PC the CPU jumps to either has a registered thunk or raises `NotImplemented` with a clear message |
| 7 | Documentation | `docs/case_study_esp32s3_plan2.md` exists and summarises shipped + Plan 3 follow-ups |

The `--diff` stretch goal is **not** in the exit-criteria table — its absence does not block Plan 2 acceptance.

## 10. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| esp-hal init code calls a ROM function we haven't anticipated | High | Medium | Cycle is fast: `NotImplemented` names the PC → add thunk → retry. Budget Task 8 explicitly for this iteration. |
| esp-hal's clock-config code reads a SYSTEM register we haven't stubbed correctly and causes a spin loop | Medium | Medium | If hello-world hangs, instrument the bus's unmapped-MMIO path to log read offsets. Add the missing register. |
| Flash-XIP MMU page table semantics are subtler than the design doc suggests | Medium | High | Fallback: place the bytes directly in the XIP window's backing store with a 1:1 page table. esp-hal hello-world is small enough that full XIP fidelity is overkill for one print loop. |
| `SYSTIMER` clock-domain math diverges from real silicon | Low | Low | The criterion is "approximately one print per second", not "exactly 1.000 s". |
| Building esp-hal hello-world requires the Xtensa toolchain | Certain | Low | Already installed for Plan 1 HW oracle work. Document setup in the example crate's README. Gate the e2e test on a feature flag so plain `cargo test` works without the toolchain. |
| `esp-hal`'s `init()` does runtime SYSTEM/RTC writes that depend on read-back behaviour we haven't anticipated | Medium | Medium | If `init()` panics or hangs, dump the last MMIO writes via tracing, identify the offending register, extend the relevant stub. |
| The chosen `esp-hal` / `esp-println` versions are unstable and the API differs from what the spec describes | Medium | Low | Pin to the latest stable at implementation time. The hello-world API is unlikely to break across minor versions. If it does, adapt the example accordingly. |
| The `--diff` HW path has cable-enumeration timing issues | Medium (stretch only) | Low | It's a stretch goal; defer to Plan 2.5 if it doesn't work cleanly. |

## 11. Schedule

Estimated effort: 5–8 working days.

| Day | Tasks |
|---|---|
| 1 | Boot module skeleton + ELF segment loader; unit tests for boot. |
| 2 | RomThunkBank + initial thunk set; BREAK 1,14 dispatch hook in CPU; unit tests. |
| 3 | UsbSerialJtag peripheral + Systimer peripheral; unit tests. |
| 4 | System / RtcCntl / Efuse stubs; FlashXipPeripheral; system glue + chip YAML. |
| 5 | Build the esp-hal hello-world example crate; first e2e attempt. |
| 6 | Iterate on missing ROM thunks until the firmware reaches `main` and prints. |
| 7 | E2E test in CI; CLI `run` subcommand wiring; case study. |
| 8 | Buffer for unexpected issues / `--diff` stretch goal. |

## 12. References

- `docs/design/2026-04-24-esp32s3-zero-digital-twin-design.md` (the design spec — §6 boot path, §8 peripheral plan, §10 fixture firmware)
- `docs/case_study_esp32s3_plan1.md` (Plan 1 closeout)
- `docs/case_study_esp32s3_plan2.md` (to be created by this plan)
- ESP32-S3 TRM v1.4 — §3.3 memory map, §16 SYSTIMER, §27 USB_SERIAL_JTAG, §31 SYSTEM, §32 RTC_CNTL
- `esp-hal` source (`vendor/esp-hal-<tag>/` — to be vendored at implementation time per design doc §10) — first authority for what register layout the firmware actually expects
- `esp-println` source — for the `jtag-serial` feature's output path
- ESP-IDF `xtensa-rt` ROM function signatures (MIT-licensed, design doc §7.6 reference rule allows reading)
- Real ESP32-S3-Zero via OpenOCD — first authority for behavioural ground truth (per design doc §7.6)
