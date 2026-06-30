# ESP32-C3 WiFi/BT radio register-map RE harness

Reverse-engineers the undocumented C3 radio register surface by tracing the real
IDF WiFi driver on live silicon. See `docs/esp32c3_radio_reverse_engineering.md`.

- `main/wifi_probe.c` — minimal `esp_wifi_init`→`set_mode`→`esp_wifi_start` probe
  with breakpoint anchors bracketing each bring-up phase, plus a tight
  `probe_start_enter`/`probe_after_start` bracket around `esp_wifi_start()`.
- `trace_radio.sh <elf> <out_dir>` — flashes, sets HW breakpoints on the anchors
  over USB-JTAG (openocd-esp32), dumps the candidate radio windows per phase.
  Recovers the **write** surface (which regs the driver configures) by diffing
  phase snapshots.
- `trace_poll.sh <elf> <out_dir> [mac_base] [mac_words]` — recovers the
  complementary **poll** surface: arms a HW read-watchpoint over the MAC window
  across the `esp_wifi_start()` bracket and logs every load (pc + window
  snapshot) the driver issues while busy-waiting on MAC status bits.
- `poll_surface_to_table.py <poll_trace.log>` — offline reducer: diffs the
  per-hit snapshots, prints the offsets whose value changed and the bit that
  rose 0→1 just before the spin exited (the candidate release bit). Feed its
  table to `crates/core/src/peripherals/esp32c3/wifi_mac.rs` (`POLL_TABLE`).

## Two-pass RE workflow

The MAC behavioral model needs both surfaces:

1. **Write surface** (done, #238/#239): `trace_radio.sh` → register map.
2. **Poll surface** (this pass, #8): `trace_poll.sh` → status bits the driver
   spins on. Without it the model can't know *which* bit to flip or *what value*
   releases `esp_wifi_start()` — and we model the real handshake, not a thunk.

```sh
idf.py set-target esp32c3 build                       # build the probe (v5.3.1)
./trace_poll.sh build/wifi_probe.elf /tmp/c3-poll     # capture on a live C3
python3 poll_surface_to_table.py /tmp/c3-poll/poll_trace.log
```

Build: `idf.py set-target esp32c3 build` (ESP-IDF v5.3.1).
