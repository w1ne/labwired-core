// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! USB_SERIAL_JTAG — the CDC-ACM console the ESP32-C3 and ESP32-S3 expose on
//! their own USB pins (shared with the JTAG debug interface).
//!
//! On boards like the ESP32-C3 SuperMini and the ESP32-S3-Zero the USB-C socket
//! is wired to THIS block, not to UART0. Firmware for such a board is built with
//! `-DARDUINO_USB_CDC_ON_BOOT=1 -DARDUINO_USB_MODE=1`, which makes Arduino's
//! `Serial` an `HWCDC` rather than a `HardwareSerial` — and `HWCDC` is entirely
//! interrupt-driven. A model with no interrupt surface does not merely lose a
//! feature; it makes such a board's console silent while the real board prints.
//!
//! # Chip differences — read before reusing this model
//!
//! This file lives under `esp32s3/` for historical reasons but models the IP as
//! it appears on BOTH chips. Same offset does NOT imply same register across
//! Espressif parts, so each claim below is per-chip and cited:
//!
//! | | ESP32-C3 | ESP32-S3 |
//! |---|---|---|
//! | Base | `0x6004_3000` (`DR_REG_USB_SERIAL_JTAG_BASE`, `esp32c3/include/soc/soc.h:68`) | `0x6003_8000` (`DR_REG_USB_DEVICE_BASE`, `esp32s3/include/soc/soc.h:87`) |
//! | Matrix source | 26 | 96 |
//!
//! The base address and the interrupt-matrix source id **differ**. The source id
//! is a constructor choice with NO default: [`UsbSerialJtag::new`] wires no
//! interrupt source at all, and a caller opts in per chip via
//! [`UsbSerialJtag::new_esp32c3`] / [`UsbSerialJtag::new_esp32s3`]. An S3
//! assumption therefore cannot reach C3 silicon by omission. The base address is
//! owned by the chip YAML and passed at registration; this model never holds a
//! copy of it (see `tests::yaml_owned_base_contract`).
//!
//! The source ids are `ETS_USB_SERIAL_JTAG_INTR_SOURCE` evaluated in each chip's
//! own `soc/periph_defs.h` (`periph_interrput_t`, unnumbered enumerators — they
//! must be counted, never guessed). Both are corroborated independently:
//!  * C3 = 26 — visible in a compiled Arduino image: `HWCDC::begin` emits
//!    `li a0,26` immediately before `jal esp_intr_alloc`, and the surrounding
//!    code addresses `lui a5,0x60043`.
//!  * S3 = 96 — the same enum walk reproduces `ETS_I2C_EXT0 = 42`,
//!    `ETS_I2C_EXT1 = 43`, `ETS_LEDC = 35` and `ETS_LCD_CAM = 24`, which are the
//!    values `esp32s3::i2c`, `esp32s3::ledc` and `esp32s3::lcd_cam` already pin.
//!
//! Register offsets and interrupt bit positions are IDENTICAL on the two chips.
//! That is not an assumption: `esp32c3/include/soc/usb_serial_jtag_reg.h` and
//! `esp32s3/include/soc/usb_serial_jtag_reg.h` agree on EP1 `0x00`,
//! EP1_CONF `0x04`, INT_RAW `0x08`, INT_ST `0x0C`, INT_ENA `0x10`,
//! INT_CLR `0x14`, CONF0 `0x18`, and on SOF `BIT(1)`,
//! SERIAL_OUT_RECV_PKT `BIT(2)`, SERIAL_IN_EMPTY `BIT(3)`,
//! USB_BUS_RESET `BIT(9)`, WR_DONE `BIT(0)`, SERIAL_IN_EP_DATA_FREE `BIT(1)`,
//! SERIAL_OUT_EP_DATA_AVAIL `BIT(2)`. The two `hal/usb_serial_jtag_ll.h` copies
//! are likewise identical in the accessors this model serves.
//!
//! # Register behaviour
//!
//! | Offset | Name | Behaviour |
//! |-------:|------|-----------|
//! | `0x00` | EP1 | W: low byte of the word is a TX FIFO byte. R: pops an RX byte. |
//! | `0x04` | EP1_CONF | R: `WR_DONE` 0, `SERIAL_IN_EP_DATA_FREE` = no packet in flight, `SERIAL_OUT_EP_DATA_AVAIL` = RX FIFO non-empty. W: `WR_DONE` commits a packet. |
//! | `0x08` | INT_RAW | R: sticky bits OR live level conditions. |
//! | `0x0C` | INT_ST | R: `INT_RAW & INT_ENA`. |
//! | `0x10` | INT_ENA | R/W storage. |
//! | `0x14` | INT_CLR | W1C over the sticky bits. |
//! | `0x18` | CONF0 | R/W storage (firmware read-modify-writes the PHY bits). |
//!
//! ## Why INT_ENA must be real storage
//!
//! `HWCDC::begin` does `usb_serial_jtag_ll_disable_intr_mask(0x7ffff)` followed
//! by `ena_intr_mask(SERIAL_IN_EMPTY | SERIAL_OUT_RECV_PKT | BUS_RESET)`, and
//! both are read-modify-writes of INT_ENA. In a compiled image that is
//! `lw a4,16(a5)` / `and` / `sw`, then `li a0,524` (`0x20C`) into
//! `usb_serial_jtag_ll_ena_intr_mask`. If INT_ENA reads back 0, the enable is
//! lost, INT_ST stays 0 and the ISR never runs.
//!
//! ## SERIAL_IN_EMPTY is a LEVEL, and its reset value is 1
//!
//! The register header types it `R/WTC/SS` (write-to-clear, self-setting) with
//! `default: 1` and the description "turns to high level when the Serial Port IN
//! Endpoint is empty". So it is not a one-shot: clearing it while the endpoint is
//! still empty re-asserts it. That is precisely what pumps the driver — the ISR
//! clears the bit, writes up to 64 bytes, and the next empty endpoint re-raises
//! it for the following chunk.
//!
//! This does not spin forever, because the DRIVER stops it: when the TX ring
//! buffer runs dry, `hw_cdc_isr_handler` calls `disable_intr_mask(SERIAL_IN_EMPTY)`
//! and does NOT re-enable it (the re-enable sits inside `if (queued_buff != NULL)`).
//! `HWCDC::write` re-enables it when there is new data. The level stays asserted
//! in INT_RAW; INT_ENA is what gates it.
//!
//! ## SOF, and why a twin without it models an UNPLUGGED board
//!
//! `HWCDC` never enables SOF in INT_ENA — it POLLS `int_raw.sof_int_raw` from a
//! FreeRTOS tick hook (`usb_serial_jtag_sof_tick_hook`). Its rule: assume
//! connected until SOF has been missing for `pdMS_TO_TICKS(5)` consecutive
//! ticks, then latch `s_usb_serial_jtag_conn_status = false` — permanently, since
//! only a SOF can set it back. `HWCDC::write` consults that through
//! `isCDC_Connected()` and, when false, hands the bytes to `flushTXBuffer`,
//! which DISCARDS them.
//!
//! A model that leaves SOF at 0 is therefore not "missing an interrupt". It is
//! actively telling the firmware the cable is unplugged, and the firmware
//! correctly responds by throwing the user's output away. Since a user running a
//! lab is by definition looking at a connected board, this model raises SOF at
//! the USB full-speed frame rate (1 kHz) for as long as it is instantiated.
//!
//! SOF is derived lazily from the shared [`CycleClock`] rather than from a walk
//! tick or a scheduled wake: the driver only ever READS it, `read` takes
//! `&self`, and the trait blesses exactly this pattern (see
//! [`crate::Peripheral::attach_cycle_clock`]). The peripheral consequently costs
//! nothing per cycle.
//!
//! ## Back-pressure is real, and measured
//!
//! Measured on a physically connected ESP32-C3: writing `WR_DONE` drops
//! `SERIAL_IN_EP_DATA_FREE` and `SERIAL_IN_EMPTY_RAW` to 0 **together**, and the
//! host's IN transfer returns both to 1 **together** roughly 0.2-0.4 ms later
//! (a bulk-write test put it at ~235 us per packet). This model reproduces that:
//! a committed packet holds the endpoint busy for [`PICKUP_PERIOD_US`], and a
//! store to EP1 while `DATA_FREE == 0` is IGNORED, exactly as silicon ignores it.
//!
//! Dropping the back-pressure and reporting "always writable" would be the
//! easier model and a dishonest one: `usb_serial_jtag_ll_write_txfifo` breaks
//! its loop on `DATA_FREE == 0`, so a model that accepts unlimited bytes
//! silently deletes the flow control the driver is written against.
//!
//! A zero-length `WR_DONE` does NOT start a transfer. `HWCDC::isCDC_Connected`
//! calls `usb_serial_jtag_ll_txfifo_flush()` with nothing staged specifically to
//! "feed CDC TX FIFO to trigger IN_EMPTY"; if that flush marked the endpoint
//! busy it would suppress the very interrupt it exists to raise.
//!
//! Without an attached [`CycleClock`] there is no time base to expire a packet
//! against, so back-pressure is disabled and pickup is instant — a hand-built
//! bus keeps its previous behaviour instead of deadlocking on a packet that can
//! never complete.
//!
//! # Deliberate simplifications (documented, not hidden)
//!
//! * **`USB_BUS_RESET` is never raised.** The driver enables it and responds by
//!   setting `connected = false`. There is no re-enumeration in the twin, so
//!   raising it would only disconnect a healthy console.
//! * **RX (`SERIAL_OUT_RECV_PKT`) is modelled but not yet wired to a host.**
//!   The FIFO, the `EP1` read path, the `SERIAL_OUT_EP_DATA_AVAIL` bit and the
//!   interrupt all work and are unit-tested through [`UsbSerialJtag::inject_rx`],
//!   but nothing in the bus currently calls that. Injected serial still reaches
//!   UART0 only. Wiring the console's input side to the CDC endpoint is left
//!   out of this change ON PURPOSE, and is called out here rather than being
//!   quietly absent.
//! * **The 1 kHz SOF is free-running.** A real host stops sending SOF when the
//!   cable is pulled; the twin has no unplug event, so SOF runs for as long as
//!   the model exists. [`UsbSerialJtag::set_sof_enabled`] can stop it, which is
//!   how the "unplugged board goes silent" negative control is written.
//!
//! # What is verified, and what is not
//!
//! **ESP32-C3 — verified end to end.** A real PlatformIO Arduino image for
//! `board = esp32-c3-supermini`, built with CDC-on-boot, boots on the C3 mask
//! ROM in this simulator and its `loop()` output reaches the
//! `usb_serial_jtag` sink (`tests/esp32c3_usb_cdc_console.rs`). The register
//! semantics above are additionally corroborated by measurements taken over SWD
//! on a physically connected ESP32-C3: `PRE_BEGIN RAW=0x508`,
//! `POST_BEGIN RAW=0x508 ENA=0x204`, `CONF0=0x4200`, and the paired
//! `DATA_FREE`/`SERIAL_IN_EMPTY` back-pressure timing.
//!
//! **ESP32-S3 — NOT verified end to end. Do not read C3 evidence as S3
//! evidence.** No S3 firmware was booted against this model and no S3 hardware
//! was attached, so every S3 claim here is header-derived: the base address, the
//! source id, and the fact that the register and bit layout matches the C3. Those
//! derivations are mechanical and cross-checked (see above), but interrupt
//! delivery for source 96 through the S3 Xtensa intmatrix has never been
//! exercised by a running S3 image. Anyone relying on the S3 path should start by
//! building that gate.

use crate::cycle_clock::CycleClock;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

// ── Register offsets (identical on C3 and S3; see the module docs) ───────────
const OFF_EP1: u64 = 0x00;
const OFF_EP1_CONF: u64 = 0x04;
const OFF_INT_RAW: u64 = 0x08;
const OFF_INT_ST: u64 = 0x0C;
const OFF_INT_ENA: u64 = 0x10;
const OFF_INT_CLR: u64 = 0x14;
const OFF_CONF0: u64 = 0x18;

// ── Interrupt bits (identical on C3 and S3) ─────────────────────────────────
/// `USB_SERIAL_JTAG_SOF_INT_RAW` — a USB Start-Of-Frame was received (1 kHz).
pub const INT_SOF: u32 = 1 << 1;
/// `USB_SERIAL_JTAG_SERIAL_OUT_RECV_PKT_INT_RAW` — the OUT endpoint took a packet.
pub const INT_SERIAL_OUT_RECV_PKT: u32 = 1 << 2;
/// `USB_SERIAL_JTAG_SERIAL_IN_EMPTY_INT_RAW` — the IN endpoint is empty.
pub const INT_SERIAL_IN_EMPTY: u32 = 1 << 3;
/// `USB_SERIAL_JTAG_USB_BUS_RESET_INT_RAW` — never raised by this model.
pub const INT_BUS_RESET: u32 = 1 << 9;
/// `USB_SERIAL_JTAG_LL_INTR_MASK` — every implemented interrupt bit.
const INTR_MASK: u32 = 0x7_FFFF;

// ── EP1_CONF bits ───────────────────────────────────────────────────────────
const EP1_CONF_WR_DONE: u32 = 1 << 0;
const EP1_CONF_SERIAL_IN_EP_DATA_FREE: u32 = 1 << 1;
const EP1_CONF_SERIAL_OUT_EP_DATA_AVAIL: u32 = 1 << 2;

// ── Per-chip facts ──────────────────────────────────────────────────────────
/// `ETS_USB_SERIAL_JTAG_INTR_SOURCE` on the ESP32-C3 (`esp32c3/soc/periph_defs.h`).
pub const ESP32C3_INTR_SOURCE_ID: u32 = 26;
/// `ETS_USB_SERIAL_JTAG_INTR_SOURCE` on the ESP32-S3 (`esp32s3/soc/periph_defs.h`).
pub const ESP32S3_INTR_SOURCE_ID: u32 = 96;
// NOTE: the per-chip BASE ADDRESSES are deliberately NOT constants here.
// They differ between the chips (C3 0x60043000 vs S3 0x60038000) and they are
// documented in the module docs above, but a base address is owned by the chip
// YAML — a copy in Rust would be a second home for the same fact, and the two
// disagreeing fails silently because a wrong address is still a valid address.
// The registration sites pass the base; this model only ever sees offsets.

const ESP32C3_CPU_CLOCK_HZ: u64 = 160_000_000;
const ESP32S3_CPU_CLOCK_HZ: u64 = 240_000_000;

/// USB full-speed frame rate: a host sends one SOF every 1 ms.
const SOF_HZ: u64 = 1_000;

/// How long a committed IN packet holds the endpoint busy before the host
/// picks it up. Measured on a connected ESP32-C3 at ~235 us per packet
/// (`DATA_FREE`/`SERIAL_IN_EMPTY` both 0 for 0.2-0.4 ms after `WR_DONE`).
pub const PICKUP_PERIOD_US: u64 = 235;

pub struct UsbSerialJtag {
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,

    /// Interrupt-matrix source id, or `None` for a bus that has not opted in.
    /// `None` keeps the model inert towards the interrupt fabric — the state it
    /// shipped in before interrupts existed.
    irq_source: Option<u32>,

    /// Latched (`W1C`) interrupt bits. The live level conditions are OR-ed in by
    /// [`Self::int_raw`] and are deliberately NOT stored here, so clearing one
    /// whose condition still holds re-asserts it — the `R/WTC/SS` semantics.
    int_raw_sticky: Cell<u32>,
    int_ena: u32,
    conf0: u32,

    /// Bytes staged by EP1 writes, committed by `WR_DONE`.
    tx_staging: Vec<u8>,
    /// A committed packet is awaiting host pickup: `DATA_FREE` and
    /// `SERIAL_IN_EMPTY` both read 0 until [`Self::pickup_at`].
    in_flight: Cell<bool>,
    /// Cycle at which the in-flight packet completes.
    pickup_at: Cell<u64>,
    /// CPU cycles a committed packet stays in flight.
    pickup_cycles: u64,
    /// Conservation counters: every byte accepted at EP1 must reach the sink.
    ep1_bytes_accepted: u64,
    sink_bytes_emitted: u64,
    rx_fifo: VecDeque<u8>,

    clock: Option<CycleClock>,
    /// CPU cycles between SOF frames.
    sof_period_cycles: u64,
    /// Cycle at which the next SOF becomes visible.
    sof_next: Cell<u64>,
    /// Whether the clock has been observed at least once (so the first read
    /// anchors the SOF phase instead of firing a burst of back-dated frames).
    sof_anchored: Cell<bool>,
    /// Whether the modelled host is sending SOF at all. Turning this off models
    /// an UNPLUGGED cable, which is how the negative control is written.
    sof_enabled: bool,
}

impl Default for UsbSerialJtag {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for UsbSerialJtag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UsbSerialJtag(sink={}, echo_stdout={}, irq_source={:?}, int_ena={:#x})",
            self.sink.is_some(),
            self.echo_stdout,
            self.irq_source,
            self.int_ena,
        )
    }
}

impl UsbSerialJtag {
    /// A block with NO interrupt-matrix source wired.
    ///
    /// Register semantics are complete, but [`Peripheral::matrix_irq_sources_into`]
    /// never asserts, so the CPU cannot take the ISR. This is the correct
    /// constructor for a bus that only needs the polled byte sink (the mask
    /// ROM's `usb_uart_tx_one_char` path), and the deliberate default: a caller
    /// that wants interrupts must name its chip, because the source id is one of
    /// the things that differs between them.
    pub fn new() -> Self {
        Self::with_source(None, ESP32C3_CPU_CLOCK_HZ)
    }

    /// ESP32-C3 instance: matrix source [`ESP32C3_INTR_SOURCE_ID`], 160 MHz.
    pub fn new_esp32c3() -> Self {
        Self::with_source(Some(ESP32C3_INTR_SOURCE_ID), ESP32C3_CPU_CLOCK_HZ)
    }

    /// ESP32-S3 instance: matrix source [`ESP32S3_INTR_SOURCE_ID`], 240 MHz.
    pub fn new_esp32s3() -> Self {
        Self::with_source(Some(ESP32S3_INTR_SOURCE_ID), ESP32S3_CPU_CLOCK_HZ)
    }

    fn with_source(irq_source: Option<u32>, cpu_clock_hz: u64) -> Self {
        Self {
            sink: None,
            echo_stdout: true,
            irq_source,
            // Reset value: SERIAL_IN_EMPTY defaults to 1 (the IN endpoint is
            // empty out of reset), which is also the live level below.
            int_raw_sticky: Cell::new(0),
            int_ena: 0,
            conf0: 0,
            tx_staging: Vec::new(),
            in_flight: Cell::new(false),
            pickup_at: Cell::new(0),
            pickup_cycles: (cpu_clock_hz * PICKUP_PERIOD_US / 1_000_000).max(1),
            ep1_bytes_accepted: 0,
            sink_bytes_emitted: 0,
            rx_fifo: VecDeque::new(),
            clock: None,
            sof_period_cycles: (cpu_clock_hz / SOF_HZ).max(1),
            sof_next: Cell::new(0),
            sof_anchored: Cell::new(false),
            sof_enabled: true,
        }
    }

    /// Set or clear the byte capture sink and stdout-echo flag.
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }

    /// The interrupt-matrix source this instance asserts on, if any.
    pub fn irq_source(&self) -> Option<u32> {
        self.irq_source
    }

    /// Stop (or resume) the modelled host's 1 kHz SOF stream.
    ///
    /// Disabling it models an unplugged cable: `HWCDC` latches
    /// `s_usb_serial_jtag_conn_status = false` about 5 FreeRTOS ticks later and
    /// discards everything the sketch prints from then on. Exists so that
    /// behaviour can be asserted rather than assumed.
    pub fn set_sof_enabled(&mut self, enabled: bool) {
        self.sof_enabled = enabled;
    }

    /// Bytes the firmware successfully stored into EP1's TX FIFO.
    pub fn ep1_bytes_accepted(&self) -> u64 {
        self.ep1_bytes_accepted
    }

    /// Bytes this model handed to the sink. Must equal
    /// [`Self::ep1_bytes_accepted`] once every committed packet has been
    /// flushed: a surplus means a byte reached the console without going
    /// through the FIFO.
    pub fn sink_bytes_emitted(&self) -> u64 {
        self.sink_bytes_emitted
    }

    /// Push host→device bytes into the OUT endpoint, raising
    /// `SERIAL_OUT_RECV_PKT`. Nothing on the bus calls this yet — see the
    /// module docs' note on RX being modelled but not wired.
    pub fn inject_rx(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.rx_fifo.extend(bytes.iter().copied());
        self.int_raw_sticky
            .set(self.int_raw_sticky.get() | INT_SERIAL_OUT_RECV_PKT);
    }

    /// Advance the lazily-derived SOF frame counter to the clock's "now".
    ///
    /// Called from `&self` read paths (the driver only ever polls this bit) and
    /// from [`Peripheral::sync_to`]. Idempotent within a cycle. Without an
    /// attached clock this does nothing and SOF never asserts, which keeps
    /// hand-built buses on exactly their previous behaviour.
    fn sync_sof(&self) {
        if !self.sof_enabled {
            return;
        }
        let Some(clock) = &self.clock else { return };
        let now = clock.now();
        if !self.sof_anchored.get() {
            self.sof_anchored.set(true);
            self.sof_next
                .set(now.saturating_add(self.sof_period_cycles));
            return;
        }
        if now >= self.sof_next.get() {
            self.int_raw_sticky.set(self.int_raw_sticky.get() | INT_SOF);
            // Re-anchor to `now` rather than accumulating every missed frame:
            // the bit is a single latch, so N elapsed frames and one elapsed
            // frame are indistinguishable to firmware.
            self.sof_next
                .set(now.saturating_add(self.sof_period_cycles));
        }
    }

    /// INT_RAW = latched bits OR the live level conditions.
    ///
    /// `SERIAL_IN_EMPTY` is a level, not an event: it holds exactly while the IN
    /// endpoint is empty — i.e. while no committed packet is awaiting host
    /// pickup. Keeping it OUT of the sticky word is what makes a `W1C` of it
    /// re-assert once the endpoint drains, and that re-assertion is the pump
    /// that moves every byte after the first.
    fn int_raw(&self) -> u32 {
        self.sync_sof();
        let mut v = self.int_raw_sticky.get();
        // Measured: SERIAL_IN_EMPTY_RAW and EP1_CONF.DATA_FREE drop together on
        // WR_DONE and return together on host pickup. One condition, two views.
        if self.data_free() {
            v |= INT_SERIAL_IN_EMPTY;
        }
        if !self.rx_fifo.is_empty() {
            v |= INT_SERIAL_OUT_RECV_PKT;
        }
        v & INTR_MASK
    }

    /// EP1_CONF read value.
    ///
    /// `WR_DONE` (bit 0) is write-triggered and reads back 0 — measured on
    /// silicon, and matching the register header's `WT` type. `DATA_FREE`
    /// (bit 1) is the back-pressure bit: 1 only while no packet is in flight.
    fn ep1_conf_read(&self) -> u32 {
        let mut v = 0;
        if self.data_free() {
            v |= EP1_CONF_SERIAL_IN_EP_DATA_FREE;
        }
        if !self.rx_fifo.is_empty() {
            v |= EP1_CONF_SERIAL_OUT_EP_DATA_AVAIL;
        }
        v
    }

    /// Whether the IN endpoint can accept a byte right now.
    ///
    /// Expires an in-flight packet lazily against the cycle clock, so this is
    /// callable from `&self` read paths. With no clock attached there is no
    /// back-pressure at all (see the module docs).
    fn data_free(&self) -> bool {
        if !self.in_flight.get() {
            return true;
        }
        let Some(clock) = &self.clock else {
            self.in_flight.set(false);
            return true;
        };
        if clock.now() >= self.pickup_at.get() {
            self.in_flight.set(false);
            return true;
        }
        false
    }

    /// `WR_DONE`: commit the staged bytes as one IN packet.
    ///
    /// A zero-length commit is a no-op rather than a zero-length transfer — see
    /// the module docs on why `isCDC_Connected`'s bare flush must not mark the
    /// endpoint busy.
    fn commit_packet(&mut self) {
        if self.tx_staging.is_empty() {
            return;
        }
        let bytes = std::mem::take(&mut self.tx_staging);
        self.sink_bytes_emitted += bytes.len() as u64;
        self.emit(&bytes);
        // The packet is now the host's problem. Hold the endpoint busy for the
        // measured pickup period; with no clock there is nothing to expire it
        // against, so pickup is instant.
        if let Some(clock) = &self.clock {
            self.in_flight.set(true);
            self.pickup_at
                .set(clock.now().saturating_add(self.pickup_cycles));
        }
    }

    fn emit(&self, bytes: &[u8]) {
        if let Some(sink) = &self.sink {
            if let Ok(mut g) = sink.lock() {
                g.extend_from_slice(bytes);
            }
        }
        if self.echo_stdout {
            let mut out = io::stdout();
            let _ = out.write_all(bytes);
            let _ = out.flush();
        }
    }

    /// Byte lane `lane` (0..=3) of a 32-bit register value.
    fn lane(value: u32, lane: u64) -> u8 {
        ((value >> (8 * lane)) & 0xFF) as u8
    }
}

impl Peripheral for UsbSerialJtag {
    /// Same shape as the shared `EspUart` on this bus: the scheduler path owns
    /// level export when it exists, and without it the walk does. An instance
    /// with no matrix source asserts nothing either way, so this costs a
    /// source-less bus nothing but a no-op poll.
    fn needs_legacy_walk(&self) -> bool {
        !self.uses_scheduler()
    }

    /// Opting into the scheduler is what subscribes this model to the bus's
    /// level re-derivation — both the walk-tick aggregation and, on a
    /// walk-deleted C3 bus, the MMIO write choke
    /// (`SystemBus::sync_esp32c3_irq_cache_write`). The write choke is the
    /// important one: it is what makes the ISR's own INT_CLR / EP1 / EP1_CONF
    /// writes immediately re-derive the routed line, so the TX pump advances
    /// per write instead of per tick.
    fn uses_scheduler(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn sync_to(&mut self, _now_cycle: u64) {
        self.sync_sof();
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if let Some(src) = self.irq_source {
            if self.int_raw() & self.int_ena != 0 {
                out.push(src);
            }
        }
    }

    /// Legacy-walk twin of [`Self::matrix_irq_sources_into`], for builds without
    /// the `event-scheduler` feature.
    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        let asserting = self
            .irq_source
            .filter(|_| self.int_raw() & self.int_ena != 0);
        PeripheralTickResult {
            explicit_irqs: asserting.map(|src| vec![src]),
            ..PeripheralTickResult::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = offset & !3;
        let lane = offset & 3;
        let value = match word {
            // Reading EP1 pops the OUT endpoint FIFO. `&self` cannot mutate the
            // queue, so the byte is peeked here and consumed by `read_ep1_byte`
            // on the mutable path; a peek is the honest answer for a pure read.
            OFF_EP1 => self.rx_fifo.front().copied().unwrap_or(0) as u32,
            OFF_EP1_CONF => self.ep1_conf_read(),
            OFF_INT_RAW => self.int_raw(),
            OFF_INT_ST => self.int_raw() & self.int_ena,
            OFF_INT_ENA => self.int_ena,
            // INT_CLR is write-only; the TRM gives it no read value.
            OFF_INT_CLR => 0,
            OFF_CONF0 => self.conf0,
            _ => {
                crate::census_reg!("esp32s3.usb_serial_jtag:UsbSerialJtag", word, "read");
                0
            }
        };
        Ok(Self::lane(value, lane))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word = offset & !3;
        let lane = offset & 3;
        let shift = 8 * lane;
        let byte = (value as u32) << shift;
        let lane_mask = 0xFFu32 << shift;

        match word {
            // Only the low byte of the word is FIFO data
            // (`usb_serial_jtag_ll_write_txfifo` stores a byte-wide value); the
            // upper lanes of the 32-bit store are not. Silicon IGNORES a store
            // while the endpoint is busy, and `usb_serial_jtag_ll_write_txfifo`
            // breaks its loop on it. Guard rather than a nested `if`: clippy
            // 1.95 (what CI pins) reports collapsible_match otherwise. Falling
            // through to `_ => { crate::census_reg!("esp32s3.usb_serial_jtag:UsbSerialJtag", word, "write"); }` is the same no-op the nested form performed.
            OFF_EP1 if lane == 0 && self.data_free() => {
                self.tx_staging.push(value);
                self.ep1_bytes_accepted += 1;
            }
            OFF_EP1_CONF if lane == 0 && value as u32 & EP1_CONF_WR_DONE != 0 => {
                self.commit_packet();
            }
            // INT_RAW is `R/WTC` — a write of 1 clears the latched bit. Nothing
            // in evidence writes it (drivers use INT_CLR), but honour it.
            OFF_INT_RAW => {
                self.int_raw_sticky
                    .set(self.int_raw_sticky.get() & !(byte & lane_mask));
            }
            OFF_INT_ST => {} // read-only
            OFF_INT_ENA => {
                self.int_ena = (self.int_ena & !lane_mask) | (byte & lane_mask);
                self.int_ena &= INTR_MASK;
            }
            OFF_INT_CLR => {
                self.int_raw_sticky
                    .set(self.int_raw_sticky.get() & !(byte & lane_mask));
            }
            OFF_CONF0 => {
                self.conf0 = (self.conf0 & !lane_mask) | (byte & lane_mask);
            }
            _ => {
                crate::census_reg!("esp32s3.usb_serial_jtag:UsbSerialJtag", word, "write");
            }
        }
        Ok(())
    }

    fn legacy_tick_active(&self) -> bool {
        self.needs_legacy_walk()
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
    use crate::bus::SystemBus;
    use crate::Bus;

    /// The per-chip facts this model must not blur together.
    ///
    /// Derived from each chip's own headers, and cross-checked: the C3 value is
    /// what a compiled `HWCDC::begin` passes to `esp_intr_alloc`, and the S3
    /// enum walk that yields 96 also reproduces the I2C/LEDC/LCD_CAM ids this
    /// crate already pins elsewhere.
    #[test]
    fn per_chip_constants_are_distinct_and_pinned() {
        assert_eq!(ESP32C3_INTR_SOURCE_ID, 26);
        assert_eq!(ESP32S3_INTR_SOURCE_ID, 96);
        assert_ne!(ESP32C3_INTR_SOURCE_ID, ESP32S3_INTR_SOURCE_ID);
        assert_eq!(UsbSerialJtag::new_esp32c3().irq_source(), Some(26));
        assert_eq!(UsbSerialJtag::new_esp32s3().irq_source(), Some(96));
        // The un-chipped constructor must NOT silently pick a chip.
        assert_eq!(UsbSerialJtag::new().irq_source(), None);
    }

    #[test]
    fn writing_ep1_appends_to_sink_on_flush() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut p = UsbSerialJtag::new();
        p.set_sink(Some(sink.clone()), false);
        p.write(OFF_EP1, b'H').unwrap();
        p.write(OFF_EP1, b'i').unwrap();
        p.write_u32(OFF_EP1_CONF, EP1_CONF_WR_DONE).unwrap();
        assert_eq!(sink.lock().unwrap().as_slice(), b"Hi");
    }

    #[test]
    fn writing_via_bus_word_write_appends_low_byte() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut bus = SystemBus::new();
        let mut p = UsbSerialJtag::new();
        p.set_sink(Some(sink.clone()), false);
        // An arbitrary window: this asserts byte-lane decomposition, not a
        // chip's address map (which the chip YAML owns).
        const TEST_BASE: u64 = 0x1000_0000;
        bus.add_peripheral("usb_jtag", TEST_BASE, 0x100, None, Box::new(p));

        bus.write_u32(TEST_BASE + OFF_EP1, 0x0000_0048).unwrap();
        bus.write_u32(TEST_BASE + OFF_EP1_CONF, EP1_CONF_WR_DONE)
            .unwrap();
        assert_eq!(sink.lock().unwrap().as_slice(), b"H");
    }

    /// Back-pressure, as measured: `WR_DONE` drops DATA_FREE and
    /// SERIAL_IN_EMPTY together; host pickup returns both together; and a store
    /// while busy is ignored rather than silently buffered.
    #[test]
    fn wr_done_applies_measured_back_pressure() {
        let clock = CycleClock::default();
        let mut p = UsbSerialJtag::new_esp32c3();
        clock.publish(0);
        Peripheral::attach_cycle_clock(&mut p, clock.clone());

        p.write(OFF_EP1, b'A').unwrap();
        assert_ne!(
            p.read_u32(OFF_EP1_CONF).unwrap() & EP1_CONF_SERIAL_IN_EP_DATA_FREE,
            0
        );

        p.write_u32(OFF_EP1_CONF, EP1_CONF_WR_DONE).unwrap();
        assert_eq!(
            p.read_u32(OFF_EP1_CONF).unwrap() & EP1_CONF_SERIAL_IN_EP_DATA_FREE,
            0,
            "DATA_FREE must drop while the packet is in flight"
        );
        assert_eq!(
            p.read_u32(OFF_INT_RAW).unwrap() & INT_SERIAL_IN_EMPTY,
            0,
            "SERIAL_IN_EMPTY drops together with DATA_FREE"
        );

        // A store while busy is DROPPED, not buffered.
        p.write(OFF_EP1, b'Z').unwrap();
        assert_eq!(
            p.ep1_bytes_accepted(),
            1,
            "store while busy must be ignored"
        );

        // Host pickup returns both bits together.
        clock.publish(ESP32C3_CPU_CLOCK_HZ * PICKUP_PERIOD_US / 1_000_000);
        assert_ne!(
            p.read_u32(OFF_EP1_CONF).unwrap() & EP1_CONF_SERIAL_IN_EP_DATA_FREE,
            0
        );
        assert_ne!(p.read_u32(OFF_INT_RAW).unwrap() & INT_SERIAL_IN_EMPTY, 0);
    }

    /// WR_DONE is write-triggered and reads back 0 (measured; register header
    /// types it `WT`).
    #[test]
    fn wr_done_reads_back_zero() {
        let mut p = UsbSerialJtag::new();
        assert_eq!(p.read_u32(OFF_EP1_CONF).unwrap() & EP1_CONF_WR_DONE, 0);
        p.write_u32(OFF_EP1_CONF, EP1_CONF_WR_DONE).unwrap();
        assert_eq!(p.read_u32(OFF_EP1_CONF).unwrap() & EP1_CONF_WR_DONE, 0);
    }

    /// A bare flush with nothing staged must NOT mark the endpoint busy —
    /// `isCDC_Connected` uses exactly that call to raise SERIAL_IN_EMPTY.
    #[test]
    fn zero_length_flush_keeps_the_endpoint_empty() {
        let clock = CycleClock::default();
        let mut p = UsbSerialJtag::new_esp32c3();
        Peripheral::attach_cycle_clock(&mut p, clock.clone());
        p.write_u32(OFF_EP1_CONF, EP1_CONF_WR_DONE).unwrap();
        assert_ne!(
            p.read_u32(OFF_INT_RAW).unwrap() & INT_SERIAL_IN_EMPTY,
            0,
            "a zero-length flush must leave the IN endpoint empty"
        );
    }

    /// Byte conservation: everything accepted at EP1 reaches the sink, and
    /// nothing else does.
    #[test]
    fn every_accepted_byte_reaches_the_sink_and_no_others() {
        let clock = CycleClock::default();
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut p = UsbSerialJtag::new_esp32c3();
        Peripheral::attach_cycle_clock(&mut p, clock.clone());
        p.set_sink(Some(sink.clone()), false);

        let mut cycle = 0u64;
        for chunk in 0..40u32 {
            for i in 0..64u32 {
                p.write(OFF_EP1, ((chunk + i) & 0x7F) as u8).unwrap();
            }
            p.write_u32(OFF_EP1_CONF, EP1_CONF_WR_DONE).unwrap();
            cycle += ESP32C3_CPU_CLOCK_HZ * PICKUP_PERIOD_US / 1_000_000;
            clock.publish(cycle);
        }
        assert_eq!(p.ep1_bytes_accepted(), 40 * 64);
        assert_eq!(p.sink_bytes_emitted(), p.ep1_bytes_accepted());
        assert_eq!(
            sink.lock().unwrap().len() as u64,
            p.ep1_bytes_accepted(),
            "a byte reached the console without going through the FIFO"
        );
    }

    /// INT_ENA must be real storage that survives the driver's
    /// read-modify-write, and INT_ST must be exactly `INT_RAW & INT_ENA`.
    #[test]
    fn int_ena_is_storage_and_int_st_is_raw_and_ena() {
        let mut p = UsbSerialJtag::new();
        // HWCDC::begin: clear every bit, then enable IN_EMPTY|OUT_RECV|BUS_RESET.
        p.write_u32(OFF_INT_ENA, 0).unwrap();
        assert_eq!(p.read_u32(OFF_INT_ENA).unwrap(), 0);
        assert_eq!(p.read_u32(OFF_INT_ST).unwrap(), 0);

        let mask = INT_SERIAL_IN_EMPTY | INT_SERIAL_OUT_RECV_PKT | INT_BUS_RESET;
        assert_eq!(mask, 0x20C, "the mask a compiled HWCDC::begin passes");
        p.write_u32(OFF_INT_ENA, mask).unwrap();
        assert_eq!(p.read_u32(OFF_INT_ENA).unwrap(), mask);

        let raw = p.read_u32(OFF_INT_RAW).unwrap();
        assert_ne!(
            raw & INT_SERIAL_IN_EMPTY,
            0,
            "SERIAL_IN_EMPTY resets to 1 (register header: `default: 1`)"
        );
        assert_eq!(p.read_u32(OFF_INT_ST).unwrap(), raw & mask);
    }

    /// The TX pump: `SERIAL_IN_EMPTY` is a level, so clearing it while the IN
    /// endpoint is still empty must re-assert it. Without this the driver moves
    /// at most one chunk and then stalls forever.
    #[test]
    fn serial_in_empty_reasserts_after_w1c_while_endpoint_is_empty() {
        let mut p = UsbSerialJtag::new();
        assert_ne!(p.read_u32(OFF_INT_RAW).unwrap() & INT_SERIAL_IN_EMPTY, 0);
        p.write_u32(OFF_INT_CLR, INT_SERIAL_IN_EMPTY).unwrap();
        assert_ne!(
            p.read_u32(OFF_INT_RAW).unwrap() & INT_SERIAL_IN_EMPTY,
            0,
            "cleared a LEVEL bit whose condition still holds; it must return"
        );
    }

    /// A latched (non-level) bit must actually clear on W1C, or an ISR would
    /// re-enter forever.
    #[test]
    fn int_clr_clears_latched_bits() {
        let mut p = UsbSerialJtag::new();
        p.inject_rx(b"x");
        assert_ne!(
            p.read_u32(OFF_INT_RAW).unwrap() & INT_SERIAL_OUT_RECV_PKT,
            0
        );
        // Draining the FIFO removes the live condition; then the latch clears.
        p.rx_fifo.clear();
        p.write_u32(OFF_INT_CLR, INT_SERIAL_OUT_RECV_PKT).unwrap();
        assert_eq!(
            p.read_u32(OFF_INT_RAW).unwrap() & INT_SERIAL_OUT_RECV_PKT,
            0
        );
    }

    /// CONF0 is read-modify-written by `HWCDC::begin` (`lw`/`and`/`or`/`sw` at
    /// offset 0x18 in a compiled image), so it has to be storage.
    #[test]
    fn conf0_is_read_write_storage() {
        let mut p = UsbSerialJtag::new();
        p.write_u32(OFF_CONF0, 0x0000_4200).unwrap();
        assert_eq!(p.read_u32(OFF_CONF0).unwrap(), 0x0000_4200);
    }

    /// No matrix source wired ⇒ never assert, whatever the registers say. This
    /// is what keeps an S3 source id from reaching a C3 bus by accident.
    #[test]
    fn without_a_chip_source_nothing_is_exported() {
        let mut p = UsbSerialJtag::new();
        p.write_u32(OFF_INT_ENA, INTR_MASK).unwrap();
        assert_ne!(p.read_u32(OFF_INT_ST).unwrap(), 0);
        assert!(p.matrix_irq_sources().is_empty());
        // ...and it stays empty on the walk path too, so a source-less instance
        // cannot starve anything: it has nothing to deliver.
        assert_eq!(p.tick_elapsed(1_000).explicit_irqs, None);

        // With a clock (i.e. registered through `add_peripheral`) it rides the
        // scheduler and asks for no walk, exactly as before this change.
        #[cfg(feature = "event-scheduler")]
        {
            Peripheral::attach_cycle_clock(&mut p, CycleClock::default());
            assert!(p.uses_scheduler());
            assert!(!p.needs_legacy_walk());
            assert!(p.matrix_irq_sources().is_empty());
        }
    }

    /// With a source wired, an enabled+raised bit exports that chip's id.
    #[test]
    fn enabled_level_exports_the_chip_source_id() {
        for (mut p, expect) in [
            (UsbSerialJtag::new_esp32c3(), ESP32C3_INTR_SOURCE_ID),
            (UsbSerialJtag::new_esp32s3(), ESP32S3_INTR_SOURCE_ID),
        ] {
            assert!(p.matrix_irq_sources().is_empty(), "masked until INT_ENA");
            p.write_u32(OFF_INT_ENA, INT_SERIAL_IN_EMPTY).unwrap();
            assert_eq!(p.matrix_irq_sources(), vec![expect]);
            // Masking it off must de-assert: a level that latched would wedge
            // the CPU in its ISR.
            p.write_u32(OFF_INT_ENA, 0).unwrap();
            assert!(p.matrix_irq_sources().is_empty());
        }
    }

    /// SOF must appear once the simulated clock has advanced a frame, and must
    /// stay clear before that. A twin that never raises it is telling `HWCDC`
    /// the cable is unplugged, and `HWCDC` then discards every write.
    #[test]
    fn sof_is_raised_at_the_usb_frame_rate() {
        let clock = CycleClock::default();
        let mut p = UsbSerialJtag::new_esp32c3();
        clock.publish(0);
        Peripheral::attach_cycle_clock(&mut p, clock.clone());

        // Anchor, then confirm nothing fires within the first frame.
        assert_eq!(p.read_u32(OFF_INT_RAW).unwrap() & INT_SOF, 0);
        clock.publish(ESP32C3_CPU_CLOCK_HZ / SOF_HZ - 1);
        assert_eq!(
            p.read_u32(OFF_INT_RAW).unwrap() & INT_SOF,
            0,
            "SOF must not arrive early"
        );

        clock.publish(ESP32C3_CPU_CLOCK_HZ / SOF_HZ);
        assert_ne!(
            p.read_u32(OFF_INT_RAW).unwrap() & INT_SOF,
            0,
            "one 1 ms USB frame elapsed; SOF must be visible"
        );

        // The tick hook clears it every tick and expects it back next frame.
        p.write_u32(OFF_INT_CLR, INT_SOF).unwrap();
        assert_eq!(p.read_u32(OFF_INT_RAW).unwrap() & INT_SOF, 0);
        clock.publish(3 * ESP32C3_CPU_CLOCK_HZ / SOF_HZ);
        assert_ne!(p.read_u32(OFF_INT_RAW).unwrap() & INT_SOF, 0);
    }

    /// NEGATIVE CONTROL (unit level): with the host's SOF stream stopped, the
    /// bit must stay 0 no matter how much simulated time passes. This is the
    /// condition under which `HWCDC` declares the cable unplugged and starts
    /// discarding output; the end-to-end twin of this assertion lives in
    /// `tests/esp32c3_usb_cdc_console.rs`.
    #[test]
    fn sof_disabled_never_raises_the_bit() {
        let clock = CycleClock::default();
        let mut p = UsbSerialJtag::new_esp32c3();
        Peripheral::attach_cycle_clock(&mut p, clock.clone());
        p.set_sof_enabled(false);
        for ms in 0..500u64 {
            clock.publish(ms * ESP32C3_CPU_CLOCK_HZ / SOF_HZ);
            assert_eq!(p.read_u32(OFF_INT_RAW).unwrap() & INT_SOF, 0);
        }
    }

    /// Without a clock the model must not invent frames — hand-built buses that
    /// bypass `add_peripheral` keep their previous behaviour.
    #[test]
    fn sof_stays_silent_without_a_cycle_clock() {
        let p = UsbSerialJtag::new_esp32c3();
        assert_eq!(p.read_u32(OFF_INT_RAW).unwrap() & INT_SOF, 0);
        assert!(!p.uses_scheduler());
    }

    /// The RX surface, exercised end to end through the register interface.
    /// Nothing on the bus feeds `inject_rx` yet — see the module docs.
    #[test]
    fn rx_fifo_sets_data_avail_and_recv_pkt() {
        let mut p = UsbSerialJtag::new_esp32c3();
        assert_eq!(
            p.read_u32(OFF_EP1_CONF).unwrap() & EP1_CONF_SERIAL_OUT_EP_DATA_AVAIL,
            0
        );
        p.inject_rx(b"hi");
        assert_ne!(
            p.read_u32(OFF_EP1_CONF).unwrap() & EP1_CONF_SERIAL_OUT_EP_DATA_AVAIL,
            0
        );
        assert_ne!(
            p.read_u32(OFF_INT_RAW).unwrap() & INT_SERIAL_OUT_RECV_PKT,
            0
        );
        assert_eq!(p.read_u32(OFF_EP1).unwrap() as u8, b'h');
    }

    /// BUS_RESET tells the driver to drop the connection. It must never fire on
    /// its own, or a healthy console would disconnect itself.
    #[test]
    fn bus_reset_is_never_raised_spontaneously() {
        let clock = CycleClock::default();
        let mut p = UsbSerialJtag::new_esp32c3();
        Peripheral::attach_cycle_clock(&mut p, clock.clone());
        p.write_u32(OFF_INT_ENA, INTR_MASK).unwrap();
        for ms in 0..50u64 {
            clock.publish(ms * ESP32C3_CPU_CLOCK_HZ / SOF_HZ);
            assert_eq!(p.read_u32(OFF_INT_RAW).unwrap() & INT_BUS_RESET, 0);
        }
    }
}
