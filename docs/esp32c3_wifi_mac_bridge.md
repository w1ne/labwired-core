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

## MAC DMA registers (RE'd from the live connect run, `LABWIRED_MAC_TRACE`)

Captured by tracing writes to the `0x6003_3000` MAC window while the real driver
brings WiFi up and starts a scan:

- **RX descriptor ring base**: `0x6003_3088` ← a DRAM pointer (e.g. `0x3fca4904`).
- **RX descriptor format** (linked list, 3 words each):
  | word | meaning |
  |---|---|
  | 0 | flags/len — `0x8064_0640`: **bit31 = owner** (HW may fill), low 16 = buffer size (`0x640` = 1600 = the "static rx buffer" size) |
  | 1 | buffer pointer (DRAM, the 1600-byte frame buffer) |
  | 2 | next-descriptor pointer (ring is a singly-linked list) |
- **Trigger / handshake**: `0x6003_3084` bit31 (written `0x8000_0000` to kick;
  the prior session's "handshake" scratch bit).
- Other config seen: `0x6003_3c60`/`c64`/`c6c` (a second ring/EOF pointer at
  `0x6003_3c64` ← `0x3fc00000`, zeroed), `0x6003_3d04`, `0x6003_3084`.

**RX descriptor is an ESP `lldesc_t`** (CONFIRMED by tracing the driver's reads
of an injected descriptor): word0 = `size[11:0] | length[23:12] | offset[28:24]
| sosf[29] | eof[30] | owner[31]`. Empty/HW-owned reads `0x80640640`
(owner=1, length=size=1600). On RX completion HW writes `owner=0, eof=1,
length=actual-rx-bytes, size preserved` (e.g. `0x40140640` for a 320-byte
frame). **VALIDATED end-to-end:** with that writeback, the real driver's RX
callback follows word1 (buffer ptr) and reads the injected frame bytes out of
the buffer, then recycles the descriptor (`owner` re-set to `0xc0140640`). The
RX inject path (queue → DMA → lldesc → MAC IRQ → `wDev_ProcessFiq` →
`lmacProcessRxSucData` → driver reads frame) works against the real firmware.
The 802.11 frame starts at buffer offset 0 (no rx-control prefix in the buffer).

**TX ring (still to RE):** the scan probe-request TX path hadn't queued a TX
descriptor within the traced window; needs a longer trace / break on the lmac
TX path to find the TX-kick register + descriptor.

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
