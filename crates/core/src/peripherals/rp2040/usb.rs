// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 USB device controller + a minimal simulated USB host (datasheet §4.1,
//! DPRAM base `0x50100000`, register base `0x50110000`).
//!
//! ## Why this exists
//!
//! When proto.cat compiles a Pico sketch with the Arduino Mbed-OS core, the
//! default `Serial` is **USB CDC**, not UART0. `Serial.println("verdict=…")`
//! goes through `arduino::USBSerial` → `USBCDC::send` → `USBDevice::write_start`
//! → the mbed `USBPhy_RP2040` driver, which talks to this controller. Crucially
//! `USBSerial::write` first calls `connected()` and drops every byte until the
//! CDC terminal is connected — i.e. until a USB **host** has enumerated the
//! device and asserted the CDC DTR line. With no host, the sketch's output never
//! leaves the chip and the survival test can capture nothing.
//!
//! So this model is two halves that talk to each other through the DPRAM exactly
//! as silicon + a real host would:
//!
//! 1. **Device controller** — the register + DPRAM semantics the mbed driver
//!    drives: `SIE_CTRL.PULLUP_EN` to attach, per-endpoint buffer-control words
//!    (`AVAILABLE`/`FULL`/`LEN`/PID), `BUFF_STATUS`, `SIE_STATUS` event bits, and
//!    the `INTR`/`INTE`/`INTS` interrupt tree feeding `USBCTRL_IRQ` (NVIC 5).
//!
//! 2. **Host enumerator** — a small state machine in [`Rp2040Usb::tick`] that,
//!    once the device pulls up, drives a real control-transfer enumeration
//!    (GET_DESCRIPTOR device/config, SET_ADDRESS, SET_CONFIGURATION) followed by
//!    the CDC class requests SET_LINE_CODING and SET_CONTROL_LINE_STATE(DTR|RTS).
//!    That last request is what flips the device's `_terminal_connected`, after
//!    which `connected()` returns true and the sketch's bulk-IN writes flow.
//!    Every byte the device pushes on the CDC **bulk** IN endpoint is routed to
//!    the UART capture sink, so `verdict=GOOD rp2040` finally reaches the test.
//!
//! Register/bit values are taken verbatim from the pico-sdk headers the Arduino
//! core ships (`hardware/regs/usb.h`, `hardware/structs/usb.h`); the driver-side
//! buffer handshake was cross-checked against the compiled `USBPhy_RP2040`
//! (`ep0_write`/`ep0_read`/`endpoint_write`/`process`) in the fixture.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Window layout. One peripheral covers both the 4 KB DPRAM (0x0000..0x1000) and
// the register block (0x10000..0x14000). The bus resolves the RP2040 atomic
// SET/CLR/XOR aliases (+0x2000/+0x3000) to the real offset before calling us, so
// we only ever see the canonical addresses below.
// ---------------------------------------------------------------------------
const DPRAM_SIZE: usize = 0x1000;
const REG_BASE: u64 = 0x10000;

// Register offsets within the register block (datasheet §4.1.2).
const MAIN_CTRL: u64 = 0x40;
const SIE_CTRL: u64 = 0x4c;
const SIE_STATUS: u64 = 0x50;
const BUFF_STATUS: u64 = 0x58;
const INTR: u64 = 0x8c;
const INTE: u64 = 0x90;
const INTF: u64 = 0x94;
const INTS: u64 = 0x98;

// MAIN_CTRL / SIE_CTRL bits.
const MAIN_CTRL_CONTROLLER_EN: u32 = 1 << 0;
const SIE_CTRL_PULLUP_EN: u32 = 1 << 16;

// SIE_STATUS bits.
const SIE_STATUS_VBUS_DETECTED: u32 = 1 << 0;
const SIE_STATUS_SPEED_FS: u32 = 1 << 8; // full-speed in the [9:8] SPEED field
const SIE_STATUS_CONNECTED: u32 = 1 << 16;
const SIE_STATUS_SETUP_REC: u32 = 1 << 17;
const SIE_STATUS_TRANS_COMPLETE: u32 = 1 << 18;
const SIE_STATUS_BUS_RESET: u32 = 1 << 19;
// Write-1-to-clear mask: every latched event/error bit the driver acknowledges.
const SIE_STATUS_W1C: u32 = 0xFF00_0000 // [31:24] ACK/STALL/NAK-rec + errors
    | SIE_STATUS_TRANS_COMPLETE
    | SIE_STATUS_SETUP_REC
    | SIE_STATUS_BUS_RESET
    | (1 << 11); // RESUME

// INTR/INTS bits (raw source register).
const INTR_BUFF_STATUS: u32 = 1 << 4;
const INTR_TRANS_COMPLETE: u32 = 1 << 3;
const INTR_BUS_RESET: u32 = 1 << 12;
const INTR_SETUP_REQ: u32 = 1 << 16;

// Endpoint buffer-control bits (hardware/structs/usb.h).
const BUF_CTRL_FULL: u32 = 0x0000_8000;
const BUF_CTRL_AVAIL: u32 = 0x0000_0400;
const BUF_CTRL_LEN_MASK: u32 = 0x0000_03FF;

// USBCTRL_IRQ is NVIC IRQ 5 on the RP2040.
const USBCTRL_IRQ: u32 = 5;

// DPRAM offsets.
const DPRAM_SETUP: usize = 0x00; // 8-byte setup packet buffer
const DPRAM_EP0_BUF: usize = 0x100; // EP0 single data buffer (in and out)

// Per-endpoint buffer-control words live at 0x80 + 8*ep (IN) / 0x84 + 8*ep (OUT).
fn ep_in_buf_ctrl(ep: usize) -> usize {
    0x80 + ep * 8
}
fn ep_out_buf_ctrl(ep: usize) -> usize {
    0x84 + ep * 8
}
// Per-endpoint control words (endpoints 1..15) live at 0x08 + 8*(ep-1) (IN) /
// 0x0c + 8*(ep-1) (OUT); the low 16 bits hold the 64-aligned data buffer offset
// and bits [27:26] the endpoint type (2 = bulk).
fn ep_in_ctrl(ep: usize) -> usize {
    0x08 + (ep - 1) * 8
}

/// One standard control-transfer the host issues during enumeration.
#[derive(Clone, Debug)]
struct SetupStage {
    packet: [u8; 8],
    /// Non-empty for host→device (OUT-data) transfers such as SET_LINE_CODING.
    out_payload: &'static [u8],
}

impl SetupStage {
    const fn new(
        bm_request_type: u8,
        b_request: u8,
        w_value: u16,
        w_index: u16,
        w_length: u16,
        out_payload: &'static [u8],
    ) -> Self {
        Self {
            packet: [
                bm_request_type,
                b_request,
                w_value as u8,
                (w_value >> 8) as u8,
                w_index as u8,
                (w_index >> 8) as u8,
                w_length as u8,
                (w_length >> 8) as u8,
            ],
            out_payload,
        }
    }
    fn dir_in(&self) -> bool {
        self.packet[0] & 0x80 != 0
    }
    fn has_data(&self) -> bool {
        (self.packet[6] as u16 | ((self.packet[7] as u16) << 8)) != 0
    }
}

/// Post-attach debounce in peripheral ticks before the host resets and starts
/// enumerating. Models the real settling window a host waits after detecting a
/// device pull-up: enough for the firmware's `connect()` to finish (endpoints
/// reset, USB IRQ enabled), but small so enumeration completes early and leaves
/// the rest of the run for the sketch's `loop()` to transmit once CDC DTR is up.
const ATTACH_DELAY_TICKS: u32 = 2_000;

/// CDC line coding: 115200 baud, 1 stop bit, no parity, 8 data bits.
const CDC_LINE_CODING: &[u8] = &[0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08];

/// The minimal enumeration + CDC bring-up sequence that reaches
/// `_terminal_connected`. A real host does more (string descriptors, a status
/// probe of the config length first), but the mbed CDC stack only needs the
/// device/config descriptors read, the device configured, and the two CDC class
/// requests to mark the terminal connected.
fn enumeration_script() -> Vec<SetupStage> {
    vec![
        // GET_DESCRIPTOR(Device, 18)
        SetupStage::new(0x80, 6, 0x0100, 0, 18, &[]),
        // SET_ADDRESS(1)
        SetupStage::new(0x00, 5, 1, 0, 0, &[]),
        // GET_DESCRIPTOR(Configuration, 0xFF) — device returns the full config
        // (interface + CDC functional + endpoint descriptors) truncated to 0xFF.
        SetupStage::new(0x80, 6, 0x0200, 0, 0xFF, &[]),
        // SET_CONFIGURATION(1) — device calls configure() + endpoint_add() here.
        SetupStage::new(0x00, 9, 1, 0, 0, &[]),
        // CDC SET_LINE_CODING (class, OUT-data, 7 bytes)
        SetupStage::new(0x21, 0x20, 0, 0, 7, CDC_LINE_CODING),
        // CDC SET_CONTROL_LINE_STATE(DTR|RTS) — flips _terminal_connected.
        SetupStage::new(0x21, 0x22, 0x0003, 0, 0, &[]),
    ]
}

/// Host enumeration progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostState {
    /// Device has not pulled up yet.
    Detached,
    /// Bus reset issued; waiting for the device to acknowledge (clear
    /// SIE_STATUS.BUS_RESET).
    ResetIssued,
    /// A SETUP packet is in flight; servicing its data/status stages.
    Transfer,
    /// Enumeration + CDC bring-up complete; device terminal is connected.
    Configured,
}

#[derive(Debug)]
pub struct Rp2040Usb {
    dpram: Vec<u8>,
    /// Register block storage, indexed by word (offset/4) for offsets 0..0x100.
    regs: [u32; 0x40],

    // ---- host enumeration state ----
    host: HostState,
    /// Post-attach debounce: a real host waits tens of ms after the device
    /// pulls up before it resets + enumerates. That window lets the firmware
    /// finish booting (RTOS start, console/singleton construction) so the USB
    /// ISR does not run into first-time lazy inits that are illegal in ISR
    /// context. Counted in peripheral ticks.
    attach_countdown: u32,
    script: Vec<SetupStage>,
    setup_index: usize,
    /// True once we have delivered the OUT-data stage of the current transfer
    /// (so the following EP0-OUT arm is not mistaken for another data stage).
    out_data_sent: bool,
    /// Set when the current control transfer's status stage has been serviced;
    /// the next SETUP is withheld until the device's ISR has fully drained the
    /// transfer (BUFF_STATUS cleared, SETUP_REC cleared). Sending a new SETUP
    /// while the device is mid-completion desynchronises the mbed USBDevice EP0
    /// state machine (mbed error 270, "waiting on a user callback").
    awaiting_idle: bool,

    /// CDC bulk-IN bytes captured from the device, mirrored to the UART sink.
    sink: Option<Arc<Mutex<Vec<u8>>>>,
}

impl Default for Rp2040Usb {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040Usb {
    pub fn new() -> Self {
        Self {
            dpram: vec![0; DPRAM_SIZE],
            regs: [0; 0x40],
            host: HostState::Detached,
            attach_countdown: 0,
            script: enumeration_script(),
            setup_index: 0,
            out_data_sent: false,
            awaiting_idle: false,
            sink: None,
        }
    }

    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>) {
        self.sink = sink;
    }

    // -- register helpers -------------------------------------------------
    fn reg(&self, off: u64) -> u32 {
        self.regs[(off / 4) as usize]
    }
    fn set_reg(&mut self, off: u64, val: u32) {
        self.regs[(off / 4) as usize] = val;
    }

    // -- DPRAM word helpers ----------------------------------------------
    fn dpram_u32(&self, off: usize) -> u32 {
        u32::from_le_bytes([
            self.dpram[off],
            self.dpram[off + 1],
            self.dpram[off + 2],
            self.dpram[off + 3],
        ])
    }
    fn set_dpram_u32(&mut self, off: usize, val: u32) {
        self.dpram[off..off + 4].copy_from_slice(&val.to_le_bytes());
    }

    /// Raw interrupt status — every bit is level-derived from the SIE_STATUS /
    /// BUFF_STATUS "source" registers, matching the RP2040 INTR semantics, so
    /// the device clearing the source also clears the interrupt.
    fn intr(&self) -> u32 {
        let sie = self.reg(SIE_STATUS);
        let mut r = 0;
        if self.reg(BUFF_STATUS) != 0 {
            r |= INTR_BUFF_STATUS;
        }
        if sie & SIE_STATUS_SETUP_REC != 0 {
            r |= INTR_SETUP_REQ;
        }
        if sie & SIE_STATUS_BUS_RESET != 0 {
            r |= INTR_BUS_RESET;
        }
        if sie & SIE_STATUS_TRANS_COMPLETE != 0 {
            r |= INTR_TRANS_COMPLETE;
        }
        r
    }
    fn ints(&self) -> u32 {
        (self.intr() & self.reg(INTE)) | self.reg(INTF)
    }

    /// Mark a buffer as completed towards the CPU: set its BUFF_STATUS bit so
    /// the driver's ISR runs the matching endpoint callback.
    fn signal_buff(&mut self, ep: usize, is_in: bool) {
        let bit = 1u32 << (ep * 2 + if is_in { 0 } else { 1 });
        let bs = self.reg(BUFF_STATUS) | bit;
        self.set_reg(BUFF_STATUS, bs);
    }

    /// Push a SETUP packet to the device: write it into the DPRAM setup buffer,
    /// latch SIE_STATUS.SETUP_REC, and let the interrupt tree pend USBCTRL_IRQ.
    fn send_setup(&mut self, idx: usize) {
        let packet = self.script[idx].packet;
        self.dpram[DPRAM_SETUP..DPRAM_SETUP + 8].copy_from_slice(&packet);
        self.out_data_sent = false;
        let sie = self.reg(SIE_STATUS) | SIE_STATUS_SETUP_REC;
        self.set_reg(SIE_STATUS, sie);
    }

    /// Mark the current transfer's status stage serviced. The actual move to the
    /// next SETUP is deferred to [`Self::try_next_transfer`] once the device has
    /// gone idle, so we never overlap two control transfers.
    fn finish_transfer(&mut self) {
        self.awaiting_idle = true;
    }

    /// True when the device has drained the last transfer: no EP0 buffers armed,
    /// BUFF_STATUS acknowledged by the ISR, and no pending SETUP.
    fn device_idle(&self) -> bool {
        self.reg(BUFF_STATUS) == 0
            && self.reg(SIE_STATUS) & SIE_STATUS_SETUP_REC == 0
            && self.dpram_u32(ep_in_buf_ctrl(0)) & BUF_CTRL_AVAIL == 0
            && self.dpram_u32(ep_out_buf_ctrl(0)) & BUF_CTRL_AVAIL == 0
    }

    /// Once idle, issue the next control transfer or complete enumeration.
    fn try_next_transfer(&mut self) {
        if !self.device_idle() {
            return;
        }
        self.awaiting_idle = false;
        self.setup_index += 1;
        if self.setup_index < self.script.len() {
            let idx = self.setup_index;
            self.send_setup(idx);
        } else {
            self.host = HostState::Configured;
        }
    }

    /// Service the current control transfer's EP0 data/status stages by
    /// reacting to the buffers the device driver arms.
    fn service_control(&mut self) {
        let stage = self.script[self.setup_index].clone();

        // EP0 IN: device has data (or a status ZLP) for the host.
        let in_ctrl = self.dpram_u32(ep_in_buf_ctrl(0));
        if in_ctrl & BUF_CTRL_AVAIL != 0 && in_ctrl & BUF_CTRL_FULL != 0 {
            // Consume the device's payload (we don't need the descriptor bytes;
            // enumeration only needs the transfers to complete). Clear the
            // buffer and report completion.
            self.set_dpram_u32(
                ep_in_buf_ctrl(0),
                in_ctrl & !(BUF_CTRL_AVAIL | BUF_CTRL_FULL),
            );
            self.signal_buff(0, true);

            if stage.dir_in() && stage.has_data() {
                // Data stage of an IN transfer — more data or an OUT status
                // stage follows; do not advance yet.
            } else {
                // Status ZLP of a no-data or OUT-data transfer → transfer done.
                self.finish_transfer();
            }
            return;
        }

        // EP0 OUT: device is ready to receive from the host.
        let out_ctrl = self.dpram_u32(ep_out_buf_ctrl(0));
        if out_ctrl & BUF_CTRL_AVAIL != 0 {
            if !stage.dir_in() && stage.has_data() && !self.out_data_sent {
                // Data stage of an OUT transfer (e.g. SET_LINE_CODING): hand the
                // payload to the device's EP0 buffer and mark it full.
                let payload = stage.out_payload;
                let len = payload.len().min((out_ctrl & BUF_CTRL_LEN_MASK) as usize);
                self.dpram[DPRAM_EP0_BUF..DPRAM_EP0_BUF + len].copy_from_slice(&payload[..len]);
                let done =
                    (out_ctrl & !(BUF_CTRL_AVAIL | BUF_CTRL_LEN_MASK)) | BUF_CTRL_FULL | len as u32;
                self.set_dpram_u32(ep_out_buf_ctrl(0), done);
                self.signal_buff(0, false);
                self.out_data_sent = true;
                // Status IN stage follows; wait for it.
            } else {
                // Status ZLP stage of an IN-data transfer.
                let done = (out_ctrl & !(BUF_CTRL_AVAIL | BUF_CTRL_LEN_MASK)) | BUF_CTRL_FULL;
                self.set_dpram_u32(ep_out_buf_ctrl(0), done);
                self.signal_buff(0, false);
                self.finish_transfer();
            }
        }
    }

    /// Drain any CDC bulk-IN endpoint the device armed, routing its bytes to the
    /// capture sink. Runs once the device is configured. Interrupt/notification
    /// endpoints are completed but not captured (they are not console text).
    fn service_bulk_in(&mut self) {
        for ep in 1..16usize {
            let ctrl_off = ep_in_buf_ctrl(ep);
            let bc = self.dpram_u32(ctrl_off);
            if bc & BUF_CTRL_AVAIL == 0 || bc & BUF_CTRL_FULL == 0 {
                continue;
            }
            let len = (bc & BUF_CTRL_LEN_MASK) as usize;
            let ep_ctrl = self.dpram_u32(ep_in_ctrl(ep));
            let is_bulk = (ep_ctrl >> 26) & 0x3 == 2;
            let buf = (ep_ctrl & 0xFFC0) as usize;
            if is_bulk && buf + len <= self.dpram.len() {
                let bytes: Vec<u8> = self.dpram[buf..buf + len].to_vec();
                if let Some(sink) = &self.sink {
                    if let Ok(mut s) = sink.lock() {
                        s.extend_from_slice(&bytes);
                    }
                }
            }
            self.set_dpram_u32(ctrl_off, bc & !(BUF_CTRL_AVAIL | BUF_CTRL_FULL));
            self.signal_buff(ep, true);
        }
    }

    /// Advance the host enumeration state machine one step.
    fn host_poll(&mut self) {
        match self.host {
            HostState::Detached => {
                let attached = self.reg(MAIN_CTRL) & MAIN_CTRL_CONTROLLER_EN != 0
                    && self.reg(SIE_CTRL) & SIE_CTRL_PULLUP_EN != 0;
                if attached {
                    // Let the device's connect() settle (endpoints reset, USB
                    // IRQ enabled) before resetting the bus and enumerating.
                    self.attach_countdown = self.attach_countdown.saturating_add(1);
                    if self.attach_countdown < ATTACH_DELAY_TICKS {
                        return;
                    }
                    // Signal cable + bus reset: VBUS present, full-speed, and a
                    // reset the device must acknowledge before we enumerate.
                    let sie = self.reg(SIE_STATUS)
                        | SIE_STATUS_VBUS_DETECTED
                        | SIE_STATUS_SPEED_FS
                        | SIE_STATUS_CONNECTED
                        | SIE_STATUS_BUS_RESET;
                    self.set_reg(SIE_STATUS, sie);
                    self.host = HostState::ResetIssued;
                }
            }
            HostState::ResetIssued => {
                // The driver's ISR memsets the DPRAM and clears BUS_RESET.
                if self.reg(SIE_STATUS) & SIE_STATUS_BUS_RESET == 0 {
                    self.setup_index = 0;
                    self.send_setup(0);
                    self.host = HostState::Transfer;
                }
            }
            HostState::Transfer => {
                if self.awaiting_idle {
                    self.try_next_transfer();
                } else {
                    self.service_control();
                }
            }
            HostState::Configured => {}
        }
        // The CDC data endpoint can be serviced as soon as it exists.
        if matches!(self.host, HostState::Transfer | HostState::Configured) {
            self.service_bulk_in();
        }
    }
}

impl Peripheral for Rp2040Usb {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_u32(offset & !3)? >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Byte writes land straight in DPRAM; register writes are word-oriented
        // and handled in write_u32 (the bus always issues 32-bit MMIO here).
        if (offset as usize) < DPRAM_SIZE {
            self.dpram[offset as usize] = value;
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if (offset as usize) < DPRAM_SIZE {
            return Ok(self.dpram_u32(offset as usize));
        }
        let local = offset - REG_BASE;
        Ok(match local {
            INTR => self.intr(),
            INTS => self.ints(),
            l if l < 0x100 => self.reg(l),
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if (offset as usize) < DPRAM_SIZE {
            self.set_dpram_u32(offset as usize, value);
            return Ok(());
        }
        let local = offset - REG_BASE;
        match local {
            SIE_STATUS => {
                // Write-1-to-clear the latched event/error bits.
                let cur = self.reg(SIE_STATUS);
                self.set_reg(SIE_STATUS, cur & !(value & SIE_STATUS_W1C));
            }
            BUFF_STATUS => {
                let cur = self.reg(BUFF_STATUS);
                self.set_reg(BUFF_STATUS, cur & !value);
            }
            INTR | INTS => {} // read-only
            l if l < 0x100 => self.set_reg(l, value),
            _ => {}
        }
        Ok(())
    }

    fn write_word_32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_u32(offset, value)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.host_poll();
        let mut res = PeripheralTickResult::default();
        if self.ints() != 0 {
            // Level-sensitive: re-pend USBCTRL_IRQ every tick until the driver
            // clears the underlying source, matching real NVIC level behaviour.
            res.explicit_irqs = Some(vec![USBCTRL_IRQ]);
        }
        res
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

    fn rd(u: &Rp2040Usb, off: u64) -> u32 {
        u.read_u32(REG_BASE + off).unwrap()
    }
    fn wr(u: &mut Rp2040Usb, off: u64, v: u32) {
        u.write_u32(REG_BASE + off, v).unwrap();
    }

    /// Bring the device to the point where the host has issued its bus reset:
    /// controller enabled, pull-up asserted, then enough ticks to clear the
    /// attach debounce.
    fn attach(u: &mut Rp2040Usb) {
        wr(u, MAIN_CTRL, MAIN_CTRL_CONTROLLER_EN);
        wr(u, SIE_CTRL, SIE_CTRL_PULLUP_EN);
        for _ in 0..ATTACH_DELAY_TICKS {
            u.tick();
        }
    }

    #[test]
    fn host_resets_bus_after_attach_debounce() {
        let mut u = Rp2040Usb::new();
        wr(&mut u, MAIN_CTRL, MAIN_CTRL_CONTROLLER_EN);
        wr(&mut u, SIE_CTRL, SIE_CTRL_PULLUP_EN);
        // Before the debounce elapses the host stays quiet.
        u.tick();
        assert_eq!(rd(&u, SIE_STATUS) & SIE_STATUS_BUS_RESET, 0);
        for _ in 0..ATTACH_DELAY_TICKS {
            u.tick();
        }
        let sie = rd(&u, SIE_STATUS);
        assert_ne!(sie & SIE_STATUS_BUS_RESET, 0, "bus reset asserted");
        assert_ne!(sie & SIE_STATUS_CONNECTED, 0);
        assert_ne!(sie & SIE_STATUS_VBUS_DETECTED, 0);
    }

    #[test]
    fn intr_is_level_derived_and_bus_reset_gates_nvic() {
        let mut u = Rp2040Usb::new();
        attach(&mut u);
        // Bus reset is latched in SIE_STATUS → INTR.BUS_RESET reflects it.
        assert_ne!(rd(&u, INTR) & INTR_BUS_RESET, 0);
        // With BUS_RESET unmasked in INTE, the tick pends USBCTRL_IRQ.
        wr(&mut u, INTE, INTR_BUS_RESET);
        assert_eq!(u.tick().explicit_irqs, Some(vec![USBCTRL_IRQ]));
        // Device acknowledges the reset (write-1-clear); the interrupt clears
        // with its source.
        wr(&mut u, SIE_STATUS, SIE_STATUS_BUS_RESET);
        assert_eq!(rd(&u, INTR) & INTR_BUS_RESET, 0);
    }

    #[test]
    fn first_setup_sent_once_device_acks_reset() {
        let mut u = Rp2040Usb::new();
        attach(&mut u);
        // Device clears the bus reset, as its ISR does.
        wr(&mut u, SIE_STATUS, SIE_STATUS_BUS_RESET);
        u.tick();
        // The host has delivered the first SETUP: the 8-byte packet lands in the
        // DPRAM setup buffer and SETUP_REC is latched.
        assert_ne!(rd(&u, SIE_STATUS) & SIE_STATUS_SETUP_REC, 0);
        let setup = &u.dpram[DPRAM_SETUP..DPRAM_SETUP + 8];
        assert_eq!(setup[0], 0x80, "GET_DESCRIPTOR is device-to-host");
        assert_eq!(setup[1], 6, "bRequest = GET_DESCRIPTOR");
        assert_eq!(setup[3], 0x01, "descriptor type = Device");
    }

    #[test]
    fn buff_status_is_write_one_clear() {
        let mut u = Rp2040Usb::new();
        u.signal_buff(0, true); // EP0 IN
        u.signal_buff(2, false); // EP2 OUT
        assert_eq!(rd(&u, BUFF_STATUS), 0b1 | (1 << 5));
        wr(&mut u, BUFF_STATUS, 0b1); // ack EP0 IN only
        assert_eq!(rd(&u, BUFF_STATUS), 1 << 5);
    }

    #[test]
    fn bulk_in_endpoint_bytes_reach_the_sink() {
        let mut u = Rp2040Usb::new();
        let sink = Arc::new(Mutex::new(Vec::new()));
        u.set_sink(Some(sink.clone()));
        u.host = HostState::Configured;

        // Model the device arming EP1 IN with a bulk buffer at DPRAM 0x1c0.
        let buf = 0x1c0usize;
        u.dpram[buf..buf + 3].copy_from_slice(b"hi!");
        // ep_ctrl: ENABLE | type=2 (bulk) in [27:26] | buffer offset in low bits.
        u.set_dpram_u32(ep_in_ctrl(1), 0x8000_0000 | (2 << 26) | buf as u32);
        // buf_ctrl: FULL | AVAIL | len=3.
        u.set_dpram_u32(ep_in_buf_ctrl(1), BUF_CTRL_FULL | BUF_CTRL_AVAIL | 3);

        u.tick();

        assert_eq!(&*sink.lock().unwrap(), b"hi!");
        // Buffer handed back and EP1 IN completion signalled.
        let bc = u.dpram_u32(ep_in_buf_ctrl(1));
        assert_eq!(bc & (BUF_CTRL_FULL | BUF_CTRL_AVAIL), 0);
        assert_ne!(rd(&u, BUFF_STATUS) & (1 << 2), 0, "EP1_IN buff status set");
    }

    #[test]
    fn interrupt_endpoint_is_completed_but_not_captured() {
        let mut u = Rp2040Usb::new();
        let sink = Arc::new(Mutex::new(Vec::new()));
        u.set_sink(Some(sink.clone()));
        u.host = HostState::Configured;
        let buf = 0x1c0usize;
        u.dpram[buf..buf + 2].copy_from_slice(&[0xa1, 0x20]);
        // type=3 (interrupt) — a CDC notification endpoint, not console text.
        u.set_dpram_u32(ep_in_ctrl(1), 0x8000_0000 | (3 << 26) | buf as u32);
        u.set_dpram_u32(ep_in_buf_ctrl(1), BUF_CTRL_FULL | BUF_CTRL_AVAIL | 2);
        u.tick();
        assert!(sink.lock().unwrap().is_empty(), "notification not captured");
        assert_eq!(u.dpram_u32(ep_in_buf_ctrl(1)) & BUF_CTRL_AVAIL, 0);
    }
}
