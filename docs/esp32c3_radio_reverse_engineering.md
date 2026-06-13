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
   path. The model must flip the bits the driver busy-waits on during
   `esp_wifi_start` so bring-up completes in sim. `trace_radio.sh` captured the
   *write* surface; the *poll* surface is captured by `trace_poll.sh` (below).

Until layer 2 lands, the WiFi/BT data path stays on the existing thunks
(`wifi_thunks.rs` + SimNet); layer 1 removes the unmapped-window faults so radio
firmware runs through configuration.

## Poll surface — captured & reproduced (live C3 v0.4)

`trace_poll.sh` arms a HW read-watchpoint over the MAC window across a tight
`esp_wifi_start()` bracket and logs every load the driver issues while bringing
the MAC + DMA up. Two runs (94 / 65 watchpoint hits, both reaching
`probe_after_start`) classify the `0x60033000` window by register *character* —
diffing the two runs separates deterministic config from live state:

| Offset band            | Character (run-to-run) | Reading |
|------------------------|------------------------|---------|
| `0x60033084`           | stable `0x80000000`    | **b31 = MAC command busy/done** — toggles `80000000→0→80000000`; prime handshake/ready bit the driver spins on |
| `0x60033088`–`0x6003309c` | **differs**         | free-running TSF/timer counter (`0x000a49xx`) — model as monotonic, not fixed |
| `0x600330a8`–`0x600330d4` | **differs (random)**| RNG / RX-FIFO data port (`0xd4b5bab8`, `0x6a2c46bf`…) — live data, never a poll bit |
| `0x600330d8`–`0x600330e4` | stable               | state words settling `0x7960→0x7940→0x7945→0x7045` |
| `0x60033100`/`0x60033104` | stable `0x05000000` | MAC control/status |
| `0x60033110`           | stable `0xa0100000`    | control |
| `0x6003311c`–`0x60033150` | stable               | config burst written right before `esp_wifi_start` returns |
| `0x60033148`/`0x6003314c` | stable `4400 4300` / `4300 4400` | MAC-address / filter bytes |
| `0x60033158`–`0x6003316c` | stable `ffff…`/`ff`  | BSSID / multicast filter masks (cold = all-ones) |

The block read `0x60033000`–`0x6003302c` (6× from PC `0x42049456`) is a register-
bank readback loop (MAC ID / cal), not a busy-wait.

### Write surface: the handshake is driver-managed scratch, not HW

A second pass with `WP_TYPE=w` (write-watchpoint over the same bracket, 99 hits)
recovers the command order, and it overturns the original "behavioral handshake"
assumption. `0x60033084` b31 is written by *firmware*, not the MAC:

```
pc=0x4202f4ca  0x084 <- 80000000   # bulk-init of the 0x080-9c block (memcpy)
pc=0x420490dc  0x084 <- 00000000   # driver clears b31 (begin sequence)
   ... driver does setup; seeds TSF counter 0x088/08c <- 000a49cc ...
pc=0x42049396  0x084 <- 80000000   # driver sets b31 back (done)
```

The driver writes `0x60033084` (and the whole `0x080`–`0x09c` block) then reads
its own values back — it's using MMIO as scratch state, **not** polling a bit the
hardware flips. A plain register-backed model (what Layer 1 already is) therefore
reproduces this faithfully with zero behavioral logic. The only genuinely
HW-sourced state is:

* `0x600330a8`–`0x600330d4` — RNG / RX-FIFO data port (must vary on read);
* `0x60033088`–`0x6003309c` — TSF/timer counter (seeded by driver, then HW-advanced).

So **#9 is far narrower than first scoped**: not a status-handshake state machine,
but (a) a varying RNG/data port and a monotonic counter, and (b) the DMA descriptor
rings (lldesc, reused from `gdma.rs`) bridged to `network::WirelessPacket` (#10).
The decisive next step is empirical — boot `wifi_probe.elf` in the C3 sim (the
register-backed Layer 1 model + real ROM) and find the exact PC where it stalls;
that pinpoints which of the above the firmware actually requires to advance,
instead of modeling speculatively.

Raw capture logs aren't committed (see the repo-root `fixtures/` ignore);
regenerate with `trace_poll.sh` (reads) and `WP_TYPE=w trace_poll.sh` (writes) on
a live C3 — both runs above reproduced the deterministic bands byte-for-byte.

## The real blocker is the boot path, not the MAC (diagnosed 2026-06-13)

Booting the real IDF `wifi_probe.elf` in the C3 sim shows the WiFi MAC can't even
be reached yet — the firmware dies in boot:

* **fast-boot, no ROM image:** runs 196,613 instructions of IDF C startup, then PC
  slides off the end of the zero-filled ROM region at `0x40060000` (it *called* a
  C3 ROM function and there was nothing there).
* **fast-boot, real ROM dumped from silicon** (`openocd dump_image 0x40000000
  0x60000` → `LABWIRED_ESP32C3_ROM`, `0x3FF00000 0x20000` → `_ROM_DATA`): crashes
  at step ~6596 inside `rom_i2c_writeReg_Mask`:

  ```
  40039234: lw a5, 1464(a5)   # a5 = *(0x3fcdf5b8)  = rom_phyFuns  (DRAM global)
  40039240: lw a5, 428(a5)    # a5 = phyFuns->fn[107]
  40039256: jalr a5           # → 0xfe38d096 (garbage) → fault
  ```

  `rom_phyFuns` (`0x3fcdf5b8`) is a ROM **function-pointer table in DRAM** that the
  BROM reset handler initializes on silicon. Fast-boot jumps straight to the app
  ELF entry (`0x403802dc`) and **skips the BROM reset sequence**, so the ROM's DRAM
  globals are never set up and the indirect call goes to garbage. (Symbols mapped
  against `~/.espressif/tools/esp-rom-elfs/.../esp32c3_rev3_rom.elf`.)

Fix, the faithful way (run the binary, don't thunk it): **RISC-V rom-boot** — reset
the CPU to the BROM vector `0x40000400`, back the XIP/flash windows with the real
flash image (`LABWIRED_ESP32C3_FLASH` = bootloader+ptable+app), and let the real
ROM run: it initializes its own DRAM globals (`rom_phyFuns` et al.), loads the
2nd-stage bootloader + app through the flash path, and jumps to `app_main` exactly
like silicon. Today `--rom-boot` is implemented only for Xtensa
(`configure_xtensa_esp32s3`, `LABWIRED_ESP32S3_FLASH`); C3 (RISC-V) only fast-boots.
That's the one remaining task (#18) gating real WiFi-in-sim — once IDF apps reach
`esp_wifi_start`, the MAC is mostly register-backed scratch (above) plus the DMA
ring → SimNet bridge (#10).

## Reproduce

```text
# build + flash the probe (needs ESP-IDF v5.3.1, esp32c3 target)
. ~/esp/esp-idf/export.sh
cd scripts/hw-oracle/wifi-re && idf.py set-target esp32c3 build
idf.py -p /dev/cu.usbmodemXXXX flash

# trace the radio register surface (write surface, by phase)
./trace_radio.sh build/wifi_probe.elf /tmp/radio-trace

# trace the MAC poll surface (status bits the driver busy-waits on)
./trace_poll.sh build/wifi_probe.elf /tmp/c3-poll
python3 poll_surface_to_table.py /tmp/c3-poll/poll_trace.log
```

Flashing over JTAG (port-unambiguous when several boards are attached, since the
board cfg locks onto the C3's USB-JTAG PID `0x1001`):

```text
openocd -s $SCRIPTS -f board/esp32c3-builtin.cfg \
  -c "init; reset halt" \
  -c "program_esp build/bootloader/bootloader.bin 0x0 verify" \
  -c "program_esp build/partition_table/partition-table.bin 0x8000 verify" \
  -c "program_esp build/wifi_probe.bin 0x10000 verify" \
  -c "reset halt; shutdown"
```
