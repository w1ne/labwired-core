# Configuration Reference

LabWired uses a YAML-based configuration system to define the simulated hardware environment. This separation allows the same firmware binary to be tested against different hardware configurations (e.g., changing memory sizes or remapping peripherals) without recompilation.

## 1. File Hierarchy

A complete simulation requires two descriptor files:

1.  **Chip Descriptor** (`chips/<name>.yaml`): Defines the internal architecture of the SoC (Flash/RAM size, internal peripheral addresses).
2.  **System Manifest** (`systems/<name>.yaml`): Instantiates a chip and defines board-level wiring (external sensors, UART loopbacks).

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

## 3. System Manifest Schema

Defines the board-level environment.

```yaml
name: "BluePill Board"
chip: "../chips/stm32f103.yaml"  # Path relative to this file

# External Device Connections (Planned)
connectors:
  - type: "uart"
    peripheral: "usart1"
    endpoint: "host_console"  # Pipes UART output to simulator stdout
```

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
