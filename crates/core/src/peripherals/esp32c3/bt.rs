// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 Bluetooth LE link-layer / baseband block (`0x6003_1000`, 4 KiB) —
//! behavioral model, built the same way as [`super::wifi_mac`].
//!
//! **There is no datasheet for this window.** Espressif does not publish the
//! WiFi/BT MAC registers, and the ESP32-C3 TRM stops at the crypto estate.
//! Everything below was reverse-engineered from the connected ESP32-C3
//! SuperMini (MAC `9c:cc:01:d0:5e:70`, built-in USB-JTAG) running an Arduino
//! `BLEDevice::init()` + `startAdvertising()` probe — **silicon capture
//! 2026-08-02**. Where a value could not be determined from silicon it is
//! called out in a comment rather than invented; a model that lies is worse
//! than a fault.
//!
//! ## How the window was mapped
//!
//! Three OpenOCD passes against the live part (`board/esp32c3-builtin.cfg`):
//!
//! 1. **Idle dumps.** `reset halt` reads the whole window as `00000000` — the
//!    block is clock-gated out of reset. After `BLEDevice::init()` it is dense
//!    with state, and the control window `i2s0` (`0x6002_D000`) still reads
//!    zero, so the non-zero reads are a real block and not bus float.
//! 2. **Write trace.** A `wp 0x60031000 0x1000 w` watchpoint from `reset halt`,
//!    resuming into 400 hits while capturing PC + all 31 GPRs each time. The
//!    RISC-V store at the trapping PC (the trigger fires *before* the store)
//!    was then decoded offline to recover the exact target offset and value.
//!    303 hits covered the whole of BLE bring-up; the watchpoint then went
//!    quiet, i.e. that IS the init write sequence.
//! 3. **Read trace.** The same with `wp ... r`, to find busy-wait polls.
//!
//! ## What the traces proved
//!
//! **Almost all of BLE bring-up is register-backed read-modify-write.** The
//! read trace found *no* status-bit spin loop anywhere in `BLEDevice::init()`:
//! every repeated `(pc, offset)` pair is a read-modify-write or a table walk
//! (e.g. `+0x2C4` re-read ten times, once per ROM-patch slot, each time OR-ing
//! in one more enable bit and writing it straight back). So plain storage with
//! a zero reset — exactly what silicon reads while gated — carries the
//! controller through init, and that is what this model gives the whole window.
//!
//! On top of that, exactly one thing needs *real behaviour*:
//!
//! * **`+0x01C` — the Bluetooth native clock (CLKN), read/write asymmetric.**
//!   The BT ROM routine at `0x4002_EE60` does: read `+0x01C` → compute a
//!   deadline → **write** `+0x01C` with `0x8000_0000 | target` (`0x4002_EE72`)
//!   → **re-read** `+0x01C` twice (`0x4002_EE78`, `0x4002_EE8E`) → read
//!   `+0x020` (`0x4002_EE92`). The write must not become what the read
//!   returns, so the model keeps them separate.
//!
//!   **The top bit is now settled** (silicon capture 2026-08-02, board
//!   `38:44:be:42:f5:58`): that routine is `r_rwip_time_get`, and the mask ROM
//!   — read back off this part and matched word-for-word against the symbolled
//!   `esp32c3_rev3_rom.elf` — does `+0x01C |= 0x8000_0000`, then
//!   **`lw a5,28(s0); bltz a5, -4`**, i.e. spins while the read-back is still
//!   negative, and only then samples `+0x01C` and `+0x020`. So bit31 is a
//!   *sample-latch request* the hardware clears when a coherent
//!   `BASETIMECNT`/`FINETIMECNT` pair is ready — the earlier "sample/latch vs.
//!   next-event comparator" ambiguity resolves to the former, and the
//!   comparators live at `+0x0E4`/`+0x0E8`/`+0x0EC` instead (see below).
//!   Keeping the write out of the read, which this model already did, is
//!   exactly what the handshake needs: the sample is always ready, so the spin
//!   exits at once. The written value is still kept in
//!   [`Esp32c3Bt::armed_event_target`] for inspection.
//!
//!   This is the one register a frozen value would deadlock: the routine
//!   schedules the *next* event off the *current* clock and re-reads to check
//!   whether its deadline already slipped, so a constant makes the controller
//!   either spin or re-arm the same instant forever.
//!
//! * **`+0x020` — the sub-tick fine counter** the same routine reads right
//!   after CLKN, for sub-CLKN resolution.
//!
//! ## Timebase provenance (silicon capture 2026-08-02)
//!
//! `+0x01C` was sampled across known wall-clock intervals with the part
//! advertising (`halt; mdw; resume; sleep N`):
//!
//! | interval | delta | rate |
//! |---|---|---|
//! | 1 s | `0x3e938c → 0x3ea0d0` = 3396 | ~3396 Hz |
//! | 2 s | `0x3ea0d0 → 0x3eba6f` = 6559 | ~3280 Hz |
//!
//! (Both are slight over-reads: each sample pays halt/resume overhead on top
//! of the sleep.) That lands on the **Bluetooth native clock, which ticks
//! every 312.5 µs = 3200 Hz** — a spec-defined period, not a fitted constant.
//! `+0x020` was never observed at or above 625 over a dozen samples (max
//! `0x236` = 566, mean ≈ 440 — consistent with uniform sampling of `0..624`),
//! and 312.5 µs × 2 MHz = 625 exactly, so it is modelled as a **half-µs
//! counter that wraps once per CLKN tick**. Together the pair is a textbook BT
//! baseband timebase, and both halves are derived from the sim's cycle clock
//! so they stay coherent with device time under idle fast-forward.
//!
//! ## Deliberately NOT modelled (say so rather than invent)
//!
//! * **`+0x024`** is live on silicon but is **not monotonic** — sampled values
//!   `0x1000, 0x1050, 0x1064, 0x108c, 0x10a0, 0x10b4` include decreases, so it
//!   is not a counter. Every observed value is `0x1000 + 20·n`, which hints at
//!   a rotating slot index, but that is a guess and the read trace shows
//!   `BLEDevice::init()` never reads it. It stays plain storage; if a firmware
//!   ever polls it the run will stall visibly rather than be lied to.
//! * **The RF PHY.** There is still no modulation, no preamble, no bit sync,
//!   no whitening and no CRC arithmetic here. Programmed radio events now run
//!   and their PDUs cross a shared air (see below and
//!   [`crate::peripherals::ble_air`]), but that air is an idealised,
//!   lossless, collision-free medium — not a faithful BLE PHY.
//!
//! ## The interrupt path (silicon capture 2026-08-02, board `38:44:be:42:f5:58`)
//!
//! **Routing.** Interrupt-matrix dump after init: `RWBLE_IRQ_MAP`
//! (`0x600C_2020`, matrix source 8) = 5 and `BT_BB_INT_MAP` (`0x600C_2014`,
//! source 5) = 8. So the RW-BLE core drives **matrix source 8**, which the
//! firmware routes to CPU line 5. That is the source this model exports.
//!
//! **Where the bit meanings come from — read the ROM, do not guess.** The
//! previous pass could name `INTCNTL`/`INTSTAT`/`INTRAWSTAT`/`INTACK` but not
//! say which bit is which event, and refused to invent them. They are now
//! settled from two independent silicon-anchored sources:
//!
//! 1. **The mask ROM's own dispatcher.** `esp32c3_rev3_rom.elf` (shipped in
//!    `esp-rom-elfs`) carries full symbols for the RW-BLE stack, and the bytes
//!    at `r_rwble_isr` (`0x4002_E64A`) were read back off this part over JTAG
//!    and match the ELF word for word (`7179 ce4e 3fce09b7 …`), so the
//!    disassembly IS this silicon. `r_rwble_isr` and `r_rwip_isr`
//!    (`0x4002_F4EC`) test the status word bit by bit; each arm W1C-writes
//!    exactly its own bit to `+0x018` and calls one entry of a dispatch table.
//! 2. **The live dispatch tables.** `r_modules_funcs_p` (`0x3FCD_FF88`) and
//!    `r_ip_funcs_p` (`0x3FCD_FF8C`) were dereferenced on the running,
//!    advertising part (`0x3FC9_C024` / `0x3FC9_C404`) and dumped. All but
//!    three of the slots the two ISRs index hold ROM thunks that resolve
//!    straight to a name; the three that ESP-IDF patches into IRAM are handled
//!    below and marked.
//!
//! That yields, bit → handler:
//!
//! | bit | mask | handler (from the live table) |
//! |---|---|---|
//! | 0  | `0x00000001` | `r_rwip_wakeup_end` † (gated on `rwip_env+0x16` bit 0) |
//! | 1  | `0x00000002` | `r_sch_prog_tx_isr` |
//! | 2  | `0x00000004` | `r_sch_prog_rx_isr` |
//! | 3  | `0x00000008` | `r_rwip_wakeup` † |
//! | 5  | `0x00000020` | `r_sch_prog_end_isr` † (end of a programmed radio event) |
//! | 6  | `0x00000040` | `r_sch_prog_skip_isr` |
//! | 7  | `0x00000080` | `r_rwip_crypt_isr_handler` |
//! | 8  | `0x00000100` | error: dumps `DIAG0/1`, `EM BASE ERROR`, `FSMERROR`; its slot is NULL on this build, so the ISR only asserts (`rwble.c 261`) |
//! | 9  | `0x00000200` | `r_rwip_timer_10ms_handler` |
//! | 10 | `0x00000400` | (armed by `r_rwip_timer_hs_set`; no handler slot — the arm/ack sequence is the whole evidence) |
//! | 11 | `0x00000800` | `r_rwip_timer_hus_handler` |
//! | 12 | `0x00001000` | `r_rwip_sw_int_handler` |
//! | 18 | `0x00040000` | `r_lld_update_rxbuf_isr` (raised by the `+0x2D0` bit15 rising edge — see [`RX_BUF_JUMP`]) |
//! | 19 | `0x00080000` | `r_ble_sw_cca_check_isr` |
//! | 21 | `0x00200000` | `IRQ FIFO ALMOST FULL:cnt %u, rem %u` (ROM string) |
//! | 22 | `0x00400000` | fatal: sets `+0x2D8` bit31, acks `0x7FFFFF`, asserts |
//!
//! † These three slots hold IRAM addresses on the live part — ESP-IDF's
//! `libble_app` patches them — so the live table alone leaves them unnamed.
//! They are named by *slot position*, which is inference, not measurement, and
//! is flagged as such: both dispatch tables are laid out in the same order as
//! the ROM's own thunk table, so a patched slot's original identity is the
//! thunk its unpatched neighbours skipped. `+1728` sits between
//! `__call_r_sch_prog_init` (`0x4000153C`) and its predecessor `0x40001538` =
//! `__call_r_sch_prog_end_isr`; `+736`/`+740` follow
//! `__call_r_rwip_timer_hus_set` (`0x400014CC`) and take `0x400014D0` =
//! `__call_r_rwip_wakeup` and `0x400014D4` = `__call_r_rwip_wakeup_end`. The
//! method is cross-checked by every unpatched slot in those runs agreeing with
//! its position, and by bit 11's independently-confirmed
//! `r_rwip_timer_hus_handler` (the `+0x0EC` comparator proves that one from
//! the register side). Nothing in the model depends on these three names.
//!
//! Cross-check: the enable word read off the live part is `INTCNTL =
//! 0x0064_0B66`, whose set bits are exactly `{1,2,5,6,8,9,11,18,21,22}` — the
//! subset above that a *controller doing nothing but advertising* needs, with
//! the crypto (7), software (12), CCA (19) and unexplained (0, 3) bits masked
//! off. The bit map and the enable mask were derived independently and agree.
//!
//! **`INTSTAT` is `INTRAWSTAT & INTCNTL`.** Measured across ~20 halts of the
//! advertising part, on every raw value that turned up:
//! `(0x0640B66, 0x011) → 0x000`, `(0x0640B66, 0x811) → 0x800`, and
//! `(0x0640B66, 0x031) → 0x020`. So `+0x010` is derived, not stored — this
//! model computes it and refuses a write to it.
//!
//! **The three comparators.** The ROM's timer setters say which register arms
//! which bit, unambiguously — each writes a target, W1C-acks its own bit, and
//! ORs its enable into `INTCNTL` (and clears that enable to disarm):
//!
//! | setter | target register | unit | bit |
//! |---|---|---|---|
//! | `r_rwip_timer_10ms_set` | `+0x0E4` | 10 ms = 32 CLKN ticks (the setter also keeps `rwip_env+8 = target << 5`, i.e. half-slots) | 9 |
//! | `r_rwip_timer_hs_set`   | `+0x0E8` | half-slot = 1 CLKN tick | 10 |
//! | `r_rwip_timer_hus_set`  | `+0x0EC` (base, 28-bit) + `+0x0F0` (fine, written as `624 - hus`) | CLKN + `FINETIMECNT` | 11 |
//!
//! And the live part confirms the hus comparator is the one driving
//! advertising: sampled four times while advertising, `+0x0EC` always sat
//! 119–150 CLKN ticks *ahead* of `+0x01C` (`0x462F→0x46B1`, `0x50BB→0x5151`,
//! `0x57E8→0x587E`, `0x6810→0x6887`), i.e. 37–47 ms out — a BLE advertising
//! interval with its random delay. `+0x0F0` read `0x270` = 624 (= `624 - 0`).
//!
//! **A masked comparator does not latch.** At the same halts `+0x0E8` held a
//! long-stale `0x91` (CLKN was `0x462F`) with `INTCNTL` bit 10 *clear*, and
//! `INTRAWSTAT` bit 10 read **0**. If the comparator latched raw status
//! regardless of the mask, bit 10 would have been set. So this model runs a
//! comparator only while its `INTCNTL` enable is set — which is also exactly
//! how the ROM arms and disarms them.
//!
//! **The IRQ FIFO at `+0x2D8`.** `sdk_cfg_priv_opts[69]` reads `0x01` on this
//! part, which selects `r_rwble_isr`'s FIFO path over its plain-`INTSTAT`
//! path — so the FIFO is NOT optional, and a model that raises the line
//! without it would spin the ISR forever (it returns without acking when the
//! FIFO is empty). The layout falls straight out of the dispatcher:
//!
//! ```text
//! bit0      write 1 = pop the head entry   (ori a5,a5,1; sw a5,728(a4))
//! bits[4:1] rem  — free slots              (printf "rem %u", (w >> 1) & 15)
//! bits[9:5] cnt  — queued entries          (printf "cnt %u", (w >> 5) & 31)
//! bits[30:10] the head entry's bitmap, in INTSTAT bit positions
//!             (s0 = (w << 1) >> 11, then dispatched with the SAME masks the
//!              plain path applies to a raw INTSTAT read at 0x4002E8EA)
//! bit31     set by the ISR on the bit-22 fatal path
//! ```
//!
//! and the live reads agree exactly, on two different interrupts and on the
//! idle case:
//!
//! | `+0x2D8` | `INTSTAT` | `INTRAWSTAT` | decodes to |
//! |---|---|---|---|
//! | `0x0020_003E` | `0x800` | `0x811` | cnt 1, rem 15, bitmap `0x800` (hus timer) |
//! | `0x0000_803E` | `0x020` | `0x031` | cnt 1, rem 15, bitmap `0x020` (`sch_prog_end`) |
//! | `0x0000_001E` | `0x000` | `0x011` | cnt 0, rem 15, empty |
//!
//! The idle row also pins the empty-FIFO word: `rem` is a 4-bit field and the
//! FIFO is 16 deep, so "15 free" and "16 free" are the same encoding, and
//! silicon reads `0x1E`. That is what this model returns rather than a guess.
//!
//! **`+0x01C` bit31 is a sample-latch handshake, not a comparator.** The
//! earlier pass could not decide; `r_rwip_time_get` settles it —
//! `+0x01C |= 0x8000_0000`, then **spin while the read-back is negative**,
//! then read `+0x01C` and `+0x020`. So the write requests a coherent
//! base/fine latch and the hardware clears the bit when the sample is ready.
//! Keeping the write out of the read (which this model already did) is what
//! that demands: the spin exits as soon as the sample is up. The same routine
//! masks the base counter with `0x0FFF_FFFF` and `r_rwip_timer_hus_set`
//! asserts on a target `& 0xF000_0000`, so **CLKN is 28 bits**, not 31.
//!
//! ## What the interrupt path buys the twin, and where it stops
//!
//! Measured on the ESP32-C3 rom-boot twin running the same Arduino
//! `BLEDevice::init()` + `startAdvertising()` probe image the silicon capture
//! used, 500 M steps (~1.32 G cycles), `LABWIRED_BT_TRACE=1`:
//!
//! * **Before** (no interrupt path): `PRE_BLE / BLE_INIT_OK / ADV_ON / ALIVE`,
//!   322 BT register writes, the last at CLKN 686 (~214 ms of BT time). The
//!   controller armed the hus comparator exactly once (`+0x0EC <= 0x25`),
//!   never got the interrupt, and spent the remaining ~8 s of device time
//!   re-reading `+0x01C` and nothing else.
//! * **After**: the same four markers (no bring-up regression), 372 writes,
//!   and **three real interrupts taken and handled** — hus at CLKN 37, hs at
//!   CLKN 40, hus again at CLKN 688. Each one is followed in the trace by the
//!   exact ROM ISR sequence: pop the FIFO (`+0x2D8 <= 0x0020_003F`), W1C the
//!   bit (`+0x018 <= 0x800`), drop the enable, mirror the ack to `+0x38C`.
//!   Behaviour that never happened before the interrupt path existed: the
//!   handler kicks `RWBLECNTL` (`+0x000 <= 0x0210_070F`, `0x0310_070F`), arms
//!   the **half-slot** comparator (`+0x0E8 <= 0x28` — a register the polled
//!   build never wrote), runs a second scheduler cycle, re-arms the hus
//!   comparator for the next advertising instant (`+0x0EC <= 0x2B0`), and then
//!   programs a radio event (`+0x32C <= 0x4000_000D`, `+0x100 <=
//!   0x8000_0000`).
//!
//! That used to stop there, waiting on `r_sch_prog_end_isr` / `_tx_isr` /
//! `_rx_isr` (bits 5, 1, 2). Silicon showed exactly that missing edge:
//! sampling the advertising part 14 times caught one mid-event, `INTSTAT =
//! 0x20` / `INTRAWSTAT = 0x31` / `+0x2D8 = 0x0000_803E` — a queued **bit 5**,
//! `sch_prog_end`. That edge is now real; see the next section.
//!
//! ## Programmed radio events, and the exchange memory behind them
//!
//! That is where the previous pass stopped. It no longer does: the controller's
//! programmed events now execute, transmit the PDU the controller staged, and
//! complete with `sch_prog_end`, so advertising runs as a sustained cadence.
//!
//! **The layout was read out of the ROM and then confirmed on silicon**, field
//! by field — nothing here is a guess about a descriptor.
//!
//! ### Exchange memory is ordinary data RAM behind a base-register window
//!
//! `r_emi_get_mem_addr_by_offset` (`0x4000_6976`) translates an
//! exchange-memory byte offset to a CPU address:
//!
//! ```text
//! reg = *(0x6003_1204 + reg_idx*4)          // bank A, reg_idx <= 47
//!     | *(0x6003_1220 + reg_idx*4)          // bank B, reg_idx 48..=55
//! covered  = (reg >> 18) << 2               // the EM offset this reg serves
//! cpu_addr = ((reg << 2) & 0x000F_FFFC) | 0x3FC0_0000  +  (em_off - covered)
//! ```
//!
//! So the RW-BLE core's "exchange memory" is a set of **1 KiB windows the
//! controller allocates out of the C3's own SRAM at runtime**
//! (`r_emi_alloc_em_mapping_by_offset`) and publishes through those registers.
//! The ROM consults a `em_base_reg_lut` table to pick the register, but that
//! table is redundant — it asserts `lut[off >> 10].base == (reg >> 18) << 2`,
//! i.e. every register already names the offset it covers — so this model picks
//! the covering register straight off the register file and hardcodes no
//! address at all. Silicon capture 2026-08-02, board `38:44:be:42:f5:58`, with
//! BLE up: `0x6003_1204 = 0x0002_9725` → EM `0x0000` at `0x3FCA_5C94`;
//! `0x6003_1208 = 0x0402_977B` → EM `0x0400` at `0x3FCA_5DEC`;
//! `0x6003_1214 = 0x1402_9961` → EM `0x1400`; `0x6003_1220 = 0x2402_B833` →
//! EM `0x2400`. The twin's own firmware writes the same registers with its own
//! addresses (`+0x220 <= 0x2402_B9F2`), which is exactly why reading them
//! beats hardcoding them.
//!
//! ### `+0x100` — the programmed-event push
//!
//! `r_sch_prog_ble_push` (`0x4003_0BDA`) fills an exchange-table entry and then
//! does `sw (0x8000_0000 | idx), 256(0x6003_1000)`, having asserted
//! `idx & ~0xF == 0`. The live window reads `+0x100` back as 0, so it is a
//! self-clearing command register.
//!
//! ### The exchange table (ET) — EM offset `0x000`, 16 entries × 16 bytes
//!
//! Every `sch_prog` routine reaches it via `r_plf_funcs_p[47](0)`, i.e.
//! `emi_get_mem_addr_by_offset(0)`, and indexes it `idx << 4`;
//! `r_sch_prog_init` walks `0..256` step 16.
//!
//! | off | field | evidence |
//! |---|---|---|
//! | `+0x0` | control; **bits[5:3] = status**, hardware-owned | all four `sch_prog` ISRs read `(lhu >> 3) & 7`; `r_sch_prog_ble_push` writes this halfword with that field ZERO |
//! | `+0x2` | start time, low 16 bits of the 28-bit CLKN half-slot count | `r_sch_prog_push` writes `time & 0xFFFF`; live entry 0 read `0x83A9` while `sch_prog_env[0]` held `0x00F8_83A9` |
//! | `+0x4` | start time, high 12 bits | same routine writes `(time >> 16) & 0xFFF`; live `0x00F8` |
//! | `+0x6` | fine start offset, encoded `624 - hus` | `r_sch_prog_push` asserts `hus <= 624` then writes `624 - hus`; live `0x0270` = 624, matching `+0x0F0` |
//! | `+0x8` | **control-structure pointer, EM offset / 2** | `r_sch_prog_ble_push` writes `(cs_idx*90 + 1024) >> 1`; live `0x0200` → EM `0x400`, which is where the control structure demonstrably is (below). 90 is the CS stride and 1024 = `EM_BLE_CS_OFFSET` |
//! | `+0xA` | duration; bit15 selects half-slots, else two half-µs per unit | `r_sch_prog_push` writes `(dur+1)>>1` below `0x8000` and `((dur+625)/625) | 0x8000` above; live `0x0AF7` = 2807 µs, a plausible 3-channel legacy advertising event |
//! | `+0xC` | two 5-bit priorities | `(min(p2,31) << 8) | min(p1,31)`; live `0x0C00` |
//! | `+0xE` | `(5-bit field << 8) | 2-bit field` | written by `r_sch_prog_ble_push`; live `0x0F00`. **Meaning not determined** — plausibly a link label, but nothing measured says so, so nothing here reads it |
//!
//! The status codes are pinned by what the ROM does with them:
//! `r_sch_prog_push` refuses to reuse an entry whose status is **1 or 2**
//! ("still in use", `sch_prog.c` 560); `r_sch_prog_tx_isr`/`_rx_isr` assert
//! `status & 6 != 0`; `r_sch_prog_end_isr` dispatches the frame callback for
//! **3, 4 or 5** and passes `irq_type = (status == 4)`; `r_sch_prog_skip_isr`
//! consumes **6**; and `r_sch_prog_init` seeds every entry with 3. Silicon
//! agrees: all sixteen live entries read `+0x0 = 0x281A`, whose status field is
//! 3, on a part that had just finished an advertising event. This model writes
//! **2 while the event executes** (the only value consistent with both the
//! "in use" rejection and the tx/rx asserts — inference from ROM constraints,
//! flagged as such) and **3 when it ends**. Status 1, 4, 5 and 6 are never
//! written: nothing measured says what produces them.
//!
//! ### The control structure (CS) — EM offset `0x400`, stride 90 bytes
//!
//! `r_sch_prog_ble_push`'s `cs_idx*90 + 1024` gives both constants. Silicon
//! confirms the stride independently: the live control structures carry a
//! per-structure marker at `+0x2` (`0x0001`, `0x0002`, `0x0003`, `0x0004`) and
//! they sit exactly 90 bytes apart. CS 0, live, decoded — and the BLE spec
//! constants in it are the proof this is what it is:
//!
//! ```text
//! +0x00 0x0404   format word; the model acts only on low byte 0x04
//! +0x06 5a f5 42 be 44 38   device address, LSB first = 38:44:be:42:f5:5a
//!                           (the board's BLE address, its base MAC + 2)
//! +0x0C d6 be 89 8e         access address = 0x8E89BED6 — the BLE
//!                           advertising access address, spec-defined
//! +0x10 55 55 55            CRC init = 0x555555 — the BLE advertising CRC
//!                           init, spec-defined
//! +0x16 0x8027              hop control; bits[6:0] = 39 = the last of the
//!                           three primary advertising channels
//! +0x1C 0x1400              EM offset of the first TX descriptor
//! ```
//!
//! ### The TX descriptor — 14 bytes, singly linked
//!
//! Two live descriptors at EM `0x1400` and `0x140E`, each pointing at the next
//! through `+0x0`:
//!
//! ```text
//! +0x0  next descriptor, EM offset  (0x140E / 0x1400 — a ring of two)
//! +0x2  (payload_len << 8) | pdu_header_byte
//!         0x0F20 -> header 0x20 (ADV_IND, ChSel=1), 15 bytes
//!         0x1904 -> header 0x04 (SCAN_RSP),         25 bytes
//! +0x4  EM offset of the payload bytes after the device address
//!         0x2400 -> 02 01 06 05 12 20 00 40 00        (9 bytes)
//!         0x2C00 -> 09 09 "lw-probe" 02 0a 09 05 12 20 00 40 00 (19 bytes)
//! ```
//!
//! 15 = 6 + 9 and 25 = 6 + 19, so the core inserts the 6-byte device address
//! from `CS+0x06` ahead of the descriptor's buffer. That is measured twice, on
//! two PDU types, and it is the only reading under which the declared lengths
//! and the buffer contents agree — but it is still an inference from two
//! samples rather than something the ROM states, and it is only claimed for
//! the legacy advertising format.
//!
//! ### The RX descriptor — EM `0x1000`, 20 bytes, singly linked
//!
//! Both `r_lld_scan_process_pkt_rx` and
//! `r_lld_scan_process_pkt_rx_legacy_adv` reach it with
//! `emi_get_mem_addr_by_offset(0x1000)` and `idx * 20`, so the base and stride
//! are ROM constants rather than inferences:
//!
//! ```text
//! +0x00 next descriptor EM offset in bits[14:0]; bit15 = RXDONE (below)
//! +0x02 reception status. r_lld_scan_process_pkt_rx rejects the packet on
//!       (status & 0x402D) != 0; bit15 is a SOFTWARE-owned "released" marker
//!       r_lld_rxdesc_check requires CLEAR (below). Every populated live
//!       descriptor read 0x8040 — which is the post-processing value, not
//!       what the core wrote
//! +0x04 (payload_len << 8) | pdu_header_byte, the mirror of the TX
//!       descriptor's +0x2. Live 0x0C03 = a 12-byte SCAN_REQ (type 3)
//! +0x08 32-bit CLKN receive timestamp (live 0x00F888AE, just behind the
//!       0x00F88A5D the part's own clock read at the same halt)
//! +0x0C bits[15:11] = link label of the receiving activity (below)
//! +0x12 EM offset of the received PAYLOAD — no header bytes; the ROM
//!       memcpy's six bytes straight off it to get the peer address
//! ```
//!
//! ### The RX descriptor ownership protocol — what the host report was waiting on
//!
//! The previous pass got a scanning node's *controller* to receive but never got
//! an advertising report to the *host*, and named `lld_update_rxbuf_isr`
//! (interrupt bit 18) as the blocker on the theory that it advanced
//! `p_lld_env[216]`, the software's descriptor cursor. **Reading the ROM shows
//! that theory was wrong on both counts**, and the real contract is three bits
//! wide:
//!
//! 1. **`p_lld_env[216]` is advanced by `r_lld_rxdesc_free` (`0x4001_FFE8`)**,
//!    once per packet the link layer consumes — `p_lld_env[216] = (…+1) % 10`,
//!    so the ring is **10 deep**, not 16. `lld_update_rxbuf_handler` only
//!    *re-seeds* it, on the reconfiguration path. Nothing about the per-packet
//!    path needs bit 18.
//! 2. **`+0x00` bit15 is `RXDONE`, and the core owns its 0→1 edge.** The ROM
//!    names the bit itself: `r_lld_update_rxbuf`'s trace string is
//!    `"RXBUF Update RXDESC: Current %04x[%d], RD %d; Jump %04x[%d], RD %d,
//!    NextPTR %04x"` and each `RD` argument is `lhu(rxdesc + 0) >> 15`.
//!    `r_lld_rxdesc_check` (`0x4002_022C`) reports a packet to the link layer
//!    only while it is **set**; every refill site clears it right after storing
//!    a fresh buffer offset in `+0x12`. Set = "the core has written here, hands
//!    off"; clear = "yours to fill".
//! 3. **`+0x02` bit15 is software's, and must be CLEAR after a reception.**
//!    `r_lld_rxdesc_check` ends with `return (lhu(rxdesc + 2) >> 15) ^ 1`, while
//!    `r_lld_rxdesc_free` and `lld_update_rxbuf_handler` both SET it when they
//!    release a descriptor. So the measured `0x8040` is what firmware left
//!    behind after processing, not what the core wrote — the live part was
//!    halted long after its own link layer had drained those receptions. A
//!    model that replays `0x8040` verbatim (which is what this one did) is
//!    handing firmware a descriptor that is permanently marked "already
//!    released", and `r_lld_rxdesc_check` will never report it. That single bit
//!    is why four real receptions produced zero advertising reports.
//! 4. **`+0x0C` bits[15:11] must carry the receiving activity's link label**, or
//!    `r_lld_rxdesc_check(label)` rejects the packet as somebody else's. The
//!    label is the activity's **control-structure index** — see
//!    [`RXD_LINK_LABEL`] for the three ROM sites that pin it and the silicon
//!    cross-check (the live advertiser's CS is at EM `0x400` = index 0, and both
//!    of its populated RX descriptors read label 0).
//!
//! With those four facts modelled the scan report reaches the application; see
//! the measured result below.
//!
//! Which descriptor is next is the register that used to be a mystery:
//! **`+0x024`** — `r_lld_update_rxbuf` reads it, masks `& 0x7FFF` and recovers
//! the index as `(ptr - 0x1000) / 20`. Every value ever sampled on the live
//! part is exactly `0x1000 + 20*n`, and the twin's own firmware seeds it with
//! `+0x024 <= 0x1000` during bring-up. The earlier note calling it "live but
//! not monotonic, so not a counter" was right that it is not a counter, and
//! the ROM now says what it is.
//!
//! ### What the model does with a pushed event
//!
//! At the programmed instant it writes status 2, walks
//! ET → CS → TX descriptor → payload buffer, emits the PDU onto the shared air
//! (see [`crate::peripherals::ble_air`]), then — if the air is carrying a frame
//! on the channel and access address this event's control structure programmed
//! and this controller did not itself transmit it — writes that frame into the
//! descriptor at `+0x024`, advances `+0x024` along the ring, and raises
//! `sch_prog_rx` (bit 2). After the programmed duration it writes status 3 and
//! raises `sch_prog_end` (bit 5) through the IRQ FIFO. It
//! raises **no** `sch_prog_tx` (bit 1), and the ROM is why:
//! `r_lld_adv_frm_cbk` (`0x4001_7550`) handles irq_type 0/1 (end), silently
//! ignores 2 (RX) and forwards 4 (skip), and **asserts on anything else** —
//! `r_sch_prog_tx_isr` passes 3. Raising bit 1 stopped the twin dead with
//! exactly that assert (`assert lld_adv.c 2328, param 00000000 00000003`), so
//! a legacy advertising event provably does not produce one on silicon.
//!
//! ### Measured result, two nodes in one world — the host report
//!
//! `labwired run --rom-boot` with `LABWIRED_BLE_DUAL=1` boots an advertiser and
//! a BLE scanner onto the shared air. Both stacks come up, node B's controller
//! receives node A's advertising PDU, and — since the descriptor ownership
//! protocol above is modelled — **node B's application sees it**. Real captured
//! serial, 400 M steps:
//!
//! ```text
//! [A] PRE_BLE / BLE_INIT_OK / ADV_ON / ALIVE …
//! [B] PRE_BLE / BLE_INIT_OK / SCAN_ON / ALIVE …
//! [B] SCAN_HIT 02:00:00:00:00:04 rssi=0 payload=020106051220004000
//! ```
//!
//! with the controller-level trace behind it:
//!
//! ```text
//! [bt<n>] radio RX ch39 aa=0x8e89bed6 rxd=0x1000 et=4 cs=0x0400 label=0
//!      pdu=20 0f 04 00 00 00 00 02 02 01 06 05 12 20 00 40 00
//! [bt<n>] +0x2d8 <= 0x0000103f      ← FIFO popped, head bitmap 0x4
//! [bt<n>] +0x018 <= 0x00000004      ← bit 2 W1C-acked
//! [bt<n>] radio: ET 4 end (clkn=7097)
//! ```
//!
//! `02:00:00:00:00:04` is node A's own BLE address (`04 00 00 00 00 02` LSB
//! first out of A's control structure) and `020106051220004000` is the
//! advertising payload A staged in *its* exchange memory, byte for byte, having
//! travelled A's exchange memory → the air → B's RX descriptor ring → `lld_scan`
//! → Bluedroid → `BLEAdvertisedDeviceCallbacks::onResult`. The descriptor
//! pointer walked the ring the firmware linked — `0x1000`, `0x1014`, `0x1028`,
//! `0x103C`.
//!
//! ### Measured result, two-way exchange between two nodes
//!
//! The same harness with BOTH nodes running one binary that advertises *and*
//! scans, putting `E5 02 <own tag> <counter>` in its manufacturer data (the tag
//! is the last byte of the node's own BLE address, so one image gives two
//! identities). Real captured serial:
//!
//! ```text
//! [ble] both nodes printed "PEER tag=" by step 44000000 — stopping
//! [A] BLE_INIT_OK / MYTAG 4 / ADV_ON / SCAN_ON / TICK 1
//! [A] PEER tag=5 val=0 from=02:00:00:00:00:05
//! [B] BLE_INIT_OK / MYTAG 5 / ADV_ON / SCAN_ON / TICK 1
//! [B] PEER tag=4 val=0 from=02:00:00:00:00:04
//! ```
//!
//! i.e. each twin's *application* read bytes the other's application chose —
//! connectionless advertising data carrying live state in both directions.
//! `crates/cli/tests/e2e_esp32c3_ble_two_node.rs` is the gate over it.
//!
//! Getting the second direction working needed one more thing, and it is worth
//! recording because the symptom was so misleading: a node that advertises AND
//! scans has two activities, and this model used to deliver an air frame into
//! **whichever event happened to be running**. An advertising event that
//! swallowed an `ADV_IND` stamped it with the advertising activity's link
//! label, `r_lld_scan_process_pkt_rx` rejected it as somebody else's, never
//! freed the descriptor, and the ring wedged on `RXDONE` — the node went
//! permanently deaf after one misdelivery, having reported an advertisement or
//! two first. Reception is now gated on the control structure's measured
//! scanning format; see [`CS_FORMAT_SCAN`] for what that costs.
//!
//! ### Measured result, CONTINUOUS re-publication — and the rule it taught
//!
//! One changed payload is not an application. The same node sketch re-publishes
//! forever — `adv->stop()`, `setAdvertisementData(counter)`, `adv->start()`,
//! `TICK n`, `delay(700)` — and that loop used to run exactly twice before the
//! twin died with the link layer's **own** assertion, ~198 M steps in.
//!
//! **Nothing was ever blocked, and it is worth being explicit about that**,
//! because "the sketch never reaches a second `loop()`" was the working theory
//! and it was wrong. `adv->stop()`, `setAdvertisementData()` and `adv->start()`
//! all return; a watch on the sketch's own call sites (`0x4200_039E` and the
//! four `jal`s inside it) has iteration 1 complete in ~44 k steps. `TICK n`
//! lands every **112 M steps**, which is the sketch's `delay(700)` at 160 M
//! steps per simulated second, to the FreeRTOS tick. The loop was on time. It
//! was the controller underneath it that died:
//!
//! ```text
//! [A] TICK 1                                              (step  42.4 M)
//! [A] TICK 2                                              (step 154.6 M)
//! [A] assert ble_util_buf.c 180, param 000000e2 00000205  (step 197.9 M)
//! [B] assert ble_util_buf.c 180, param 000000e2 00000204
//! ```
//!
//! ### The cause: a payload pointer masked with `0x7FFF`
//!
//! `ble_util_buf.c` 180 is `r_ble_util_buf_rx_free` (`0x4000_315C`) refusing a
//! buffer that is not in the RX pool: it computes `((buf - 0x7805) >> 10) & 0xFF`
//! and asserts it is `<= 8`. That range check *is* the pool's address map —
//! nine 1 KiB buffers whose data pointers are `0x7805, 0x7C05, 0x8005, 0x8405,
//! 0x8805, 0x8C05, 0x9005, 0x9405, 0x9805`, and the descriptor ring in the twin
//! walks exactly those. **Five of the nine are at or above `0x8000`.**
//!
//! This model masked the RX descriptor's `+0x12` — and the TX descriptor's
//! `+0x4`, and `CS+0x1C` — with `0x7FFF`, on the theory that bit15 was "a flag
//! whose meaning was not determined", because a live descriptor's `+0x0` read
//! `0x903C` where its neighbours read `0x103C`. That bit15 is [`RXD_DONE`] and
//! it belongs to `+0x0`. Every ROM site that reads a payload pointer
//! (`0x4002_46EE`, `0x4002_4878`) zero-extends the halfword with no mask at all.
//!
//! So the mask folded the top five pool buffers onto the bottom 8 KiB of
//! exchange memory, and the model wrote received advertising payloads into it:
//!
//! ```text
//! 0x8005 -> 0x0005   the EXCHANGE TABLE
//! 0x8405 -> 0x0405   the CONTROL STRUCTURES
//! 0x9005 -> 0x1005   the RX DESCRIPTOR RING ITSELF
//! ```
//!
//! The last one closes the loop: bytes 13 and 14 of a 15-byte legacy `ADV_IND`
//! land on descriptor 0's own `+0x12`, and in the two-node run those two bytes
//! are the peer's `<tag> <counter>`. Hence `0x0205` on node A and `0x0204` on
//! node B — the same counter, the two different tags — and hence a free of a
//! buffer that was never a buffer. It took ~198 M steps because the ring has to
//! reach its third buffer first, which is why no unit test and neither earlier
//! gate had ever seen it. See [`RXD_DATA_PTR`].
//!
//! ### The second defect it was hiding, and the rule behind it
//!
//! Fixing only the descriptor *fields* first made the twin die one assert
//! earlier instead (`assert emi.c 159, param 0000ff33 0000003f` —
//! `r_emi_get_mem_addr_by_offset` refusing an offset whose 1 KiB index is 63),
//! which is how `+0xE` came to be understood: it is the resolving-address-list
//! pointer, `r_lld_scan_process_pkt_rx_adv_rep` copies it into the advertising
//! report, and ESP-IDF's handler dereferences it as an exchange-memory pointer
//! whenever it is non-zero. In the failing runs it was non-zero *because* the
//! aliased payload writes had scribbled on the descriptor ring — so the pointer
//! mask was the single root cause of both asserts.
//!
//! The field writes stay anyway, and the reason generalises past this bug:
//! **a hardware-owned field the model leaves alone is not *unmodelled*, it is
//! *stale*, and firmware cannot tell the difference.** The model now writes
//! **every** core-owned RX-descriptor field on every reception — `+0x0` bit15,
//! `+0x2`, `+0x4`, `+0x6`, `+0x8`, `+0xC`, `+0xE`, `+0x10` — with the ones it
//! has nothing to say about explicitly zeroed and each one's status stated at
//! its constant ([`RXD_RSSI`], [`RXD_RAL_PTR`], [`RXD_UNKNOWN_10`]). Only
//! `+0x0` bits[14:0] and `+0x12`, which the ROM's own refill paths own, are
//! left alone. Reading the ROM for that also *identified* `+0x6`: it is the raw
//! RSSI byte `rf_api.rssi_convert` is fed, which the earlier pass had recorded
//! as "looks like it could carry RSSI — nothing measured says so".
//!
//! ### Measured result
//!
//! Real captured serial, both nodes, no assertion anywhere in the run:
//!
//! ```text
//! TICK 1  42.4 M   TICK 2 154.6 M   TICK 3 266.8 M   TICK 4 379.7 M   TICK 5 491.9 M
//! [A] PEER tag=5 val=0 / 1 / 2 / 3 / 4 / 5 from=02:00:00:00:00:05
//! [B] PEER tag=4 val=0 / 1 / 2 / 3 / 4 / 5 from=02:00:00:00:00:04
//! ```
//!
//! i.e. each node's *application* read six successive values the other's
//! application chose, one per `loop()` iteration — a data stream, not a
//! payload. `crates/cli/tests/e2e_esp32c3_ble_adv_republish.rs` is the gate
//! over it (162 s), and it fails on `assert ` appearing in the serial at all —
//! the sharpest oracle this stack has, and the one that caught both defects.
//!
//! **What the advertising stop does NOT do here, stated plainly.**
//! `r_lld_adv_stop` (`0x4001_898A`), when the activity has an event in flight
//! (state 1 — set at the tail of `r_lld_adv_evt_start_cbk` `0x4001_696E`, i.e.
//! the instant the event is handed to `sch_prog_push`), writes `CS+0x20 = 1`
//! and sets **`RWBLECNTL` bit 25** before moving the activity to state 2.
//!
//! Bit 25 is now **cleared** on write, along with bit 24 — they are abort
//! *requests* the hardware consumes, and latching them made the twin hand the
//! firmware a control word silicon cannot produce. The ROM sites, the silicon
//! read-backs and the twin trace that pin that are all at
//! [`RWBLECNTL_SELF_CLEARING`].
//!
//! What is still **not** modelled is the abort's *effect*: ending the event
//! that is already programmed. Two things about it are now settled that were
//! not before, and one is still open.
//!
//! * Settled — **how silicon reports an aborted event.**
//!   `r_sch_prog_end_isr` (`0x4003_0AB8`) reads the exchange-table status
//!   `(ET+0x0 >> 3) & 7`, dispatches 3/4/5, and computes the callback's third
//!   argument as `seqz(status - 4)` (`0x4003_0B52`) — i.e. **`irq_type = 1`
//!   exactly when the status is 4**. `r_lld_adv_frm_cbk` (`0x4001_7550`) then
//!   routes `irq_type` 1 to `lld_adv_frm_isr(act, ts, 1)` and 0 to
//!   `lld_adv_frm_isr(act, ts, 0)`. So **status 4 is the abort encoding** — one
//!   of the "status codes 1, 4, 5, 6 are named nowhere that was measured" gaps
//!   listed below, now closed from the ROM. (For the *stop* path specifically
//!   the distinction is inert: `lld_adv_frm_isr` branches on state 2 at
//!   `0x4001_6FF8` before it ever looks at the abort flag.)
//! * Settled — **what it costs not to do it**, measured rather than argued.
//!   `LABWIRED_BT_TRACE=1`, two nodes, 150 M cycles/node ≈ CLKN 0..2840
//!   (0.89 s), the published BLE Pong sketch rebuilt at two cadences.
//!
//!   At the sketch's own **~51 ms** — and `PUBLISH_MS` 20 and 40 both land
//!   there, because one iteration of that `loop()` costs ~51 ms of device time
//!   in the twin (49 status lines per 400 M cycles) and the SSD1306 repaint is
//!   nearly all of it — `r_lld_adv_stop` took the state-1 branch **4 times on
//!   node A and 3 on node B**, roughly one publish in four. Every one was
//!   followed by the ordinary event end **0–10 CLKN ticks later (0–3.1 ms)**,
//!   which is the whole of the delay the missing abort adds.
//!
//!   With the panel removed the loop runs at 6 ms and the achieved cadence is
//!   **24 ms** (`pub=103` at `t=2494`). There the branch is taken **zero**
//!   times, across 212 programmed events per node: each stop lands a few ms
//!   *before* its activity's next programmed event, so it takes the
//!   synchronous state-0 path at `0x4001_89E6` and never touches bit 25.
//!   Republishing faster is not more likely to catch an event in flight — it
//!   phase-locks the stop ahead of one — which is worth stating because the
//!   opposite was the working theory.
//!
//!   Neither cadence stalls. Air frames stay at 3–5 (51 ms) and 5–6 (24 ms)
//!   per 20 M cycles per node right to the end of a 400 M-cycle two-node run,
//!   the host election settles, and at 24 ms both nodes agree on `score=7:6`
//!   with the guest's ball identical to the host's.
//! * Open — **whether the core also discards the event's transmission.** In
//!   this model a legacy advertising event puts its PDU on the air when it
//!   *starts*, so truncating a `Pending` one throws away a frame silicon has
//!   already sent. That is not a theory: core#772 tried exactly that and
//!   silenced both nodes.
//!
//! So the abort's effect stays unmodelled **on the measurement**, not on
//! ignorance: at ≤3.1 ms per stop it changes nothing observable, while
//! synthesising an event ending has a merged-and-reverted-the-same-day track
//! record. Two independent defects in that attempt are worth recording so the
//! next one does not inherit them: it keyed on the 0→1 edge of a bit this model
//! then latched (so it could fire at most once per boot — see
//! [`RWBLECNTL_SELF_CLEARING`]), and it shipped `self.rx_cursor = frame.seq`
//! in `receive_event`, which re-delivers one frame forever and makes every
//! receiver in the world deaf. Any measurement taken over that tree says
//! nothing about the abort.
//!
//! The in-flight event therefore still runs to its programmed duration and
//! `r_lld_adv_frm_isr` finds state 2 and completes the stop through
//! `r_lld_adv_end` → `lld_adv_end_ind_handler` exactly as it does on silicon.
//! The traces above are of a stop that completes that way, every iteration.
//!
//! ### Measured result (single node, 500 M steps, `LABWIRED_BT_TRACE=1`)
//!
//! `PRE_BLE / BLE_INIT_OK / ADV_ON / ALIVE`, no asserts, 810 BT register
//! writes, and **19 complete radio events**, cycling exchange-table entries
//! `0..15` and wrapping to `0, 1, 2` — the real 16-deep ring. Each ends with
//! the exact ROM ack sequence (`+0x2D8 <= 0x0000_803F` pop, `+0x018 <= 0x20`
//! W1C) and is followed by the controller re-arming the half-µs comparator for
//! the next advertising instant. End-to-end intervals were 130–158 CLKN ticks
//! (40.6–49.4 ms), against 119–150 ticks (37–47 ms) measured on the live part —
//! the same advertising interval with the same random delay on top. Every one
//! of the 19 frames carried
//! `20 0f <AdvA> 02 01 06 05 12 20 00 40 00`: header `0x20` = ADV_IND with
//! ChSel, 15 bytes, and an advertising payload byte-identical to the
//! `ADVDATATXBUF` read off the live board.
//!
//! ### Radio-event gaps still open (say so rather than invent)
//!
//! * **Channel sweeping.** A real legacy advertising event transmits on every
//!   enabled primary advertising channel (37/38/39) within the single
//!   programmed event; this model emits **one** frame per event, on the channel
//!   the control structure's hop word names. Sampling `CS+0x16` six times
//!   across three seconds of continuous advertising on board
//!   `38:44:be:42:f5:58` returned `0x8027` — channel 39 — every single time, so
//!   the firmware demonstrably does not rewrite the hop word between events and
//!   whatever field the core sweeps from was not identified. Sweeping is
//!   therefore idealised away rather than faked, and the practical consequence
//!   is that an advertiser and a scanner in one world only meet on the
//!   scanner's channel-39 dwell.
//! * **`+0x32C`.** The controller writes `0x4000_000D` here immediately before
//!   every push. No ROM routine touches this offset (it comes from ESP-IDF's
//!   IRAM `libble_app`), so its meaning is unknown. Plain storage.
//! * **RX descriptor fields `+0x2` (beyond the ROM's error mask and its bit15),
//!   `+0x6` and `+0xC` bits[10:0].** The live values (`0x8040`,
//!   `0x26B6`/`0x27BD`, `0xED`/`0x1D`) were captured but only what the ROM
//!   itself reads is interpreted. `+0x6` looks like it could carry RSSI —
//!   nothing measured says so, and the ROM's own legacy-advertising handler
//!   fabricates a constant `0x7F04` instead of reading it, so this model leaves
//!   `+0x6` at zero.
//! * **The RX-buffer *reconfiguration* path is modelled only as far as its
//!   handshake.** The `+0x2D0` bit15 rising edge raises bit 18 and the core
//!   adopts the requested descriptor (see [`RX_BUF_JUMP`]), which is what
//!   `r_lld_update_rxbuf` and its ISR demand of each other. What is NOT modelled
//!   is any hardware effect of the *size*/*count* the call is actually changing
//!   (`lld_update_rxbuf(SZ, NB)`), because nothing here reads a buffer size —
//!   the payload length comes off the air frame. `+0x2D4` bits[8:0], which
//!   `lld_update_rxbuf_handler` fills with a computed "RX MAX LENGTH", is plain
//!   storage for the same reason. Neither probe firmware exercised this path at
//!   all (zero bit-18 raises across the two-node runs), so it is implemented
//!   from the ROM and **not** confirmed by an end-to-end run.
//! * **One unexplained host-level artifact.** The single-node scanner probe
//!   (`lw-scanner`, whose source is not in this tree) emitted, alongside its
//!   correct `SCAN_HIT 02:00:00:00:00:04 … payload=020106051220004000`, one
//!   extra `SCAN_HIT 00:00:00:00:00:00 … payload=000000000000000000`. Nothing
//!   was found that produces it: the model never writes a descriptor it did not
//!   fill (it refuses one whose `RXDONE` is set), the descriptor ring
//!   demonstrably self-heals (`r_lld_rxdesc_free` refills index `[217]+1` eight
//!   slots ahead of the `[216]` it advances, so the one buffer-less descriptor
//!   is always replenished before the cursor reaches it), and the two-way node
//!   run over the same engine produced only well-formed reports. It is recorded
//!   here **unexplained** rather than explained away.
//! * **Status codes 1, 4, 5, 6** and the `ET+0xE` field are named nowhere that
//!   was measured, so they are neither written nor interpreted. `ET+0x0`
//!   bits[15:11] (`min(sch_prog_params[20], 31)`) is likewise written by
//!   firmware and read by nothing here — note it is NOT the link label, which
//!   is the control-structure index (`sch_prog_params[24]`).
//! * **Event skipping and cancellation.** `sch_prog_skip` (bit 6) is never
//!   raised: every pushed event runs.
//!
//! ## Interrupt-path gaps still open (say so rather than invent)
//!
//! * **Bits 0 and 4 read raw-set on live silicon** (`INTRAWSTAT = 0x11`, then
//!   `0x811`) and this model never sets them, because nothing here knows what
//!   *raises* them. Both are masked off in `INTCNTL`, so they cannot reach the
//!   CPU either way. Bit 0 at least has a plausible story — its handler slot
//!   is `r_rwip_wakeup_end` and this model has no sleep path — but "which
//!   hardware condition sets the latch" was not measured, so it is not
//!   modelled. Bit 4 has no handler in either ISR at all and no story
//!   whatsoever. Recorded, not faked.
//! * **Bits 1, 6 and 19 (`sch_prog_tx` / `sch_prog_skip` / `ble_sw_cca_check`)
//!   are still never raised.** Bit 1 is a deliberate, ROM-attested omission for
//!   advertising (see the programmed-event section); bits 6 and 19 have no
//!   modelled cause. Bit 2 (`sch_prog_rx`) and bit 18 (`lld_update_rxbuf`) now
//!   have real, ROM-attested causes and are raised.
//! * **The comparator edge rule** was not measured. This model fires on
//!   "reached or passed" (28-bit wrapping compare) rather than strict
//!   equality, because a strict-equality comparator that misses its instant
//!   would deadlock the controller for a full 28-bit wrap (~23 h of device
//!   time). The two are indistinguishable whenever the deadline is not missed.

use crate::peripherals::ble_air::{default_ble_air_bus, BleAirBus, BleAirFrame};
use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

/// Block size. The watchpoint that produced the write trace covered
/// `0x60031000 + 0x1000` and caught every store BLE bring-up made; the highest
/// offset it ever touched is `+0x530`. Silicon reads `0x6003_2000` (and
/// `0x6003_0000`, `0x6002_F000`, `0x6002_E000`) as all-zero with BLE up, so the
/// live block ends before the next window and 4 KiB is the honest extent.
pub const BT_SIZE: u64 = 0x1000;

/// Base address of the block (between `i2s0` at `0x6002_D000` and the WiFi MAC
/// at `0x6003_3000`).
pub const BT_BASE: u64 = 0x6003_1000;

/// `BASETIMECNT` — the Bluetooth native clock. READ = the free-running
/// 312.5 µs counter; WRITE = a control write (`0x8000_0000` set, low bits
/// tracking the clock — see the module docs) that must NOT shadow the read.
const CLKN: u64 = 0x01C;
/// `FINETIMECNT` — sub-CLKN fine counter, `0..=624` at 2 MHz (half-µs), read
/// straight after CLKN by the BT ROM's event scheduler.
const CLKN_FINE: u64 = 0x020;

/// Top bit of a `CLKN` write — the sample-latch request. `r_rwip_time_get`
/// sets it and then spins while the read-back is negative, so the hardware
/// clears it when the coherent base/fine pair is ready. Modelled by never
/// letting the write reach the read: the sample is always ready.
const CLKN_TARGET_ARM: u32 = 0x8000_0000;

/// CLKN is 28 bits: `r_rwip_time_get` masks the counter with this, and
/// `r_rwip_timer_hus_set` asserts on a target with any bit above it set.
const CLKN_MASK: u32 = 0x0FFF_FFFF;

/// `INTCNTL` — interrupt enable mask.
const INTCNTL: u64 = 0x00C;
/// `INTSTAT` — masked status. Derived: `INTRAWSTAT & INTCNTL` (measured).
const INTSTAT: u64 = 0x010;
/// `INTRAWSTAT` — raw status latch.
const INTRAWSTAT: u64 = 0x014;
/// `INTACK` — write-1-to-clear on the raw latch. Reads 0 on silicon.
const INTACK: u64 = 0x018;

/// 10 ms timer comparator target, in units of 32 CLKN ticks
/// (`r_rwip_timer_10ms_set`). Raises bit 9.
const TIMER_10MS_TARGET: u64 = 0x0E4;
/// Half-slot timer comparator target, in CLKN ticks (`r_rwip_timer_hs_set`).
/// Raises bit 10.
const TIMER_HS_TARGET: u64 = 0x0E8;
/// Half-µs timer comparator: base-time target in CLKN ticks
/// (`r_rwip_timer_hus_set`). Paired with [`TIMER_HUS_FINE`]. Raises bit 11.
const TIMER_HUS_TARGET: u64 = 0x0EC;
/// Half-µs timer comparator: fine target, written as `624 - hus`, compared
/// against `FINETIMECNT` directly.
const TIMER_HUS_FINE: u64 = 0x0F0;

/// The IRQ FIFO the ROM ISR reads when `sdk_cfg_priv_opts[69] != 0` (it does,
/// on this silicon). See the module docs for the field layout.
const IRQ_FIFO: u64 = 0x2D8;
/// Secondary `INTACK` alias: the ROM mirrors every ack it writes to `+0x018`
/// here whenever the FIFO path is selected (and the timer *disable* paths
/// write only here). Reads 0 on silicon, like `INTACK`.
const INTACK_FIFO: u64 = 0x38C;

/// Interrupt bits this model can actually raise — the three timer comparators.
const INT_TIMER_10MS: u32 = 1 << 9;
const INT_TIMER_HS: u32 = 1 << 10;
const INT_TIMER_HUS: u32 = 1 << 11;

/// `r_sch_prog_rx_isr` — a programmed event received a frame.
const INT_SCH_PROG_RX: u32 = 1 << 2;
/// `r_sch_prog_end_isr` — a programmed radio event ended.
const INT_SCH_PROG_END: u32 = 1 << 5;
/// `r_lld_update_rxbuf_isr` — the core has taken the RX-descriptor "jump"
/// request software parked in [`RX_BUF_JUMP`]. See the RX-buffer lifecycle
/// section of the module docs for the ROM evidence.
const INT_LLD_UPDATE_RXBUF: u32 = 1 << 18;

// ── The programmed-event interface (see the module docs) ─────────────────────

/// `+0x100` — the programmed-event push register. `r_sch_prog_ble_push`
/// (`0x4003_0BDA`) ends with `s2 |= 0x8000_0000; sw s2, 256(0x6003_1000)`
/// after asserting the index is 4 bits wide, and the live part reads it back
/// as 0 (silicon capture 2026-08-02, board `38:44:be:42:f5:58`), i.e. it is a
/// self-clearing command register, not storage.
const PROG_PUSH: u64 = 0x100;
/// Bit31 of a `PROG_PUSH` write — "execute the entry named in bits[3:0]".
const PROG_PUSH_GO: u32 = 0x8000_0000;
/// Index field of a `PROG_PUSH` write. `r_sch_prog_ble_push` asserts
/// `idx & ~0xF == 0` (`sch_prog.c` line 6648) — the exchange table is 16 deep.
const PROG_PUSH_IDX: u32 = 0xF;

/// Exchange-memory base registers, bank A (`reg_idx` 0..=47).
/// `r_emi_get_mem_addr_by_offset` (`0x4000_6976`) computes the register
/// address as `(0x1800_C481 + reg_idx) << 2` = `0x6003_1204 + reg_idx*4`.
const EM_BASE_REG_BANK_A: u64 = 0x204;
/// Exchange-memory base registers, bank B (`reg_idx` 48..=55): the same ROM
/// routine switches to `(0x1800_C488 + reg_idx) << 2` = `0x6003_1220 + idx*4`.
/// Every bank-B register read 0 on the live part; the bank exists in the ROM's
/// arithmetic, so it is honoured here, but nothing was ever measured in it.
const EM_BASE_REG_BANK_B: u64 = 0x220;
/// Highest `reg_idx` in bank A.
const EM_BASE_REG_BANK_A_MAX: u64 = 47;
/// Highest `reg_idx` the ROM will accept at all (`assert reg_idx <= 55`).
const EM_BASE_REG_MAX: u64 = 55;
/// The data-RAM window every exchange-memory base register resolves into:
/// `r_emi_get_mem_addr_by_offset` finishes with
/// `((reg << 2) & 0x000F_FFFC) | 0x3FC0_0000`.
const EM_RAM_WINDOW: u64 = 0x3FC0_0000;
/// Address mask the same routine applies (`0x0010_0000 - 4`).
const EM_RAM_ADDR_MASK: u32 = 0x000F_FFFC;

/// Exchange-memory offset of the **exchange table (ET)**. `r_sch_prog_init`,
/// `r_sch_prog_push`, `r_sch_prog_ble_push` and all four `sch_prog` ISRs reach
/// it through `r_plf_funcs_p[47](0)`, i.e. `emi_get_mem_addr_by_offset(0)`.
const EM_ET_OFFSET: u32 = 0x000;
/// Stride of one ET entry. Every ROM site indexes it as `idx << 4`
/// (`r_sch_prog_init` walks `0..256` step 16 = 16 entries).
const ET_ENTRY_BYTES: u32 = 16;
/// Number of ET entries — `r_sch_prog_ble_push` asserts the index is 4 bits.
const ET_ENTRIES: u32 = 16;

/// ET entry `+0x0`: the control/status halfword. Bits[5:3] are the status the
/// hardware owns; the rest is written by `r_sch_prog_ble_push`.
const ET_CTRL: u32 = 0x0;
/// ET entry `+0x2`/`+0x4`: the 28-bit half-slot (CLKN) start time, low then
/// high. `r_sch_prog_push` writes `time & 0xFFFF` and `(time >> 16) & 0xFFF`.
const ET_START_LO: u32 = 0x2;
const ET_START_HI: u32 = 0x4;
/// ET entry `+0x6`: the fine start offset, written as `624 - hus` — the same
/// down-counting encoding as the `+0x0F0` half-µs comparator.
const ET_START_FINE: u32 = 0x6;
/// ET entry `+0x8`: the control-structure pointer, as an exchange-memory byte
/// offset divided by two. `r_sch_prog_ble_push` writes
/// `(cs_idx * 90 + 1024) >> 1`, i.e. `(EM_BLE_CS_OFFSET + cs_idx * CS_SIZE)/2`.
const ET_CS_PTR: u32 = 0x8;
/// ET entry `+0xA`: the event duration. `r_sch_prog_push` writes
/// `(dur + 1) >> 1` for `dur < 0x8000` (units of two half-µs) and
/// `((dur + 625) / 625) | 0x8000` above that (units of half-slots).
const ET_DURATION: u32 = 0xA;
/// Bit15 of [`ET_DURATION`]: the unit selector.
const ET_DURATION_HALF_SLOTS: u16 = 0x8000;

/// Shift of the 3-bit hardware status field in [`ET_CTRL`]. Every `sch_prog`
/// ISR reads it as `(lhu(et) >> 3) & 7`.
const ET_STATUS_SHIFT: u32 = 3;
const ET_STATUS_FIELD: u16 = 0x7 << ET_STATUS_SHIFT;
/// Status the model writes while an event is executing. Inferred, and flagged
/// as such: `r_sch_prog_push` rejects an entry whose status is 1 or 2 as still
/// in use, while `r_sch_prog_tx_isr`/`_rx_isr` assert `status & 6 != 0` — so
/// the state an event is in while it transmits and receives has to be 2.
const ET_STATUS_ONGOING: u16 = 2;
/// Status the model writes when the event ends. `r_sch_prog_end_isr` dispatches
/// the frame callback for status 3, 4 or 5 and passes `irq_type = (status == 4)`
/// — 3 is therefore the plain end, and it is also what `r_sch_prog_init` seeds
/// every entry with. Silicon confirms: all 16 live entries read `+0x0 = 0x281A`,
/// whose status field is 3.
const ET_STATUS_END: u16 = 3;

/// Exchange-memory offset of the first **control structure**, and the stride
/// between them. `r_sch_prog_ble_push` writes `ET+0x8 = (cs_idx*90 + 1024) >> 1`
/// — both constants come straight out of that one instruction sequence, and the
/// live part confirms the stride independently (per-structure markers 90 bytes
/// apart).
const EM_CS_OFFSET: u32 = 1024;
const CS_STRIDE: u32 = 90;

/// Control-structure `+0x06`: the 6-byte device address, LSB first.
const CS_BDADDR: u32 = 0x06;
/// Control-structure `+0x0C`: the 32-bit access address (sync word).
const CS_ACCESS_ADDR: u32 = 0x0C;
/// Control-structure `+0x10`: the 24-bit CRC init.
const CS_CRC_INIT: u32 = 0x10;
/// Control-structure `+0x16`: the hop-control word; bits[6:0] are the RF
/// channel index.
const CS_HOP_CTRL: u32 = 0x16;
/// Channel-index field of [`CS_HOP_CTRL`].
const CS_CHANNEL_MASK: u16 = 0x7F;
/// Control-structure `+0x1C`: exchange-memory offset of the first TX
/// descriptor, or 0 for an event that does not transmit.
const CS_TX_DESC_PTR: u32 = 0x1C;
/// Control-structure `+0x00`: the format word. Only the value measured on the
/// live advertising part is acted on — see [`CS_FORMAT_LEGACY_ADV`].
const CS_FORMAT: u32 = 0x00;
/// Low byte of [`CS_FORMAT`] on the live, legacy-advertising part
/// (`+0x00 = 0x0404`). The model transmits only for this format; any other
/// value is traced and left alone rather than guessed at.
const CS_FORMAT_LEGACY_ADV: u16 = 0x04;
/// Low byte of [`CS_FORMAT`] for a **scanning** activity: `0x0208`, which is
/// what the real Arduino `BLEScan` firmware programs in the twin (756 events in
/// one two-node run, every one of them a control structure with no TX
/// descriptor). Measured from firmware rather than from silicon — no scanning
/// capture off the board exists — and used only to *narrow* what the model
/// does, never to widen it.
///
/// **Why receive is gated on it.** An air frame is delivered only to an event
/// whose control structure is programmed in this format. Delivering to any
/// event that happened to be running (which is what this model did before) is
/// not merely imprecise, it desynchronises the link layer: the descriptor is
/// stamped with the *running* activity's link label, so when an advertising
/// event picks up an `ADV_IND` meant for the scanner, `r_lld_rxdesc_check`
/// rejects it on the label, never frees it, and the descriptor stays `RXDONE`
/// forever — the ring wedges after exactly one such misdelivery. That is
/// precisely what happened to a node doing both at once: it reported a couple
/// of advertisements and then went permanently deaf.
///
/// The cost of the narrowing, stated plainly: an advertising event's own
/// receive window (`SCAN_REQ` / `CONNECT_IND` addressed to the advertiser) is
/// NOT modelled. Nothing on this air ever transmits one — the model only
/// transmits for [`CS_FORMAT_LEGACY_ADV`], and a passive scanner sends
/// nothing — so today the narrowing loses no traffic that exists. It would have
/// to be revisited to model active scanning or connection setup.
const CS_FORMAT_SCAN: u16 = 0x08;

/// TX descriptor `+0x2`: `(payload_len << 8) | pdu_header_byte`.
const TXD_HEADER: u32 = 0x2;
/// TX descriptor `+0x4`: exchange-memory offset of the payload bytes that
/// follow the device address.
const TXD_DATA_PTR: u32 = 0x4;
/// Bytes of the advertiser's device address the core inserts ahead of the
/// descriptor's own buffer, taken from [`CS_BDADDR`]. Measured on both live
/// descriptors: `ADV_IND` declared 15 bytes over a 9-byte buffer, `SCAN_RSP`
/// declared 25 over a 19-byte buffer.
const TXD_ADDR_PREFIX_LEN: u16 = 6;

/// `+0x024` — `CURRENTRXDESCPTR`, the exchange-memory offset of the RX
/// descriptor the core will fill next. `r_lld_update_rxbuf` (`0x4002_13C0`)
/// reads it, masks `& 0x7FFF`, and recovers the descriptor index as
/// `(ptr - 0x1000) / 20`. Every value ever sampled on the live part
/// (`0x1000, 0x1050, 0x1064, 0x108C, 0x10A0, 0x10B4`) is exactly
/// `0x1000 + 20*n` — which is why this register looked like a non-monotonic
/// mystery counter before the ROM named it.
const RX_DESC_PTR: u64 = 0x024;
/// Address field of [`RX_DESC_PTR`] (`r_lld_update_rxbuf` masks with this).
const RX_DESC_PTR_MASK: u32 = 0x7FFF;

/// `+0x2D0` — the RX-descriptor **jump request**. `r_lld_update_rxbuf`
/// (`0x4002_13C0`) is `lld_update_rxbuf(rx_buf_size, rx_buf_nb)` (its own
/// failure log is `"RXBUF Update failed, SZ %d, NB %d!"`, and it range-checks
/// `size <= 272`, `1 <= nb <= 9`), and it ends with exactly two stores here:
///
/// ```text
/// +0x2D0 = (+0x2D0 & 0xFFFF8000) | (0x1000 + idx*20)   // bits[14:0] = target
/// +0x2D0 = (+0x2D0 & 0xFFFF7FFF) | 0x8000              // bit15 = go
/// ```
///
/// having first set `p_lld_env[257] = 1`. `r_lld_update_rxbuf_isr`
/// (`0x4002_1586`) — the bit-18 handler — logs `+0x2D0` and `+0x024`
/// (`"RXBUF Update ISR %04x %04x"`), **clears bit15**, and defers
/// `lld_update_rxbuf_handler`, which re-seeds `p_lld_env[216]` from `+0x2D0`,
/// re-walks the whole 10-deep ring handing out fresh buffers, and clears
/// `p_lld_env[257]`.
///
/// So bit15 is a request/acknowledge handshake owned by software on the 0→1
/// edge and by the ISR on the 1→0 edge, and **that edge is what raises
/// interrupt bit 18** — the one causal link the ROM states. It is a
/// buffer-pool reconfiguration path, not part of the per-packet receive path.
const RX_BUF_JUMP: u64 = 0x2D0;
/// Descriptor-offset field of [`RX_BUF_JUMP`].
const RX_BUF_JUMP_PTR_MASK: u32 = 0x7FFF;
/// Bit15 of [`RX_BUF_JUMP`] — the go bit whose rising edge raises bit 18.
const RX_BUF_JUMP_GO: u32 = 0x8000;

/// Exchange-memory offset of the RX descriptor array. ROM constant:
/// `r_lld_scan_process_pkt_rx` and `r_lld_scan_process_pkt_rx_legacy_adv` both
/// call `emi_get_mem_addr_by_offset(0x1000)` and index it `idx * 20`.
const EM_RX_DESC_OFFSET: u32 = 0x1000;
/// Stride of one RX descriptor (`mul idx, 20` at both sites).
const RX_DESC_BYTES: u32 = 20;

/// RX descriptor `+0x0`: next descriptor, exchange-memory offset in
/// bits[14:0], plus [`RXD_DONE`] in bit15.
const RXD_NEXT: u32 = 0x0;
/// Pointer field of [`RXD_NEXT`].
const RXD_NEXT_PTR_MASK: u16 = 0x7FFF;
/// RX descriptor `+0x0` bit15 — **`RXDONE`**, and the ROM names it: the trace
/// `r_lld_update_rxbuf` prints is
/// `"RXBUF Update RXDESC: Current %04x[%d], RD %d; Jump %04x[%d], RD %d, NextPTR %04x"`
/// and the argument behind each `RD %d` is `lhu(rxdesc + 0) >> 15` (the
/// `NextPTR` argument is the same halfword masked with `0x7FFF`). It is the
/// ownership bit of the descriptor ring, and every ROM site agrees on the
/// direction:
///
/// * `r_lld_rxdesc_check` (`0x4002_022C`) reports a packet to the link layer
///   ONLY when this bit is **set** on descriptor `p_lld_env[216]` — the core
///   sets it when it has finished writing a reception.
/// * the buffer-refill paths (`r_lld_rxdesc_check`, `r_lld_rxdesc_free`
///   `0x4001_FFE8`, `lld_update_rxbuf_handler` `0x4002_15EC`) **clear** it right
///   after storing a fresh buffer offset in `+0x12`, i.e. clear = "handed to the
///   core, free to fill"; and `lld_update_rxbuf_handler` **sets** it on every
///   descriptor it could not give a buffer to, i.e. set = "the core must not
///   write here".
///
/// So the core owns the 0→1 edge and firmware owns the 1→0 edge. This model
/// honours both: it refuses to fill a descriptor whose `RXDONE` is already set
/// (that reception has not been consumed yet) and sets the bit last, after the
/// payload and every other field, so firmware can never observe a half-written
/// descriptor.
const RXD_DONE: u16 = 0x8000;
/// RX descriptor `+0x2`: the reception status word. `r_lld_scan_process_pkt_rx`
/// rejects the packet when `status & 0x402D != 0`.
const RXD_STATUS: u32 = 0x2;
/// The bad-packet mask the ROM applies to [`RXD_STATUS`] (`s7 = 0x402D`).
const RXD_STATUS_ERROR_MASK: u16 = 0x402D;
/// Bit15 of [`RXD_STATUS`] — the *software*-owned "released" marker, NOT part
/// of what a reception leaves behind. `r_lld_rxdesc_check` finishes with
/// `return (lhu(rxdesc + 2) >> 15) ^ 1`, i.e. it reports a packet only while
/// this bit is **clear**; `r_lld_rxdesc_free` and `lld_update_rxbuf_handler`
/// both set it when they release a descriptor. A reception the core left with
/// bit15 set could therefore never be reported to the host on real silicon, so
/// the core must write it clear.
///
/// This is why the live capture read `0x8040` on every populated descriptor and
/// why that value is NOT what the hardware wrote: the part was halted long
/// after its own link layer had processed and released those receptions. The
/// model writes [`RXD_STATUS_GOOD`] — the same measured word with this
/// software-owned bit taken back out.
const RXD_STATUS_RELEASED: u16 = 0x8000;
/// The status word this model leaves after a **good** reception: the live
/// measurement `0x8040` minus the software-owned [`RXD_STATUS_RELEASED`] bit
/// (see above). `0x0040 & 0x402D == 0`, so the ROM's own error test passes on
/// it — what bit 6 means was not determined, it is simply the bit silicon had
/// set.
const RXD_STATUS_GOOD: u16 = 0x8040 & !RXD_STATUS_RELEASED;
/// The good-reception word must clear the ROM's own bad-packet test, or every
/// frame this model delivers would be dropped by `r_lld_scan_process_pkt_rx`
/// before firmware ever saw it — and it must clear the released bit, or
/// `r_lld_rxdesc_check` would never report it.
const _: () = assert!(RXD_STATUS_GOOD & RXD_STATUS_ERROR_MASK == 0);
const _: () = assert!(RXD_STATUS_GOOD & RXD_STATUS_RELEASED == 0);
/// RX descriptor `+0x4`: `(payload_len << 8) | pdu_header_byte`, the exact
/// mirror of [`TXD_HEADER`]. `r_lld_scan_process_pkt_rx` takes the PDU type
/// from `lhu & 0xF`; `r_lld_scan_process_pkt_rx_legacy_adv` takes the
/// advertising-data length from `(lhu >> 8) - 6` and the address type from
/// bit 6. Live: `0x0C03` — a 12-byte `SCAN_REQ` (type 3), i.e. ScanA + AdvA.
const RXD_HEADER: u32 = 0x4;
/// RX descriptor `+0x6`: **the raw RSSI byte**, and the ROM says so outright.
/// `r_lld_scan_process_pkt_rx_adv_rep` (`0x4002_482C`) does, at `0x4002_49C6`:
///
/// ```text
/// s2 += 6                                  // rxdesc + 6
/// a0 = lhu(emi(0x1000) + s2) & 0xFF        // the low byte only
/// jalr *(0x3FCD_FBE8)                      // rwip_rf + 0x20 = rf_api.rssi_convert
/// sb a0, 116(lld_scan_env[act])            // -> the advertising report's RSSI
/// ```
///
/// The earlier pass looked only at `r_lld_scan_process_pkt_rx_legacy_adv`,
/// which fabricates a constant `0x7F04` into a *different* field
/// (`sh 0x7F04, 114(env)`), and concluded "+0x6 looks like it could carry
/// RSSI — nothing measured says so". The report builder settles it.
///
/// This model has **no PHY and therefore no RSSI** (see the module docs), so it
/// writes **zero** here — which is what the twin's own advertising reports have
/// always carried (`SCAN_HIT … rssi=0`). What it must NOT do is leave the
/// halfword at whatever the RAM under exchange memory happened to hold; see
/// [`RXD_RAL_PTR`] for what leaving a core-owned field stale costs.
const RXD_RSSI: u32 = 0x6;
/// RX descriptor `+0xE`: **the resolving-address-list (RAL) entry pointer** the
/// core writes when it resolved the sender's private address — an exchange-memory
/// offset, or **0 for "no resolution"**.
///
/// This one is load-bearing and cost the whole two-node run. The chain, all
/// static and all attested:
///
/// 1. `r_lld_scan_process_pkt_rx_adv_rep` (`0x4002_482C`) at `0x4002_4978`:
///    `a5 = lhu(emi(0x1000) + idx*20 + 14)` → `sh a5, 90(lld_scan_env[act])`;
/// 2. the same function then `memcpy`s 44 bytes from `lld_scan_env+88` into the
///    `LLD_ADV_REP_IND` message body (`0x4002_49E4`), so `env+90` **is** the
///    message's halfword at `+2`;
/// 3. ESP-IDF's own `lld_adv_rep_ind` handler (in `libble_app`, `0x4208_C42C`
///    in the gate's image) reads `lhu(msg + 2)` into a local, and **when it is
///    non-zero** treats it as a RAL entry pointer: it recovers the index as
///    `(ptr - 0xC60) / 52`, and dereferences `emi_get_mem_addr_by_offset(ptr + 24)`
///    and `emi_get_mem_addr_by_offset(ptr + 46)` to `memcpy` two 6-byte
///    addresses out of the entry.
///
/// So a stale non-zero halfword here is dereferenced as an exchange-memory
/// pointer. `r_emi_get_mem_addr_by_offset` asserts `offset >> 10 <= 50`
/// (`emi.c` line 159), and the twin died on exactly that:
///
/// ```text
/// assert emi.c 159, param 0000ff33 0000003f
/// ```
///
/// `0xFF33 = 0xFF05 + 46`, i.e. the `+46` dereference of a garbage pointer, and
/// `0x3F = 0xFF33 >> 10 = 63 > 50`. The model had never written `+0xE`, so the
/// descriptor carried whatever the SRAM behind exchange memory last held —
/// which, in that run, was the wreckage of [`RXD_DATA_PTR`]'s aliased payload
/// writes. Fixing the pointer removes the *source* of the garbage; writing the
/// field removes the model's dependence on the SRAM being clean, which is not a
/// property any model gets to assume.
///
/// This air has no privacy and no resolvable private addresses — nothing on it
/// ever transmits an RPA — so the honest value is **0, "the core resolved
/// nothing"**, which is also the value every reception that worked was
/// accidentally getting. Address resolution itself is NOT modelled.
const RXD_RAL_PTR: u32 = 0xE;
/// RX descriptor `+0x10`: **meaning not determined.** No ROM site that was read
/// (`r_lld_rxdesc_check`, `r_lld_rxdesc_free`, `r_lld_scan_process_pkt_rx*`,
/// `r_lld_adv_pkt_rx`) touches it — the fields those routines use are `+0x0`,
/// `+0x2`, `+0x4`, `+0x6`, `+0x8`, `+0xC`, `+0xE` and `+0x12`. It is written
/// as zero for the same reason as [`RXD_RAL_PTR`]: a core-owned field the model
/// leaves alone is not "unmodelled", it is *stale*, and the link layer cannot
/// tell the difference. Zero is stated as a choice, not as a measurement.
const RXD_UNKNOWN_10: u32 = 0x10;
/// RX descriptor `+0xC`: bits[15:11] carry the **link label** of the activity
/// that received the packet. `r_lld_rxdesc_check(link_label)` (`0x4002_022C`)
/// compares `lhu(rxdesc + 12) >> 11` against its argument and reports nothing
/// when they differ, so a reception stamped with the wrong label is invisible
/// to the host.
///
/// The label is the activity's **control-structure index**, and three ROM sites
/// pin that together:
///
/// * `r_lld_scan_evt_start_cbk` (`0x4002_68F8`) takes `lld_scan_env[act]->[56]`,
///   uses it BOTH as the control-structure index it reaches exchange memory
///   with (`emi(1024) + label*90`) and as `sch_prog_params[24]`, the field
///   `r_sch_prog_ble_push` turns into `ET+0x8 = (cs_idx*90 + 1024) >> 1`;
/// * `r_lld_scan_process_pkt_rx` (`0x4002_581A`) passes that same
///   `lld_scan_env[act]->[56]` byte to `r_lld_rxdesc_check` as the label.
///
/// Silicon agrees: the live advertising part's control structure sits at EM
/// `0x400` = `1024 + 0*90`, i.e. index 0, and both of its populated RX
/// descriptors read `+0xC` = `0x00ED` / `0x001D` — label 0. (Bits[10:0] of that
/// halfword were captured but their meaning was NOT determined; this model
/// writes only the label field and leaves the rest zero.)
const RXD_LINK_LABEL: u32 = 0xC;
/// Shift of the link-label field in [`RXD_LINK_LABEL`].
const RXD_LINK_LABEL_SHIFT: u32 = 11;
/// RX descriptor `+0x8`/`+0xA`: the 32-bit CLKN timestamp of the reception.
/// Live values (`0x00F8_88AE`, `0x00F8_8939`, `0x00F8_89C0`) sit just behind
/// the CLKN the part read at the same halt (`0x00F8_8A5D`).
const RXD_TIMESTAMP: u32 = 0x8;
/// RX descriptor `+0x12`: exchange-memory offset of the received **payload**
/// (the PDU body, with no header bytes — those live in [`RXD_HEADER`]).
///
/// ## It is a FULL 16-BIT offset. Masking bit15 out of it corrupts the world.
///
/// Every ROM site reads this halfword and uses it whole:
/// `r_lld_scan_process_pkt_rx_legacy_adv` (`0x4002_46EE`) and
/// `r_lld_scan_process_pkt_rx_adv_rep` (`0x4002_4878`) both do
/// `lhu` → `sll 16` → `srl 16`, i.e. a plain zero-extend with no mask, and
/// `r_lld_rxdesc_check` (`0x4002_035C`) stores whatever
/// `ble_util_buf_rx_alloc_in_isr` returned straight into it.
///
/// The buffers that allocator hands out are **above 0x8000**.
/// `r_ble_util_buf_rx_free` (`0x4000_315C`) range-checks its argument as
/// `((buf - 0x7805) >> 10) & 0xFF <= 8`, i.e. the RX pool is nine 1 KiB
/// buffers whose data pointers are `0x7805, 0x7C05, 0x8005, 0x8405, 0x8805,
/// 0x8C05, 0x9005, 0x9405, 0x9805`. Measured in the twin — the descriptor
/// ring walks exactly those, in that order.
///
/// This model used to mask the pointer with `0x7FFF`, on the theory that
/// bit15 was "a flag whose meaning was not determined" because a live
/// descriptor's `+0x0` read `0x903C` where its neighbours read `0x103C`. That
/// bit15 is [`RXD_DONE`], and it belongs to `+0x0` — the ownership flag on the
/// **next-descriptor link**, not a convention every pointer field shares. The
/// mask therefore folded the top five buffers of the pool onto the bottom of
/// exchange memory and the model wrote received advertising payloads into it:
///
/// ```text
/// 0x8005 -> 0x0005   the EXCHANGE TABLE (entries 0 and 1)
/// 0x8405 -> 0x0405   the CONTROL STRUCTURE array
/// 0x9005 -> 0x1005   the RX DESCRIPTOR RING ITSELF
/// ```
///
/// and the last of those is self-referential: payload bytes 13 and 14 of a
/// 15-byte legacy `ADV_IND` land exactly on descriptor 0's own `+0x12`. In the
/// two-node gate those two bytes are the peer's `<tag> <counter>`, so
/// descriptor 0's buffer pointer became `0x0205` on node A and `0x0204` on node
/// B — the peer's tag with the peer's counter above it — and the link layer
/// then handed that to `ble_util_buf_rx_free`, which asserted:
///
/// ```text
/// [A] assert ble_util_buf.c 180, param 000000e2 00000205
/// [B] assert ble_util_buf.c 180, param 000000e2 00000204
/// ```
///
/// (`0xE2 = ((0x205 - 0x7805) >> 10) & 0xFF`, the failed range check.) It took
/// ~198 M steps to show up because the ring has to reach the third buffer
/// first, which is why no unit test and neither earlier gate ever saw it.
///
/// `r_lld_scan_process_pkt_rx_legacy_adv` reads this halfword and `memcpy`s
/// six bytes from `emi_get_mem_addr_by_offset(that + 0)` to get the
/// advertiser address.
const RXD_DATA_PTR: u32 = 0x12;

/// C3 interrupt-matrix source for the RW-BLE core. Silicon capture
/// 2026-08-02: `0x600C_2020` (the source-8 map register) reads 5, i.e. the
/// firmware routes source 8 to CPU line 5.
const RWBLE_IRQ_SOURCE: u32 = 8;

/// Depth of the IRQ FIFO. Silicon read `+0x2D8 = 0x0020_003E` → `cnt = 1`,
/// `rem = 15`, so `rem = depth - cnt` with a depth of 16.
const IRQ_FIFO_DEPTH: u32 = 16;

/// `RWBLECNTL` — the core control word.
const RWBLECNTL: u64 = 0x000;
/// `RWBLECNTL` bit31: a **self-clearing** command bit (the RW-BLE core's
/// master soft-reset / kick). Silicon capture 2026-08-02 attests this directly
/// and twice over:
///
/// * the write trace shows the controller writing `+0x000` in pairs — first
///   the plain control word, then the same word with bit31 set
///   (`0x0010_060f` → `0x8010_060f`, later `0x0010_070f` → `0x8010_070f`);
/// * yet **every** idle dump of a live, advertising part reads `+0x000` back
///   as `0x0010_070f`, i.e. with bit31 CLEAR, even though the last write set
///   it.
///
/// So the hardware consumes and drops the bit. Storing the write verbatim
/// (which is what a plain register-backed window does) wedges the controller:
/// it writes the kick and then spins waiting to read the bit go away. That is
/// exactly where the twin parked before this — the last BT write it ever made
/// was `+0x000 <= 0x8010_070f`, and the CPU sat on the instruction immediately
/// after that store while the real part carried straight on into the next
/// bring-up step.
///
/// **Bits 25 and 24 are the same kind of bit, and for the same two reasons.**
/// They are the RW-BLE core's two abort *requests* — bit 25 aborts the
/// advertising event in flight, bit 24 aborts the scanning one — and firmware
/// only ever ORs them in, never clears them:
///
/// | ROM site | writes |
/// |---|---|
/// | `r_lld_adv_stop` `0x4001_8A2C` (state 1 branch) | `(x & 0xFDFF_FFFF) \| 0x0200_0000` |
/// | `r_lld_per_adv_stop` `0x4002_3E48` | the same, bit 25 |
/// | `r_lld_scan_end` `0x4002_4634` | `(x & 0xFEFF_FFFF) \| 0x0100_0000`, bit 24, and then parks the scan activity in state 2 exactly as the advertising stop parks its own |
/// | `r_lld_rpa_renew_evt_start_cbk` `0x4001_FF06`/`0x4001_FF18` | bit 25, then bit 24 — one after the other, aborting both activities so the resolvable private address can be renewed |
///
/// Those are **every** site in the mask ROM that touches either bit, and not
/// one of them ever writes a zero into it: `r_lld_adv_stop`'s
/// `and 0xFDFF_FFFF` immediately precedes an `or 0x0200_0000`, so it is a
/// set, not a clear. No ROM site reads either bit back, either (the only
/// `RWBLECNTL` *read* that is not part of one of those read-modify-writes is
/// `r_lld_scan_process_pkt_rx_aux_adv_ind` `0x4002_4F92`, which tests bit 10).
/// A request bit that software only ever sets and never polls has to be
/// cleared by the hardware, or it latches on the first stop and stays.
///
/// And silicon says it is cleared: every dump of the live advertising part
/// reads `+0x000 = 0x0010_070f` — bits 25 and 24 clear — on a part running
/// firmware that demonstrably writes them set.
///
/// **The twin's own trace shows what latching them costs**, and it is a value
/// silicon cannot produce. `LABWIRED_BT_TRACE=1`, two-node BLE Pong,
/// 2026-08-07, both nodes, at CLKN 37:
///
/// ```text
/// [bt1] +0x000 <= 0x0210070f   ← rpa_renew sets bit 25
/// [bt1] +0x000 <= 0x0310070f   ← ... then ORs bit 24 onto what it read BACK
/// ```
///
/// `r_lld_rpa_renew_evt_start_cbk` computes that second word as
/// `(read_back & 0xFEFF_FFFF) | 0x0100_0000`. On silicon the read-back is
/// `0x0010_070f`, so the second store is `0x0110_070f`. `0x0310_070f` only
/// exists because this model handed the firmware back its own abort request.
///
/// The practical trap that follows, recorded because it has already cost one
/// merged-and-reverted attempt (core#772 → #774): with the bits latched, an
/// abort model keyed on the 0→1 *edge* of bit 25 can fire **at most once per
/// boot**. Every later `r_lld_adv_stop` writes the identical word back — the
/// isolated `+0x000 <= 0x0310070f` entries at CLKN 1576/1723/2161/2747 in that
/// same trace are real stops landing on a live advertising event, and not one
/// of them is an edge. Clearing the bits here is what makes any future abort
/// model observable at all.
const RWBLECNTL_SELF_CLEARING: u32 = 0x8300_0000;

/// Read-only hardware identity/configuration words, seeded from the silicon
/// capture of 2026-08-02. These are the ONLY snapshot-seeded values in the
/// model; everything else is either derived (the timebase) or plain storage.
///
/// Both are read by controller bring-up and never appear as a store target in
/// the 303-hit write trace, i.e. they are hardwired in the IP, not firmware
/// state — and both read identically across every boot and every session
/// captured.
///
/// `+0x004` is **firmware-attested**, not inferred: with the window mapped but
/// this word left at 0, the controller stops with its own assertion naming the
/// value it demands —
///
/// ```text
/// assert lld.c 318, param 00000000 09001b00
///                        ^read     ^expected
/// ```
///
/// `lld.c` / `llm_adv.c` / `rwble.c` / the `EM_BLE_*_OFFSET` log strings in the
/// app image identify this block as a **RivieraWaves RW-BLE core**, whose
/// register file opens `RWBLECNTL`(+0x00), `VERSION`(+0x04), `RWBLECONF`(+0x08),
/// `INTCNTL`(+0x0C), `INTSTAT`(+0x10), `INTRAWSTAT`(+0x14), `INTACK`(+0x18),
/// `BASETIMECNT`(+0x1C), `FINETIMECNT`(+0x20) — which is exactly the shape the
/// write trace shows (firmware writes +0x0C and W1C-writes +0x18 with the same
/// bits that read back at +0x10/+0x14, and reads/writes +0x1C/+0x20 as the
/// timebase). `+0x008` is `RWBLECONF`, the build-option word the controller
/// sizes its exchange memory from.
const HW_IDENTITY: &[(u64, u32)] = &[
    (0x004, 0x0900_1b00), // VERSION
    (0x008, 0x0f22_d0b0), // RWBLECONF
];

/// CPU cycles per CLKN tick. 312.5 µs at the C3's 160 MHz CPU clock — the same
/// "hardcode against 160 MHz and say so" convention the WiFi MAC's beacon
/// cadence uses (`MEDIUM_BEACON_INTERVAL_CYCLES`). Peripherals are not handed
/// `cpu_hz`, and every C3 system descriptor in-tree runs at 160 MHz.
const CYCLES_PER_CLKN_TICK: u64 = 50_000;
/// Fine-counter ticks per CLKN tick: 312.5 µs × 2 MHz.
const FINE_TICKS_PER_CLKN: u64 = 625;
/// CPU cycles per fine tick (`CYCLES_PER_CLKN_TICK / FINE_TICKS_PER_CLKN`).
const CYCLES_PER_FINE_TICK: u64 = CYCLES_PER_CLKN_TICK / FINE_TICKS_PER_CLKN;

/// Process-cached `LABWIRED_BT_TRACE` gate. Read ONCE per process — the write
/// path is hot and `std::env::var` is a syscall-backed lookup (same reasoning
/// as the WiFi MAC's `rxbuf_trace_enabled`).
/// Hand every controller instance a distinct identity on the shared air.
fn next_node_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn bt_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("LABWIRED_BT_TRACE").is_ok())
}

#[derive(Debug, Default)]
pub struct Esp32c3Bt {
    /// The whole 4 KiB window as plain storage. Reset 0 — which is what
    /// silicon reads at `reset halt`, because the block is clock-gated until
    /// the controller enables it.
    regs: Vec<u32>,
    /// Last value written to `CLKN` (low 31 bits), and whether the latch bit
    /// was set. Kept out of `regs` because a `CLKN` read must return the
    /// running clock, not this — see the `+0x01C` note in the module docs.
    /// Inspection only: the timer comparators are `+0x0E4`/`+0x0E8`/`+0x0EC`.
    event_target: u32,
    event_armed: bool,
    /// `INTRAWSTAT` (`+0x014`) — the raw interrupt latch. Set by the
    /// comparators, cleared W1C through `INTACK` (`+0x018`) or its `+0x38C`
    /// mirror. `INTSTAT` is derived from this and `INTCNTL`, never stored.
    int_raw: Cell<u32>,
    /// Comparator bits whose target register has actually been programmed.
    ///
    /// Deliberate deviation, called out rather than hidden: the ROM always
    /// writes a target and *then* ORs the enable in, so on silicon an enabled
    /// comparator always has a real deadline. A model that ran the comparison
    /// against an unwritten register would treat the reset value 0 as a
    /// deadline in the past and fire the instant firmware set the enable —
    /// a fabricated interrupt out of a register nobody programmed. So an
    /// un-programmed comparator counts as disarmed.
    comparators_programmed: u32,
    /// Comparator bits that have already latched for their CURRENT arming.
    ///
    /// A comparator fires ONCE per arm. Without this the "reached or passed"
    /// edge rule would re-latch on every tick after the deadline, so the
    /// instant firmware acked the interrupt the model would raise it again —
    /// an interrupt storm, not a periodic event. Cleared when the target is
    /// reprogrammed or the enable is re-asserted, which is exactly what the
    /// ROM's `r_rwip_timer_*_set` do to schedule the next one.
    comparators_fired: Cell<u32>,
    /// The RW-BLE IRQ FIFO (`+0x2D8`): one bitmap per interrupt the hardware
    /// queued, oldest first. `r_rwble_isr` reads the head, pops it with a
    /// bit0 write, and returns without acking anything when `cnt == 0` — so a
    /// raised line with an empty FIFO is an interrupt storm, not progress.
    irq_fifo: RefCell<VecDeque<u32>>,
    /// Last cycle the bus anchored this model to via `sync_to` — the same
    /// `current_cycle` it then turns a `take_scheduled_events` delay into
    /// `current_cycle + 1 + delay` against. Kept so the scheduled deadline is
    /// exact rather than off by however far the published `CycleClock` lags
    /// mid-batch.
    sync_cycle: Cell<u64>,
    /// Generation stamp for the in-flight scheduled comparator event. Bumped
    /// on every write that could re-arm, so an event scheduled under an older
    /// deadline dies on arrival instead of firing a stale interrupt.
    arm_seq: u32,
    /// Cycle at which the block was first written, i.e. when the controller
    /// un-gated it. CLKN counts from here, so a read before any BLE activity
    /// returns 0 exactly like silicon at `reset halt`. `None` until then.
    clock_base: Option<u64>,
    /// Bus-published cycle clock, attached by
    /// [`SystemBus::add_peripheral`](crate::bus::SystemBus). Drives CLKN and
    /// the fine counter. Not serialized — re-attached by the bus.
    clock: Option<CycleClock>,
    /// Exchange-table indices the controller has pushed through `+0x100` and
    /// the core has not started yet, oldest first. `r_sch_prog_push` hands the
    /// hardware entries in ring order and `r_sch_prog_end_isr` consumes them
    /// from its own head index in the same order, so a FIFO is the shape the
    /// software half already assumes.
    prog_queue: VecDeque<u32>,
    /// The programmed event currently being executed, if any.
    radio: Option<RadioEvent>,
    /// Duration of [`Self::radio`], in CPU cycles, decoded from `ET + 0xA`.
    radio_duration: u64,
    /// Sequence number of the oldest air frame this controller has not looked
    /// at yet. The air broadcasts, so advancing this consumes nothing for any
    /// other listener.
    rx_cursor: u64,
    /// This controller's identity on the air, so it never decodes its own
    /// transmission.
    node_id: u64,
    /// The shared air this controller transmits into and listens on.
    air: BleAirBus,
}

/// Where a programmed radio event is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RadioPhase {
    /// Pushed and decoded; waiting for its programmed start instant.
    Pending,
    /// Started (status [`ET_STATUS_ONGOING`]); waiting out its duration.
    Running,
}

/// One programmed radio event in flight.
#[derive(Debug, Clone, Copy)]
struct RadioEvent {
    /// Index of its exchange-table entry.
    et_idx: u32,
    phase: RadioPhase,
    /// Next instant this event does something, in the same elapsed-cycle
    /// domain as [`Esp32c3Bt::elapsed_cycles`].
    deadline: u64,
}

impl Esp32c3Bt {
    pub fn new() -> Self {
        Self {
            regs: vec![0u32; (BT_SIZE / 4) as usize],
            event_target: 0,
            event_armed: false,
            int_raw: Cell::new(0),
            comparators_programmed: 0,
            comparators_fired: Cell::new(0),
            irq_fifo: RefCell::new(VecDeque::new()),
            sync_cycle: Cell::new(0),
            arm_seq: 0,
            clock_base: None,
            clock: None,
            prog_queue: VecDeque::new(),
            radio: None,
            radio_duration: 0,
            rx_cursor: default_ble_air_bus().current_seq(),
            node_id: next_node_id(),
            air: default_ble_air_bus().clone(),
        }
    }

    /// Build a controller on an explicitly-owned air, so two nodes in one
    /// world share a medium and two worlds do not. Mirrors
    /// [`Nrf52Radio::with_air`](crate::peripherals::nrf52::radio::Nrf52Radio::with_air).
    /// Rebind the shared BLE air (browser multi-chip lab-group isolation).
    ///
    /// The cursor joins at the air's CURRENT sequence, never at 0: a radio has
    /// no history buffer, so frames that crossed before this controller existed
    /// are gone. A lab reuses its `AirBus` across a simulation restart (the
    /// playground mints a new one only when the source/diagram hash changes),
    /// and `next_node_id()` hands the restarted controller a fresh identity —
    /// so without this join it would neither age out nor recognise the previous
    /// run's frames, and would replay up to `AIR_DEPTH` of them per channel as
    /// live peer traffic before ever seeing anything current.
    pub fn set_air(&mut self, air: BleAirBus) {
        self.rx_cursor = air.current_seq();
        self.air = air;
    }

    pub fn with_air(air: BleAirBus) -> Self {
        let mut bt = Self::new();
        bt.set_air(air);
        bt
    }

    /// The air this controller is on (tests, inspection, the air view).
    pub fn air(&self) -> &BleAirBus {
        &self.air
    }

    /// Cycles elapsed since the controller un-gated the block, or 0 while it is
    /// still gated (nothing written yet).
    #[inline]
    fn elapsed_cycles(&self) -> u64 {
        match (self.clock.as_ref(), self.clock_base) {
            (Some(c), Some(base)) => c.now().saturating_sub(base),
            _ => 0,
        }
    }

    /// Bluetooth native clock: one tick per 312.5 µs. 31 bits — the top bit of
    /// `CLKN` is the comparator arm on the write side, and every read captured
    /// from silicon had it clear.
    #[inline]
    fn clkn(&self) -> u32 {
        Self::clkn_at(self.elapsed_cycles())
    }

    #[inline]
    fn clkn_at(elapsed: u64) -> u32 {
        ((elapsed / CYCLES_PER_CLKN_TICK) as u32) & CLKN_MASK
    }

    /// Sub-CLKN fine counter: a half-µs **down** counter, 624 → 0 across each
    /// CLKN tick.
    ///
    /// The direction is not guesswork and it is not the direction this model
    /// first assumed. `r_rwip_time_get` — the ROM routine that samples the
    /// timebase — returns the pair as `(BASETIMECNT & 0x0FFF_FFFF,
    /// 624 - FINETIMECNT)`, and `r_rwip_timer_hus_set(hs, hus)` writes
    /// `624 - hus` into the comparator at `+0x0F0`. Both only make sense if
    /// `FINETIMECNT` runs *down*: the driver's `hus` (half-µs elapsed within
    /// the half-slot, which must increase for `rwip_time_t` arithmetic to
    /// work) is `624 - FINETIMECNT`, and arming "fire `hus` into the target
    /// half-slot" then reduces to the direct register compare
    /// `FINETIMECNT <= +0x0F0`. Direct sampling could NOT settle this — a
    /// dozen JTAG reads land uniformly in `0..624` either way (max seen 566)
    /// — so the ROM's own arithmetic is the evidence. It also matches the
    /// RW-BLE core's `FINECNT` elsewhere in the RivieraWaves family, which is
    /// corroboration, not measurement.
    #[inline]
    fn clkn_fine(&self) -> u32 {
        if self.clock_base.is_none() {
            // Still clock-gated: silicon reads the whole window as zero.
            return 0;
        }
        Self::fine_at(self.elapsed_cycles())
    }

    #[inline]
    fn fine_at(elapsed: u64) -> u32 {
        let within = (elapsed / CYCLES_PER_FINE_TICK) % FINE_TICKS_PER_CLKN;
        (FINE_TICKS_PER_CLKN - 1 - within) as u32
    }

    /// The bus-published cycle count, or 0 before a clock is attached.
    #[inline]
    fn clock_now(&self) -> u64 {
        self.clock.as_ref().map(|c| c.now()).unwrap_or(0)
    }

    /// Cycles since the un-gate as of absolute CPU cycle `now`.
    #[inline]
    fn elapsed_at(&self, now: u64) -> u64 {
        now.saturating_sub(self.clock_base.unwrap_or(now))
    }

    /// True once the bus has handed over a cycle clock and the event-scheduler
    /// build is active — the same predicate `ledc`/`i2c0` use. Without a clock
    /// (feature off, hand-built bus, `force_legacy_walk`) the model stays on
    /// the legacy per-cycle walk so those callers keep the old semantics.
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Test/differential knob mirroring `Esp32c3Ledc::force_legacy_walk`: drop
    /// the cycle clock so the model runs the legacy walk instead.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
    }

    /// The comparator target last armed via a `CLKN` write, if armed. Exposed
    /// for the interrupt follow-up and for tests.
    pub fn armed_event_target(&self) -> Option<u32> {
        self.event_armed.then_some(self.event_target)
    }

    #[inline]
    fn reg(&self, offset: u64) -> u32 {
        *self.regs.get((offset / 4) as usize).unwrap_or(&0)
    }

    /// `INTCNTL` — the enable mask. Also the arm/disarm switch for the three
    /// timer comparators: the ROM's setters OR their bit in after writing the
    /// target and AND it out to disarm.
    #[inline]
    fn int_enable(&self) -> u32 {
        self.reg(INTCNTL)
    }

    /// `INTSTAT` (`+0x010`) — derived, never stored. Silicon:
    /// `0x0640B66 & 0x811 = 0x800`, `0x0640B66 & 0x011 = 0x000`.
    #[inline]
    fn int_status(&self) -> u32 {
        self.int_raw.get() & self.int_enable()
    }

    /// Absolute cycle (since the un-gate) at which comparator `bit` fires, or
    /// `None` when it is not armed — not programmed, or its enable is clear.
    ///
    /// Worked in the elapsed-cycle domain rather than by comparing CLKN
    /// values, so "already past" needs no wrap arithmetic. The model therefore
    /// does NOT handle the 28-bit CLKN wrap, which is ~23 hours of device time
    /// — stated rather than pretended away.
    fn deadline_cycles(&self, bit: u32) -> Option<u64> {
        if self.int_enable() & self.comparators_programmed & bit == 0 {
            return None;
        }
        Some(match bit {
            // +0x0E4 counts in 10 ms units = 32 CLKN ticks (the setter keeps
            // `rwip_env+8 = target << 5` half-slots alongside it).
            INT_TIMER_10MS => u64::from(self.reg(TIMER_10MS_TARGET)) * 32 * CYCLES_PER_CLKN_TICK,
            // +0x0E8 counts in half-slots = CLKN ticks.
            INT_TIMER_HS => u64::from(self.reg(TIMER_HS_TARGET)) * CYCLES_PER_CLKN_TICK,
            // +0x0EC is the CLKN target; +0x0F0 holds `624 - hus`, compared
            // against the DOWN-counting FINETIMECNT, so the offset into the
            // target tick is `624 - target` fine ticks.
            INT_TIMER_HUS => {
                let fine_target =
                    u64::from(self.reg(TIMER_HUS_FINE) & 0xFFFF).min(FINE_TICKS_PER_CLKN - 1);
                u64::from(self.reg(TIMER_HUS_TARGET) & CLKN_MASK) * CYCLES_PER_CLKN_TICK
                    + (FINE_TICKS_PER_CLKN - 1 - fine_target) * CYCLES_PER_FINE_TICK
            }
            _ => return None,
        })
    }

    /// Comparator bits due at `elapsed` that have not already latched for
    /// their current arming. A comparator fires ONCE per arm.
    fn expired_at(&self, elapsed: u64) -> u32 {
        if self.clock_base.is_none() {
            return 0;
        }
        let spent = self.comparators_fired.get() | self.int_raw.get();
        let mut fired = 0;
        for bit in [INT_TIMER_10MS, INT_TIMER_HS, INT_TIMER_HUS] {
            if spent & bit != 0 {
                continue;
            }
            if matches!(self.deadline_cycles(bit), Some(d) if elapsed >= d) {
                fired |= bit;
            }
        }
        fired
    }

    /// Latch every comparator due at `elapsed` into `INTRAWSTAT` and queue one
    /// IRQ FIFO entry per rising edge. `&self` (via `Cell`/`RefCell`) so a
    /// read or a level poll can materialise a deadline that has just come due
    /// without waiting for the next event — the `ledc` `sync_from_clock`
    /// pattern.
    fn latch_at(&self, elapsed: u64) -> bool {
        let rising = self.expired_at(elapsed);
        if rising == 0 {
            return false;
        }
        self.int_raw.set(self.int_raw.get() | rising);
        self.comparators_fired
            .set(self.comparators_fired.get() | rising);
        // The FIFO carries the bits the ISR is meant to dispatch, i.e. the
        // enabled ones (`r_rwble_isr` feeds the head bitmap through the same
        // masks it applies to a raw INTSTAT read).
        let queued = rising & self.int_enable();
        let mut fifo = self.irq_fifo.borrow_mut();
        if queued != 0 && (fifo.len() as u32) < IRQ_FIFO_DEPTH {
            fifo.push_back(queued);
        }
        if bt_trace_enabled() {
            let nid = self.node_id;
            eprintln!(
                "[bt{nid}] IRQ raw|={rising:#010x} stat={:#010x} fifo_cnt={} (clkn={} fine={})",
                self.int_raw.get() & self.int_enable(),
                fifo.len(),
                Self::clkn_at(elapsed),
                Self::fine_at(elapsed)
            );
        }
        true
    }

    /// Materialise any comparator that has come due as of the bus-published
    /// clock. Cheap no-op while nothing is armed.
    fn sync_from_clock(&self) {
        if self.clock_base.is_some() {
            self.latch_at(self.elapsed_cycles());
        }
    }

    /// Cycles from `elapsed` to the nearest armed, unspent comparator or
    /// pending radio-event phase, or `None` when nothing is scheduled. Zero
    /// when one is already due.
    fn cycles_to_next_deadline(&self, elapsed: u64) -> Option<u64> {
        let spent = self.comparators_fired.get() | self.int_raw.get();
        [INT_TIMER_10MS, INT_TIMER_HS, INT_TIMER_HUS]
            .into_iter()
            .filter(|bit| spent & bit == 0)
            .filter_map(|bit| self.deadline_cycles(bit))
            .map(|deadline| deadline.saturating_sub(elapsed))
            .chain(self.cycles_to_radio_deadline(elapsed))
            .min()
    }

    /// `+0x2D8` read: `cnt`/`rem` plus the head entry's bitmap, in the exact
    /// field positions `r_rwble_isr` decodes.
    fn irq_fifo_word(&self) -> u32 {
        let fifo = self.irq_fifo.borrow();
        let cnt = (fifo.len() as u32).min(31);
        // `rem` is a 4-bit field and the FIFO is 16 deep, so "15 free" and
        // "16 free" share an encoding — silicon reads `+0x2D8 = 0x0000_001E`
        // (cnt 0, rem 15) while idle, which is what this clamp reproduces.
        // Only `cnt` gates the ISR; `rem` reaches nothing but a log string.
        let rem = IRQ_FIFO_DEPTH.saturating_sub(cnt).min(15);
        let head = fifo.front().copied().unwrap_or(0) & 0x001F_FFFF;
        (rem << 1) | (cnt << 5) | (head << 10)
    }

    /// True while a comparator is armed or the line is asserted — the only
    /// states in which the per-cycle walk has anything to do.
    fn irq_work_pending(&self) -> bool {
        // The radio engine only runs from `on_event` (it needs the bus), so it
        // is deliberately NOT a reason to join the legacy walk — a walk that
        // could never make progress on it would spin forever.
        self.int_status() != 0
            || (self.clock_base.is_some()
                && self.int_enable()
                    & self.comparators_programmed
                    & !(self.comparators_fired.get() | self.int_raw.get())
                    != 0)
    }

    // ── The programmed-event engine ─────────────────────────────────────────

    /// Latch `bits` into `INTRAWSTAT` and queue one IRQ FIFO entry for them,
    /// the same way [`Self::latch_at`] does for a comparator. One entry per
    /// hardware interrupt: `r_rwble_isr` pops the head, dispatches its bitmap
    /// and returns without acking anything when the FIFO is empty.
    fn raise_irq_bits(&self, bits: u32) {
        if bits == 0 {
            return;
        }
        self.int_raw.set(self.int_raw.get() | bits);
        let queued = bits & self.int_enable();
        if queued == 0 {
            return;
        }
        let mut fifo = self.irq_fifo.borrow_mut();
        if (fifo.len() as u32) < IRQ_FIFO_DEPTH {
            fifo.push_back(queued);
        }
    }

    /// Translate an exchange-memory byte offset to the CPU data-RAM address the
    /// controller mapped it at, or `None` while the region is unmapped.
    ///
    /// This is `r_emi_get_mem_addr_by_offset` (`0x4000_6976`) with its ROM
    /// lookup table dropped, because the base registers make it redundant: each
    /// register carries the exchange-memory offset it covers in bits[31:18]
    /// (the ROM asserts `lut[off >> 10].base == (reg >> 18) << 2`), so the
    /// covering register is simply the one with the largest covered offset not
    /// past `em_off`. Nothing is hardcoded — the addresses come out of
    /// registers the controller itself wrote, which is why the model works
    /// unchanged against a firmware that lays its exchange memory out
    /// differently.
    fn em_cpu_addr(&self, em_off: u32) -> Option<u64> {
        let mut best: Option<(u32, u32)> = None;
        for idx in 0..=EM_BASE_REG_MAX {
            let reg_off = if idx <= EM_BASE_REG_BANK_A_MAX {
                EM_BASE_REG_BANK_A + idx * 4
            } else {
                EM_BASE_REG_BANK_B + idx * 4
            };
            let reg = self.reg(reg_off);
            let addr_lo = (reg << 2) & EM_RAM_ADDR_MASK;
            if addr_lo == 0 {
                // Region never allocated: `r_emi_alloc_em_mapping_by_offset`
                // has not run for it and the live part reads the whole
                // register as `<covered offset> | 0`.
                continue;
            }
            let covered = (reg >> 18) << 2;
            if covered > em_off {
                continue;
            }
            if best.is_none_or(|(c, _)| covered >= c) {
                best = Some((covered, reg));
            }
        }
        let (covered, reg) = best?;
        let addr_lo = u64::from((reg << 2) & EM_RAM_ADDR_MASK);
        Some((EM_RAM_WINDOW | addr_lo) + u64::from(em_off - covered))
    }

    fn em_read_u8(&self, bus: &dyn Bus, em_off: u32) -> Option<u8> {
        bus.read_u8(self.em_cpu_addr(em_off)?).ok()
    }

    fn em_read_u16(&self, bus: &dyn Bus, em_off: u32) -> Option<u16> {
        let lo = u16::from(self.em_read_u8(bus, em_off)?);
        let hi = u16::from(self.em_read_u8(bus, em_off + 1)?);
        Some(lo | (hi << 8))
    }

    fn em_read_u32(&self, bus: &dyn Bus, em_off: u32) -> Option<u32> {
        let lo = u32::from(self.em_read_u16(bus, em_off)?);
        let hi = u32::from(self.em_read_u16(bus, em_off + 2)?);
        Some(lo | (hi << 16))
    }

    fn em_write_u16(&self, bus: &mut dyn Bus, em_off: u32, value: u16) -> Option<()> {
        let addr = self.em_cpu_addr(em_off)?;
        bus.write_u8(addr, value as u8).ok()?;
        bus.write_u8(addr + 1, (value >> 8) as u8).ok()
    }

    /// Byte offset of exchange-table entry `idx`.
    fn et_entry(idx: u32) -> u32 {
        EM_ET_OFFSET + (idx & (ET_ENTRIES - 1)) * ET_ENTRY_BYTES
    }

    /// Absolute elapsed cycle at which the exchange-table entry says its event
    /// starts, and how long it runs, or `None` while exchange memory is not
    /// mapped yet.
    fn read_et_schedule(&self, bus: &dyn Bus, idx: u32) -> Option<(u64, u64)> {
        let et = Self::et_entry(idx);
        let lo = u32::from(self.em_read_u16(bus, et + ET_START_LO)?);
        let hi = u32::from(self.em_read_u16(bus, et + ET_START_HI)?) & 0x0FFF;
        let clkn = (lo | (hi << 16)) & CLKN_MASK;
        // `+0x6` holds `624 - hus`, compared against the DOWN-counting
        // FINETIMECNT, so the offset into the target tick is `624 - field`
        // fine ticks — the same arithmetic the `+0x0F0` comparator uses.
        let fine_field =
            u64::from(self.em_read_u16(bus, et + ET_START_FINE)?).min(FINE_TICKS_PER_CLKN - 1);
        let start = u64::from(clkn) * CYCLES_PER_CLKN_TICK
            + (FINE_TICKS_PER_CLKN - 1 - fine_field) * CYCLES_PER_FINE_TICK;

        let dur = self.em_read_u16(bus, et + ET_DURATION)?;
        let duration = if dur & ET_DURATION_HALF_SLOTS != 0 {
            u64::from(dur & !ET_DURATION_HALF_SLOTS) * CYCLES_PER_CLKN_TICK
        } else {
            // `(hus + 1) >> 1` going in, so two half-µs per stored unit.
            u64::from(dur) * 2 * CYCLES_PER_FINE_TICK
        };
        Some((start, duration))
    }

    /// Write the 3-bit hardware status field of an exchange-table entry,
    /// leaving every firmware-owned bit alone.
    fn set_et_status(&self, bus: &mut dyn Bus, idx: u32, status: u16) {
        let et = Self::et_entry(idx);
        let Some(ctrl) = self.em_read_u16(bus, et + ET_CTRL) else {
            return;
        };
        let next = (ctrl & !ET_STATUS_FIELD) | ((status << ET_STATUS_SHIFT) & ET_STATUS_FIELD);
        let _ = self.em_write_u16(bus, et + ET_CTRL, next);
    }

    /// Build the PDU this event's control structure staged and push it onto the
    /// air. Returns `true` if a frame was actually transmitted.
    ///
    /// The whole chain is read out of exchange memory — nothing is synthesised:
    /// `ET[idx] + 8` × 2 is the control-structure offset, the control structure
    /// carries the device address, access address, CRC init and channel, and
    /// its `+0x1C` points at a TX descriptor whose `+0x2` is
    /// `(length << 8) | header` and whose `+0x4` points at the payload bytes.
    fn transmit_event(&self, bus: &mut dyn Bus, idx: u32) -> bool {
        let et = Self::et_entry(idx);
        let Some(cs_ptr) = self.em_read_u16(bus, et + ET_CS_PTR) else {
            return false;
        };
        let cs = u32::from(cs_ptr) * 2;

        let Some(format) = self.em_read_u16(bus, cs + CS_FORMAT) else {
            return false;
        };
        if format & 0xFF != CS_FORMAT_LEGACY_ADV {
            // Only the legacy-advertising format was measured. Anything else
            // is left alone rather than guessed at; the event still ends
            // normally so the controller is never wedged by our ignorance.
            if bt_trace_enabled() {
                let nid = self.node_id;
                eprintln!("[bt{nid}] radio: CS format {format:#06x} not modelled — no TX");
            }
            return false;
        }

        let Some(tx_desc_ptr) = self.em_read_u16(bus, cs + CS_TX_DESC_PTR) else {
            return false;
        };
        // FULL 16 BITS. The `0x903C`-vs-`0x103C` observation that used to
        // justify masking bit15 out of every descriptor pointer was of the RX
        // descriptor's `+0x0`, whose bit15 is [`RXD_DONE`] — an ownership flag
        // on the NEXT-descriptor link, not a convention shared by pointers in
        // general. Nothing measured says bit15 means anything here, and
        // masking it is not "ignoring an unknown flag": it silently aliases
        // every exchange-memory offset at or above 0x8000 onto the first
        // 8 KiB, where the exchange table and the control structures live.
        // See [`RXD_DATA_PTR`] for what that cost on the receive side.
        let tx_desc = u32::from(tx_desc_ptr);
        if tx_desc == 0 {
            return false; // an event that only listens
        }

        let Some(hdr_word) = self.em_read_u16(bus, tx_desc + TXD_HEADER) else {
            return false;
        };
        let header = (hdr_word & 0xFF) as u8;
        let len = hdr_word >> 8;
        let Some(data_ptr) = self.em_read_u16(bus, tx_desc + TXD_DATA_PTR) else {
            return false;
        };
        if len < TXD_ADDR_PREFIX_LEN {
            return false;
        }

        let mut payload = Vec::with_capacity(usize::from(len));
        for b in 0..u32::from(TXD_ADDR_PREFIX_LEN) {
            payload.push(self.em_read_u8(bus, cs + CS_BDADDR + b).unwrap_or(0));
        }
        // Full 16 bits, same reasoning as the descriptor pointer above.
        let data_off = u32::from(data_ptr);
        for b in 0..u32::from(len - TXD_ADDR_PREFIX_LEN) {
            payload.push(self.em_read_u8(bus, data_off + b).unwrap_or(0));
        }

        let access_address = self.em_read_u32(bus, cs + CS_ACCESS_ADDR).unwrap_or(0);
        let crc_init = self.em_read_u32(bus, cs + CS_CRC_INIT).unwrap_or(0) & 0x00FF_FFFF;
        let channel =
            (self.em_read_u16(bus, cs + CS_HOP_CTRL).unwrap_or_default() & CS_CHANNEL_MASK) as u8;

        let mut pdu = Vec::with_capacity(payload.len() + 2);
        pdu.push(header);
        pdu.push(len as u8);
        pdu.extend_from_slice(&payload);

        if bt_trace_enabled() {
            let hex: String = pdu.iter().map(|b| format!("{b:02x} ")).collect();
            let nid = self.node_id;
            eprintln!(
                "[bt{nid}] radio TX ch{channel} aa={access_address:#010x} crcinit={crc_init:#08x} \
                 et={idx} pdu={hex}"
            );
        }
        self.air.transmit(BleAirFrame {
            seq: 0,
            source: self.node_id,
            channel,
            access_address,
            crc_init,
            pdu,
        });
        true
    }

    /// Deliver one air frame this event's control structure is listening for,
    /// if the air has one this controller has not seen. Returns `true` if a
    /// frame was written into exchange memory.
    ///
    /// The write-back is the mirror of the transmit path and every field is
    /// ROM-attested (see the constants): the descriptor at `+0x024` gets the
    /// status word a good reception leaves, `(len << 8) | header`, the CLKN
    /// timestamp, and the payload bytes at the exchange-memory offset its
    /// `+0x12` names — then `+0x024` advances along the descriptor ring.
    fn receive_event(&mut self, bus: &mut dyn Bus, idx: u32, elapsed: u64) -> bool {
        let et = Self::et_entry(idx);
        let Some(cs_ptr) = self.em_read_u16(bus, et + ET_CS_PTR) else {
            return false;
        };
        let cs = u32::from(cs_ptr) * 2;
        // Only a SCANNING activity receives. See [`CS_FORMAT_SCAN`] for why this
        // gate is load-bearing rather than cosmetic: delivering into whichever
        // event happened to be running stamps the wrong link label and wedges
        // the descriptor ring on the first misdelivery.
        let Some(format) = self.em_read_u16(bus, cs + CS_FORMAT) else {
            return false;
        };
        if format & 0xFF != CS_FORMAT_SCAN {
            return false;
        }
        let Some(access_address) = self.em_read_u32(bus, cs + CS_ACCESS_ADDR) else {
            return false;
        };
        let channel =
            (self.em_read_u16(bus, cs + CS_HOP_CTRL).unwrap_or_default() & CS_CHANNEL_MASK) as u8;
        // The link label the core stamps into the descriptor is the activity's
        // control-structure index, which the exchange-table entry already
        // names: `ET+0x8 = (cs_idx*90 + 1024) >> 1`. Invert that rather than
        // carry a second copy. A control-structure pointer below the array base
        // is not a control structure at all, so there is no label to stamp and
        // the model refuses to receive rather than invent one.
        let Some(cs_rel) = cs.checked_sub(EM_CS_OFFSET) else {
            return false;
        };
        let link_label = ((cs_rel / CS_STRIDE) as u16) & 0x1F;

        // The descriptor the core would fill next. Below the array base it has
        // not been programmed, so there is nowhere honest to put a frame.
        let rxd = self.reg(RX_DESC_PTR) & RX_DESC_PTR_MASK;
        if rxd < EM_RX_DESC_OFFSET || (rxd - EM_RX_DESC_OFFSET) % RX_DESC_BYTES != 0 {
            return false;
        }
        // `RXDONE` still set means firmware has not consumed the reception
        // already in this descriptor. The core does not overwrite it — that is
        // the whole point of the ownership bit — so the frame is simply not
        // received, exactly as a real receiver with no free buffer misses one.
        let Some(next_word) = self.em_read_u16(bus, rxd + RXD_NEXT) else {
            return false;
        };
        if next_word & RXD_DONE != 0 {
            if bt_trace_enabled() {
                let nid = self.node_id;
                eprintln!(
                    "[bt{nid}] radio RX ch{channel}: rxd={rxd:#06x} still RXDONE — no free buffer"
                );
            }
            return false;
        }
        let Some(data_ptr) = self.em_read_u16(bus, rxd + RXD_DATA_PTR) else {
            return false;
        };
        if data_ptr == 0 {
            return false;
        }

        let Some(frame) =
            self.air
                .receive_from(channel, access_address, self.rx_cursor, self.node_id)
        else {
            return false;
        };
        self.rx_cursor = frame.seq + 1;
        if frame.pdu.len() < 2 {
            return false;
        }
        let header = frame.pdu[0];
        let payload = &frame.pdu[2..];
        let len = payload.len() as u16;

        // FULL 16 BITS — no `& 0x7FFF`. See [`RXD_DATA_PTR`]: bit15 is the
        // ownership flag of `+0x0`, and masking it out of a *buffer* pointer
        // aliased the top half of the RX pool onto exchange memory's first
        // 8 KiB, which is where the exchange table, the control structures and
        // the descriptor ring itself live.
        let data_off = u32::from(data_ptr);
        for (i, b) in payload.iter().enumerate() {
            let Some(addr) = self.em_cpu_addr(data_off + i as u32) else {
                return false;
            };
            if bus.write_u8(addr, *b).is_err() {
                return false;
            }
        }
        let clkn = Self::clkn_at(elapsed);
        let _ = self.em_write_u16(bus, rxd + RXD_HEADER, (len << 8) | u16::from(header));
        let _ = self.em_write_u16(bus, rxd + RXD_STATUS, RXD_STATUS_GOOD);
        let _ = self.em_write_u16(bus, rxd + RXD_TIMESTAMP, clkn as u16);
        let _ = self.em_write_u16(bus, rxd + RXD_TIMESTAMP + 2, (clkn >> 16) as u16);
        let _ = self.em_write_u16(
            bus,
            rxd + RXD_LINK_LABEL,
            link_label << RXD_LINK_LABEL_SHIFT,
        );
        // EVERY core-owned field, every reception — including the ones this
        // model has nothing to say about. A descriptor field the core writes
        // and the model does not is not "unmodelled", it is whatever the SRAM
        // behind exchange memory last held, and the link layer reads it as
        // hardware output either way. `+0xE` is the one that proved it: the
        // twin ran fine for ~150 M steps and then died on
        // `assert emi.c 159, param 0000ff33 0000003f` — ESP-IDF's advertising
        // report handler dereferencing a stale halfword as a resolving-list
        // pointer. See [`RXD_RSSI`], [`RXD_RAL_PTR`] and [`RXD_UNKNOWN_10`] for
        // what each one is and why zero is the honest value here.
        //
        // `+0x0` bits[14:0] (the next-descriptor link) and `+0x12` (the buffer
        // offset) are SOFTWARE-owned and are deliberately left alone.
        let _ = self.em_write_u16(bus, rxd + RXD_RSSI, 0);
        let _ = self.em_write_u16(bus, rxd + RXD_RAL_PTR, 0);
        let _ = self.em_write_u16(bus, rxd + RXD_UNKNOWN_10, 0);
        // `RXDONE` LAST, after every other field and the payload: it is the
        // handshake `r_lld_rxdesc_check` gates on, so setting it earlier would
        // let firmware read a half-written descriptor.
        let _ = self.em_write_u16(bus, rxd + RXD_NEXT, next_word | RXD_DONE);

        // Advance the core's descriptor pointer along the ring the firmware
        // linked, preserving whatever the register's high bits hold.
        let keep = self.reg(RX_DESC_PTR) & !RX_DESC_PTR_MASK;
        if let Some(slot) = self.regs.get_mut((RX_DESC_PTR / 4) as usize) {
            *slot = keep | (u32::from(next_word & RXD_NEXT_PTR_MASK) & RX_DESC_PTR_MASK);
        }

        if bt_trace_enabled() {
            let hex: String = frame.pdu.iter().map(|b| format!("{b:02x} ")).collect();
            let nid = self.node_id;
            eprintln!(
                "[bt{nid}] radio RX ch{channel} aa={access_address:#010x} rxd={rxd:#06x} \
                 et={idx} cs={cs:#06x} label={link_label} pdu={hex}"
            );
        }
        true
    }

    /// Advance every programmed event that has come due as of `elapsed`.
    fn service_radio(&mut self, elapsed: u64, bus: &mut dyn Bus) {
        loop {
            if self.radio.is_none() {
                let Some(idx) = self.prog_queue.pop_front() else {
                    return;
                };
                let Some((start, duration)) = self.read_et_schedule(bus, idx) else {
                    // Exchange memory is not mapped: the event cannot be
                    // decoded, so it is dropped rather than completed with
                    // invented state. Firmware will stall visibly.
                    if bt_trace_enabled() {
                        let nid = self.node_id;
                        eprintln!("[bt{nid}] radio: ET {idx} unreadable (EM unmapped) — dropped");
                    }
                    continue;
                };
                self.radio = Some(RadioEvent {
                    et_idx: idx,
                    phase: RadioPhase::Pending,
                    // A deadline already in the past starts now. The
                    // alternative — refusing a late event — would deadlock the
                    // controller, and the two are indistinguishable whenever
                    // the event is programmed ahead of its instant, which is
                    // what `sch_arb` does.
                    deadline: start.max(elapsed),
                });
                self.radio_duration = duration;
            }
            let Some(ev) = self.radio else { return };
            if elapsed < ev.deadline {
                return;
            }
            match ev.phase {
                RadioPhase::Pending => {
                    self.set_et_status(bus, ev.et_idx, ET_STATUS_ONGOING);
                    // Deliberately NO `sch_prog_tx` (bit 1) here, and the ROM
                    // is the reason rather than trial and error:
                    // `r_lld_adv_frm_cbk` (`0x4001_7550`) dispatches irq_type
                    // 0 and 1 to `lld_adv_frm_isr`, ignores 2 (RX) with a bare
                    // `ret`, forwards 4 (SKIP), and **asserts on anything
                    // else** — `assert lld_adv.c 2328`. `r_sch_prog_tx_isr`
                    // passes irq_type 3. So a legacy advertising event
                    // provably does not raise bit 1 on silicon; raising it
                    // here stopped the controller dead with that exact assert.
                    // Which event types DO raise it was not determined.
                    self.transmit_event(bus, ev.et_idx);
                    // A listening event picks up whatever the air is carrying
                    // on the channel and access address its control structure
                    // programmed. `sch_prog_rx` (bit 2) is raised only when a
                    // frame was actually written into exchange memory —
                    // `r_lld_adv_frm_cbk` ignores irq_type 2 with a bare
                    // `ret`, so it is safe for an advertiser too.
                    if self.receive_event(bus, ev.et_idx, elapsed) {
                        self.raise_irq_bits(INT_SCH_PROG_RX);
                    }
                    self.radio = Some(RadioEvent {
                        phase: RadioPhase::Running,
                        deadline: ev.deadline + self.radio_duration,
                        ..ev
                    });
                }
                RadioPhase::Running => {
                    self.set_et_status(bus, ev.et_idx, ET_STATUS_END);
                    self.raise_irq_bits(INT_SCH_PROG_END);
                    if bt_trace_enabled() {
                        let nid = self.node_id;
                        eprintln!(
                            "[bt{nid}] radio: ET {} end (clkn={})",
                            ev.et_idx,
                            Self::clkn_at(elapsed)
                        );
                    }
                    self.radio = None;
                }
            }
        }
    }

    /// Cycles from `elapsed` to the next thing the radio engine must do, or
    /// `None` when it is idle. Zero when an entry is queued but not decoded —
    /// decoding needs the bus, which only [`Peripheral::on_event`] has.
    fn cycles_to_radio_deadline(&self, elapsed: u64) -> Option<u64> {
        match self.radio {
            Some(ev) => Some(ev.deadline.saturating_sub(elapsed)),
            None if !self.prog_queue.is_empty() => Some(0),
            None => None,
        }
    }
}

impl Peripheral for Esp32c3Bt {
    /// Walk-free: once the bus hands over a cycle clock the comparators ride
    /// scheduled events (`take_scheduled_events` / `on_event`), so each
    /// deadline lands on its exact cycle and the walk has nothing to do. The
    /// C3 walk-free campaign's `EXPECTED_PINNERS` gate is the reason this is
    /// not simply left on the walk. Without a clock the walk does the real
    /// work and the conservative `true` stands.
    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    /// Only tick while a comparator is armed or the line is up — an un-gated
    /// block with nothing scheduled costs nothing.
    fn legacy_tick_active(&self) -> bool {
        self.irq_work_pending()
    }

    /// `INTCNTL` writes arm and disarm the comparators, and an `INTACK` write
    /// drops the level, so walk membership changes outside `tick()`.
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    /// Legacy per-cycle drive: expire the timer comparators and hold the
    /// RW-BLE matrix source up while `INTSTAT` is non-zero. Level-sensitive,
    /// like the WiFi MAC's — it stays asserted until firmware W1C-acks through
    /// `+0x018`. In scheduler mode the walk skips this model entirely and the
    /// same latch happens in `on_event`, at the exact deadline cycle.
    fn tick(&mut self) -> PeripheralTickResult {
        self.sync_from_clock();
        PeripheralTickResult {
            explicit_irqs: (self.int_status() != 0).then(|| vec![RWBLE_IRQ_SOURCE]),
            ..Default::default()
        }
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// Anchor the comparators to `now_cycle` before every MMIO write, so a
    /// firmware ack or re-arm observes a deadline that has just come due.
    fn sync_to(&mut self, now_cycle: u64) {
        if !self.scheduler_mode() {
            return;
        }
        self.sync_cycle.set(now_cycle);
        if self.clock_base.is_some() {
            self.latch_at(self.elapsed_at(now_cycle));
        }
    }

    /// Arm the nearest comparator deadline as a single in-flight event, under
    /// a fresh generation so an event scheduled against an older target dies
    /// on arrival. The `- 1` mirrors `ledc`: the bus turns a write-path delay
    /// into the absolute deadline `anchor + 1 + delay`.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() || self.clock_base.is_none() {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        // Anchored on the bus's `current_cycle` (handed over by `sync_to` just
        // before this write), not on the published clock, so the deadline the
        // bus builds as `current_cycle + 1 + delay` lands on the exact cycle.
        let anchor = self.sync_cycle.get().max(self.clock_now());
        match self.cycles_to_next_deadline(self.elapsed_at(anchor)) {
            Some(cycles) => vec![(cycles.saturating_sub(1), self.arm_seq)],
            None => Vec::new(),
        }
    }

    /// Fire the comparator this event was scheduled for at its exact cycle,
    /// then chain to the next armed one. The bus re-derives the matrix source
    /// from [`Self::matrix_irq_sources_into`] after this handler, so the level
    /// goes up here and stays up until firmware acks.
    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != self.arm_seq {
            // Stale chain (re-armed since this event was scheduled): die.
            return crate::sched::EventResult::default();
        }
        let elapsed = self.elapsed_at(sched.now());
        self.latch_at(elapsed);
        // The radio engine runs here and only here: decoding a programmed
        // event means reading the controller's exchange memory, and `on_event`
        // is the one hook that is handed the bus.
        self.service_radio(elapsed, bus);
        crate::sched::EventResult {
            reschedule_delay: self.cycles_to_next_deadline(elapsed),
            ..Default::default()
        }
    }

    /// The live level for the walk-free re-derivation path, same condition as
    /// [`Self::tick`]. Syncs first so a deadline that has just come due is
    /// reflected even between events.
    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        self.sync_from_clock();
        if self.int_status() != 0 {
            out.push(RWBLE_IRQ_SOURCE);
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let w = self.read_u32(offset & !3)?;
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        let cur = *self.regs.get((aligned / 4) as usize).unwrap_or(&0);
        self.write_u32(aligned, (cur & !(0xFFu32 << sh)) | ((value as u32) << sh))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        // Materialise any comparator that has come due, so a poll of
        // INTSTAT/INTRAWSTAT/+0x2D8 between scheduled events is not stale.
        self.sync_from_clock();
        if let Some((_, v)) = HW_IDENTITY.iter().find(|(o, _)| *o == offset) {
            // NOTE (deliberate, documented deviation): silicon reads these as 0
            // while the block is still clock-gated, and we do not model that
            // gate — it lives in SYSTEM/APB_CTRL, not in this window. So they
            // read their hardwired value from cycle 0 rather than from BT
            // enable. The asymmetry is on purpose: reading the ID too early
            // harms nothing (no firmware reads it before enabling BT), while
            // reading 0 too late is a hard controller assert. The counters
            // below keep the honest gate, because they demonstrably restart at
            // BT enable (a `reset halt; resume; sleep 3000` capture read CLKN
            // as 0x799 ≈ 0.6 s, not the 3 s since reset).
            return Ok(*v);
        }
        Ok(match offset {
            CLKN => self.clkn(),
            CLKN_FINE => self.clkn_fine(),
            // Derived, not stored: silicon reads INTSTAT == INTRAWSTAT & INTCNTL.
            INTSTAT => self.int_status(),
            INTRAWSTAT => self.int_raw.get(),
            // W1C / command registers read back 0 on a live advertising part.
            // `+0x100` is the programmed-event push: the ROM writes
            // `0x8000_000D` and the live window still reads 0.
            INTACK | INTACK_FIFO | PROG_PUSH => 0,
            IRQ_FIFO => self.irq_fifo_word(),
            _ => self.reg(offset),
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // `LABWIRED_BT_TRACE=1` mirrors the WiFi MAC's `LABWIRED_MAC_TRACE`:
        // dump the controller's register programming so a stall can be read
        // straight off the tail of the log and compared with the OpenOCD write
        // trace this model was built from.
        if bt_trace_enabled() {
            let nid = self.node_id;
            eprintln!(
                "[bt{nid}] +{offset:#05x} <= {value:#010x}  (clkn={} fine={})",
                self.clkn(),
                self.clkn_fine()
            );
        }
        // First touch of the block = the controller un-gated it; start CLKN
        // here so reads before BLE bring-up stay 0 like gated silicon.
        if self.clock_base.is_none() {
            self.clock_base = Some(self.clock.as_ref().map(|c| c.now()).unwrap_or(0));
        }
        match offset {
            // Arm the next-event comparator. Deliberately NOT stored into
            // `regs`: a subsequent read of this offset must return the running
            // clock (silicon capture 2026-08-02 — the BT ROM writes
            // 0x8000_xxxx here and immediately reads back a plain counter).
            CLKN => {
                self.event_armed = value & CLKN_TARGET_ARM != 0;
                self.event_target = value & !CLKN_TARGET_ARM;
            }
            // Re-asserting an enable re-arms that comparator: `r_rwip_timer_*_set`
            // finishes by OR-ing its bit back into INTCNTL, and the disable
            // paths AND it out. Only the 0->1 edge re-arms, so an unrelated
            // read-modify-write of INTCNTL does not resurrect a spent one.
            INTCNTL => {
                let rearmed = value & !self.int_enable();
                self.comparators_fired
                    .set(self.comparators_fired.get() & !rearmed);
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
            // Consume the self-clearing command bits — bit31's kick and the
            // two abort REQUESTS, bits 25 (advertising) and 24 (scanning). The
            // hardware takes each one and drops it, so none of them may read
            // back set; see [`RWBLECNTL_SELF_CLEARING`] for the ROM sites and
            // the silicon/twin values that pin all three.
            //
            // What the core does with an abort request BEYOND clearing it —
            // ending the event in flight early — is still not modelled, and
            // that is a measured decision rather than an omission. See the
            // "What the advertising stop does NOT do here" section of the
            // module docs for the cost, in CLKN ticks, off the twin's trace.
            RWBLECNTL => {
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value & !RWBLECNTL_SELF_CLEARING;
                }
            }
            // W1C on the raw latch. `+0x38C` is the mirror the ROM writes
            // alongside (and, on the timer-disable paths, instead of) `+0x018`.
            INTACK | INTACK_FIFO => {
                self.int_raw.set(self.int_raw.get() & !value);
            }
            // `INTSTAT` is derived and `INTRAWSTAT` is driven by the hardware:
            // neither is a storage slot, so a write to them is dropped rather
            // than allowed to shadow the derivation.
            INTSTAT | INTRAWSTAT => {}
            // Self-clearing command: execute the exchange-table entry named in
            // bits[3:0]. Silicon reads `+0x100` back as 0, so nothing is
            // stored. The entry itself cannot be decoded here — that needs the
            // bus — so it is queued and `on_event` picks it up at once.
            PROG_PUSH => {
                if value & PROG_PUSH_GO != 0 {
                    let idx = value & PROG_PUSH_IDX;
                    if (self.prog_queue.len() as u32) < ET_ENTRIES {
                        self.prog_queue.push_back(idx);
                    }
                }
            }
            // The RX-descriptor jump request. The 0->1 edge of bit15 is what
            // raises `lld_update_rxbuf_isr` (bit 18) — see [`RX_BUF_JUMP`] for
            // the ROM evidence. The register itself is plain storage, because
            // the ISR read-modify-writes it to clear the bit.
            //
            // The core also adopts the requested descriptor as its current one:
            // `lld_update_rxbuf_handler` re-seeds the SOFTWARE cursor
            // `p_lld_env[216]` from this register, and the ROM's own trace names
            // the pair `"Current %04x[%d]"` (= `+0x024`) and `"Jump %04x[%d]"`
            // (= this request), so the two halves are meant to land on the same
            // descriptor. That the hardware pointer follows the jump is an
            // INFERENCE from that resync, not something measured directly, and
            // it is flagged as such — but the alternative silently desynchronises
            // the core from its own link layer, which is the failure this whole
            // path exists to avoid.
            RX_BUF_JUMP => {
                let rising = value & !self.reg(RX_BUF_JUMP) & RX_BUF_JUMP_GO;
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
                if rising != 0 {
                    let target = value & RX_BUF_JUMP_PTR_MASK;
                    let keep = self.reg(RX_DESC_PTR) & !RX_DESC_PTR_MASK;
                    if let Some(slot) = self.regs.get_mut((RX_DESC_PTR / 4) as usize) {
                        *slot = keep | target;
                    }
                    if bt_trace_enabled() {
                        let nid = self.node_id;
                        eprintln!("[bt{nid}] rxbuf jump -> {target:#06x}, raising bit 18");
                    }
                    self.raise_irq_bits(INT_LLD_UPDATE_RXBUF);
                }
            }
            // Bit 0 pops the head entry. Bit 31 is the ISR's fatal-path flag;
            // it is stored nowhere because nothing here reads it back.
            IRQ_FIFO => {
                if value & 1 != 0 {
                    self.irq_fifo.borrow_mut().pop_front();
                }
            }
            // Programming a target arms its comparator (the enable still has
            // to be set — the ROM ORs it in immediately after).
            TIMER_10MS_TARGET | TIMER_HS_TARGET | TIMER_HUS_TARGET => {
                let bit = match offset {
                    TIMER_10MS_TARGET => INT_TIMER_10MS,
                    TIMER_HS_TARGET => INT_TIMER_HS,
                    _ => INT_TIMER_HUS,
                };
                self.comparators_programmed |= bit;
                self.comparators_fired
                    .set(self.comparators_fired.get() & !bit);
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
            _ => {
                if let Some(slot) = self.regs.get_mut((offset / 4) as usize) {
                    *slot = value;
                }
            }
        }
        Ok(())
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Silicon capture 2026-08-02: the whole window reads `00000000` at
    /// `reset halt` (clock-gated), so an untouched model must too — everywhere
    /// except the two hardwired identity words, which are deliberately always
    /// readable (see the note in `read_u32`).
    #[test]
    fn gated_window_reads_zero() {
        let bt = Esp32c3Bt::new();
        for off in [0x000u64, 0x01C, 0x020, 0x024, 0x204, 0x2C4, 0x370, 0x530] {
            assert_eq!(bt.read_u32(off).unwrap(), 0, "offset {off:#05x} at reset");
        }
    }

    /// `RWBLECNTL` bit31 is a self-clearing command bit: the controller writes
    /// the control word, then writes it again with bit31 set as a kick, and
    /// spins until the bit reads back clear. Silicon reads `+0x000` as
    /// `0x0010_070f` (bit31 clear) on a live part whose last write was
    /// `0x8010_070f`. Regression for the stall that pinned the twin on the
    /// instruction after that store.
    #[test]
    fn rwblecntl_command_bit_self_clears() {
        let mut bt = Esp32c3Bt::new();
        bt.write_u32(RWBLECNTL, 0x0010_070f).unwrap();
        assert_eq!(bt.read_u32(RWBLECNTL).unwrap(), 0x0010_070f);
        bt.write_u32(RWBLECNTL, 0x8010_070f).unwrap();
        assert_eq!(
            bt.read_u32(RWBLECNTL).unwrap(),
            0x0010_070f,
            "bit31 must be consumed, not stored — otherwise the controller \
             spins forever waiting for its own kick to clear"
        );
    }

    /// **The two abort REQUEST bits read back clear, like bit31.** Silicon
    /// reads `+0x000 = 0x0010_070f` on a live advertising part whose own
    /// firmware writes bit 25 (`r_lld_adv_stop` `0x4001_8A2C`,
    /// `r_lld_per_adv_stop` `0x4002_3E48`, `r_lld_rpa_renew_evt_start_cbk`
    /// `0x4001_FF06`) and bit 24 (`r_lld_scan_end` `0x4002_4634`,
    /// `r_lld_rpa_renew_evt_start_cbk` `0x4001_FF18`). No ROM site ever writes
    /// a zero into either, and none reads either back, so nothing in software
    /// could clear them.
    ///
    /// The values here are the exact ones off the two sides: what the C3's
    /// firmware stores, and what OpenOCD reads back afterwards.
    #[test]
    fn rwblecntl_abort_requests_read_back_clear() {
        for request in [0x0210_070fu32, 0x0110_070f, 0x0310_070f] {
            let mut bt = Esp32c3Bt::new();
            bt.write_u32(RWBLECNTL, 0x0010_070f).unwrap();
            bt.write_u32(RWBLECNTL, request).unwrap();
            assert_eq!(
                bt.read_u32(RWBLECNTL).unwrap(),
                0x0010_070f,
                "wrote {request:#010x}; every live dump of an advertising C3 \
                 reads +0x000 back as 0x0010_070f, abort requests consumed"
            );
        }
    }

    /// **The firmware's OWN next control word proves it**, which is what makes
    /// this a silicon check rather than a restatement of the line above.
    ///
    /// `r_lld_rpa_renew_evt_start_cbk` (`0x4001_FEE0`) aborts both activities
    /// back to back, and the second store is computed from what the first one
    /// read BACK:
    ///
    /// ```text
    /// 4001ff06: lw   a5,0(a4)      ; read RWBLECNTL
    /// 4001ff0e: and  a5,a5,a3      ; a3 = 0xfdffffff
    /// 4001ff14: or   a5,a5,a3      ; a3 = 0x02000000   -> set bit 25
    /// 4001ff16: sw   a5,0(a4)
    /// 4001ff18: lw   a5,0(a4)      ; read it BACK
    /// 4001ff20: and  a5,a5,a3      ; a3 = 0xfeffffff
    /// 4001ff26: or   a5,a5,a3      ; a3 = 0x01000000   -> set bit 24
    /// 4001ff28: sw   a5,0(a4)
    /// ```
    ///
    /// So the second word is `(read_back & 0xFEFF_FFFF) | 0x0100_0000`. On
    /// silicon the read-back has bit 25 already gone, so that is
    /// `0x0110_070f`. A model that latches the request makes the same
    /// firmware compute `0x0310_070f` — which is exactly what the twin's
    /// trace showed at CLKN 37 on both BLE Pong nodes, and a word the real
    /// part can never produce.
    #[test]
    fn the_rpa_renew_sequence_computes_the_silicon_control_word() {
        let mut bt = Esp32c3Bt::new();
        bt.write_u32(RWBLECNTL, 0x0010_070f).unwrap();

        // Replay `r_lld_rpa_renew_evt_start_cbk` instruction for instruction.
        let first = (bt.read_u32(RWBLECNTL).unwrap() & 0xFDFF_FFFF) | 0x0200_0000;
        assert_eq!(first, 0x0210_070f, "the abort-advertising store");
        bt.write_u32(RWBLECNTL, first).unwrap();

        let second = (bt.read_u32(RWBLECNTL).unwrap() & 0xFEFF_FFFF) | 0x0100_0000;
        assert_eq!(
            second, 0x0110_070f,
            "the abort-scanning store the firmware computes from its own \
             read-back. 0x0310_070f means bit 25 was still there to be read, \
             which is the twin latching a request the core consumes"
        );
    }

    /// The controller validates `VERSION` during `lld` bring-up and asserts on
    /// a mismatch, quoting the value it wants:
    /// `assert lld.c 318, param 00000000 09001b00`. Regression for that stop.
    #[test]
    fn hardware_identity_words_read_their_silicon_values() {
        let mut bt = Esp32c3Bt::new();
        assert_eq!(bt.read_u32(0x004).unwrap(), 0x0900_1b00, "VERSION");
        assert_eq!(bt.read_u32(0x008).unwrap(), 0x0f22_d0b0, "RWBLECONF");
        // Read-only: a stray write must not be able to break the assert.
        bt.write_u32(0x004, 0xdead_beef).unwrap();
        bt.write_u32(0x008, 0xdead_beef).unwrap();
        assert_eq!(bt.read_u32(0x004).unwrap(), 0x0900_1b00, "VERSION is RO");
        assert_eq!(bt.read_u32(0x008).unwrap(), 0x0f22_d0b0, "RWBLECONF is RO");
    }

    /// The register-backed majority: BLE bring-up is read-modify-write, so a
    /// written value must read straight back. Values are real ones from the
    /// silicon write trace.
    #[test]
    fn window_is_register_backed() {
        let mut bt = Esp32c3Bt::new();
        for (off, val) in [
            (0x204u64, 0x0002_9725u32), // ROM patch/veneer table entry 0
            (0x2c4, 0x07fe_01ff),       // patch-enable mask, fully populated
            (0x0e0, 0x0190_012c),       // advertising interval pair
            (0x530, 0x0000_0001),
        ] {
            bt.write_u32(off, val).unwrap();
            assert_eq!(bt.read_u32(off).unwrap(), val, "offset {off:#05x}");
        }
    }

    /// `CLKN` is read/write asymmetric: the BT ROM writes a comparator target
    /// with the arm bit and immediately re-reads the *clock*. If the write were
    /// stored the scheduler would read its own deadline back as "now" and
    /// re-arm the same instant forever.
    #[test]
    fn clkn_write_arms_comparator_and_does_not_shadow_the_clock() {
        let mut bt = Esp32c3Bt::new();
        let clock = CycleClock::default();
        bt.attach_cycle_clock(clock.clone());
        clock.publish(0);
        bt.write_u32(CLKN, 0x8000_e8f7).unwrap(); // a real traced value
        assert_eq!(bt.armed_event_target(), Some(0x0000_e8f7));
        assert_eq!(
            bt.read_u32(CLKN).unwrap(),
            0,
            "CLKN read must be the clock, not the armed target"
        );
        // A write without the arm bit disarms.
        bt.write_u32(CLKN, 0x0000_1234).unwrap();
        assert_eq!(bt.armed_event_target(), None);
    }

    /// CLKN advances at the Bluetooth native rate (312.5 µs / 3200 Hz), and the
    /// fine counter wraps `0..=624` once per CLKN tick.
    #[test]
    fn timebase_advances_at_the_bluetooth_native_rate() {
        let mut bt = Esp32c3Bt::new();
        let clock = CycleClock::default();
        bt.attach_cycle_clock(clock.clone());
        clock.publish(1_000); // un-gate at a non-zero cycle
        bt.write_u32(0x000, 0x0010_060f).unwrap();
        assert_eq!(bt.read_u32(CLKN).unwrap(), 0, "CLKN starts at the un-gate");

        // One second of device time at 160 MHz = 3200 CLKN ticks.
        clock.publish(1_000 + 160_000_000);
        assert_eq!(bt.read_u32(CLKN).unwrap(), 3200);

        // Fine counter: half-µs ticks, counting DOWN 624 -> 0 once per CLKN
        // tick (the direction `r_rwip_time_get`'s `624 - FINETIMECNT` and
        // `r_rwip_timer_hus_set`'s `624 - hus` both require).
        clock.publish(1_000);
        assert_eq!(bt.read_u32(CLKN_FINE).unwrap(), 624, "starts full");
        clock.publish(1_000 + CYCLES_PER_FINE_TICK * (624 - 566));
        assert_eq!(bt.read_u32(CLKN_FINE).unwrap(), 566); // max value seen on silicon
        assert_eq!(bt.read_u32(CLKN).unwrap(), 0, "still inside the first tick");
        clock.publish(1_000 + CYCLES_PER_CLKN_TICK);
        assert_eq!(bt.read_u32(CLKN_FINE).unwrap(), 624, "reloads with CLKN");
        assert_eq!(bt.read_u32(CLKN).unwrap(), 1);

        // Never leaves the range silicon showed.
        for n in 0..2_000u64 {
            clock.publish(1_000 + n * 137);
            assert!(bt.read_u32(CLKN_FINE).unwrap() < FINE_TICKS_PER_CLKN as u32);
        }
    }

    /// Bring a model up to the point a live advertising part is at: block
    /// un-gated, the enable word silicon reads, and the hus comparator armed
    /// the way `r_rwip_timer_hus_set` arms it.
    fn advertising_part(clock: &CycleClock) -> Esp32c3Bt {
        let mut bt = Esp32c3Bt::new();
        bt.attach_cycle_clock(clock.clone());
        clock.publish(0);
        bt.write_u32(RWBLECNTL, 0x0010_070f).unwrap(); // un-gate
        bt.write_u32(INTCNTL, 0x0064_0b66).unwrap(); // silicon enable word
        bt
    }

    /// Silicon capture 2026-08-02, board `38:44:be:42:f5:58`: `INTSTAT` is
    /// `INTRAWSTAT & INTCNTL`, not a stored register. Both measured pairs.
    #[test]
    fn int_status_is_raw_and_enable() {
        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        assert_eq!(bt.read_u32(INTCNTL).unwrap(), 0x0064_0b66);

        bt.int_raw.set(0x0000_0011); // measured raw at one halt
        assert_eq!(bt.read_u32(INTSTAT).unwrap(), 0x0000_0000);
        bt.int_raw.set(0x0000_0811); // measured raw at three later halts
        assert_eq!(bt.read_u32(INTSTAT).unwrap(), 0x0000_0800);

        // W1C through INTACK, which itself reads back 0.
        bt.write_u32(INTACK, 0x0000_0800).unwrap();
        assert_eq!(bt.read_u32(INTRAWSTAT).unwrap(), 0x0000_0011);
        assert_eq!(bt.read_u32(INTSTAT).unwrap(), 0);
        assert_eq!(bt.read_u32(INTACK).unwrap(), 0, "INTACK reads 0 on silicon");
        // `+0x38C` is the mirror the ROM writes alongside INTACK.
        bt.int_raw.set(0x0000_0800);
        bt.write_u32(INTACK_FIFO, 0x0000_0800).unwrap();
        assert_eq!(bt.read_u32(INTRAWSTAT).unwrap(), 0);
        assert_eq!(bt.read_u32(INTACK_FIFO).unwrap(), 0);
    }

    /// The half-µs comparator is what drives advertising: `r_rwip_timer_hus_set`
    /// writes the base-time target to `+0x0EC`, the fine target to `+0x0F0`,
    /// acks bit 11 and ORs `0x800` into `INTCNTL`. On the live part `+0x0EC`
    /// sat 119–150 CLKN ticks ahead of `+0x01C` every time it was sampled.
    /// When the timebase gets there the model must raise `INTSTAT` bit 11 and
    /// assert the RW-BLE matrix source — the whole point of this milestone.
    #[test]
    fn hus_comparator_raises_rwble_irq_at_its_target() {
        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);

        // Arm 130 CLKN ticks out, exactly as the ROM does.
        bt.write_u32(TIMER_HUS_TARGET, 130).unwrap();
        bt.write_u32(TIMER_HUS_FINE, 624).unwrap();
        bt.write_u32(INTACK, INT_TIMER_HUS).unwrap();

        // Not yet: one tick short, the line stays down.
        clock.publish(129 * CYCLES_PER_CLKN_TICK);
        assert!(bt.tick().explicit_irqs.is_none());
        assert_eq!(bt.read_u32(INTSTAT).unwrap(), 0);
        assert!(bt.matrix_irq_sources().is_empty());

        // At the target: raw latches, INTSTAT shows it, matrix source 8 up.
        clock.publish(130 * CYCLES_PER_CLKN_TICK);
        assert_eq!(
            bt.tick().explicit_irqs,
            Some(vec![RWBLE_IRQ_SOURCE]),
            "the hus comparator must raise the RWBLE matrix source"
        );
        assert_eq!(bt.read_u32(INTRAWSTAT).unwrap(), INT_TIMER_HUS);
        assert_eq!(bt.read_u32(INTSTAT).unwrap(), INT_TIMER_HUS);
        assert_eq!(bt.matrix_irq_sources(), vec![RWBLE_IRQ_SOURCE]);

        // Level-sensitive: it stays up until firmware W1C-acks, exactly like
        // the WiFi MAC's event level.
        clock.publish(131 * CYCLES_PER_CLKN_TICK);
        assert_eq!(bt.tick().explicit_irqs, Some(vec![RWBLE_IRQ_SOURCE]));
        bt.write_u32(INTACK, INT_TIMER_HUS).unwrap();
        assert!(bt.matrix_irq_sources().is_empty());
        assert!(bt.tick().explicit_irqs.is_none());

        // Re-arming pushes the deadline out again — the advertising cadence.
        bt.write_u32(TIMER_HUS_TARGET, 280).unwrap();
        clock.publish(279 * CYCLES_PER_CLKN_TICK);
        assert!(bt.tick().explicit_irqs.is_none());
        clock.publish(280 * CYCLES_PER_CLKN_TICK);
        assert_eq!(bt.tick().explicit_irqs, Some(vec![RWBLE_IRQ_SOURCE]));
    }

    /// A comparator runs only while its `INTCNTL` enable is set. Silicon
    /// attests it: `+0x0E8` held a long-past `0x91` while CLKN was `0x462F`
    /// with `INTCNTL` bit 10 clear, and `INTRAWSTAT` bit 10 read 0. Modelling
    /// it the other way would raise a phantom interrupt out of a stale target.
    #[test]
    fn a_masked_comparator_does_not_latch() {
        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        assert_eq!(
            bt.read_u32(INTCNTL).unwrap() & INT_TIMER_HS,
            0,
            "the silicon enable word leaves the hs timer disarmed"
        );
        bt.write_u32(TIMER_HS_TARGET, 0x91).unwrap();
        clock.publish(0x462f * CYCLES_PER_CLKN_TICK);
        bt.tick();
        assert_eq!(
            bt.read_u32(INTRAWSTAT).unwrap() & INT_TIMER_HS,
            0,
            "a stale target behind a clear enable must not latch"
        );
        // Arm it (INTCNTL |= 0x400, as `r_rwip_timer_hs_set` does) and the same
        // stale target fires at once — a missed deadline, not a 23-hour wrap.
        bt.write_u32(INTCNTL, 0x0064_0b66 | INT_TIMER_HS).unwrap();
        assert_eq!(bt.tick().explicit_irqs, Some(vec![RWBLE_IRQ_SOURCE]));
        assert_eq!(
            bt.read_u32(INTRAWSTAT).unwrap() & INT_TIMER_HS,
            INT_TIMER_HS
        );
    }

    /// The 10 ms comparator counts in units of 32 CLKN ticks —
    /// `r_rwip_timer_10ms_set` writes the target to `+0x0E4` and keeps
    /// `rwip_env+8 = target << 5` (half-slots) alongside it.
    #[test]
    fn ten_ms_comparator_counts_in_32_clkn_units() {
        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        assert_ne!(bt.read_u32(INTCNTL).unwrap() & INT_TIMER_10MS, 0);
        bt.write_u32(TIMER_10MS_TARGET, 100).unwrap(); // 1 s = 3200 CLKN ticks
        clock.publish(3199 * CYCLES_PER_CLKN_TICK);
        assert!(bt.tick().explicit_irqs.is_none());
        clock.publish(3200 * CYCLES_PER_CLKN_TICK);
        assert_eq!(bt.tick().explicit_irqs, Some(vec![RWBLE_IRQ_SOURCE]));
        assert_eq!(
            bt.read_u32(INTSTAT).unwrap() & INT_TIMER_10MS,
            INT_TIMER_10MS
        );
    }

    /// The IRQ FIFO at `+0x2D8`. `sdk_cfg_priv_opts[69]` reads 1 on this
    /// silicon, so `r_rwble_isr` dispatches from here rather than from a raw
    /// `INTSTAT` read — and returns WITHOUT acking when `cnt == 0`, which
    /// would turn a raised level into an interrupt storm. Silicon read
    /// `+0x2D8 = 0x0020_003E` with `INTSTAT = 0x800`: cnt 1, rem 15,
    /// bitmap `0x800`.
    #[test]
    fn irq_fifo_matches_the_silicon_word() {
        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        assert_eq!(
            bt.read_u32(IRQ_FIFO).unwrap(),
            0x0000_001E,
            "empty FIFO: the exact word silicon reads while idle (cnt 0, rem 15)"
        );

        bt.write_u32(TIMER_HUS_TARGET, 130).unwrap();
        bt.write_u32(TIMER_HUS_FINE, 624).unwrap();
        clock.publish(130 * CYCLES_PER_CLKN_TICK);
        bt.tick();
        assert_eq!(
            bt.read_u32(IRQ_FIFO).unwrap(),
            0x0020_003E,
            "one queued hus interrupt must read back the exact silicon word"
        );

        // `ori a5,a5,1; sw` pops the head.
        bt.write_u32(IRQ_FIFO, 0x0020_003F).unwrap();
        assert_eq!(bt.read_u32(IRQ_FIFO).unwrap() >> 5 & 31, 0, "cnt back to 0");
        assert_eq!(bt.read_u32(IRQ_FIFO).unwrap() >> 10, 0, "no head bitmap");
        // Popping the FIFO is NOT the ack: the raw latch is separate, and the
        // ISR clears it through `+0x018`.
        assert_eq!(bt.read_u32(INTRAWSTAT).unwrap(), INT_TIMER_HUS);
    }

    /// One entry per rising edge, capped at the 16-deep FIFO — never a
    /// re-queue while the same bit is still latched, which would let one
    /// unacked interrupt flood the queue.
    #[test]
    fn irq_fifo_queues_one_entry_per_rising_edge() {
        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        bt.write_u32(TIMER_HUS_TARGET, 10).unwrap();
        bt.write_u32(TIMER_HUS_FINE, 624).unwrap();
        for n in 10..40u64 {
            clock.publish(n * CYCLES_PER_CLKN_TICK);
            bt.tick();
        }
        assert_eq!(bt.irq_fifo.borrow().len(), 1, "still one unacked interrupt");
    }

    /// The walk must not run while there is nothing scheduled, and must run
    /// the moment a comparator is armed or the line is up.
    #[test]
    fn walk_membership_follows_the_comparators() {
        let clock = CycleClock::default();
        let mut bt = Esp32c3Bt::new();
        bt.attach_cycle_clock(clock.clone());
        clock.publish(0);
        // Walk membership is only claimed in legacy mode; under the scheduler
        // the comparators ride events instead (and the C3 walk-pinner ledger
        // requires that).
        assert_eq!(bt.needs_legacy_walk(), !bt.uses_scheduler());
        assert!(bt.legacy_tick_dynamic());
        assert!(!bt.legacy_tick_active(), "gated block has nothing to tick");
        bt.write_u32(RWBLECNTL, 0x0010_070f).unwrap();
        assert!(!bt.legacy_tick_active(), "un-gated but nothing armed");
        bt.write_u32(INTCNTL, INT_TIMER_HUS).unwrap();
        assert!(
            !bt.legacy_tick_active(),
            "an enable over an unprogrammed target is not an armed comparator"
        );
        bt.write_u32(TIMER_HUS_TARGET, 130).unwrap();
        assert!(bt.legacy_tick_active(), "hus comparator armed");
    }

    /// Scheduler mode: the hus comparator must arrive as a scheduled event at
    /// its exact cycle, with no per-cycle walk and no firmware poll — the same
    /// contract `ledc` holds for `LSTIMERx_OVF`. This is what keeps the model
    /// off the C3 walk-pinner ledger.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn scheduled_event_delivers_the_comparator_without_a_walk() {
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        assert!(bt.uses_scheduler(), "a clocked model is scheduler-driven");
        assert!(!bt.needs_legacy_walk(), "and must not pin the walk");

        // Arm 130 CLKN ticks out, exactly as `r_rwip_timer_hus_set` does.
        bt.write_u32(TIMER_HUS_TARGET, 130).unwrap();
        bt.write_u32(TIMER_HUS_FINE, 624).unwrap();
        bt.write_u32(INTACK, INT_TIMER_HUS).unwrap();
        let events = bt.take_scheduled_events();
        assert_eq!(events.len(), 1, "one in-flight comparator event");
        let (delay, token) = events[0];
        assert_eq!(
            delay,
            130 * CYCLES_PER_CLKN_TICK - 1,
            "the bus adds the +1 anchor offset back"
        );

        let mut sched = EventScheduler::new();
        let mut bus = crate::bus::SystemBus::new();
        sched.advance_to(130 * CYCLES_PER_CLKN_TICK);
        clock.publish(130 * CYCLES_PER_CLKN_TICK);
        let res = bt.on_event(token, &mut sched, &mut bus);
        assert_eq!(bt.read_u32(INTSTAT).unwrap(), INT_TIMER_HUS);
        assert_eq!(bt.matrix_irq_sources(), vec![RWBLE_IRQ_SOURCE]);
        assert!(
            res.reschedule_delay.is_none(),
            "nothing else armed, so the chain stops until firmware re-arms"
        );

        // A stale generation must not fire anything. The clock stays behind
        // the new deadline so the lazy read-path latch cannot mask the check.
        bt.write_u32(INTACK, INT_TIMER_HUS).unwrap();
        bt.write_u32(TIMER_HUS_TARGET, 300).unwrap();
        let fresh = bt.take_scheduled_events()[0].1;
        assert_ne!(token, fresh, "re-arming stamps a fresh generation");
        sched.advance_to(300 * CYCLES_PER_CLKN_TICK);
        bt.on_event(token, &mut sched, &mut bus);
        assert_eq!(bt.read_u32(INTSTAT).unwrap(), 0, "stale token is inert");
        bt.on_event(fresh, &mut sched, &mut bus);
        assert_eq!(bt.read_u32(INTSTAT).unwrap(), INT_TIMER_HUS);
    }

    // ── Programmed radio events ─────────────────────────────────────────────

    /// A flat byte-addressed memory standing in for the C3's data RAM, so the
    /// radio engine can be driven without a whole `Machine`.
    #[derive(Default)]
    // Fixture for the event-scheduler tests only; dead without the feature.
    #[cfg(feature = "event-scheduler")]
    struct RamBus {
        ram: std::collections::HashMap<u64, u8>,
        cfg: crate::SimulationConfig,
    }

    // Fixture for the event-scheduler tests only; dead without the feature.
    #[cfg(feature = "event-scheduler")]
    impl RamBus {
        fn put(&mut self, addr: u64, bytes: &[u8]) {
            for (i, b) in bytes.iter().enumerate() {
                self.ram.insert(addr + i as u64, *b);
            }
        }
        fn u16_at(&self, addr: u64) -> u16 {
            u16::from(*self.ram.get(&addr).unwrap_or(&0))
                | (u16::from(*self.ram.get(&(addr + 1)).unwrap_or(&0)) << 8)
        }
    }

    #[cfg(feature = "event-scheduler")]
    impl crate::Bus for RamBus {
        fn read_u8(&self, addr: u64) -> SimResult<u8> {
            Ok(*self.ram.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
            self.ram.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[crate::DmaRequest]) -> SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &crate::SimulationConfig {
            &self.cfg
        }
    }

    /// Base CPU address the fixture maps exchange memory at — the same
    /// `0x3FC0_0000` data-RAM window `r_emi_get_mem_addr_by_offset` resolves
    /// into. Offset chosen to match the live part's `0x3FCA_5C94`.
    // Fixture for the event-scheduler tests only; dead without the feature.
    #[cfg(feature = "event-scheduler")]
    const FIXTURE_EM_BASE: u64 = 0x3FCA_5C94;

    /// Program a base register that maps the 1 KiB exchange-memory bucket
    /// starting at `em_off` to `cpu_addr`, in the exact encoding
    /// `r_emi_get_mem_addr_by_offset` decodes:
    /// bits[31:18] = `em_off >> 2`, bits[17:0] = `cpu_addr >> 2`.
    // Only the event-scheduler tests stage a programmed event; without the
    // feature this helper has no caller and clippy is right to say so.
    #[cfg(feature = "event-scheduler")]
    fn em_base_reg(em_off: u32, cpu_addr: u64) -> u32 {
        ((em_off >> 2) << 18) | (((cpu_addr as u32) & EM_RAM_ADDR_MASK) >> 2)
    }

    /// Stage exactly what the live board had staged: the exchange table at EM
    /// `0x000`, the control structure at `0x400`, the TX descriptor at
    /// `0x1400` and the advertising payload at `0x2400`, then push entry 0.
    ///
    /// Every byte here is a value read off board `38:44:be:42:f5:58` on
    /// 2026-08-02 (silicon capture), except the start time, which is set to
    /// the caller's `start_clkn` so the test can drive the schedule.
    // Only the event-scheduler tests stage a programmed event; without the
    // feature this helper has no caller and clippy is right to say so.
    #[cfg(feature = "event-scheduler")]
    fn stage_advertising_event(bt: &mut Esp32c3Bt, bus: &mut RamBus, start_clkn: u32) {
        // Exchange-memory windows. Laid out non-contiguously on purpose: the
        // live part's allocator packs them (EM 0x400 lands only 0x158 bytes
        // after EM 0x000), so a model that assumed a flat map would break.
        let map = [
            (0x0000u32, FIXTURE_EM_BASE),
            (0x0400, FIXTURE_EM_BASE + 0x158),
            (0x1400, FIXTURE_EM_BASE + 0x800),
            (0x2400, FIXTURE_EM_BASE + 0xC00),
        ];
        for (i, (em_off, cpu)) in map.iter().enumerate() {
            bt.write_u32(
                EM_BASE_REG_BANK_A + (i as u64) * 4,
                em_base_reg(*em_off, *cpu),
            )
            .unwrap();
        }

        // ET entry 0: status 0, start at `start_clkn` with fine offset 624
        // (= hus 0), CS pointer 0x200 (EM 0x400), duration 0x0AF7.
        let et = FIXTURE_EM_BASE;
        bus.put(et, &0x2802u16.to_le_bytes()); // +0x0 control, status field 0
        bus.put(et + 2, &(start_clkn as u16).to_le_bytes());
        bus.put(
            et + 4,
            &(((start_clkn >> 16) & 0x0FFF) as u16).to_le_bytes(),
        );
        bus.put(et + 6, &624u16.to_le_bytes());
        bus.put(et + 8, &0x0200u16.to_le_bytes());
        bus.put(et + 10, &0x0AF7u16.to_le_bytes());
        bus.put(et + 12, &0x0C00u16.to_le_bytes());
        bus.put(et + 14, &0x0F00u16.to_le_bytes());

        // Control structure, verbatim from the live part.
        let cs = FIXTURE_EM_BASE + 0x158;
        bus.put(cs, &0x0404u16.to_le_bytes());
        bus.put(cs + 0x06, &[0x5a, 0xf5, 0x42, 0xbe, 0x44, 0x38]);
        bus.put(cs + 0x0C, &0x8E89_BED6u32.to_le_bytes());
        bus.put(cs + 0x10, &[0x55, 0x55, 0x55]);
        bus.put(cs + 0x16, &0x8027u16.to_le_bytes()); // channel 39
        bus.put(cs + 0x1C, &0x1400u16.to_le_bytes());

        // TX descriptor: ADV_IND (header 0x20), 15 bytes, payload at 0x2400.
        let txd = FIXTURE_EM_BASE + 0x800;
        bus.put(txd, &0x140Eu16.to_le_bytes());
        bus.put(txd + 2, &0x0F20u16.to_le_bytes());
        bus.put(txd + 4, &0x2400u16.to_le_bytes());

        // The advertising payload the live board had staged.
        bus.put(
            FIXTURE_EM_BASE + 0xC00,
            &[0x02, 0x01, 0x06, 0x05, 0x12, 0x20, 0x00, 0x40, 0x00],
        );
    }

    /// Stage a **scanning** activity the way the real firmware does: its own
    /// exchange-table entry, its own control structure at `cs_idx` in the
    /// measured scanning format ([`CS_FORMAT_SCAN`], `0x0208` — no TX
    /// descriptor), plus the RX descriptor array and one receive buffer.
    ///
    /// A separate activity from the advertising one on purpose: a node that
    /// advertises AND scans has two, with different control-structure indices,
    /// and getting the receive path to deliver into the right one is exactly
    /// what these tests are about.
    #[cfg(feature = "event-scheduler")]
    fn stage_scan_event(
        bt: &mut Esp32c3Bt,
        bus: &mut RamBus,
        et_idx: u32,
        cs_idx: u32,
        start_clkn: u32,
    ) -> (u64, u64) {
        let et = FIXTURE_EM_BASE + u64::from(Esp32c3Bt::et_entry(et_idx));
        let cs_em = EM_CS_OFFSET + cs_idx * CS_STRIDE;
        bus.put(et, &0x2802u16.to_le_bytes());
        bus.put(et + 2, &(start_clkn as u16).to_le_bytes());
        bus.put(
            et + 4,
            &(((start_clkn >> 16) & 0x0FFF) as u16).to_le_bytes(),
        );
        bus.put(et + 6, &624u16.to_le_bytes());
        bus.put(et + 8, &((cs_em / 2) as u16).to_le_bytes());
        bus.put(et + 10, &0x0AF7u16.to_le_bytes());
        bus.put(et + 12, &0x0C00u16.to_le_bytes());
        bus.put(et + 14, &0x0F00u16.to_le_bytes());

        // EM 0x400..0x7FF is one 1 KiB bucket, so the advertising fixture's
        // base register already covers every control structure in it.
        let cs = FIXTURE_EM_BASE + 0x158 + u64::from(cs_em - 0x400);
        bus.put(cs, &0x0208u16.to_le_bytes()); // the measured scan format
        bus.put(cs + 0x06, &[0x5a, 0xf5, 0x42, 0xbe, 0x44, 0x38]);
        bus.put(cs + 0x0C, &0x8E89_BED6u32.to_le_bytes());
        bus.put(cs + 0x10, &[0x55, 0x55, 0x55]);
        bus.put(cs + 0x16, &0x8027u16.to_le_bytes()); // channel 39
        bus.put(cs + 0x1C, &0u16.to_le_bytes()); // listens only

        let rxd_cpu = FIXTURE_EM_BASE + 0x1000;
        let rxbuf_cpu = FIXTURE_EM_BASE + 0x1400;
        bt.write_u32(EM_BASE_REG_BANK_A + 16, em_base_reg(0x1000, rxd_cpu))
            .unwrap();
        bt.write_u32(EM_BASE_REG_BANK_A + 20, em_base_reg(0x1C00, rxbuf_cpu))
            .unwrap();
        // `next = 0x1014` with RXDONE CLEAR: the state firmware's own refill
        // leaves a descriptor in when it hands it to the core.
        bus.put(rxd_cpu, &0x1014u16.to_le_bytes());
        bus.put(rxd_cpu + 0x12, &0x1C00u16.to_le_bytes());
        bt.write_u32(RX_DESC_PTR, 0x0000_1000).unwrap();
        (rxd_cpu, rxbuf_cpu)
    }

    /// The base-register window is decoded exactly as
    /// `r_emi_get_mem_addr_by_offset` does, including the packed non-contiguous
    /// layout the live allocator produces. Values are the live registers.
    #[test]
    fn exchange_memory_offsets_resolve_through_the_base_registers() {
        let mut bt = Esp32c3Bt::new();
        // Silicon capture 2026-08-02, board `38:44:be:42:f5:58`.
        for (i, reg) in [
            0x0002_9725u32,
            0x0402_977B,
            0x0C02_988D,
            0x1002_992B,
            0x1402_9961,
        ]
        .into_iter()
        .enumerate()
        {
            bt.write_u32(EM_BASE_REG_BANK_A + (i as u64) * 4, reg)
                .unwrap();
        }
        assert_eq!(bt.em_cpu_addr(0x0000), Some(0x3FCA_5C94));
        assert_eq!(bt.em_cpu_addr(0x0400), Some(0x3FCA_5DEC));
        assert_eq!(bt.em_cpu_addr(0x1400), Some(0x3FCA_6584));
        // Bucket 2 (EM 0x800..0xBFF) is served by the register that covers
        // 0x400 — exactly what `em_base_reg_lut` encodes.
        assert_eq!(bt.em_cpu_addr(0x0800), Some(0x3FCA_5DEC + 0x400));
        // Offsets inside a region are byte-addressable.
        assert_eq!(bt.em_cpu_addr(0x0406), Some(0x3FCA_5DEC + 6));
        // Nothing mapped yet -> no address, rather than a fabricated one.
        assert_eq!(Esp32c3Bt::new().em_cpu_addr(0), None);
    }

    /// `+0x100` is a self-clearing command register: `r_sch_prog_ble_push`
    /// writes `0x8000_0000 | idx` and the live window reads back 0.
    #[test]
    fn prog_push_is_a_command_register() {
        let mut bt = Esp32c3Bt::new();
        bt.write_u32(PROG_PUSH, 0x8000_000D).unwrap();
        assert_eq!(bt.read_u32(PROG_PUSH).unwrap(), 0, "reads 0 on silicon");
        assert_eq!(bt.prog_queue.front().copied(), Some(13));
        // Without the go bit nothing is queued.
        bt.prog_queue.clear();
        bt.write_u32(PROG_PUSH, 0x0000_0003).unwrap();
        assert!(bt.prog_queue.is_empty());
    }

    /// The whole milestone in one test: a pushed event runs at its programmed
    /// instant, drives its exchange-table status through the ROM's own state
    /// values, emits the PDU the controller staged, and raises `sch_prog_end`
    /// through the IRQ FIFO after its programmed duration.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn a_programmed_event_transmits_the_staged_pdu_and_ends() {
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        let air = crate::peripherals::ble_air::BleAirBus::new();
        bt.air = air.clone();
        let mut bus = RamBus::default();
        let mut sched = EventScheduler::new();

        let start: u32 = 200;
        stage_advertising_event(&mut bt, &mut bus, start);
        bt.write_u32(PROG_PUSH, 0x8000_0000).unwrap();

        // The push schedules immediately: the entry cannot be decoded without
        // the bus, so the engine asks for a zero-delay event to do it.
        let events = bt.take_scheduled_events();
        let token = events[0].1;
        assert_eq!(events[0].0, 0, "decode the entry at once");

        // Before the programmed instant: decoded, but nothing has happened.
        let before = (start as u64 - 1) * CYCLES_PER_CLKN_TICK;
        sched.advance_to(before);
        clock.publish(before);
        let res = bt.on_event(token, &mut sched, &mut bus);
        assert!(air.trace_snapshot().is_empty(), "not transmitted early");
        assert_eq!(bt.read_u32(INTSTAT).unwrap() & INT_SCH_PROG_END, 0);
        assert_eq!(
            res.reschedule_delay,
            Some(CYCLES_PER_CLKN_TICK),
            "chained to the programmed instant"
        );

        // At the programmed instant: status 2 and the PDU is on the air.
        let at = start as u64 * CYCLES_PER_CLKN_TICK;
        sched.advance_to(at);
        clock.publish(at);
        let res = bt.on_event(token, &mut sched, &mut bus);
        assert_eq!(
            (bus.u16_at(FIXTURE_EM_BASE) & ET_STATUS_FIELD) >> ET_STATUS_SHIFT,
            ET_STATUS_ONGOING,
            "the core owns the status field while the event runs"
        );
        let frames = air.trace_snapshot();
        assert_eq!(frames.len(), 1, "exactly one frame per event");
        let f = &frames[0];
        assert_eq!(f.channel, 39, "the channel the hop word named");
        assert_eq!(f.access_address, 0x8E89_BED6);
        assert_eq!(f.crc_init, 0x0055_5555);
        assert_eq!(
            f.pdu,
            vec![
                0x20, 0x0f, // ADV_IND with ChSel, 15 bytes
                0x5a, 0xf5, 0x42, 0xbe, 0x44, 0x38, // AdvA from CS+0x06
                0x02, 0x01, 0x06, 0x05, 0x12, 0x20, 0x00, 0x40, 0x00, // AdvData
            ],
            "the real bytes the controller staged, not a synthesised packet"
        );
        // No `sch_prog_tx`: `r_lld_adv_frm_cbk` asserts on irq_type 3.
        assert_eq!(bt.read_u32(INTRAWSTAT).unwrap() & 0x2, 0);
        assert_eq!(
            bt.read_u32(INTSTAT).unwrap() & INT_SCH_PROG_END,
            0,
            "not over"
        );

        // The duration is the one the entry programmed: 0x0AF7 units of two
        // half-µs = 2807 µs.
        let duration = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
        assert_eq!(res.reschedule_delay, Some(duration));

        // At the end: status 3 and `sch_prog_end`, queued in the FIFO exactly
        // as `r_rwble_isr` expects to find it.
        let end = at + duration;
        sched.advance_to(end);
        clock.publish(end);
        bt.on_event(token, &mut sched, &mut bus);
        assert_eq!(
            (bus.u16_at(FIXTURE_EM_BASE) & ET_STATUS_FIELD) >> ET_STATUS_SHIFT,
            ET_STATUS_END,
        );
        assert_eq!(
            bt.read_u32(INTSTAT).unwrap() & INT_SCH_PROG_END,
            INT_SCH_PROG_END
        );
        assert_eq!(bt.matrix_irq_sources(), vec![RWBLE_IRQ_SOURCE]);
        assert_eq!(
            bt.read_u32(IRQ_FIFO).unwrap(),
            0x0000_803E,
            "cnt 1, rem 15, bitmap 0x20 — the exact word silicon read mid-event"
        );
        assert_eq!(air.trace_snapshot().len(), 1, "one event, one frame");
    }

    /// The model must not invent an event it cannot read. With exchange memory
    /// unmapped the push is dropped and nothing is completed — firmware stalls
    /// visibly instead of being lied to.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn an_undecodable_event_is_dropped_not_faked() {
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        let mut bus = RamBus::default();
        let mut sched = EventScheduler::new();
        bt.write_u32(PROG_PUSH, 0x8000_0000).unwrap();
        let token = bt.take_scheduled_events()[0].1;
        clock.publish(CYCLES_PER_CLKN_TICK);
        sched.advance_to(CYCLES_PER_CLKN_TICK);
        bt.on_event(token, &mut sched, &mut bus);
        assert_eq!(
            bt.read_u32(INTRAWSTAT).unwrap() & INT_SCH_PROG_END,
            0,
            "no end interrupt out of an entry that was never read"
        );
        assert!(bt.radio.is_none());
    }

    /// A control structure whose format is not the measured legacy-advertising
    /// one transmits nothing — but the event still completes, so an unmodelled
    /// activity cannot wedge the controller.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn an_unmeasured_cs_format_transmits_nothing_but_still_ends() {
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        let air = crate::peripherals::ble_air::BleAirBus::new();
        bt.air = air.clone();
        let mut bus = RamBus::default();
        let mut sched = EventScheduler::new();
        stage_advertising_event(&mut bt, &mut bus, 0);
        // Anything but the measured 0x04.
        bus.put(FIXTURE_EM_BASE + 0x158, &0x0405u16.to_le_bytes());
        bt.write_u32(PROG_PUSH, 0x8000_0000).unwrap();
        let token = bt.take_scheduled_events()[0].1;
        let end = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
        for at in [0, end] {
            sched.advance_to(at);
            clock.publish(at);
            bt.on_event(token, &mut sched, &mut bus);
        }
        assert!(air.trace_snapshot().is_empty(), "no invented frame");
        assert_eq!(
            bt.read_u32(INTRAWSTAT).unwrap() & INT_SCH_PROG_END,
            INT_SCH_PROG_END,
            "the event still ends"
        );
    }

    /// A listening event writes the air frame into the RX descriptor at
    /// `+0x024` in the exact layout the ROM reads back, advances the pointer
    /// along the ring, and raises `sch_prog_rx`.
    ///
    /// The activity is a real SCANNING one (its own exchange-table entry and a
    /// control structure in the measured `0x0208` format), programmed alongside
    /// the advertising activity — which is what a node doing both actually
    /// looks like, and what caught the misdelivery bug this fixture now guards.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn a_listening_event_writes_the_frame_into_the_rx_descriptor() {
        use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        let air = BleAirBus::new();
        bt.air = air.clone();
        let mut bus = RamBus::default();
        let mut sched = EventScheduler::new();
        stage_advertising_event(&mut bt, &mut bus, 0);
        let (rxd_cpu, rxbuf_cpu) = stage_scan_event(&mut bt, &mut bus, 1, 1, 0);

        // Somebody else advertises on the channel this control structure names.
        air.transmit(BleAirFrame {
            seq: 0,
            source: bt.node_id + 1,
            channel: 39,
            access_address: 0x8E89_BED6,
            crc_init: 0x0055_5555,
            pdu: vec![0x20, 0x07, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x42],
        });

        // Run the advertising event first (it transmits and, correctly, does
        // NOT consume the peer's frame), then the scan event.
        bt.write_u32(PROG_PUSH, 0x8000_0000).unwrap();
        bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
        let token = bt.take_scheduled_events()[0].1;
        let adv_end = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
        for at in [0, adv_end, adv_end] {
            sched.advance_to(at);
            clock.publish(at);
            bt.on_event(token, &mut sched, &mut bus);
        }

        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_HEADER)),
            (7 << 8) | 0x20,
            "(len << 8) | header, the mirror of the TX descriptor"
        );
        let status = bus.u16_at(rxd_cpu + u64::from(RXD_STATUS));
        assert_eq!(status, RXD_STATUS_GOOD);
        assert_eq!(
            status & RXD_STATUS_ERROR_MASK,
            0,
            "the ROM's own bad-packet mask must clear on it"
        );
        for (i, b) in [0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x42]
            .iter()
            .enumerate()
        {
            assert_eq!(
                bus.read_u8(rxbuf_cpu + i as u64).unwrap(),
                *b,
                "payload byte {i} at the offset +0x12 named"
            );
        }
        assert_eq!(
            bt.read_u32(RX_DESC_PTR).unwrap() & RX_DESC_PTR_MASK,
            0x1014,
            "the pointer walks the ring the firmware linked"
        );
        assert_eq!(
            bt.read_u32(INTSTAT).unwrap() & INT_SCH_PROG_RX,
            INT_SCH_PROG_RX
        );
        // And the same controller never decodes its own transmission: its
        // outgoing ADV_IND is on the very same channel and access address.
        assert_eq!(air.trace_snapshot().len(), 2, "one in, one out");
        assert_eq!(bt.rx_cursor, 1, "cursor past the peer's frame only");

        // ── The three bits `r_lld_rxdesc_check` gates the host report on ─────
        // Getting any one of them wrong is invisible at this level and fatal at
        // the application level: the controller receives and the host never
        // hears about it, which is exactly where this model sat before.
        let w0 = bus.u16_at(rxd_cpu + u64::from(RXD_NEXT));
        assert_eq!(
            w0 & RXD_DONE,
            RXD_DONE,
            "RXDONE must be SET — r_lld_rxdesc_check reports nothing without it"
        );
        assert_eq!(w0 & RXD_NEXT_PTR_MASK, 0x1014, "and the next pointer kept");
        assert_eq!(
            status & RXD_STATUS_RELEASED,
            0,
            "the software-owned released bit must be CLEAR — r_lld_rxdesc_check \
             returns `(status >> 15) ^ 1`, so a set bit means 'already consumed'"
        );
        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_LINK_LABEL)) >> RXD_LINK_LABEL_SHIFT,
            1,
            "link label = the control-structure index: this scan activity's CS \
             is at EM 0x45A = 1024 + 1*90, i.e. index 1, while the advertising \
             activity next to it is index 0"
        );
    }

    /// A receive buffer ABOVE 0x8000 is written where the ROM would read it —
    /// the descriptor's payload pointer is a FULL 16-bit exchange-memory
    /// offset, not a 15-bit one with a flag on top.
    ///
    /// ## Derived from the ROM, not from this file
    ///
    /// `r_ble_util_buf_rx_free` (`0x4000_315C`) range-checks the buffer it is
    /// handed as `((buf - 0x7805) >> 10) & 0xFF <= 8`, so the RX pool is nine
    /// 1 KiB buffers whose data pointers are `0x7805, 0x7C05, 0x8005, 0x8405,
    /// 0x8805, 0x8C05, 0x9005, 0x9405, 0x9805`. **Five of the nine are at or
    /// above 0x8000.** `r_lld_scan_process_pkt_rx_legacy_adv` (`0x4002_46EE`)
    /// and `r_lld_scan_process_pkt_rx_adv_rep` (`0x4002_4878`) read `+0x12`
    /// with `lhu` and a plain zero-extend — no mask anywhere.
    ///
    /// So this test uses `0x8005`, one of the pool's real offsets, and asserts
    /// the bytes land in the 1 KiB window mapped for EM `0x8000`. It also
    /// asserts they do NOT land at `0x0005`, which is where the 0x7FFF mask
    /// this model used to apply put them: inside the EXCHANGE TABLE. That
    /// aliasing is what produced
    /// `assert ble_util_buf.c 180, param 000000e2 00000205` ~198 M steps into
    /// the two-node run — see [`RXD_DATA_PTR`].
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn a_receive_buffer_above_0x8000_is_not_aliased_into_the_exchange_table() {
        use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        let air = BleAirBus::new();
        bt.air = air.clone();
        let mut bus = RamBus::default();
        let mut sched = EventScheduler::new();
        stage_advertising_event(&mut bt, &mut bus, 0);
        let (rxd_cpu, _) = stage_scan_event(&mut bt, &mut bus, 1, 1, 0);

        // Repoint the descriptor at a REAL pool buffer — the third of the nine,
        // the first one with bit15 set — and map the 1 KiB EM bucket it lives
        // in somewhere far away from every other window in the fixture.
        const POOL_BUF: u16 = 0x8005;
        let high_cpu = FIXTURE_EM_BASE + 0x4000;
        bt.write_u32(EM_BASE_REG_BANK_A + 24, em_base_reg(0x8000, high_cpu))
            .unwrap();
        bus.put(rxd_cpu + u64::from(RXD_DATA_PTR), &POOL_BUF.to_le_bytes());

        let et0_before: Vec<u8> = (5..9u64)
            .map(|i| bus.read_u8(FIXTURE_EM_BASE + i).unwrap())
            .collect();

        air.transmit(BleAirFrame {
            seq: 0,
            source: bt.node_id + 1,
            channel: 39,
            access_address: 0x8E89_BED6,
            crc_init: 0x0055_5555,
            pdu: vec![0x20, 0x04, 0xDE, 0xAD, 0xBE, 0xEF],
        });

        bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
        let token = bt.take_scheduled_events()[0].1;
        let end = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
        for at in [0, end] {
            sched.advance_to(at);
            clock.publish(at);
            bt.on_event(token, &mut sched, &mut bus);
        }

        let et0_after: Vec<u8> = (5..9u64)
            .map(|i| bus.read_u8(FIXTURE_EM_BASE + i).unwrap())
            .collect();

        // EM 0x8005 is 5 bytes into the bucket mapped at `high_cpu`.
        for (i, b) in [0xDEu8, 0xAD, 0xBE, 0xEF].iter().enumerate() {
            assert_eq!(
                bus.read_u8(high_cpu + 5 + i as u64).unwrap(),
                *b,
                "payload byte {i} must land at EM {POOL_BUF:#06x}, the offset \
                 the descriptor names and the ROM reads back"
            );
        }
        // And NOT at the 15-bit alias. EM 0x0005 is 5 bytes into the exchange
        // table, whose bucket the fixture maps at FIXTURE_EM_BASE; writing
        // there corrupts exchange-table entry 0 in place. Compared against what
        // the fixture staged rather than against zero, because "unchanged" is
        // the property and zero is not what is there.
        assert_eq!(
            &et0_after[..],
            &et0_before[..],
            "exchange-table entry 0 bytes 5..9 were overwritten by the received \
             payload — the buffer pointer is being masked with 0x7FFF, which \
             folds the top five RX pool buffers onto EM 0x0000..0x1FFF (and \
             0x9005 onto the descriptor ring itself)"
        );
        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_DATA_PTR)),
            POOL_BUF,
            "the buffer pointer is SOFTWARE-owned; the core must not touch it"
        );
    }

    /// A reception writes EVERY core-owned descriptor field, including the ones
    /// this model has nothing to say about.
    ///
    /// A hardware-owned field the model leaves alone is not "unmodelled", it is
    /// whatever the SRAM behind exchange memory last held, and the link layer
    /// reads it as hardware output either way. `+0xE` is the one that proved
    /// it: `r_lld_scan_process_pkt_rx_adv_rep` (`0x4002_4978`) copies it into
    /// the advertising report, and ESP-IDF's `lld_adv_rep_ind` handler
    /// dereferences it as a resolving-list exchange-memory pointer whenever it
    /// is non-zero.
    ///
    /// The descriptor is POISONED first, because that is the only way this test
    /// can fail: a zero-initialised fixture cannot tell "written 0" from "never
    /// written".
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn a_reception_writes_every_core_owned_descriptor_field() {
        use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        let air = BleAirBus::new();
        bt.air = air.clone();
        let mut bus = RamBus::default();
        let mut sched = EventScheduler::new();
        stage_advertising_event(&mut bt, &mut bus, 0);
        let (rxd_cpu, rxbuf_cpu) = stage_scan_event(&mut bt, &mut bus, 1, 1, 0);

        // Poison every core-owned field with a value that would be catastrophic
        // if it survived. 0xFF05 is the exact shape that killed the twin: the
        // handler dereferences `emi_get_mem_addr_by_offset(0xFF05 + 46)` and the
        // ROM asserts `emi.c 159` because `0xFF33 >> 10 = 63 > 50`.
        for off in [RXD_RSSI, RXD_RAL_PTR, RXD_UNKNOWN_10] {
            bus.put(rxd_cpu + u64::from(off), &0xFF05u16.to_le_bytes());
        }

        air.transmit(BleAirFrame {
            seq: 0,
            source: bt.node_id + 1,
            channel: 39,
            access_address: 0x8E89_BED6,
            crc_init: 0x0055_5555,
            pdu: vec![0x20, 0x02, 0x11, 0x22],
        });

        bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
        let token = bt.take_scheduled_events()[0].1;
        let end = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
        for at in [0, end] {
            sched.advance_to(at);
            clock.publish(at);
            bt.on_event(token, &mut sched, &mut bus);
        }

        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_RAL_PTR)),
            0,
            "+0xE is the resolving-list pointer ESP-IDF's lld_adv_rep_ind \
             handler dereferences when non-zero. Address resolution is not \
             modelled, so the core must write 0 — leaving the field alone hands \
             the link layer a stale exchange-memory pointer"
        );
        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_RSSI)),
            0,
            "+0x6 low byte is the raw RSSI r_lld_scan_process_pkt_rx_adv_rep \
             feeds to rf_api.rssi_convert; there is no PHY here, so 0"
        );
        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_UNKNOWN_10)),
            0,
            "+0x10 has no identified reader, but stale is not the same as \
             unmodelled"
        );
        // The reception itself still landed, so this is not passing because
        // nothing happened.
        assert_eq!(bus.read_u8(rxbuf_cpu).unwrap(), 0x11);
        assert_eq!(bus.read_u8(rxbuf_cpu + 1).unwrap(), 0x22);
        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_NEXT)) & RXD_DONE,
            RXD_DONE
        );
    }

    /// An ADVERTISING event does not swallow a frame the scanning activity is
    /// waiting for. This is not tidiness: the core stamps the RUNNING
    /// activity's link label into the descriptor, so a frame delivered to the
    /// advertising activity is stamped with its label,
    /// `r_lld_scan_process_pkt_rx` rejects it as somebody else's, never frees
    /// it, and the descriptor stays `RXDONE` forever. One misdelivery and the
    /// node is permanently deaf — which is exactly what a node advertising and
    /// scanning at once did before this gate existed.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn an_advertising_event_does_not_swallow_the_scanners_frame() {
        use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        let air = BleAirBus::new();
        bt.air = air.clone();
        let mut bus = RamBus::default();
        let mut sched = EventScheduler::new();
        stage_advertising_event(&mut bt, &mut bus, 0);
        // The scan activity lives at a DIFFERENT control-structure index, as it
        // does in real firmware — so a misdelivery would be visible as a wrong
        // label even if the ring survived it.
        let (rxd_cpu, _) = stage_scan_event(&mut bt, &mut bus, 1, 2, 0);

        air.transmit(BleAirFrame {
            seq: 0,
            source: bt.node_id + 1,
            channel: 39,
            access_address: 0x8E89_BED6,
            crc_init: 0x0055_5555,
            pdu: vec![0x20, 0x06, 1, 2, 3, 4, 5, 6],
        });

        // ONLY the advertising event runs.
        bt.write_u32(PROG_PUSH, 0x8000_0000).unwrap();
        let token = bt.take_scheduled_events()[0].1;
        let adv_end = 0x0AF7 * 2 * CYCLES_PER_FINE_TICK;
        for at in [0, adv_end] {
            sched.advance_to(at);
            clock.publish(at);
            bt.on_event(token, &mut sched, &mut bus);
        }
        assert_eq!(air.trace_snapshot().len(), 2, "it transmitted");
        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_HEADER)),
            0,
            "and wrote NOTHING into the RX descriptor"
        );
        assert_eq!(bt.read_u32(INTRAWSTAT).unwrap() & INT_SCH_PROG_RX, 0);
        assert_eq!(bt.rx_cursor, 0, "the frame is still unread");

        // Now the scan event runs and picks it up, stamped with ITS index.
        bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
        let token = bt.take_scheduled_events()[0].1;
        for at in [adv_end, adv_end] {
            sched.advance_to(at);
            clock.publish(at);
            bt.on_event(token, &mut sched, &mut bus);
        }
        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_HEADER)),
            (6 << 8) | 0x20,
            "the scanning activity received it"
        );
        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_LINK_LABEL)) >> RXD_LINK_LABEL_SHIFT,
            2,
            "stamped with the SCAN activity's control-structure index"
        );
    }

    /// The link label the core stamps into `+0x0C` is the *control-structure
    /// index* of the activity that received, not a constant: a scan activity
    /// whose control structure sits at CS index 2 gets label 2. Without this
    /// `r_lld_rxdesc_check` would reject every packet a non-zero-index activity
    /// received, and the host would see nothing while the trace showed a
    /// perfectly healthy reception.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn the_link_label_is_the_control_structure_index() {
        use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        let air = BleAirBus::new();
        bt.air = air.clone();
        let mut bus = RamBus::default();
        let mut sched = EventScheduler::new();
        stage_advertising_event(&mut bt, &mut bus, 0);
        let (rxd_cpu, _) = stage_scan_event(&mut bt, &mut bus, 3, 2, 0);

        air.transmit(BleAirFrame {
            seq: 0,
            source: bt.node_id + 1,
            channel: 39,
            access_address: 0x8E89_BED6,
            crc_init: 0x0055_5555,
            pdu: vec![0x20, 0x06, 1, 2, 3, 4, 5, 6],
        });

        bt.write_u32(PROG_PUSH, 0x8000_0003).unwrap();
        let token = bt.take_scheduled_events()[0].1;
        sched.advance_to(0);
        clock.publish(0);
        bt.on_event(token, &mut sched, &mut bus);

        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_LINK_LABEL)) >> RXD_LINK_LABEL_SHIFT,
            2,
            "the label follows the control-structure index the ET named"
        );
    }

    /// The core does not overwrite a descriptor whose `RXDONE` firmware has not
    /// cleared — that reception has not been consumed yet. Nothing is written,
    /// no `sch_prog_rx` is raised, the pointer does not move, and the frame is
    /// still on the air for the next event to pick up.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn a_descriptor_the_link_layer_still_owns_is_not_overwritten() {
        use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        let air = BleAirBus::new();
        bt.air = air.clone();
        let mut bus = RamBus::default();
        let mut sched = EventScheduler::new();
        stage_advertising_event(&mut bt, &mut bus, 0);
        let (rxd_cpu, rxbuf_cpu) = stage_scan_event(&mut bt, &mut bus, 1, 1, 0);
        // RXDONE still SET: the previous reception has not been released.
        bus.put(rxd_cpu, &(0x1014u16 | RXD_DONE).to_le_bytes());

        air.transmit(BleAirFrame {
            seq: 0,
            source: bt.node_id + 1,
            channel: 39,
            access_address: 0x8E89_BED6,
            crc_init: 0x0055_5555,
            pdu: vec![0x20, 0x02, 0x11, 0x22],
        });

        bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
        let token = bt.take_scheduled_events()[0].1;
        sched.advance_to(0);
        clock.publish(0);
        bt.on_event(token, &mut sched, &mut bus);

        assert_eq!(
            bus.u16_at(rxd_cpu + u64::from(RXD_HEADER)),
            0,
            "not written"
        );
        assert_eq!(bus.read_u8(rxbuf_cpu).unwrap(), 0, "payload not written");
        assert_eq!(bt.read_u32(INTRAWSTAT).unwrap() & INT_SCH_PROG_RX, 0);
        assert_eq!(bt.read_u32(RX_DESC_PTR).unwrap() & RX_DESC_PTR_MASK, 0x1000);
        assert_eq!(bt.rx_cursor, 0, "the frame was not consumed");
        assert!(
            air.receive_from(39, 0x8E89_BED6, 0, bt.node_id).is_some(),
            "and it is still on the air for a later event"
        );
    }

    /// `+0x2D0` bit15 is the RX-buffer jump request, and its **rising edge** is
    /// what raises `lld_update_rxbuf_isr` (bit 18) — the exact two-store
    /// sequence `r_lld_update_rxbuf` ends with. The core adopts the requested
    /// descriptor as its current one, and the ISR's clearing write must not
    /// raise anything.
    #[test]
    fn rx_buf_jump_raises_bit_18_on_its_go_edge() {
        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        assert_ne!(
            bt.read_u32(INTCNTL).unwrap() & INT_LLD_UPDATE_RXBUF,
            0,
            "the silicon enable word arms bit 18"
        );
        bt.write_u32(RX_DESC_PTR, 0x0000_1000).unwrap();

        // Store 1: the target descriptor, no go bit. Nothing happens.
        bt.write_u32(RX_BUF_JUMP, 0x0000_1064).unwrap();
        assert_eq!(bt.read_u32(INTRAWSTAT).unwrap() & INT_LLD_UPDATE_RXBUF, 0);
        assert_eq!(bt.read_u32(RX_DESC_PTR).unwrap(), 0x0000_1000);

        // Store 2: set bit15. Bit 18 latches, the FIFO carries it, and the core
        // adopts the descriptor.
        bt.write_u32(RX_BUF_JUMP, 0x0000_1064 | RX_BUF_JUMP_GO)
            .unwrap();
        assert_eq!(
            bt.read_u32(INTSTAT).unwrap() & INT_LLD_UPDATE_RXBUF,
            INT_LLD_UPDATE_RXBUF
        );
        assert_eq!(bt.read_u32(IRQ_FIFO).unwrap() >> 10, INT_LLD_UPDATE_RXBUF);
        assert_eq!(
            bt.read_u32(RX_DESC_PTR).unwrap() & RX_DESC_PTR_MASK,
            0x1064,
            "the core jumps to the descriptor software handed it"
        );
        // The register reads back what was written: the ISR read-modify-writes
        // it to drop bit15.
        assert_eq!(
            bt.read_u32(RX_BUF_JUMP).unwrap(),
            0x0000_1064 | RX_BUF_JUMP_GO
        );

        // The ISR's clear is not a new request.
        bt.write_u32(IRQ_FIFO, 1).unwrap();
        bt.write_u32(INTACK, INT_LLD_UPDATE_RXBUF).unwrap();
        bt.write_u32(RX_BUF_JUMP, 0x0000_1064).unwrap();
        assert_eq!(
            bt.read_u32(INTRAWSTAT).unwrap() & INT_LLD_UPDATE_RXBUF,
            0,
            "clearing the go bit must not re-raise the interrupt"
        );
        // A fresh request does raise again.
        bt.write_u32(RX_BUF_JUMP, 0x0000_1078 | RX_BUF_JUMP_GO)
            .unwrap();
        assert_eq!(
            bt.read_u32(INTRAWSTAT).unwrap() & INT_LLD_UPDATE_RXBUF,
            INT_LLD_UPDATE_RXBUF
        );
        assert_eq!(bt.read_u32(RX_DESC_PTR).unwrap() & RX_DESC_PTR_MASK, 0x1078);
    }

    /// No frame on the air means no interrupt and no exchange-memory write —
    /// a reception is never invented to keep a scanner busy.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn a_silent_air_delivers_nothing() {
        use crate::sched::EventScheduler;

        let clock = CycleClock::default();
        let mut bt = advertising_part(&clock);
        bt.air = crate::peripherals::ble_air::BleAirBus::new();
        let mut bus = RamBus::default();
        let mut sched = EventScheduler::new();
        stage_advertising_event(&mut bt, &mut bus, 0);
        stage_scan_event(&mut bt, &mut bus, 1, 1, 0);
        bt.write_u32(PROG_PUSH, 0x8000_0001).unwrap();
        let token = bt.take_scheduled_events()[0].1;
        sched.advance_to(0);
        clock.publish(0);
        bt.on_event(token, &mut sched, &mut bus);
        assert_eq!(bt.read_u32(INTRAWSTAT).unwrap() & INT_SCH_PROG_RX, 0);
        assert_eq!(
            bt.read_u32(RX_DESC_PTR).unwrap(),
            0x0000_1000,
            "the descriptor pointer does not move without a reception"
        );
    }

    /// CLKN must be monotonic — the event scheduler re-reads it to decide
    /// whether its deadline already slipped.
    #[test]
    fn clkn_is_monotonic() {
        let mut bt = Esp32c3Bt::new();
        let clock = CycleClock::default();
        bt.attach_cycle_clock(clock.clone());
        clock.publish(0);
        bt.write_u32(0x000, 1).unwrap();
        let mut last = 0;
        for n in 0..5_000u64 {
            clock.publish(n * 9_973);
            let now = bt.read_u32(CLKN).unwrap();
            assert!(now >= last, "CLKN went backwards: {last} -> {now}");
            last = now;
        }
        assert!(last > 0, "CLKN never advanced");
    }
    /// A simulation RESTART reuses the lab's `AirBus` and builds fresh
    /// controllers. The previous run's frames are still on that air, and
    /// `next_node_id()` gives the restarted controller a new identity — so the
    /// old frames are not filtered out as its own. It must still not see them:
    /// a radio hears what is transmitted while it is listening, not a backlog.
    #[test]
    fn a_controller_built_after_a_restart_skips_the_previous_runs_backlog() {
        use crate::peripherals::ble_air::{BleAirBus, BleAirFrame};
        let air = BleAirBus::new();

        // Run 1: two nodes trade advertising frames on the primary channels.
        for _ in 0..10 {
            for ch in [37u8, 38, 39] {
                air.transmit(BleAirFrame {
                    seq: 0,
                    source: 1,
                    channel: ch,
                    access_address: 0x8E89_BED6,
                    crc_init: 0x0055_5555,
                    pdu: vec![0x20, 0x02, 0xE5, 0x02],
                });
            }
        }
        let backlog = air.current_seq();
        assert_eq!(backlog, 30, "the previous run left frames on the air");

        // Restart: same air, brand-new controller.
        let restarted = Esp32c3Bt::with_air(air.clone());
        assert_eq!(
            restarted.rx_cursor, backlog,
            "a restarted controller joins the air where it is now, not at 0",
        );
        assert!(
            air.receive_from(37, 0x8E89_BED6, restarted.rx_cursor, restarted.node_id)
                .is_none(),
            "and therefore sees none of the previous run's traffic",
        );
    }
}
