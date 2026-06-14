# Two ESP32-C3 devices over simulated WiFi

LabWired can boot **two real, unmodified ESP-IDF ESP32-C3 firmwares** at once and
let them talk over a simulated 802.11 network — each its own CPU + real WiFi MAC,
with only a virtual AP and the (air-gapped) air between them. No thunks: the
firmware's real driver, lwIP stack, DHCP client and UDP sockets all run.

## What runs

```
 ┌─────────────── station A (C3) ──────────────┐      ┌─────────────── station B (C3) ──────────────┐
 │ real ROM boot → FreeRTOS → esp_wifi → lwIP    │      │ real ROM boot → FreeRTOS → esp_wifi → lwIP    │
 │ MAC 02:00:00:00:00:02                         │      │ MAC 02:00:00:00:00:03                         │
 └───────────────── wifi_mac ───────────────────┘      └───────────────── wifi_mac ───────────────────┘
            │ TX submit / RX inbox                                 │ TX submit / RX inbox
            └──────────────────────┐         ┌────────────────────┘
                                   ▼         ▼
                        ┌───────── VirtualWifi (shared medium) ─────────┐
                        │  OPEN infrastructure AP "labwired-ap"          │
                        │   • beacon / probe / auth / assoc              │
                        │   • DHCP server  → A=192.168.4.2, B=.3         │
                        │   • ARP (gateway + station resolution)         │
                        │   • UDP echo :9999                            │
                        │   • routes station↔station IPv4               │
                        └───────────────────────────────────────────────┘
```

Each station independently:
1. boots from the real mask ROM and 2nd-stage bootloader,
2. brings up WiFi, **scans and associates** to `labwired-ap`,
3. runs the **full DHCP DORA + lwIP ARP-check** and binds a distinct IP, and
4. opens a **UDP socket** and round-trips a datagram with the AP's echo server.

The only modelled "cheat" is the RF air-gap — there is no radio in a simulator,
so the `VirtualWifi` medium carries frames between MACs instead of photons.

## Architecture

- `crates/core/src/peripherals/esp32c3/virtual_wifi.rs` — the process-global
  shared medium + infrastructure AP (the WiFi analog of the BLE `VirtualAir`).
  `submit(mac, frame)` processes a station's transmission (responds to mgmt,
  serves DHCP/ARP/UDP-echo, routes IPv4 between stations); `take_inbox(mac)`
  returns frames queued for a station.
- `crates/core/src/peripherals/esp32c3/wifi_mac.rs` — `attach_to_medium()` puts
  the MAC in *medium mode*: transmitted frames go to the medium, the medium's
  inbox is pulled into the RX ring, and the station learns its own MAC from the
  SA of its first frame. (Default off → single-device CLI bridge path is
  unchanged.)
- `crates/cli/src/main.rs` — `build_c3_rom_boot_machine()` is the shared C3
  ROM-boot constructor; `run_two_c3_wifi()` builds two instances with distinct
  eFuse MACs, attaches both to the medium, and steps them in lockstep.

## Run it (CLI)

```sh
LABWIRED_WIFI_DUAL=1 \
LABWIRED_ESP32C3_ROM=/path/esp32c3_rom.bin \
LABWIRED_ESP32C3_ROM_DATA=/path/esp32c3_rom_data.bin \
LABWIRED_ESP32C3_FLASH=/path/c3-flash.bin \
labwired run --chip configs/chips/esp32c3.yaml \
  --firmware wifi_probe.elf --rom-boot --max-steps 430000000
```

Both firmwares share one stdout; their logs are prefixed `[A]` / `[B]`. Expected:

```
[A] I (…) probe: STA CONNECTED
[B] I (…) probe: STA CONNECTED
[A] I (…) probe: GOT IP 192.168.4.2
[B] I (…) probe: GOT IP 192.168.4.3
[A] I (…) probe: UDP TX -> 192.168.4.1:9999 'hello from c3'
[A] I (…) probe: UDP RX <- echo 'hello from c3'
[B] …
```

DHCP CHECKING (the lwIP ARP self-probe) makes each bind take ~1 s of simulated
time, so allow a few hundred million steps per station.

## On the web

Because `VirtualWifi` is a process-global (exactly like the BLE virtual air),
two `WasmSimulator` instances in the same WASM module **share it automatically**.
Create two C3 simulators, give each a distinct eFuse MAC, call the equivalent of
`attach_to_medium`, and step both — they associate and exchange traffic with no
extra wiring. The **WiFi network analyzer** (`wifi_trace_snapshot()`) surfaces
every 802.11 frame each station sends/receives for visualization.
