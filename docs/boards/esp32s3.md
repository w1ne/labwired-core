# ESP32-S3

The Espressif ESP32-S3 (dual-core Xtensa LX7, 512 KB SRAM, external QSPI
flash, Wi-Fi + BLE 5, USB) is LabWired's reference Xtensa target. All
declared peripherals are **declarative** — the chip yaml references
external YAML descriptors under `configs/peripherals/esp32s3/`.

There are two chip yamls for ESP32-S3 today:

- `configs/chips/esp32s3.yaml` — minimal declarative model (UART0 + GPIO +
  TIMG0 + INTERRUPT_CORE0) used by older blinky / hello-world examples.
- `configs/chips/esp32s3-zero.yaml` — fuller "Plan 2/4" model used by
  the [`esp32s3-i2c-tmp102`](../../examples/esp32s3-i2c-tmp102/) demo;
  adds I²C0, SYSTIMER, USB-Serial-JTAG, RTC_CNTL stub, eFuse stub,
  IRAM + ROM-thunks + flash-XIP mapping. **Use this variant for new
  ESP32-S3 work** — the simulator's `configure_xtensa_esp32s3` is the
  authoritative wiring code that this yaml documents.

## Status at a glance


> **Live status:** the table below is a hand-maintained snapshot. For the authoritative, auto-generated view see the [chip conformance scoreboard](../coverage/chip-conformance.md) (level · modelled peripherals · register-match vs silicon) and the [tier-1 matrix](../coverage/tier1-scoreboard.md) (per-peripheral pass/fail).

| Aspect              | Status                                                                          |
|---------------------|---------------------------------------------------------------------------------|
| Chip yamls          | [`esp32s3.yaml`](../../configs/chips/esp32s3.yaml) (minimal) · [`esp32s3-zero.yaml`](../../configs/chips/esp32s3-zero.yaml) (Plan 2/4) |
| System yaml         | [`configs/systems/esp32s3-zero.yaml`](../../configs/systems/esp32s3-zero.yaml)  |
| Reference firmware  | [`examples/esp32s3-i2c-tmp102/`](../../examples/esp32s3-i2c-tmp102/) (Plan 4, canonical) · `examples/esp32s3-blinky/` · `examples/esp32s3-hello-world/` |
| Validation          | Plan 4 end-to-end test: `crates/core/tests/e2e_i2c_tmp102.rs`                   |
| Tier                | structural — declarative + Plan 2/4 peripheral models                           |

## Peripherals (esp32s3-zero, Plan 2/4 model)

| Peripheral       | Base       | Status              | Notes                                                                          |
|------------------|------------|---------------------|--------------------------------------------------------------------------------|
| Xtensa LX7       | —          | ✅ modeled (dual)   | PRO_CPU + APP_CPU round-robin (`Machine::with_secondary_cpu`); FPU coprocessor not modeled |
| IRAM             | 0x40370000 | ✅ ram region       | 512 KiB                                                                        |
| ROM thunks       | 0x40000000 | ✅ thunk bank       | 384 KiB internal ROM call sites                                                |
| Flash (I-cache)  | 0x42000000 | ✅ XIP mapping      | 4 MiB                                                                          |
| Flash (D-cache)  | 0x3C000000 | ✅ XIP mapping      | 4 MiB                                                                          |
| USB Serial/JTAG  | 0x60038000 | ✅ modeled          | 4 KiB — used as console in Plan 4 demo                                         |
| SYSTIMER         | 0x60023000 | ✅ modeled          | 4 KiB — periodic alarm in Plan 4 demo                                          |
| SYSTEM           | 0x600C0000 | ⚙ stub             | 4 KiB                                                                          |
| RTC_CNTL         | 0x60008000 | ⚙ stub             | 4 KiB                                                                          |
| eFuse            | 0x60007000 | ⚙ stub             | 4 KiB                                                                          |
| I²C0             | 0x60013000 | ✅ modeled          | 4 KiB — round-trip exercised in [`esp32s3-i2c-tmp102`](../../examples/esp32s3-i2c-tmp102/) |
| GDMA             | 0x6003F000 | ✅ modeled          | 5 channels; M2M descriptor walks **and** peripheral-coupled byte movement (see below) |
| SPI2 / SPI3      | 0x60024000 / 0x60025000 | ✅ modeled | GP-SPI master: CPU W-buffer path + GDMA-coupled DMA transactions; device attach surface (e-paper e2e) |
| I²S0 / I²S1      | 0x6000F000 / 0x6002D000 | ✅ modeled | Config/control twin + GDMA-coupled sample streaming (`RXEOF_NUM` byte semantics) |
| LCD_CAM          | 0x60041000 | ✅ modeled (LCD TX + CAM RX) | Config/control twin + i80 pixel streaming: `LCD_CMD_VAL` command phase and the GDMA outlink payload drive an attached parallel panel. CAM slave: an attached DVP sensor (`ov3660`) streams frames into the GDMA inlink with the VSYNC / `IN_SUC_EOF` sequence `esp32-camera` counts (see [Camera](#camera-dvp)). RGB-mode sync generation is not modelled |
| UART0/1/2        | 0x60000000 / 0x60010000 / 0x6002E000 | ✅ modeled | FIFO + STATUS + baud-paced drain; UART0 doubles as the UHCI0 DMA endpoint |

The authoritative wiring (`configure_xtensa_esp32s3`) additionally registers
LEDC, RMT, PCNT, TIMG0/1, SAR-ADC, TWAI, AES, SHA, RSA, HMAC, DS, MCPWM0/1,
SD/MMC, LCD_CAM and USB-OTG models — see
`crates/core/src/system/xtensa.rs` for the full list and fidelity notes.

The plain `esp32s3.yaml` (used by older examples) is a strict subset:
UART0 @ 0x60000000 + the declarative GPIO/TIMG0/INTERRUPT_CORE0 peripherals
under [`configs/peripherals/esp32s3/`](../../configs/peripherals/esp32s3/).
See [`docs/declarative_registers.md`](../declarative_registers.md) for
how declarative peripheral descriptors work.

## DMA (GDMA)

The GDMA model (`crates/core/src/peripherals/esp32s3/gdma.rs`) moves real
bytes:

- **Memory-to-memory** (`MEM_TRANS_EN`): full descriptor-chain walks with
  owner-bit writeback.
- **Peripheral-coupled mode** (routed by `IN_PERI_SEL` / `OUT_PERI_SEL`):
  UART (UHCI0), SPI2, SPI3, I²S0 and I²S1 transfer real bytes between
  descriptor chains and the peripheral models — incremental 64-byte-per-tick
  pumps, per-descriptor owner/length writeback, per-direction EOF
  interrupts (sources 66–75).
- **LCD_CAM OUT** (`OUT_PERI_SEL = 5`): pixel streaming for the `esp_lcd`
  i80 driver. The outlink chain feeds the LCD_CAM i80 master, which drives
  an attached 8080 parallel panel (`ili9341-16bit`) with the command word
  from `LCD_CMD_VAL` and the DMA payload as data. The walk honours
  `suc_eof`, and `LCD_TRANS_DONE` is withheld until the chain drains so the
  driver cannot remount descriptors mid-transfer.
- **LCD_CAM IN** (`IN_PERI_SEL = 5`): camera receive. The CAM slave hands
  the attached sensor's bytes to the inlink as they arrive at the PCLK rate;
  `IN_SUC_EOF` latches every `CAM_REC_DATA_BYTELEN + 1` bytes (or at VSYNC
  with `CAM_VS_EOF_EN`), closing the current descriptor with its length and
  `suc_eof`. `IN_CHECK_OWNER` is honoured, so the driver's circular ring
  keeps filling on its second lap, and `IN_LINK.STOP` ends the receive.
- Other peripheral ids (AES, SHA, ADC, RMT, unknown/unbound) keep an
  auto-complete fallback: EOF latches without byte movement so polling
  firmware progresses.

## Camera (DVP)

An `ov3660` part (`configs` type `ov3660`, kit in
`crates/core/src/peripherals/components/ov3660.rs`) puts a camera on the
board: its SCCB port answers at 0x3C on the I²C controller named by
`connection:` (`esp32-camera` uses `i2c1`), and its DVP output is bound to
LCD_CAM's CAM slave.

```yaml
external_devices:
  - id: "cam"
    type: "ov3660"
    connection: "i2c1"
```

A camera named on `i2c0` with a pad `route:` (what the Studio canvas emits)
works too. Any S3 I²C controller can drive any pad through the GPIO matrix,
so the camera also answers the other controller, exactly while the firmware
routes that controller's SCL/SDA output signals (`FUNCn_OUT_SEL_CFG`, I2CEXT1
= 91/92) to the camera's pads. `esp32-camera` opens I2C1 and routes it to
SIOC/SIOD, so it finds the camera however the lab names the bus:

```yaml
    connection: "i2c0"
    route: { sda: "GPIO4", scl: "GPIO5" }
```

Unmodified `esp32-camera` firmware runs against it: probe and chip ID
(0x3660), the sensor init table, `set_framesize` / `set_pixformat`, then
`esp_camera_fb_get()` returning frames in internal RAM or PSRAM. Frames
follow the window (0x3808–0x380B) and format (0x4300: RGB565, YUV422, Y8)
the driver programs, or are baseline JPEG (below). Checked with Espressif's
`take_picture` example (RGB565 and JPEG, QVGA, one PSRAM frame buffer) and a
two-buffer `CAMERA_GRAB_LATEST` preview to an ST7789.

The picture is synthetic and driven by stimulus: `pattern` (0 colour bars,
1 checkerboard, 2 gradient, 3 flat), `brightness` (0–100 %) and `motion`
(pixels the scene scrolls per frame). The sensor's own colour-bar bit
(0x503D[7]) overrides the stimulus, as on silicon.

JPEG (`0x3821[5]`): each frame is a real baseline JPEG of the scene, YCbCr
4:2:2 at the programmed window, quantised by `COMPRESSION_CTRL07` (0x4407,
the QS `set_quality` writes). It is sized for the driver: `esp32-camera`
reports a PSRAM JPEG frame as (EOFs + 1) × 1 KiB and looks for `FF D9` only in
that last block, so the stream is zero-padded to one byte short of the next
1 KiB boundary, and the one length that could never be found (an exact
multiple of 1 KiB) gets a 4-byte COM segment. `esp_camera_fb_get()` returns
the trimmed JPEG; decoders accept it. Not the OV3660's own encoder, so sizes
are representative, not bit-exact.

The camera pins in the example are Espressif's `CAMERA_MODEL_ESP32S3_EYE`
map (arduino-esp32 `camera_pins.h`), the one ESP32-S3 N16R8 camera boards
are documented against.

Timing: one byte per 20 MHz PCLK, a 200 µs back porch and 1 ms of
blanking. Two guards keep the driver's event count intact when the
simulator fast-forwards idle time: VSYNC waits until a captured frame has
been handed to GDMA, and GDMA moves no bytes past an `IN_SUC_EOF` the CPU
has not acknowledged.

## RTC IO MUX

`rtc_io` (0x6000_8400) models the RTC IO MUX and RTC GPIO: GPIO0..21 =
RTC_GPIO0..21 at bit `10 + n`. A pad whose `MUX_SEL` is set is driven by
`RTC_GPIO_OUT` / `RTC_GPIO_ENABLE` (open drain with `RTC_GPIO_PINn.PAD_DRIVER`,
`RUE` / `RDE` pulls) instead of `GPIO_OUT` / `GPIO_ENABLE` and the GPIO
matrix, and `RTC_GPIO_IN` / `GPIO_IN` read it through its `FUN_IE`;
`RTC_GPIO_PINn.INT_TYPE` latches `RTC_GPIO_STATUS`, which reaches no CPU
interrupt (none on silicon either). The pad holds (`RTC_CNTL_PAD_HOLD_REG`)
are not modelled. See [GPIO nets](../howto/gpio-nets.md#rtc-io-esp32-and-esp32-s3).

## Not yet modeled (commonly expected on ESP32-S3)

Still absent from the model: **WIFI / BT controller**,
**XTS_AES**, **TEMP_SENSOR**, **APB_CTRL**, **PSRAM cache controller**,
**FPU coprocessor**.

Firmware that touches unmapped registers will hit
`MemoryAccessViolation` or stall in a polling loop. See
[`docs/getting_started_firmware.md`](../getting_started_firmware.md).
