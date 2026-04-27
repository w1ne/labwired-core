# Plan 3 — GPIO + IO_MUX + Interrupt Matrix + SYSTIMER Alarms (esp-hal blinky end-to-end)

**Date:** 2026-04-26
**Branch target:** `feature/esp32s3-plan3-gpio-blinky` (off `feature/esp32s3-plan2-boot-uart`)
**Predecessor:** Plan 2 (`docs/case_study_esp32s3_plan2.md`, branch `feature/esp32s3-plan2-boot-uart`)
**Successor:** Plan 3.5 — WS2812 + RMT (the iconic S3-Zero RGB demo) and Plan 4 — I²C + SPI (sensors)

---

## 1. Goal

Run a real `esp-hal` Rust firmware that toggles a GPIO from a SYSTIMER alarm ISR end-to-end in the LabWired simulator. The simulator captures every GPIO transition with a cycle stamp; the e2e test asserts the expected toggle pattern.

Plan 3 closes the M4 milestone from `docs/design/2026-04-24-esp32s3-zero-digital-twin-design.md`: "Blinky + GPIO interrupt. First end-to-end `labwired compare` PASS verdict." The "compare" stretch portion (HW oracle GPIO diff) is deferred to Plan 3.5.

The demo proves the **interrupt matrix → SYSTIMER alarm → CPU vector → ISR → GPIO write** round-trip works in the simulator, identical to what would happen on real silicon.

## 2. Non-goals

- **WS2812 + RMT.** The iconic S3-Zero "blink the onboard RGB" demo lands in Plan 3.5. Plan 3 uses a plain GPIO output (visible on real HW only via a logic analyzer / multimeter; visible in the simulator via the new GPIO observer trace).
- **HW oracle diff for the GPIO trace.** OpenOCD GPIO polling has unbounded latency and we don't have a logic analyzer wired up. Plan 3.5 work.
- **GPIO input interrupts.** The demo doesn't use them; the registers exist but the GPIO source isn't wired into the intmatrix yet.
- **TIMG WDT / TIMG general-purpose timers.** Separate timer block; not required for the demo.
- **Multi-core SMP.** Still cpu0 only.
- **VCD trace export.** The `GpioObserver` trait is the abstraction; a VCD writer is a future Plan.
- **Full 99-source × 32-CPU-IRQ intmatrix routing semantics.** The demo only routes one source (SYSTIMER_TARGET0); the peripheral implements the full register surface but only the path the firmware exercises is validated.
- **Anything Plan 1.5 (HW oracle cleanup) was going to fix** — still deferred.
- **GPIOs above index 31.** ESP32-S3 has 49 GPIOs split across two register sets (GPIO0..31 + GPIO32..48 at offset +0x40). Plan 3 implements GPIO0..31 only. Demo uses GPIO2; safely in scope.

## 3. Context

Plan 2 closed with a real esp-hal Rust binary printing "Hello world!" via USB_SERIAL_JTAG end-to-end. The boot path, SYSTIMER counter, USB_SERIAL_JTAG MMIO surface, and 14 ROM thunks are all in place. The CPU's interrupt-dispatch machinery (`pending_irq_level` / `dispatch_irq`) was implemented in Plan 1 against a hardcoded `IRQ_LEVELS` table; Plan 3 replaces that hardcoded routing with a configurable bus-side intmatrix lookup.

What's missing for "the LED toggles from an ISR" is:
- A GPIO peripheral with observable output transitions.
- An IO_MUX peripheral so esp-hal's pin-config code completes.
- An intmatrix peripheral that records peripheral-source → CPU-IRQ-slot mappings.
- Alarm support in SYSTIMER (the counter is there; alarms aren't).
- A wiring update so the bus delivers source-IDs from peripheral ticks through the intmatrix to the CPU's interrupt-dispatch path.

The demo firmware is structurally similar to Plan 2's hello-world but exercises an entirely new code path (interrupt delivery instead of polling) and a new peripheral (GPIO) with a new observability mechanism (`GpioObserver`).

## 4. Architecture

### 4.1 Three-layer split (unchanged from Plan 2)

- **Simulator core** (`crates/core/`) — three new peripheral modules (gpio, io_mux, intmatrix); SYSTIMER extended with alarms; `Bus` trait gains a default `route_irq_source_to_cpu_irq`; CPU's `pending_irq_level` updated to consult it.
- **Firmware fixtures** (`examples/esp32s3-blinky/`) — a real `esp-hal` Cargo crate, target `xtensa-esp32s3-none-elf`, builds via `cargo +esp build --release`.
- **CLI** (`crates/cli/`) — installs a default tracing `GpioObserver` on `run`; new optional `--gpio-trace <path>` flag emits a JSON trace of GPIO transitions.

### 4.2 GPIO + GpioObserver

The GPIO peripheral lives at `0x6000_4000` and models GPIO0..31 with the standard Espressif register layout (OUT, OUT_W1TS, OUT_W1TC, ENABLE, ENABLE_W1TS, ENABLE_W1TC, IN, PINn config). The peripheral's distinguishing feature is its observability:

```rust
pub trait GpioObserver: Send + Sync + std::fmt::Debug {
    /// Called synchronously inside the bus write path on every pin transition.
    /// Observers must not panic.
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64);
}
```

On every write to `OUT`, `OUT_W1TS`, or `OUT_W1TC`, the peripheral compares the old and new `out` field, finds each changed bit, and notifies all registered observers. The cycle stamp is the peripheral's monotonic `cycle` counter (incremented on `tick()`).

Observers register via `Esp32s3Gpio::add_observer(Arc<dyn GpioObserver>)`. The CLI's wiring path uses `Esp32s3Wiring::add_gpio_observer(bus, observer)` which walks `bus.peripherals` to find the GPIO entry and pushes the observer onto its list.

The GPIO peripheral's `int_enable` and `int_type` per-pin registers exist (esp-hal writes them at boot) but the GPIO source isn't yet routed into the intmatrix — Plan 3 doesn't model GPIO-input interrupts.

### 4.3 IO_MUX

A small declarative-shaped peripheral at `0x6000_9000`. Holds a per-pin function-select word (49 entries × 4 bytes). Reads round-trip whatever was written. The simulator does not actually enforce the matrix routing implied by FUN_MUX; the peripheral exists so esp-hal's pin-config sequence completes successfully.

### 4.4 Interrupt Matrix

The intmatrix peripheral lives at `0x600C_2000` and models the cpu0 source→IRQ-slot mapping. Per ESP32-S3 TRM §9.4, each peripheral source ID (0..98) has a 32-bit `PRO_<source>_INTR_MAP_REG` at `0x000 + 4*src` that selects the cpu0 IRQ slot (0..31) the source delivers to. The peripheral records every write to these registers in a `[u8; 99]` array.

The peripheral exposes `pub fn route(&self, source_id: u32) -> Option<u8>` returning the registered cpu0 IRQ slot, or None if unbound. The CPU's `pending_irq_level()` consults this through a new default method on the `Bus` trait:

```rust
fn route_irq_source_to_cpu_irq(&self, _source_id: u32) -> Option<u8> { None }
```

`SystemBus` overrides this method to scan registered peripherals via `as_any().downcast_ref::<Esp32s3IntMatrix>()` and forward the lookup — same pattern as `bus.get_rom_thunk` from Plan 2.

The cpu0 IRQ slot then maps to an interrupt level via the static `XCHAL_INT_LEVEL` table (already known from Plan 1's interrupt dispatch work — preserved and consulted as before once we have the slot).

APP-core (cpu1) `APP_<source>_INTR_MAP_REG` writes are silently accepted but not modeled (cpu1 is out of scope).

### 4.5 SYSTIMER alarms

Plan 2's SYSTIMER counter is preserved unchanged. Plan 3 extends it with three alarms per unit (UNIT0_ALARM0/1/2 + UNIT1_ALARM0/1/2 = 6 total). Each alarm has:
- A 64-bit target value (LO + HI registers).
- An auto-reload bit + a period (for periodic alarms).
- An enable bit.
- A pending bit (set in INT_RAW when `counter >= target`; cleared via INT_CLR write-1-to-clear).

`tick()` checks each enabled alarm against its unit's counter. On a match, sets the alarm's INT_RAW bit. If `INT_ENA[alarm_n]` is also set, the tick result includes the matching SYSTIMER source ID in `explicit_irqs`. SYSTIMER source IDs per TRM §9.4 table: TARGET0 = 79, TARGET1 = 80, TARGET2 = 81 (Plan 3 only routes UNIT0 alarms to these three IDs; UNIT1 alarms share the same source IDs since the table doesn't distinguish — match real silicon behavior).

### 4.6 Bus tick → CPU IRQ delivery

The data flow on each step:

1. CPU executes one instruction.
2. `bus.tick_peripherals()` runs every peripheral's `tick()`. SYSTIMER returns `PeripheralTickResult { explicit_irqs: vec![79], ... }` if alarm 0 fired this tick.
3. The bus aggregates returned source IDs. For each source ID, it calls its own `route_irq_source_to_cpu_irq(src)` which delegates to the intmatrix peripheral. If a CPU IRQ slot is returned, the bus marks that bit in a `pending_cpu_irqs: u32` field.
4. Before the next CPU step, `cpu.step` reads `bus.pending_cpu_irqs`, ANDs with `INTENABLE`, computes the highest priority via `XCHAL_INT_LEVEL`, and dispatches via the existing Plan 1 `dispatch_irq(level)` machinery.

The CPU's existing `pending_irq_level()` is updated to consult `bus.pending_cpu_irqs` instead of (or in addition to) the SR-file `INTERRUPT` register. The SR-file `INTERRUPT` register is preserved as a software-write path (firmware can also raise IRQs via `wsr.intset`).

This means: the intmatrix and the bus collaborate to translate "peripheral source N fired" into "CPU IRQ slot M is pending"; the CPU reads pending CPU IRQ slots and dispatches as before.

### 4.7 Firmware: esp-hal blinky

A standard esp-hal crate, structurally similar to Plan 2's hello-world. Key shape:

```rust
#![no_std]
#![no_main]
use esp_backtrace as _;
use esp_hal::{
    delay::Delay, gpio::{Level, Output, OutputConfig},
    interrupt::{self, Priority},
    main, peripherals::SYSTIMER,
    timer::systimer::{SystemTimer, Alarm, Periodic, FrozenUnit},
};
use core::cell::RefCell;
use critical_section::Mutex as CsMutex;

static LED: CsMutex<RefCell<Option<Output<'static>>>> = CsMutex::new(RefCell::new(None));
static ALARM: CsMutex<RefCell<Option<Alarm<'static, Periodic>>>> = CsMutex::new(RefCell::new(None));

#[handler(priority = Priority::Priority3)]
fn alarm_isr() {
    critical_section::with(|cs| {
        if let Some(led) = LED.borrow_ref_mut(cs).as_mut() { led.toggle(); }
        if let Some(alarm) = ALARM.borrow_ref_mut(cs).as_mut() { alarm.clear_interrupt(); }
    });
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    let led = Output::new(p.GPIO2, Level::Low, OutputConfig::default());
    let st = SystemTimer::new(p.SYSTIMER);
    let frozen = FrozenUnit::new(&st.unit0);
    let alarm = Alarm::new(st.alarm0, &frozen);
    alarm.set_period(500u32.millis());
    alarm.enable_interrupt(true);
    critical_section::with(|cs| {
        LED.replace(cs, Some(led));
        ALARM.replace(cs, Some(alarm));
    });
    interrupt::enable(esp_hal::peripherals::Interrupt::SYSTIMER_TARGET0,
                     Priority::Priority3).unwrap();
    loop { core::hint::spin_loop(); }
}
```

The exact API may shift between esp-hal versions. The implementer pins to the latest stable esp-hal at task start and adapts. The structural shape (LED + alarm in critical-section mutexes, ISR toggles LED + clears IRQ, main spin-loops) stays.

## 5. Components

### 5.1 `crates/core/src/peripherals/esp32s3/gpio.rs` (new, ~250 LoC)

```rust
pub trait GpioObserver: Send + Sync + std::fmt::Debug {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64);
}

pub struct Esp32s3Gpio {
    enable: u32,
    out: u32,
    in_data: u32,
    int_enable: u32,
    int_type: [u8; 32],
    cycle: u64,
    observers: Vec<Arc<dyn GpioObserver>>,
}

impl Esp32s3Gpio {
    pub fn new() -> Self;
    pub fn add_observer(&mut self, obs: Arc<dyn GpioObserver>);
    pub fn set_pin_input(&mut self, pin: u8, level: bool);  // for tests
}

impl Peripheral for Esp32s3Gpio {
    fn read(&self, offset: u64) -> SimResult<u8>;
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()>;
    fn tick(&mut self) -> PeripheralTickResult;  // increments cycle counter
}
```

Register subset (per ESP32-S3 TRM §5.5):
- `0x04` GPIO_OUT_REG (32 bits)
- `0x08` GPIO_OUT_W1TS_REG
- `0x0C` GPIO_OUT_W1TC_REG
- `0x20` GPIO_ENABLE_REG
- `0x24/0x28` ENABLE_W1TS/W1TC
- `0x3C` GPIO_IN_REG (read-only)
- `0x74 + pin*4` GPIO_PINn_REG (per-pin int_type, int_ena bits)

On `out` change, scan for flipped bits; call `on_pin_change(pin, old, new, cycle)` for each registered observer.

### 5.2 `crates/core/src/peripherals/esp32s3/io_mux.rs` (new, ~150 LoC)

```rust
pub struct Esp32s3IoMux {
    pin_func: [u32; 49],
}
```

MMIO at `0x6000_9000`, size 0x100. Per-pin function-select register (round-trip storage). No simulator-side enforcement of function routing — esp-hal writes these to configure pins; the simulator records the writes for completeness.

### 5.3 `crates/core/src/peripherals/esp32s3/intmatrix.rs` (new, ~250 LoC)

```rust
pub struct Esp32s3IntMatrix {
    cpu0_route: [u8; 99],
}

impl Esp32s3IntMatrix {
    pub fn new() -> Self;
    pub fn route(&self, source_id: u32) -> Option<u8>;
}
```

MMIO at `0x600C_2000`, size 0x800. Register layout per TRM §9.4: `0x000 + 4*src` PRO_<source>_INTR_MAP_REG (cpu0 IRQ slot). cpu1 mapping registers (offset > 0x400) silently accepted but not modeled.

The peripheral's `route()` is called from `SystemBus::route_irq_source_to_cpu_irq` via `as_any().downcast_ref`.

### 5.4 `crates/core/src/peripherals/esp32s3/systimer.rs` (modify, +200 LoC)

Add three alarms per unit. New struct field:

```rust
struct AlarmState {
    target: u64,
    pending: bool,
    enabled: bool,
    auto_reload: bool,
    period: u64,
}

pub struct Systimer {
    // existing fields preserved...
    unit0_alarms: [AlarmState; 3],
    unit1_alarms: [AlarmState; 3],
    int_ena: u32,
    int_clr_pending: u32,
}
```

New register offsets per TRM §16.5:
- `0x4C-0x60` ALARM0_TARGET_HI/LO/CONF, ALARM1_*, ALARM2_*
- `0x64` TARGET0_CONF (auto-reload + alarm-enable bits)
- `0x68/0x6C/0x70` TARGET0/1/2_INT_ENA
- `0x74` INT_RAW (read-only)
- `0x78` INT_ST (= INT_RAW & INT_ENA)
- `0x7C` INT_ENA
- `0x80` INT_CLR (W1C)

`tick()` checks each enabled alarm, sets pending bits, returns source IDs in `explicit_irqs`.

Source IDs: SYSTIMER_TARGET0 = 79, TARGET1 = 80, TARGET2 = 81.

### 5.5 `crates/core/src/lib.rs` (modify, +30 LoC)

Extend the `Bus` trait with a default `route_irq_source_to_cpu_irq(source_id) -> Option<u8>` returning None. Override on `SystemBus`. Document that `PeripheralTickResult.explicit_irqs` carries source IDs (not CPU IRQ numbers) on ESP32-S3, so the bus can route them through the intmatrix.

(For ARM/Cortex-M peripherals, `explicit_irqs` continues to mean direct NVIC IRQ numbers per Plan 1; the `route_irq_source_to_cpu_irq` default-of-None preserves that behavior.)

### 5.6 `crates/core/src/cpu/xtensa_lx7.rs` (modify, +20 LoC)

Update `pending_irq_level()` to also consult `bus.pending_cpu_irqs` (the new field that aggregates peripheral-tick source IDs through the intmatrix). Either OR with the existing SR `INTERRUPT` register, or replace — to be decided during implementation by reading the existing Plan 1 code carefully.

### 5.7 `crates/core/src/system/xtensa.rs` (modify, +50 LoC)

Three new peripheral registrations: `Esp32s3IoMux`, `Esp32s3Gpio`, `Esp32s3IntMatrix`. New method on `Esp32s3Wiring`:

```rust
impl Esp32s3Wiring {
    pub fn add_gpio_observer(&self, bus: &mut SystemBus, obs: Arc<dyn GpioObserver>);
}
```

Walks `bus.peripherals` to find the GPIO entry by name, downcasts to `Esp32s3Gpio`, pushes the observer.

### 5.8 `crates/cli/src/main.rs` (modify, +40 LoC) + `crates/cli/src/gpio_observer.rs` (new, ~50 LoC)

`gpio_observer.rs` exposes:

```rust
pub struct TracingGpioObserver;
pub struct JsonGpioObserver { /* file handle */ }
```

Both implement `GpioObserver`. `TracingGpioObserver::on_pin_change` calls `tracing::info!(target: "gpio", "GPIO{}: {}->{}  (cycle={})", pin, from as u8, to as u8, sim_cycle)`. `JsonGpioObserver` writes one JSON line per call.

In `run_firmware`, after `configure_xtensa_esp32s3` returns, call `wiring.add_gpio_observer(&mut bus, Arc::new(TracingGpioObserver))`. If `--gpio-trace path` was given, also install a `JsonGpioObserver(path)`.

### 5.9 `examples/esp32s3-blinky/` (new directory, new esp-hal crate, ~180 LoC)

Standard esp-hal template:

```
examples/esp32s3-blinky/
  Cargo.toml
  rust-toolchain.toml
  .cargo/
    config.toml
  src/
    main.rs
  build.rs
  README.md
```

`Cargo.toml` mirrors `examples/esp32s3-hello-world/Cargo.toml` with the same esp-hal version pin.

### 5.10 `crates/core/tests/intmatrix_alarm.rs` (new, ~150 LoC)

Integration test (no firmware required, gated on default features). Builds a minimal `SystemBus` with `configure_xtensa_esp32s3`, manually:
1. Routes SYSTIMER_TARGET0 (source 79) to CPU IRQ slot 15 via intmatrix MMIO writes.
2. Sets SYSTIMER UNIT0_ALARM0 target to 100 ticks.
3. Enables ALARM0 + ALARM0_INT_ENA.
4. Sets `INTENABLE` bit 15.
5. Sets VECBASE to a known address.
6. Plants a tiny hand-written Xtensa ISR at that vector that toggles GPIO2 and clears the INT_CLR alarm bit.
7. Runs the CPU step loop for enough cycles to fire 3 alarms.
8. Asserts via a recording GpioObserver that GPIO2 saw 3 transitions at the expected cycles.

This validates the whole intmatrix → CPU dispatch → ISR → GPIO chain without depending on esp-hal's runtime.

### 5.11 `crates/core/tests/e2e_blinky.rs` (new, ~120 LoC, gated on `esp32s3-fixtures`)

Mirrors `e2e_hello_world.rs`. Builds `examples/esp32s3-blinky` via `cargo +esp build --release`, runs the simulator for 240M cycles, captures GPIO transitions via a recording observer, asserts ≥4 transitions on pin 2.

### 5.12 `configs/chips/esp32s3-zero.yaml` (modify, +20 LoC)

Add `gpio` (0x6000_4000), `io_mux` (0x6000_9000), `intmatrix` (0x600C_2000) entries under `peripherals:`.

### 5.13 `crates/core/src/peripherals/esp32s3/mod.rs` (modify, +3 lines)

```rust
pub mod gpio;
pub mod intmatrix;
pub mod io_mux;
```

(Alphabetical placement.)

### 5.14 `docs/case_study_esp32s3_plan3.md` (new)

Closeout document analogous to Plan 2's. Records what shipped, plan corrections caught during implementation, what's next for Plan 3.5 / Plan 4.

## 6. Data flow

**Boot (unchanged from Plan 2):** ELF → `goblin::Elf::parse` → segment loads (IRAM, DRAM, flash-XIP) → CPU state synthesis.

**Pin config (new):** firmware writes IO_MUX_GPIO2_REG (function 1 = GPIO) → IoMux peripheral round-trips. firmware writes GPIO_ENABLE_W1TS bit 2 → GPIO peripheral sets `enable[2] = 1`. firmware writes GPIO_OUT bit 2 (initial level) → GPIO peripheral updates `out`, fires observers.

**Intmatrix bind (new):** firmware writes INTMATRIX_PRO_SYSTIMER_TARGET0_INTR_MAP_REG = 15 → intmatrix records `cpu0_route[79] = 15`.

**SYSTIMER alarm setup (new):** firmware writes UNIT0_ALARM0_TARGET_LO/HI = 8000000 (100ms at 16 MHz SYSTIMER → ~80M CPU cycles at 80 MHz). firmware writes TARGET0_CONF = (auto_reload | enable). firmware writes INT_ENA bit 0 = 1. firmware writes `wsr.intenable` to set bit 15 in CPU INTENABLE.

**Alarm fires:** SYSTIMER's `tick()` finds `unit0.counter >= alarm0.target`, sets INT_RAW bit 0, returns `PeripheralTickResult { explicit_irqs: vec![79], ... }`. Bus calls `route_irq_source_to_cpu_irq(79)` → returns `Some(15)`. Bus sets `pending_cpu_irqs |= 1 << 15`.

**CPU dispatches:** `cpu.step` reads `bus.pending_cpu_irqs & cpu.sr.read(INTENABLE)`, picks highest level via `XCHAL_INT_LEVEL[15]`, dispatches via `dispatch_irq(level)` (existing Plan 1 path). Vector handler dispatches to esp-hal's registered `alarm_isr`.

**ISR runs:** `alarm_isr` calls `led.toggle()` → GPIO peripheral writes OUT_W1TS bit 2 → fires observer with `pin=2, from=0, to=1, cycle=80000023`. `alarm_isr` calls `alarm.clear_interrupt()` → SYSTIMER INT_CLR bit 0 written → INT_RAW bit 0 cleared. ISR returns via RFI.

**Observer captures:** `TracingGpioObserver` emits `[INFO gpio] GPIO2: 0->1  (cycle=80000023)`. If `--gpio-trace`, `JsonGpioObserver` writes `{"sim_cycle":80000023,"pin":2,"from":false,"to":true}`.

## 7. Error handling

| Failure mode | Surface |
|---|---|
| Unrouted source ID | Silently dropped; `tracing::warn!` once per source per session |
| GPIO write to pin >= 32 | Silently accepted (Plan 3 scope limit) |
| IO_MUX write to pin >= 49 | Silently accepted |
| Observer panics | Propagates out of `bus.write_u8` and crashes the simulator. Observers must not panic (documented in trait doc). |
| Alarm fires before INT_ENA is set | INT_RAW accumulates; IRQ doesn't deliver. Matches silicon. |
| Firmware doesn't clear INT_RAW after handling | Next tick, alarm fires again immediately (storm). Matches silicon. esp-hal's `alarm.clear_interrupt()` is correct. |
| Intmatrix MMIO read at offset > 0x400 | Returns 0 (cpu1 mapping read; we don't model cpu1) |

## 8. Testing strategy

### 8.1 Unit tests (per peripheral, no firmware required)

- **GPIO:** OUT_W1TS sets bit + fires observer with correct `from`/`to`/`pin`. OUT_W1TC clears bit + fires observer. Direct OUT write fires observers for each changed bit. ENABLE_W1TS/W1TC manipulate the enable mask. PINn_REG round-trips int_type/int_ena bits.
- **IO_MUX:** Per-pin function-select register round-trip.
- **Intmatrix:** Bind source 79 to CPU IRQ 15, look up returns Some(15). Unbound source returns None. MMIO write to PRO_<src>_INTR_MAP records correctly.
- **SYSTIMER alarms:** Set target to 50, tick 50 times, INT_RAW bit set. INT_CLR write clears the bit. Auto-reload regenerates after target wrap.

### 8.2 Integration test (no firmware, hand-rolled ISR)

`crates/core/tests/intmatrix_alarm.rs` (see §5.10 above). Validates the whole IRQ delivery chain without depending on esp-hal.

### 8.3 End-to-end test (gated on `esp32s3-fixtures`)

`crates/core/tests/e2e_blinky.rs` (see §5.11 above). Builds the firmware; runs the simulator; captures GPIO transitions via observer; asserts ≥4 transitions at the expected cadence.

### 8.4 CLI smoke test

`labwired-cli run --chip esp32s3-zero.yaml --firmware esp32s3-blinky` should print `[INFO gpio] GPIO2: 0->1  (cycle=...)` and `[INFO gpio] GPIO2: 1->0  (cycle=...)` alternating to stdout, paced ~40M cycles apart (500 ms at 80 MHz → 40M cycles).

## 9. Exit criteria

| # | Criterion | Verification |
|---|---|---|
| 1 | Sim suite stays green | `cargo test --workspace --exclude ...` ≥ 568 baseline + new tests |
| 2 | esp-hal blinky builds | `cd examples/esp32s3-blinky && cargo +esp build --release` exits 0 |
| 3 | Integration test passes | `cargo test -p labwired-core intmatrix_alarm` PASS |
| 4 | E2E demo ticks LED | `cargo test -p labwired-core --features esp32s3-fixtures e2e_blinky` PASS |
| 5 | CLI runs the firmware end-to-end | `labwired-cli run --chip ... --firmware ...esp32s3-blinky` shows alternating `GPIO2: 0->1` / `GPIO2: 1->0` lines |
| 6 | `--gpio-trace path.json` produces valid JSON | manual inspection: structured `{sim_cycle, pin, from, to}` records, one per line |
| 7 | Documentation | `docs/case_study_esp32s3_plan3.md` exists |

## 10. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| esp-hal interrupt API has changed substantially in the latest version vs the spec example | Medium | Low | Latest esp-hal examples are the source of truth; structural shape (alarm + ISR + GPIO toggle) is stable. |
| Intmatrix routing exposes CPU IRQ → level via a static table that doesn't match real silicon for some specific level | Medium | Medium | If a routed source delivers at the wrong level, the firmware's vector dispatch picks the wrong handler. Cross-check via real silicon (Plan 3.5 work). |
| ISR enters but `Output::toggle` doesn't update GPIO_OUT because W1TS/W1TC handling is wrong | Medium | High | Unit test for OUT_W1TS specifically writes 0x4 (set bit 2) and asserts `out & 0x4 != 0` and that the observer fired with `pin=2, from=0, to=1`. |
| GPIO observer notification ordering vs CPU step ordering causes the test to record at the wrong cycle stamp | Low | Low | Observers fire synchronously inside the bus write path; cycle stamp is the cycle of the write, not the next tick. Documented in GpioObserver trait. |
| esp-hal's blinky requires a peripheral we don't model | Medium | Medium | Same iteration loop as Plan 2 Task 11. Mitigated by the broad MMIO catch-all stubs that landed in Plan 2. |
| The recording observer in the e2e test allocates per-transition and the simulator runs slow under heavy GC | Low | Low | The recording observer is a `Mutex<Vec<...>>`. 4 transitions × 16 bytes each = 64 bytes total. Trivial. |
| ISR re-entry: alarm fires again before the previous ISR finishes clearing INT_CLR | Low | Medium | The CPU's `dispatch_irq` sets PS.EXCM so further interrupts at that level are masked until RFI. Plan 1 already validated this against silicon. |

## 11. Schedule

Estimated effort: 4–6 working days.

| Day | Tasks |
|---|---|
| 1 | GPIO + IO_MUX + intmatrix peripherals (Tasks 1-3 in the plan), unit tests for each. |
| 2 | SYSTIMER alarm extension + CPU `pending_irq_level` update + bus trait extension (Tasks 4-5). |
| 3 | system/xtensa.rs wiring + integration test (Tasks 6-7). |
| 4 | CLI extension + blinky example crate build (Tasks 8-9). |
| 5 | Iterate firmware until LED toggles (Task 10). |
| 6 | E2E test + case study (Tasks 11-12). |

## 12. References

- `docs/design/2026-04-24-esp32s3-zero-digital-twin-design.md` (the design spec — §8 peripheral plan)
- `docs/case_study_esp32s3_plan2.md` (Plan 2 closeout)
- `docs/case_study_esp32s3_plan3.md` (to be created by this plan)
- ESP32-S3 TRM v1.4 — §5 GPIO + IO_MUX, §9 Interrupt Matrix, §16 SYSTIMER
- `esp-hal` source — first authority for the firmware API and interrupt-binding sequence
- ESP-IDF `xtensa-rt` `XCHAL_INT_LEVEL` table — for the cpu IRQ slot → level mapping
- Real ESP32-S3-Zero via OpenOCD — first authority for behavioural ground truth (per design doc §7.6)
