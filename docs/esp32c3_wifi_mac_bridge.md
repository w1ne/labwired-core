# ESP32-C3 WiFi MAC ↔ SimNet bridge (design + RE notes)

Status: **in progress.** Boot brings WiFi fully up in sim (see
`esp32c3_rom_boot.md`); this doc covers the next phase — making the **real** MAC
move packets to/from the in-sim virtual network, *without* the `wifi_thunks`
shortcut the ESP32-S3 used.

## Why this is its own phase (the impedance mismatch)

- The **real C3 MAC** operates on raw **802.11 frames** in hardware DMA rings.
  The running firmware (lmac/pp/wdev + the libnet80211 driver) does real
  scan → auth → assoc → data, programming the MAC registers and DMA descriptors.
- The existing **`SimNet`** (`crates/core/src/network/sim.rs`) is an **L4
  socket** simulation (TCP `connect`/`send`/`recv`, HTTP/echo servers,
  `VirtualAp.associate`). The S3 bridged to it by **thunking at the lwIP socket
  layer** (`esp32s3::wifi_thunks`) and faking `WL_CONNECTED` — i.e. it never ran
  esp_wifi/MAC at all. That is the thunk we are explicitly removing.

Bridging the real MAC to `SimNet` therefore needs a **frame-level** layer in
between (802.11 ↔ Ethernet ↔ the existing L4 SimNet), not a socket shim.

## MAC interrupt / event anatomy (RE'd from `wifi_probe.elf` + ROM)

- **MAC interrupt event register**: `hal_mac_interrupt_get_event` reads
  `0x6003_3C3C`; `hal_mac_interrupt_clr_event` writes `0x6003_3C40` (W1C).
- **ISR** `wDev_ProcessFiq` (0x4038_34A4) reads the event word and dispatches:
  | event mask | handler | meaning |
  |---|---|---|
  | `0x0100_4000` | `lmacProcessRxSucData` (ROM 0x4000_1614) | **RX frame received** |
  | `0x80`        | `lmacPostTxComplete`   (ROM 0x4000_1608) | TX complete |
  | `0x100`       | `lmacProcessCollisions`(ROM 0x4000_1610) | TX collision |
  | `0x1E`        | `wdev_process_tbtt`    | beacon timing |
  | `0x1E0`       | `wdev_process_tsf_timer` | TSF |
- **RX is a descriptor linked list** (`wdev_record_rx_linked_list`,
  `wdev_dump_rx_linked_list`); `lmacProcessRxSucData` walks it.
- **MAC interrupt = interrupt-matrix source 0** (`MAC_INTR_MAP` @ offset 0 in
  `interrupt_core0.yaml`), routed to a CPU line by the C3 interrupt matrix we
  already model — so raising it delivers to `wDev_ProcessFiq` via the normal
  trap path.

## RX-inject mechanism (target design)

1. Place the received 802.11 frame into the next free RX DMA descriptor's
   buffer (RX ring base register: **TODO — finish RE'ing where the driver
   programs it in `mac_txrx_init` / `ppRxPkt`**).
2. Set the RX-success bits in the event register `0x6003_3C3C` (`0x0100_4000`).
3. Assert MAC interrupt source 0 → matrix → CPU line → trap → `wDev_ProcessFiq`
   → `lmacProcessRxSucData` consumes the descriptor and hands the frame up.

## TX-capture mechanism (target design)

The driver fills a TX descriptor and writes a TX-kick register; the model reads
the frame out of the descriptor buffer and hands it to the frame-level AP, then
sets the TX-complete event bit (`0x80`) + raises the MAC interrupt so
`lmacPostTxComplete` runs. **TODO — RE the TX-kick register + descriptor format.**

## Remaining build (sequence)

1. **MAC DMA model** (`esp32c3::wifi_mac`, behavioral, replacing the declarative
   window but preserving the bring-up register-backing + MAC-ready bit): event
   register + interrupt raise + RX descriptor inject + TX descriptor capture.
   Finish the RX-ring-base / TX-kick RE first.
2. **Frame-level `VirtualAp`**: handle the 802.11 management the driver sends
   (probe/auth/assoc) so it associates, and relay data-frame payloads to/from
   the existing L4 `SimNet` (de/encapsulate 802.11 ↔ Ethernet ↔ IP).
3. **A connecting C3 app**: the current `wifi_probe` fixture brings WiFi up and
   idles ("idling for trace") — it never scans/connects, so it generates **no
   MAC traffic**. A minimal `esp_wifi_connect` + socket app (C3 IDF build) is
   needed to exercise and validate the bridge end-to-end.

The natural first milestone is **association over the real MAC** (driver TX of
probe/auth/assoc via the real DMA ring → frame-level AP responds via RX inject →
driver reaches connected) — the first true "real MAC, no thunks" comms.
