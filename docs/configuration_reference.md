---
description: "LabWired uses YAML configuration to define the simulated hardware, so the same firmware binary runs against different hardware configurations."
---

# Configuration Reference

LabWired uses a YAML-based configuration system to define the simulated hardware environment. This separation allows the same firmware binary to be tested against different hardware configurations (e.g., changing memory sizes or remapping peripherals) without recompilation.

## 1. File Hierarchy

A complete simulation requires two descriptor files:

1.  **Chip Descriptor** (`chips/<name>.yaml`): Defines the internal architecture of the SoC (Flash/RAM size, internal peripheral addresses).
2.  **System Manifest** (`systems/<name>.yaml`): Instantiates a chip and defines board-level wiring (external sensors, UART loopbacks).

A chip descriptor loaded from a path may `include:` other chip YAMLs (§2), and
a manifest's `external_devices:` entries resolve either to a model built into
the engine or to a declarative device descriptor — `configs/devices/*.yaml` —
whose `behavior:` primitives are the third, data-only configuration surface
(§3, External devices).

## 2. Chip Descriptor Schema

Defines the invariant properties of the silicon.

```yaml
name: "STM32F103"
arch: "arm"
core: "cortex-m3"   # Exact CPU core; gates core-specific behavior
                    # (e.g. bit-band aliasing exists only on M3/M4)
flash:
  base: 0x08000000
  size: "64KB"  # Supports KB/MB suffixes
ram:
  base: 0x20000000
  size: "20KB"

peripherals:
  # Internal Peripheral Definition
  - id: "usart1"
    type: "uart"
    base_address: 0x40013800
    irq: 37
    config:
      profile: "stm32f1"  # Loads architecture-specific register map

  - id: "gpioa"
    type: "gpio"
    base_address: 0x40010800
    config:
      profile: "stm32f1"

  # Declarative Peripheral (Custom)
  - id: "my_custom_timer"
    type: "declarative"
    base_address: 0x40004000
    config:
      path: "../peripherals/custom_timer.yaml"
```

### Every chip key

This is the complete `ChipDescriptor` surface (`crates/config/src/chip.rs`); the
sample above is its common shape. `name`, `arch`, `flash`, `ram`, and
`peripherals` are required (`peripherals: []` is a valid chip with no MMIO);
everything else is optional and has a default.

| Key | Meaning | Minimal example |
|---|---|---|
| `schema_version` | Free-form descriptor version; defaults to `"1.0"`. | `schema_version: "1.0"` |
| `name` | Part label; also how a manifest's bare-word `chip:` finds a built-in. | `name: "STM32F103"` |
| `arch` | Instruction-set family: `arm`, `riscv`, `xtensa`, `avr`, `unknown` (family aliases like `cortex-m3` are accepted). | `arch: "arm"` |
| `core` | Exact core (`cortex-m3`, `cortex-m33`, `cortex-m0+`); gates per-core bus behavior — bit-band aliasing exists only on M3/M4. | `core: "cortex-m3"` |
| `cpu_hz` | The clock the simulator runs the core at, in Hz. Declared, not derived — see [Clock, schema and includes](#clock-schema-and-includes). `0` = undeclared. | `cpu_hz: 72_000_000` |
| `flash` | Primary flash window: `base`, `size`, optional `cpu_writes` policy (see below). | `flash: {base: 0x08000000, size: "64KB"}` |
| `ram` | Primary RAM window: `base`, `size`, optional `cpu_writes`. | `ram: {base: 0x20000000, size: "20KB"}` |
| `reset_vector_offset` | Bytes from the flash base to the application vector table when a stage-2 bootloader precedes it (`0` = vectors at the flash base). | `reset_vector_offset: 0x100` |
| `flash_boot_alias` | `true` (default): address `0x0` mirrors flash (the STM32 boot alias). `false` for a part whose low addresses are real memory (i.MX RT ITCM at `0x0`). | `flash_boot_alias: false` |
| `atomic_register_aliases` | Register-alias flavour: `false` (default), `true` (= `rp2040`), `rp2040`, `efr32s2`. | `atomic_register_aliases: "efr32s2"` |
| `ns_alias_offset` | TrustZone non-secure alias offset; applied only to otherwise-unmapped addresses, so it cannot shadow a declared peripheral. | `ns_alias_offset: 0x10000000` |
| `memory_regions` | Extra CPU-visible windows beyond `flash`/`ram` — see [Memory-map keys](#memory-map-keys-beyond-flashram). | see below |
| `peripherals` | MMIO peripheral list; see [Peripheral entries](#peripheral-entries). | `peripherals: []` |
| `pins` | Authoritative pad→GPIO map; once declared, an unlisted pad no longer resolves (no silent label-letter fallback). | `pins: {PA5: {gpio: gpio_a, bit: 5}}` |
| `analog_pins` | Pad→ADC map (`peripheral`, `channel`); kept apart from `pins` so naming only the ADC inputs does not force a full GPIO transcription. | `analog_pins: {PA0: {peripheral: "adc1", channel: 0}}` |
| `io_voltage_v` | The supply the chip's GPIO pads run from, in volts; reference for `gpio_input_thresholds`. Absent = "not transcribed" (nothing defaults it). | `io_voltage_v: 3.3` |
| `gpio_input_thresholds` | Digital input thresholds as ratios of `io_voltage_v`; must satisfy `0 < vil < vih < 1`. | `gpio_input_thresholds: {vil: 0.35, vih: 0.65}` |
| `supply_monitor` | POR/PDR/BOR supply thresholds — see [Supply supervisor](#supply-supervisor). | see below |
| `include` | Chip YAML include(s), expanded only when the descriptor is loaded from a path. | `include: nrf52-common.yaml` |

### `cpu_writes` — the memory-region store policy

`flash:` and `ram:` are memory regions: `base`, `size`, and the optional
`cpu_writes` policy. The policy says what the bus does with a **CPU-initiated
store** into that region; it is independent per region and defaults to `allow`
(the only behaviour that existed before the key):

```yaml
# configs/chips/ci-fixture-stm32l476-readonly.yaml
flash:
  base: 0x08000000
  size: "1024KB"
  cpu_writes: fault   # allow (default) | warn | fault
ram:
  base: 0x20000000
  size: "96KB"        # omitted cpu_writes ⇒ allow
```

| Mode | A CPU store into the region… | Recorded when the store is flagged or refused |
|---|---|---|
| `allow` (default) | commits silently, exactly as before | this region records nothing |
| `warn` | commits; one deduplicated `WARN` per region | violation count + first `{addr, cycle, pc}` |
| `fault` | does not commit; raises `SimulationError::MemoryViolation(addr)` | violation count + first `{addr, cycle, pc}` |

The semantics, precisely (`crates/config/src/chip.rs` `CpuWritePolicy`,
`crates/core/src/bus/memory_policy.rs`, `crates/core/src/bus/accessors.rs`):

- **CPU stores only.** The gate sits at the top of the CPU store path for every
  access width (`write_u8`/`16`/`32`/`64`) before anything commits. Host/loader
  writes never pass through it: loading an image whose segments land inside
  `flash:`/`ram:` writes the backing memory directly (`Machine::load_firmware` →
  `LinearMemory::load_from_segment`), and a snapshot restore is the same. The
  generic DMA request path runs with the gate explicitly bypassed;
  peripheral-internal DMA models that write through `&mut dyn Bus` are a
  recorded deferral, not part of this rule yet.
- **The boot alias counts as flash.** A store through the boot alias (address
  `0x0` on a flash-at-`0x08000000` part) is gated as a flash store.
- **`fault` → precise BusFault.** The store raises
  `SimulationError::MemoryViolation(addr)`. On an ARMv7-M core with BusFault
  enabled that is a precise data BusFault with `BFAR` valid, and the CLI's
  verdict names the region when the fault address matches the first recorded
  violation in a `fault`-mode region (`crates/cli/src/fault_report.rs`):
  `data access at 0x0800_1000 (BFAR valid; store to read-only flash)`.
- **`warn` commits and records.** The store lands exactly as with `allow`; one
  WARN is logged for the first violation in the region and no more — the count
  and the first violation are still recorded in full.
- **`result.json.memory_policy` is skip-on-default.** `labwired test` writes the
  block only when at least one of the two regions is non-`allow`; an all-default
  config produces byte-identical artifacts to before the key existed. Each
  region carries `mode`, `violations`, and `first` (`addr`, `cycle`, `pc`;
  omitted when there were none).

Runnable receipt: [examples/readonly-flash/](../examples/readonly-flash/README.md)
runs a firmware store into `cpu_writes: fault` flash end to end, and
[Add a board](howto/extend/add-a-board.md) shows the flip to `warn`.

**Known caveat — images linked at the boot alias.** A segment whose address is
not inside the flash/RAM backing (an ELF linked at `0x0` on a part whose flash
sits at `0x0800_0000`) is loaded byte-by-byte through the bus
(`Machine::load_firmware` falls back to one bus store per byte), and those load
stores are subject to `cpu_writes` like any other. With `cpu_writes: fault` on
flash, such an image cannot load: the first store is refused and the segment
fails — the Cortex-M loader logs
`Failed to load segment at 0x0 - outside of memory map` and continues with a
blank flash. Link firmware at the flash base, as real parts' images are; the
[read-only-flash example](../examples/readonly-flash/README.md) is linked at
`0x0800_0000`, so its own load is direct and recorded as zero violations.

### Memory-map keys beyond `flash`/`ram`

- `reset_vector_offset` — where the real reset vector hides when a stage-2
  bootloader precedes the application. The simulator does not execute the
  stage-2 blob (flash is directly mapped); it uses this offset to find the
  vector table when the flash-base vectors are not valid. RP2040 sets `0x100`
  (the boot2 blob).
  ```yaml
  reset_vector_offset: 0x100            # configs/chips/rp2040.yaml
  ```
- `flash_boot_alias` (default `true`) — whether address `0x0` mirrors the flash
  image. Set `false` for a part whose low addresses are memory of their own;
  on the i.MX RT the ITCM sits at `0x0`, and with the alias on a store there
  would be written into flash.
  ```yaml
  flash_boot_alias: false               # no in-tree chip needs it today
  ```
- `atomic_register_aliases` — the 0x1000-strided aliases of every peripheral
  register that a family's HAL uses for read-modify-write. Two families use the
  SAME stride with DIFFERENT ops, so the value names the flavour:
  - `rp2040`: `+0x1000` XOR, `+0x2000` SET, `+0x3000` CLR (also RP2350);
  - `efr32s2`: `+0x1000` SET, `+0x2000` CLR, `+0x3000` TGL (EFR32/EFM32 Series 2).

  The historical boolean spelling still loads: `true` == `rp2040`; `false` (or
  omitting the key) means an alias address is ordinary unmapped MMIO.
  ```yaml
  atomic_register_aliases: true         # configs/chips/rp2040.yaml
  atomic_register_aliases: "efr32s2"    # configs/chips/efr32mg26.yaml
  ```
- `ns_alias_offset` — the TrustZone non-secure alias window. An MMIO access
  that maps to no peripheral but whose address plus this offset DOES map to one
  is served by that peripheral; the offset applies only as a fallback, so it
  can never shadow a declared peripheral. The nRF54L family exposes every
  peripheral at `0x5000_0000+` (secure) and exactly `0x1000_0000` below it.
  ```yaml
  ns_alias_offset: 0x10000000           # configs/chips/nrf54l15.yaml
  ```
- `memory_regions` — additional CPU-visible windows beyond `flash`/`ram`
  (ESP32-C3 IRAM `0x4037C000` and flash-DROM view `0x3C000000`, the RP2040
  bootrom). Each entry has `name`, `base`, `size`, and optionally:
  - `image_env` — an environment variable naming a raw binary loaded into the
    region at `base` (a mask-ROM dump, say). The region stays zero-filled when
    the variable is unset or the file is missing.
  - `erased` — fill the region with `0xFF` instead of `0x00`. An erased flash
    byte reads ones, so a blank settings page must read `ffffffff …` in the
    twin too.

  ```yaml
  # configs/chips/efr32mg26.yaml (excerpt)
  memory_regions:
    - name: "userdata"
      base: 0x0FE00000
      size: "1KB"
      erased: true
  ```

### Peripheral entries

Each `peripherals:` entry is a `PeripheralConfig` (`crates/config/src/chip.rs`):

| Key | Meaning | Minimal example |
|---|---|---|
| `id` | Instance name; `clock:`, `board_io:` and `external_devices[].connection` cite it. | `id: "usart1"` |
| `type` | Model selector; an unrecognised type fails the load (see below). | `type: "uart"` |
| `base_address` | MMIO window base; accepts `0x…` or an underscored integer. | `base_address: 0x40013800` |
| `size` | Optional window length. | `size: "1KB"` |
| `irq` | Vector line: `irq: 37`, or `irq: nvic@37` to name the controller in one token. | `irq: 37` |
| `irq_controller` | Controller id when not given by the `controller@line` sugar. | `irq_controller: "nvic"` |
| `clock` | RCC gate(s) that must ALL be set for the block to answer: one `{reg, bit}` or a list. `reg` is a symbolic enable-register name resolved at build time; a name the family does not expose is a hard build error. Absent ⇒ never gated. | `clock: {reg: "apb2enr", bit: 14}` |
| `config` | Instance knobs. Unknown top-level keys flatten in here (`easyDMA: true` ≡ `config: {easyDMA: true}`); nested `config:` wins on a colliding key. | `config: {profile: "stm32f1"}` |

The `clock:` mechanism (one gate or several ANDed) is documented in the
[Peripheral modeling guide](peripherals.md); declarative register blocks in
[Declarative Register Definitions](declarative_registers.md).

### Supported Peripheral Types

- `uart`, `usart`: Universal Asynchronous Receiver Transmitter
- `gpio`: General Purpose I/O
- `rcc`: Reset and Clock Control
- `timer`: Basic Timer
- `i2c`: Inter-Integrated Circuit
- `spi`: Serial Peripheral Interface
- `exti`: External Interrupt Controller
- `afio`: Alternate Function I/O
- `dma`: Direct Memory Access Controller
- `systick`: System Tick Timer
- `declarative`: Loads a generic peripheral from a YAML register description.
- `strict_ir`: Loads a register map from Strict IR JSON.
- `stub`: A zero-filled window — only for types on the explicit known-stub
  allowlist (`crates/core/src/bus/known_stubs.rs`, shrink-only, each entry with
  a written reason).

Family factories add family-specific types (`nrf52_uart`, `esp32c3_i2c`,
`imxrt_lpi2c`, …). An unrecognised type is a **load error**, never a silent
zero-filled stub: the message tells you to model it, describe it with
`type: declarative` plus a descriptor `path`, or — if answering zeros really is
right — declare `type: stub` (or add the type to the allowlist with a reason).

### Pin maps and pad voltage

| Key | Meaning | Minimal example |
|---|---|---|
| `pins` | Authoritative pad label → `{gpio, bit}` map. When a chip declares any pins, resolution goes through the map, so a pad whose label implies the wrong port still resolves correctly — and an undeclared pin fails to resolve instead of silently letter-parsing onto a port. Extra fields (e.g. `functions:`) are ignored by the simulator. | `pins: {PA5: {gpio: gpio_a, bit: 5}}` (`configs/chips/atsamd21g18a.yaml`) |
| `analog_pins` | Pad label → the ADC that samples it (`peripheral`, `channel`), transcribed from the datasheet pinout. A pad absent here means "no analog pad is modelled", not "channel 0": the co-simulation `board.analog.<pad>_volts` path refuses such a pad rather than guessing. | `analog_pins: {PA0: {peripheral: "adc1", channel: 0}}` (`configs/chips/stm32f103.yaml`) |
| `io_voltage_v` | The supply the GPIO pads run from, in volts. Absent means "not transcribed", and nothing defaults it — a board at another supply would get thresholds for the wrong rail. | `io_voltage_v: 3.3` |
| `gpio_input_thresholds` | Datasheet digital input thresholds as ratios of `io_voltage_v`: a pad reads low at or below `vil` and high at or above `vih`. | `gpio_input_thresholds: {vil: 0.35, vih: 0.65}` (`configs/chips/stm32f103.yaml`) |

### Supply supervisor

`supply_monitor:` transcribes the part's reset and power-control thresholds: the
MCU is held in reset until VDD rises to `por_rising_v` (and, with a brown-out
level active, to that level's `rising_v`); it resets again when VDD falls below
`pdr_falling_v` or the active level's `falling_v`. It is only consulted when a
board routes its VDD net into the engine (`board.power.vdd_volts`); absent means
the chip cannot be held in reset by its supply, and such a route is refused
rather than guessed.

```yaml
# configs/chips/stm32f401.yaml (excerpt)
supply_monitor:
  por_rising_v: 1.72
  pdr_falling_v: 1.68
  bor_levels:
    bor1: { falling_v: 2.19, rising_v: 2.29 }
    bor2: { falling_v: 2.50, rising_v: 2.59 }
    bor3: { falling_v: 2.83, rising_v: 2.92 }
  # bor_level: bor1    # optional: the level in effect (absent = BOR disabled)
```

### Clock, schema and includes

- **`cpu_hz` — declared, not derived.** Nothing walks RCC/PLL registers to
  compute a clock. This number is the single source of truth for every "how
  many cycles is a microsecond" question, and self-timed device models (DHT22,
  WS2812, the rotary encoder) scale their datasheet windows by it, so it must
  agree with the clock the firmware was built against or the firmware decodes
  noise. `0` means undeclared; every in-tree chip declares it
  (`every_chip_descriptor_declares_a_cpu_hz` fails the build if a new one does
  not). A board that runs the part slower overrides it with the manifest's
  `cpu_hz:` — the NUCLEO-L476RG never leaves the 4 MHz MSI reset clock and
  says so in its manifest.
  ```yaml
  cpu_hz: 80_000_000
  ```
- `schema_version` — descriptor version string, defaults to `"1.0"`. No engine
  behaviour keys off it; it is provenance.
  ```yaml
  schema_version: "1.0"
  ```
- `include` — chip YAML include(s), expanded **only** when the descriptor is
  loaded from a path (`ChipDescriptor::from_file`); built-in chips cannot
  include because there is no filesystem. Paths are relative to the including
  file, includes load first, and local keys win. `peripherals` merge by `id`,
  `memory_regions` by `name`, `pins`/`analog_pins` as maps.
  ```yaml
  include: nrf52-common.yaml       # or: include: [a.yaml, b.yaml]
  ```

## 3. System Manifest Schema

Defines the board-level environment.

```yaml
name: "BluePill Board"
chip: "../chips/stm32f103.yaml"  # Path relative to this file

# Accepted but ignored today: the engine has no `connectors:` field (see the
# note under "Every manifest key"). Console routing is `debug_uart:`; external
# buses are `external_devices:`. Nine in-tree system manifests still carry this
# block, so it is documented rather than deleted.
connectors:
  - type: "uart"
    peripheral: "usart1"
    endpoint: "host_console"  # Pipes UART output to simulator stdout
```

### Every manifest key

This is the complete `SystemManifest` surface (`crates/config/src/manifest.rs`).
`name` and `chip` are required; everything else is optional.

| Key | Meaning | Minimal example |
|---|---|---|
| `schema_version` | Defaults to `"1.0"`; environment manifests must declare `"1.0"` exactly. | `schema_version: "1.0"` |
| `name` | Board label; used in logs and to name a private lab air. | `name: "BluePill Board"` |
| `chip` | A **bare word** (`stm32f103`) is a built-in/plugin chip name; anything containing `/`, `\`, or ending `.yaml`/`.yml`/`.json` is a path resolved relative to THIS manifest, so a manifest is relocatable with the files it names. | `chip: "stm32f103"` |
| `cpu_hz` | Board-level override of the chip's declared clock, in Hz. Absent ⇒ the chip's default. | `cpu_hz: 4_000_000` (`configs/systems/nucleo-l476rg.yaml`) |
| `memory_overrides` | Parsed but **not consulted by the engine today** — region sizes come from the chip descriptor. The field exists so manifests may carry it; do not rely on it. | `memory_overrides: {flash: "512KB"}` |
| `peripherals` | Chip-peripheral patches/additions. An entry whose `id` matches a chip peripheral merges its `config` keys and overrides `base_address` (when nonzero; `0` keeps the chip's), `irq` and `size`; an unmatched `id` adds a new peripheral. `id`, `type` and `base_address` are required on every entry. | `peripherals: [{id: "usart1", type: "uart", base_address: 0, config: {profile: "stm32f1"}}]` |
| `external_devices` | The parts on the board's buses — see [External devices](#external-devices). | see below |
| `parts` | Manifest-carried part packs (private/vendor catalogs). The `labwired.part/v1` contract lives in [Part packs](part-packs.md); `- path:` is a CLI convenience that `SystemManifest::from_file` inlines, so a manifest handed straight to the browser/engine must carry the pack inline. | `parts: [{path: "./acme.yaml"}]` |
| `cosim_models` | Scalar signal-level co-simulation plants — see [Co-simulation and motor models](#co-simulation-and-motor-models). | see below |
| `motor_models` | Typed motor plants (`kind: dc` or `kind: bldc`) with SI-unit fields. | see below |
| `board_io` | Board-level pin bindings — see [Board I/O](#board-io). | see below |
| `debug_uart` | Which console the board's USB socket is wired to; see its section below. | `debug_uart: "uart0"` |
| `wifi_ap` | Opt-in per-lab virtual WiFi AP — see [WiFi AP](#wifi-ap). Absent ⇒ no AP. | see below |
| `walk_deleted` | Per-cycle peripheral-walk deletion: absent = auto-derive, `true` = force deleted, `false` = pin on. Only consulted in `event-scheduler` builds; a legacy build always walks. | `walk_deleted: true` (`examples/nokia5110-invaders-lab/system.yaml`) |

`connectors:` — nine in-tree system manifests (arduino-uno-r4-minima,
calliope-mini-v1, metro-m4, microbit-v1, microbit-v2, nano-33-iot, pybadge,
stm32f7-discovery, teensy-41) still carry a `connectors:` block that the engine
ignores; their consoles work through the defaults meanwhile. One day core may
parse it; today it is not a configuration surface.

`walk_deleted` in full (`crates/config/src/manifest.rs`): absent means the core
auto-derives walk-deletability at build time — the walk is deleted iff EVERY
peripheral on the bus is provably walk-independent for all firmware states
(conservative). `true` is a hand opt-in for a config verified byte-identical
walk-free; `false` pins the walk on.

### External devices

Each `external_devices:` entry:

| Key | Meaning |
|---|---|
| `id` | Instance name. `board_io` and stimulus channels address the device by it, and mux children name it as their `connection`. |
| `type` | The device model — see the catalog below. An unknown type is a hard load error; nothing is silently skipped. |
| `connection` | What the device hangs off: normally a controller peripheral id (`"i2c1"`, `"spi1"`, `"uart2"`), or a GPIO peripheral id for pin-timing parts. It may also name ANOTHER external device's `id` — that is how a slave is placed behind a TCA9548A bus switch. |
| `channel` | Downstream channel on a bus switch (`0..=7`; absent behind a switch means 0). Meaningless and ignored when `connection` names a controller. |
| `route` | Optional signal→pad route: `sda`/`scl`, `mosi`/`miso`/`sck`, `tx`/`rx`. Required by controllers whose pads are runtime-routed — an ESP32-C3 I²C rejects a missing route because its GPIO matrix makes the wiring runtime-configurable. |
| `config` | Per-device knobs (`i2c_address`, pin labels, seeds, …); the model's metadata lists the keys it reads. |

Resolution order — one implementation in `crates/core/src/bus/external_devices.rs`:

1.  the manifest's own `parts:` packs (most specific wins);
2.  the built-in kit registry (the catalog below);
3.  the declarative `configs/devices/*.yaml` descriptors.

Anything still unclaimed fails the build with the manifest id and type named.

The catalog is generated from the kit registry into
`crates/core/tests/fixtures/peripherals/manifest.json` (regenerate with
`cargo run -p labwired-cli --bin gen-peripherals-manifest`). One line per
canonical `type:`; the table below is rendered from that file by
`python3 scripts/ci/device_catalog_doc.py --write` and checked in CI:

<!-- device-catalog:start -->
| `type` | Bus | Model |
|---|---|---|
| `bg770a-cellular` | UART | LTE-M / NB-IoT cellular modem with the full Quectel AT command surface |
| `bt201` | UART | Dual-mode Bluetooth 5.0 audio + BLE module with a UART AT interface and BLE transparent data |
| `esp32c3_esp_at` | UART | Bounded AT peer with explicit mock connection state |
| `hc-05` | UART | Bluetooth SPP module with AT command shell over UART |
| `iolink-master` | UART | IO-Link master state machine over UART |
| `max485` | UART | Half-duplex RS-485 transceiver between a UART and a shared A/B pair |
| `neo6m-gps` | UART | GPS module streaming NMEA sentences over UART RX |
| `sim800l` | UART | SIM800L UART AT command shell (no cellular RF) |
| `xy-md02` | UART | SHT20 RS-485 temperature and humidity transmitter speaking Modbus RTU |
| `ydlidar-scanner` | UART | Spinning lidar streaming YDLIDAR scan frames over UART RX at 230400 baud |
| `ads1115` | I²C | 4-channel 16-bit I²C ADC (TI ADS1115) |
| `adxl345` | I²C | 3-axis ±2/4/8/16 g digital accelerometer over I2C |
| `aht20` | I²C | Aosong AHT20 I²C temperature and humidity sensor |
| `as5600` | I²C | 12-bit contactless magnetic rotary position sensor over I²C |
| `at24c256` | I²C | I²C serial EEPROM with a 16-bit address pointer and 64-byte pages |
| `atecc608a` | I²C | I²C secure element: P-256 sign/verify, RNG, key slots (HSM/TPM role) |
| `bh1750` | I²C | Ambient-light sensor over I²C (single-byte opcodes) |
| `bme280` | I²C | Bosch BME280 temp + humidity + pressure sensor over I2C |
| `bmi270` | I²C | Bosch BMI270 low-power IMU with hardware step counter over I2C |
| `bmp280` | I²C | Bosch BMP280 I²C pressure + temperature sensor (no humidity) |
| `bno055` | I²C | 9-DoF absolute orientation sensor (Bosch BNO055) over I²C |
| `cap1188` | I²C | Microchip CAP1188 8-channel capacitive touch controller over I2C |
| `drv2605` | I²C | TI DRV2605 haptic motor driver over I2C |
| `ds3231` | I²C | I²C real-time clock with temperature-compensated crystal |
| `fxos8700` | I²C | NXP FXOS8700 6-axis accelerometer + magnetometer over I2C |
| `ina219` | I²C | High-side current and bus voltage sensor over I²C |
| `lcd1602` | I²C | 16×2 HD44780 character LCD behind a PCF8574 I2C backpack |
| `lsm303agr_accel` | I²C | ST LSM303AGR accelerometer half over I2C (address 0x19) |
| `lsm303agr_mag` | I²C | ST LSM303AGR magnetometer half over I2C (address 0x1E) |
| `max30102` | I²C | Maxim MAX30102 pulse-oximetry / heart-rate front-end over I2C |
| `mcp9808` | I²C | ±0.25 °C I²C temperature sensor with seeded noise + thermal lag |
| `mlx90614` | I²C | Single-point IR (non-contact) thermometer over SMBus/I2C |
| `mlx90640` | I²C | Melexis MLX90640 32×24 far-IR thermal array over I2C |
| `mma8451q` | I²C | 3-axis 14-bit accelerometer over I2C with standby/active gating |
| `mpu6050` | I²C | 6-axis gyro + accelerometer over I2C |
| `nau88l21` | I²C | Nuvoton NAU88L21 stereo audio codec: I2C control port and register file |
| `oled-sh1107` | I²C | 128×128 monochrome OLED display over I2C with a paged framebuffer |
| `oled-ssd1306` | I²C | 128×64 monochrome OLED display over I2C with a paged framebuffer |
| `oled-ssd1306-128x32` | I²C | 0.91″ 128×32 monochrome OLED display over I2C with a paged framebuffer |
| `ov3660` | I²C | OmniVision 3 MP camera: SCCB control at 0x3C plus 8-bit DVP frames into the ESP32-S3 LCD_CAM |
| `pca9685` | I²C | 16-channel 12-bit I²C PWM / servo controller |
| `pcf8574` | I²C | NXP PCF8574 8-bit I²C quasi-bidirectional port expander |
| `pn532` | I²C | NXP PN532 I²C NFC controller probe shell (no RF) |
| `scd41` | I²C | Photoacoustic CO₂ + temperature + humidity sensor over I2C |
| `sgp41` | I²C | MOx gas sensor returning raw VOC/NOx signals over I2C |
| `sht30` | I²C | Sensirion SHT30 I²C temperature and humidity sensor |
| `sht31` | I²C | Temperature + humidity sensor over I²C (Sensirion CRC-8) |
| `sps30` | I²C | Laser particulate-matter sensor (PM1/2.5/4/10) over I2C |
| `tmp102` | I²C | 12-bit I²C temperature sensor (self-drifting demo) |
| `tmp117` | I²C | TI TMP117 high-accuracy digital temperature sensor over I2C |
| `vcnl4010` | I²C | Proximity + ambient-light sensor over I2C (fixed address 0x13) |
| `veml7700` | I²C | Ambient-light sensor (lux) over I2C |
| `vl53l0x` | I²C | Laser time-of-flight distance sensor over I²C (ST VL53L0X) |
| `vl53l1x` | I²C | Laser time-of-flight distance sensor over I2C |
| `74hc595` | SPI | 8-bit serial-in / parallel-out shift register over SPI |
| `adxl345_spi` | SPI | 3-axis ±16 g accelerometer over SPI (full-resolution, 256 LSB/g) |
| `amoled-rm67162` | SPI | 240x536 RGB565 AMOLED, 4-wire SPI (DBI Type-C) |
| `apa102` | SPI | SPI-clocked addressable RGB LED strip (write-only sink) |
| `hc595-7seg` | SPI | 4-digit 7-segment LED display driven by two chained 74HC595 shift registers over SPI |
| `ili9341` | SPI | 240×320 RGB565 SPI TFT display |
| `inmp441` | SPI | Omnidirectional MEMS microphone, 24-bit I2S digital output |
| `led-matrix` | SPI | 8×8 LED matrix driven by a MAX7219 serial display driver over SPI |
| `lora-sx1278` | SPI | Semtech SX1278 SPI LoRa register shell (no RF) |
| `max31855` | SPI | K-type thermocouple-to-digital converter over SPI (read-only 32-bit frame) |
| `mcp2515` | SPI | Functional SPI classical CAN controller for standard 11-bit data frames |
| `microsd` | SPI | SPI SD card command shell (init + single-block read zeros) |
| `nrf24l01` | SPI | 2.4 GHz SPI transceiver register shell (no air link) |
| `pcd8544` | SPI | Nokia 5110 (PCD8544) monochrome 84×48 LCD over SPI |
| `rc522` | SPI | MFRC522 SPI RFID reader register shell (no RF) |
| `sn74hc165` | SPI | 8-bit parallel-in / serial-out shift register over SPI |
| `ssd1680_tricolor_290` | SPI | 2.9" tri-color (black/white/red) e-paper over SPI. Driver class GxEPD2_290_C90c -- the WeAct / Waveshare 2.9" B/W/R module. Not interchangeable with the UC8151D part |
| `st7789-170x320` | SPI | 1.9in 170x320 IPS TFT on a Sitronix ST7789V, 4-wire SPI |
| `uc8151d_tricolor_290` | SPI | 2.9" tri-color (black/white/red) e-paper over SPI. Driver class GxEPD2_290_Z13c. Different command set from the SSD1680 part -- pick by the driver class your firmware instantiates, not by how the panel looks |
| `74cbtlv3257` | GPIO | Four 1-of-2 bus switches; S picks the A or B side, OE opens all four |
| `74hc00` | GPIO | Four independent 2-input NAND gates, 9 ns propagation delay |
| `74hc04` | GPIO | Six independent inverting buffers, 7 ns propagation delay |
| `74hc08` | GPIO | Four independent 2-input AND gates, 8 ns propagation delay |
| `74hc32` | GPIO | Four independent 2-input OR gates, 9 ns propagation delay |
| `74hc125` | GPIO | Four non-inverting buffers, each with its own active-low output enable |
| `74hc245` | GPIO | Eight bidirectional bits; DIR picks the direction, OE isolates both buses |
| `74hc4051` | GPIO | Routes one of eight analog inputs (Y0..Y7) to an ADC channel, selected by three GPIO pins |
| `74lvc1t45` | GPIO | Single-bit bidirectional translator; DIR picks the direction, OE isolates |
| `a4988` | GPIO | A4988/DRV8825/TMC2209-class STEP/DIR twin (step count + angle) |
| `bldc-motor` | GPIO | Configurable deterministic three-phase BLDC motor plant |
| `bldc_hall_driver` | GPIO | PWM/DIR/ENABLE in; 6-step Hall pattern out. Duty→RPM not modelled |
| `dc-motor` | GPIO | Configurable deterministic brushed DC motor plant |
| `dfplayer_mini` | GPIO | MP3 player module: serial commands in, BUSY low while playing |
| `dht11` | GPIO | One-wire temperature + humidity sensor |
| `dht22` | GPIO | One-wire temperature + humidity sensor |
| `hc-sr04` | GPIO | Ultrasonic distance sensor (trigger / echo) |
| `hx711` | GPIO | 24-bit load-cell amplifier (DT/SCK bit-bang) |
| `ili9341-16bit` | GPIO | 240×320 RGB565 TFT over Intel 8080 16-bit GPIO bit-bang |
| `keypad` | GPIO | 4×4 membrane keypad (row-drive / column-read) |
| `l298n` | GPIO | L298N/TB6612/L293D-class dual H-bridge twin (direction + enable effort) |
| `led-matrix-mux` | GPIO | Row/column LED matrix (micro:bit 5x5, bare 8x8) multiplexed by GPIO row and column lines, with per-pixel brightness |
| `logic_level_shifter_4ch` | GPIO | BSS138-class 4-channel bidirectional shifter; DIR is a sim strap (not on the PCB) |
| `neopixel` | GPIO | Addressable RGB LED strip on one data pin (edge-timed bit stream) |
| `pybadge_buttons` | GPIO | Eight active-low buttons through the board's SN74HC165 |
| `rotary_encoder` | GPIO | Incremental quadrature knob (EC11) |
| `segment-display-mux` | GPIO | N-digit 7/14/16-segment LED display, multiplexed by GPIO segment and digit-select lines |
| `servo` | GPIO | RC hobby servo driven by a single PWM control pin (GPIO edges and/or LEDC duty) |
| `seven-segment` | GPIO | Bare single-digit 7-segment LED display driven straight from nine GPIO pins |
| `tm1637-7seg` | GPIO | 4-digit 7-segment LED display with the TM1637 2-wire (CLK/DIO) driver |
| `uln2003` | GPIO | Four-phase unipolar stepper twin (IN1..IN4) |
| `gp2y0a21` | Analog | Sharp GP2Y0A21YK0F analogue infrared distance sensor, 10–80 cm (Vo on ADC) |
| `ldr` | Analog | CdS photoresistor + voltage divider on an ADC channel |
| `lipo_charger` | Analog | Single-cell LiPo pack + TP4056-class analog charger on an ADC channel |
| `mq-6` | Analog | Hanwei MQ-6 LPG/propane semiconductor gas sensor module (AOUT on ADC) |
| `ntc-thermistor` | Analog | 10 kΩ NTC + voltage divider on an ADC channel |
| `potentiometer` | Analog | 3-pin linear potentiometer on an ADC channel (rotary or slide) |
| `soil-moisture` | Analog | Capacitive soil moisture probe module (AOUT on ADC; wetter → lower voltage) |
| `tmp36` | Analog | Analog temperature sensor, 10 mV/°C with 500 mV at 0 °C |
| `xl4015` | Analog | Adjustable XL4015 buck; Vout setpoint observed through a ÷11 ADC divider |
| `can-bridge` | CAN | Boundary between a CAN controller and traffic from outside the simulation |
| `can-diagnostic-tester` | CAN | One-shot single-frame UDS-style request injector on bxCAN/FDCAN |
| `can-player` | CAN | Replays candump-format traffic into a bxCAN/FDCAN controller |
| `uds-tester` | CAN | Stateful multi-frame UDS tester (SecurityAccess-class handshakes) |
<!-- device-catalog:end -->

Legacy and alternate spellings resolve to the same canonical models:
`sn74cbtlv3257` = `74cbtlv3257`; `74ls00`/`74hct00` = `74hc00`;
`74ls04`/`74hct04` = `74hc04`; `74ls08`/`74hct08` = `74hc08`;
`74ls125`/`74lvth125`/`sn74lvth125` = `74hc125`;
`74ls245`/`74lvc245`/`sn74lvc245a` = `74hc245`; `74ls32`/`74hct32` = `74hc32`;
`cd4051`/`cd74hc4051` = `74hc4051`; `hc595` = `74hc595`;
`sn74lvc1t45` = `74lvc1t45`; `drv8825`/`tmc2209` = `a4988`; `dc_motor` =
`dc-motor`; `bldc_motor` = `bldc-motor`; `bldc-hall-driver` =
`bldc_hall_driver`; `uds-diagnostic-tester` = `can-diagnostic-tester`; `am2302`
= `dht22`; `drv2605l` = `drv2605`; `hc05` = `hc-05`; `hcsr04` = `hc-sr04`;
`hc595_7seg` = `hc595-7seg`; `ili9341_16bit` = `ili9341-16bit`;
`bts7960`/`ibt-2`/`ibt2`/`l293d`/`tb6612` = `l298n`; `max7219` = `led-matrix`;
`led_matrix_mux` = `led-matrix-mux`; `logic-level-shifter-4ch` =
`logic_level_shifter_4ch`; `ws2812` = `neopixel`; `slide-potentiometer` =
`potentiometer`; `rotary-encoder` = `rotary_encoder`; `segment_display_mux` =
`segment-display-mux`; `sg90`/`mg996r` = `servo`; `seven_segment` =
`seven-segment`; `epd-2in9-tricolor`/`gxepd2_290_c90c` =
`ssd1680_tricolor_290`; `tm1637_7seg` = `tm1637-7seg`; `epd-2in9-uc8151d` =
`uc8151d_tricolor_290`; `stepper-28byj48` = `uln2003`.

Two I²C types are attached by the I²C factory rather than the kit registry and
so are absent from the generated catalog: `tca9548a` (aliases `pca9548a`,
`tca9548`) — the 8-channel bus switch whose `channel:` places children behind
it; and `shm_i2c` — a shared-memory I²C bridge for host-injected traffic.

Beyond the catalog and those two, any `type:` is valid if the manifest's own
`parts:` defines it — see [Part packs](part-packs.md) for the
`labwired.part/v1` contract and the `behavior:` primitive reference.

### Device descriptors (external parts as data)

Every model in the catalog is a device descriptor: the in-tree ones live in
`configs/devices/*.yaml`, and a manifest can carry its own in `parts:`. The
top-level keys (`crates/config/src/peripherals.rs` `DeviceDescriptor`):

| Key | Meaning |
|---|---|
| `type` | The `type:` string the manifest uses — a descriptor's identity. |
| `behavior` | Which runtime primitive backs the part and how its pins/registers are declared. |
| `emit` | How the canvas compiler emits this device into a manifest (used by the browser compiler; optional). |
| `metadata` | Display labels, accepted `config:` keys, and the stimulus channels the part serves — load-bearing: the same channel definitions back the device's `SimInput` impl. |
| `schema` | Contract version — required of a manifest-carried pack (`labwired.part/v1`). |
| `source` | Provenance (which catalog/vendor shipped the pack), so errors can name where a model came from. |
| `overrides` | The built-in `type` this pack deliberately replaces; shadowing a built-in is otherwise a hard error. |
| `catalog` | App-layer catalog record (opaque to the simulator). |

`behavior.primitive` selects the interpreter (one line each):

| Primitive | What the engine interprets |
|---|---|
| `i2c_device` | Register-pointer I²C part (the `i2c:` block). |
| `spi_device` | SPI register/frame part (the `spi:` block). |
| `display` | Framebuffer panel: geometry, pixel format, RAM layout, command/data framing. |
| `led_strip` | Addressable LED strip: wire protocol plus the artifact it publishes. |
| `analog_source` | ADC-channel plant: the datasheet's output curve. |
| `uart_device` | Byte-stream part: frame shape, command table, unsolicited output. |
| `uart_transceiver` | A part that gates a UART (driver/receiver enable pads). |
| `gpio_device` | Pin-timing part driven by rules and finite edge schedules. |
| `logic_gate` | 74-series truth table, enables, direction/select, propagation delay. |
| `segment_display` | Multiplexed 7/14/16-segment display. |
| `led_matrix` | Row/column multiplexed LED matrix. |
| `analog_mux` | Analog multiplexer: select pins, channel count, enable. |
| `dc_motor` / `bldc_motor` | Motor plants — converted to the typed plants at the loader boundary. |

The wire-level detail (register encoding keys, schedules, artifacts, rules)
lives in [Part packs](part-packs.md); declarative register blocks for *chip*
peripherals live in [Declarative Register Definitions](declarative_registers.md).
Worked examples: `configs/devices/adxl345.yaml` (I²C), `configs/devices/dht22.yaml`
(one-wire GPIO), and the howtos under Extend LabWired.

### Co-simulation and motor models

`cosim_models:` declares scalar signal-level plants stepped in lockstep with the
machine: `id`, `adapter` (`external_process`, `fmi`, `mock`, `analog`),
`model` (required for `external_process` and `fmi`), `step_ns` (default
`1000`), `inputs`/`outputs` (signal name → `board.*` route) and free-form
`config`. The `analog` adapter needs `config.netlist` (a path) or
`config.netlist_text` (inline) plus `config.probes`. Deep reference:
[External-process co-simulation](cosim-external-process.md).

```yaml
# examples/cosim-spice-rc/system.yaml (excerpt)
cosim_models:
  - id: "rc_lowpass"
    adapter: "external_process"
    model: "./models/rc_lowpass.py"
    step_ns: 100000            # 100 us; RC tau is 1 ms
    inputs:
      gpio: "board.gpio.pa5"
    outputs:
      v_out: "board.analog.pa0_volts"
    config:
      protocol: "jsonl"
      engine: "ngspice"
      netlist: "./rc.cir"
```

`motor_models:` declares typed deterministic motor plants —
`kind: dc` (brushed) or `kind: bldc` — with SI-unit fields (resistance,
inductance, torque/back-EMF constants, inertia, friction, supply, encoder CPR)
and pad bindings; see
[examples/nucleo-l476rg-bldc/system.yaml](../examples/nucleo-l476rg-bldc/system.yaml)
for the full BLDC form. A motor can equivalently be declared as an
`external_devices` entry with `type: dc-motor` or `bldc-motor`: the loader
converts it to the same typed plant (`MotorModelConfig::from_external_device`),
so the two spellings cannot drift.

```yaml
# examples/f401-hbridge-dc-motor/system.yaml (excerpt)
external_devices:
  - id: motor
    type: dc-motor
    connection: gpio
    config:
      resistance_ohm: 1.2
      inductance_h: 0.002
      torque_constant_nm_per_a: 0.08
      back_emf_constant_v_per_rad_s: 0.08
      rotor_inertia_kg_m2: 0.00004
      viscous_friction_nm_per_rad_s: 0.00001
      supply_voltage_v: 12.0
      load_torque_nm: 0.0
      encoder_cpr: 1024
      simulation_clock_hz: 84000000
      in1_pin: PA0
      in2_pin: PA1
      both_inputs_low: brake
      pwm_pin: PA6
      timer_name: tim3
      timer_channel: 1
```

### Board I/O

`board_io:` is the board-level pin vocabulary: an LED soldered to a pad, a
button a user can press, an ADC input. Entries are `BoardIoBinding`:

| Key | Meaning |
|---|---|
| `id` | Binding name; also the stimulus channel for a drivable contact. |
| `kind` | `led`, `button`, `adc_input`, `pwm_output`, `i2c_device`, `spi_device`, `uart_device`. |
| `peripheral` | The peripheral `id` (from the chip or a manifest patch) that owns the pin. |
| `pin` | Bit within that peripheral's port / ADC channel number. |
| `signal` | `output` (default) or `input`. A `kind: button` binding with `signal: input` is additionally materialised as a drivable button stimulus; an output-kind button is not a contact the firmware samples. |
| `active_high` | Polarity, default `true`. |
| `i2c_address` | For `kind: i2c_device` bindings. |
| `device_type` | The external-device `type` this binding surfaces, for the browser. |
| `channel` | Stimulus channel this contact exposes when it is not a plain `pressed` button (`obstacle`, `field`, `vibration`, …); absent means `pressed`. |

```yaml
# configs/systems/nucleo-l476rg.yaml (excerpt)
board_io:
  - id: "led_ld2"
    kind: "led"
    peripheral: "gpioa"
    pin: 5
    signal: "output"
    active_high: true
  - id: "button_b1"
    kind: "button"
    peripheral: "gpioc"
    pin: 13
    signal: "input"
    active_high: false
```

### WiFi AP

`wifi_ap:` is the opt-in virtual access point, emitted when a lab diagram
contains a `wifi-ap` component. Absent ⇒ no AP, and WiFi MACs stay
unassociated (the honest "no AP present"). No in-tree system manifest uses the
key yet.

```yaml
wifi_ap:
  ssid: "labwired-ap"           # defaults shown
  ip: "192.168.4.1"             # its /24 is the DHCP pool
  serves: "labwired-stats"      # or "none"
  password: ""                  # empty/absent = open AP; a WPA handshake is not modelled yet
```

### ST7789 physical glass window

For `st7789-170x320` external devices, the optional config keys `col_offset`,
`row_offset`, `cols`, and `rows` describe the glass in physical 240×320
frame-memory coordinates. Supply all four together. For a module exposing
columns 35–204 and all 320 rows, use `col_offset: 35`, `row_offset: 0`,
`cols: 170`, and `rows: 320`; the offsets are module integration values.

The configured framebuffer and artifact dimensions remain 170×320 regardless
of MADCTL rotation or mirroring. MADCTL controls how firmware writes map into
physical memory, while the glass stays fixed. Without a configured window,
the framebuffer uses firmware coordinates and reports 240×320 or 320×240
according to MADCTL's row/column exchange bit.

### `debug_uart` — which console the board's USB socket is wired to

```yaml
debug_uart: "usb_serial_jtag"   # ESP32-C3 SuperMini, ESP32-S3 Zero
debug_uart: "uart0"             # CP210x/CH34x bridge boards
debug_uart: "uart1"             # a board that routes its console elsewhere
```

An ESP32-C3 or -S3 has two consoles in silicon: UART0 and the chip's own
USB-Serial-JTAG block. Which one carries `Serial` to the developer's cable is
neither a chip fact nor a firmware fact — it is a BOARD fact, decided by what
the USB socket is soldered to:

| board's USB socket | console        | `Serial` build flag        |
|--------------------|----------------|----------------------------|
| native USB         | USB-Serial-JTAG| `ARDUINO_USB_CDC_ON_BOOT=1`|
| USB-UART bridge IC | UART0          | `ARDUINO_USB_CDC_ON_BOOT=0`|

The build flags and this key must come from the SAME board fact. If they
disagree, firmware built for one console runs against a twin listening on the
other and the Serial pane stays empty — which is exactly what a real board does
when the flags are wrong, and exactly what shipped: a hosted build flashed to a
real ESP32-C3 SuperMini printed the ROM and bootloader banners and then nothing,
because `Serial` was on UART0 and a SuperMini's USB-C is not.

The engine taps ONE console, the declared one, because a real board gives you
one cable. It records the other and reports what the firmware said there
(`WasmSimulator::console_mismatch`) so a silent pane has a reason. A declared
console the bus does not have is refused at construction rather than silently
substituted — see `crates/core/src/console.rs`.

Omit the key to keep each path's historical default (UART0 on the ESP32-C3
merged-flash paths; every console-capable UART elsewhere).

For an `inputs.env` CI world, each `nodes[].system` value points to this
same System Manifest format used by the Playground. The environment manifest
adds node IDs, firmware paths, and explicit interconnects; it does not create a
second board-configuration dialect. v0.19 world manifests reject
`nodes[].config_overrides`: the field must be omitted, including `{}` and `null`.
Each resolved node chip must resolve to `arch: arm` and declare
`core: cortex-m*`; its firmware must be an `EM_ARM` ELF with a valid Cortex-M
Thumb reset vector at `flash.base + reset_vector_offset` (SP in RAM and
Thumb-bit reset handler in flash). See the [CI Test Runner](ci_test_runner.md)
for the closed world-interconnect schema.

A node may set `profile: arduino-esp32`, the same boot profile a test script
names in `inputs.profile`: a classic-ESP32 (Xtensa LX6) node running an
Arduino-ESP32 sketch ELF then takes the Arduino fast boot
(`install_arduino_esp32_profile`). Any other value, or the profile on any other
chip, is refused, and a world admits one such node (the profile's boot thunks
keep per-thread state). ESP32-S3 and ESP32-C3 nodes need no profile; their mask
ROMs are provisioned like the single-chip engine's (in the browser, passed as
the node's `blobs`).

World completion stays in the `inputs.env` test script, not the manifest. Set
`limits.stop_when_assertions_pass: true` to opt into a durable early completion:
all node-qualified assertions must pass at or after the optional
`stop_when_assertions_pass_min_steps` floor and remain true for the optional
`stop_when_assertions_pass_settle_steps` window (default `100000`). The default
is `false`; a same-round runtime failure or `wall_time_ms`, `max_cycles`, or
`max_uart_bytes` limit takes precedence over `assertions_passed`. `max_steps`
remains the outer execution bound, so an all-pass final world round reports
`assertions_passed`.

## 4. CLI Usage

To run a simulation, provide both the firmware and the system manifest:

```bash
labwired --firmware firmware.elf --system configs/systems/bluepill.yaml
```

The simulator loads the system manifest, resolves the chip descriptor, initializes the memory map, and begins execution at the Reset Vector.
