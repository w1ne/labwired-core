# Onboard a part (I²C, SPI, actuators)

Add an **external device** to the twin: temperature sensor, accelerometer, display, servo, motor, buzzer, and similar. Firmware uses normal bus drivers. You describe the device in data when possible.

For a new **MCU or board**, use the [Board playbook](../board_onboarding_playbook.md) instead.

---

## Before you start

1. Confirm the part is not already in the catalog: [Parts](../parts/index.md) or `labwired_list` / `labwired_describe`.
2. Have the **datasheet** (address map, SPI framing, default I²C address, pin names).
3. Know the **host board** you will attach to (must already have a working I²C/SPI/GPIO model).

---

## Path overview

```text
1. Device descriptor  →  configs/devices/<id>.yaml
2. Wire it            →  system YAML or Playground diagram
3. Smoke firmware     →  read a register or drive a pin
4. Prove it           →  labwired test  or  labwired_verify
5. Document           →  docs/parts/<id>.md (template)
```

Many sensors and SPI chips use a **declarative** descriptor (`primitive: i2c_device` or `spi_device`). Actuators often use a small plant model (`dc_motor`, `servo`, GPIO devices).

---

## Worked examples (real files in this repo)

### 1. I²C sensor — TMP102

| Item | Value |
|------|--------|
| Descriptor | [`configs/devices/tmp102.yaml`](../../configs/devices/tmp102.yaml) |
| Type id | `tmp102` |
| Default address | `0x48` |
| Primitive | `i2c_device` |
| Demo firmware | [`examples/esp32s3-i2c-tmp102/`](../../examples/esp32s3-i2c-tmp102/) |

The temperature register can self-drift for demos (see YAML comments). Another I²C temp sensor with a **temperature** SimInput channel: [`mcp9808.yaml`](../../configs/devices/mcp9808.yaml) at `0x18`.

**Minimal attach pattern** (system YAML — field names must match your chip’s system schema; copy a working system and add one device):

```yaml
# Pattern only — copy a real system under configs/systems/ or examples/*/ and edit.
external_devices:
  - id: "tmp102"
    type: "tmp102"
    connection: "i2c0"   # or the bus id your chip system uses
    config:
      i2c_address: 0x48
```

Then:

```bash
labwired test --script path/to/your-smoke.yaml
# or
labwired run --firmware build/firmware.elf --system path/to/system.yaml
```

Step-by-step narrative: [I²C sensor example](../examples/i2c_sensor_example.md).

### 2. SPI peripheral — ADXL345 (SPI)

| Item | Value |
|------|--------|
| Descriptor | [`configs/devices/adxl345_spi.yaml`](../../configs/devices/adxl345_spi.yaml) |
| Type id | `adxl345_spi` (distinct from I²C `adxl345`) |
| Primitive | `spi_device` |
| Framing | 1-byte command; R/W in bit 7 |

Copy SPI framing and register table from a similar device YAML when you add a new SPI part. Keep **limitations** in the file header (what the model does *not* do).

Related lab (I²C ADXL path on STM32): [`examples/adxl345-sensor-lab/`](../../examples/adxl345-sensor-lab/).

### 3. Actuator — DC motor

| Item | Value |
|------|--------|
| Descriptor | [`configs/devices/dc_motor.yaml`](../../configs/devices/dc_motor.yaml) |
| Type id | `dc-motor` |
| Primitive | `dc_motor` |
| Pins | PWM, direction, brake, enable, encoder… |

Wire GPIO/PWM nets in the system or diagram. Smoke: drive PWM + direction and assert encoder or plant state in a test script when available.

Simpler GPIO actuators also live under `configs/devices/` and as catalog types such as `servo` / `buzzer` in [Parts](../parts/index.md).

---

## Step-by-step (new declarative I²C device)

### 1. Create the descriptor

Add `configs/devices/my_sensor.yaml`:

```yaml
type: my_sensor

behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x3C
    pointer_mask: 0xFF
    registers:
      - { name: WHO_AM_I, addr: 0x0F, width: 1, endian: be, access: r, reset: 0x6A }

metadata:
  label: "My sensor"
  summary: "Short one-line description."
  category: i2c
```

Start from [`tmp102.yaml`](../../configs/devices/tmp102.yaml) or [`mcp9808.yaml`](../../configs/devices/mcp9808.yaml). For SPI, start from [`adxl345_spi.yaml`](../../configs/devices/adxl345_spi.yaml).

Register the type with the engine the same way existing devices are registered (see nearby devices and [part packs deep dive](../part-packs.md) only if the simple path fails).

### 2. Attach to a board

- **CLI / examples:** extend a `configs/systems/*.yaml` or `examples/*/system.yaml`.
- **Playground:** place the part and wire SDA/SCL (or SPI/CS) to the MCU.
- **Agent:** `labwired_describe` the board, then validate the diagram before run.

### 3. Smoke firmware

Firmware should:

- Init the bus at the right pins and speed
- Read a **known register** (WHO_AM_I) or drive the actuator once
- Print a line on UART or toggle a GPIO you can assert

No LabWired-specific APIs in firmware.

### 4. Prove it

```bash
labwired test --script examples/.../io-smoke.yaml
```

Or with an agent: `labwired_run` then **`labwired_verify`** with serial/GPIO/register checks. No verify → not proven ([Fidelity](../fidelity.md)).

### 5. Document

Copy [parts/_TEMPLATE.md](../parts/_TEMPLATE.md) to `docs/parts/<id>.md`, fill the matrix, add the page under **Parts** in `mkdocs.yml`.

---

## SimInput (sensors)

Some devices source values from channels such as `temperature`, `x` / `y` / `z` (accel), distance, etc. Drive them:

- In tests / stimuli where supported
- Via agent `labwired_run` **stimuli** when the part exposes SimInput

MCP9808 uses a `temperature` channel with noise and lag — see comments in [`mcp9808.yaml`](../../configs/devices/mcp9808.yaml).

---

## Agent path

```text
1) labwired_list (components) / labwired_describe id=<type>
2) labwired_validate_device with your YAML (if available on the surface)
3) Wire diagram → labwired_validate
4) labwired_run → labwired_verify
```

Do not invent pins or register maps. Prefer datasheet + descriptor.

---

## Done checklist

- [ ] Device type loads without error
- [ ] Smoke read/drive works on a known board
- [ ] `labwired test` or `labwired_verify` is green
- [ ] Limitations listed (header comment + part page matrix)
- [ ] Part page + catalog entry (when publishing)

---

## Next

| Topic | Link |
|-------|------|
| Hardware tracks | [Onboard hardware](onboard-hardware.md) |
| Board / MCU | [Board playbook](../board_onboarding_playbook.md) |
| Parts list | [Parts](../parts/index.md) |
| Packs / registry (deep) | [Part packs](../part-packs.md) |
| Declarative registers | [Declarative registers](../declarative_registers.md) |
