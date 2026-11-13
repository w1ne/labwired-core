# RFM95W

HopeRF RFM95W LoRa module (Semtech SX1276) as an SPI register shell. **No RF air link.**

!!! tip "Live catalog"
    Prefer `labwired_describe` with the catalog id over guessing pins or addresses.

## Status at a glance

| Aspect | Status |
|--------|--------|
| Catalog id | `rfm95` |
| Bus | SPI (mode 0) |
| Address | — (NSS via `cs_pin`) |
| Source | [`configs/devices/rfm95.yaml`](../../configs/devices/rfm95.yaml) (declarative, no Rust model) |
| Datasheet | Semtech *SX1276/77/78/79* rev. 4, §4.3 SPI Interface, §6.1 Table 41; HopeRF *RFM95/96/97/98(W)* V1 |
| Tier | smoke-proven (register-level tests: `crates/core/tests/everyday_sensor_set.rs`) |

## Pins / attachment

| Pin / net | Role |
|-----------|------|
| SCK / MOSI / MISO | SPI |
| NSS | `cs_pin` |
| DIO0..DIO5, RESET | Not modelled |
| 3.3V/GND | Power (digital twin) |

## Support matrix

| Behavior | Status | Notes |
|----------|--------|-------|
| Address byte: bit 7 = write, bits 6:0 = address | ✅ | |
| RegVersion 0x42 = 0x12, read-only | ✅ | The HopeRF V1 sheet prints 0x11 (earlier silicon) |
| Table 41 power-on values (RegOpMode 0x01, RegFrf 434 MHz, RegPaConfig 0x4F, …) | ✅ | FSK/OOK-mode values, as at power-on |
| Register storage (write, read back) | ✅ | |
| Burst auto-increment | ❌ | One byte per address frame |
| FSK ⇄ LoRa register banking, mode sequencing | ❌ | 0x0D–0x3F keep what they hold when LongRangeMode is set |
| RF air link, packet engine, FIFO, DIO interrupts, RSSI | ❌ | A transmit never produces a packet; TxDone never fires |

## Stimulus

None — no measurement channel.

## How to run

1. Place the part and an MCU in Playground, or add it to a system YAML:
   ```yaml
   external_devices:
     - id: "lora"
       type: "rfm95"
       connection: "spi1"   # the SPI bus id your chip system uses
       config:
         cs_pin: "PA4"
   ```
2. Agent: `labwired_list` → `labwired_describe id=rfm95` → wire → `labwired_validate` → `labwired_run` / `labwired_verify`.
3. Register-level smoke: `cargo test -p labwired-core --test everyday_sensor_set` — RegVersion, power-on RegOpMode / RegFrf / RegPaConfig / RegPaDac, and a RegOpMode write. The SPI negative control proves the check fails against a part that answers only 0x00 or only 0xFF; `rfm95_tracks_stimulus` stores a 915 MHz setup and checks RegVersion refuses a write.

## Limitations

- Use this to check that a driver probes and configures the radio. It cannot show a link working; for that, see the `lora_sx1278` shell's notes in [`docs/part-packs.md`](../part-packs.md).

See the YAML header for the full datasheet trace and every named approximation.

## Related

- [Parts index](index.md)
- [Onboard a part](../howto/onboard-part.md)
- [Fidelity](../fidelity.md)
