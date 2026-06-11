# ESP32-C3 Radio (WiFi/BT) — Register-Map Reverse Engineering

The ESP32-C3 WiFi/BT MAC and RF front-end registers are **not published** by
Espressif: they are absent from the TRM and from the SVD
(`tests/fixtures/real_world/esp32c3.svd`). LabWired therefore could not onboard
the radio the way every other block was onboarded (wire an SVD descriptor,
validate reset values). This document records how the radio register surface was
**reverse-engineered from live silicon**, and the resulting memory map — the
foundation for a functional radio model.

## Method — dynamic trace off the live chip

Rather than disassemble the opaque `libphy`/`libpp` blobs, the register surface
was recovered by watching what the **real driver** does to the silicon:

1. A minimal probe app (`scripts/hw-oracle/wifi-re/wifi_probe.c`) brings the
   WiFi driver up — `esp_wifi_init()` → `esp_wifi_set_mode(STA)` →
   `esp_wifi_start()` — with no scan/connect, so the trace is just PHY + MAC
   bring-up. Four no-op anchor functions (`probe_before_init`,
   `probe_after_init`, `probe_after_start`, `probe_idle`) bracket the phases.
2. `scripts/hw-oracle/wifi-re/trace_radio.sh` flashes the build, sets hardware
   breakpoints on the anchors over the built-in USB-JTAG (openocd-esp32), and
   dumps every candidate radio window at each phase.
3. Diffing the phases yields exactly which registers the driver configures, and
   in which bring-up phase — recovering the map without any vendor docs.

Captured against the live C3 (rev v0.4, MAC `38:44:be:42:f5:58`); raw dumps in
`scripts/hw-oracle/captures/esp32c3/wifi-radio-<ts>/`.

## Discovered radio memory map

All windows reset to `0x0` (radio is powered down until `phy_enable`), except
the analog-master block which carries hardware defaults.

| Base | Block | Regs configured | Role |
|------|-------|-----------------|------|
| `0x60006000` | **FE** — RF front-end | 9 | TX gain / attenuation tables (e.g. `0xc4c0c0c0`, `0xf4e8dcd0`) |
| `0x6001cc00` | **NRX** — receiver | 33 | RX chain / AGC config |
| `0x6001d000` | **BB** — baseband | 33 | baseband / modem config |
| `0x60033000` | **WiFi MAC (WDEV)** | 29 | addr/BSSID filters (`0x42be4438…`), RX masks (`0xffffffff`) |
| `0x60034000` | **WiFi MAC (cont.)** | 11 | descriptor / buffer base pointers (`0x00400000`) |
| `0x60035000` | **analog master / MAC** | 6 | RF synth control; 12 non-zero HW reset defaults |
| `0x60042000` | clock | 1 | radio clock enable |

**Surface size: ~122 registers across 7 windows** (peaks at 164 non-zero during
`phy_enable`, settles to 130). Bounded and modelable — not thousands.

The `0x60033000` base is the headline find: the **C3 WiFi MAC**, which appears
in no SVD or TRM. `bb` (`0x6001d000`) was already wired from a stub descriptor
in #236; FE/NRX/MAC are newly located here.

## Model plan

Two layers, by register character:

1. **PHY/FE/BB/NRX config (~76 regs) — register-backed.** These are static
   calibration/gain tables `libphy` blasts in. A `declarative`
   (`GenericPeripheral`) block per window — accepts writes, reads back, reset
   value `0x0` — is faithful and lets the firmware's config writes succeed
   instead of faulting on an unmapped window.
2. **WiFi MAC (`0x60033000`–`0x60035000`, ~46 regs) — behavioral.** Descriptor
   rings, TX/RX, and IRQ need real semantics, bridged to **SimNet** for the data
   path. This requires one more RE pass: trace the driver's **status-poll**
   reads (the bits it busy-waits on during `esp_wifi_start`) so the model can
   flip them and let bring-up complete in sim. The phase trace here captured
   the *write* surface; the poll surface is the next capture.

Until layer 2 lands, the WiFi/BT data path stays on the existing thunks
(`wifi_thunks.rs` + SimNet); layer 1 removes the unmapped-window faults so radio
firmware runs through configuration.

## Reproduce

```text
# build + flash the probe (needs ESP-IDF v5.3.1, esp32c3 target)
. ~/esp/esp-idf/export.sh
cd scripts/hw-oracle/wifi-re && idf.py set-target esp32c3 build
idf.py -p /dev/cu.usbmodemXXXX flash

# trace the radio register surface
./trace_radio.sh build/wifi_probe.elf /tmp/radio-trace
```
