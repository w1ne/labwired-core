// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::uart::UartStreamDevice;
use std::any::Any;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// IO-Link message checksum (Spec V1.1.5 A.1.6): XOR all octets with seed
/// `0x52`, then compress the 8-bit result to 6 bits with equations (A.1).
/// The checksum/type octet (CKT for master messages, CKS for device replies)
/// is part of the message with its checksum bits (0-5) zeroed; its
/// type/status bits are included as-is. Callers OR the returned 6-bit value
/// into that octet.
pub(crate) fn checksum6(octets: &[u8]) -> u8 {
    let mut ck8: u8 = 0x52;
    for &o in octets {
        ck8 ^= o;
    }
    let b = |n: u8| (ck8 >> n) & 1;
    ((b(7) ^ b(5) ^ b(3) ^ b(1)) << 5)
        | ((b(6) ^ b(4) ^ b(2) ^ b(0)) << 4)
        | ((b(7) ^ b(6)) << 3)
        | ((b(5) ^ b(4)) << 2)
        | ((b(3) ^ b(2)) << 1)
        | (b(1) ^ b(0))
}

/// Encode a Type 0 master frame: `[MC, CKT(type=0, ck6)]`. The CKT octet is
/// passed to the checksum with bits 0-5 zeroed (A.1.6).
pub(crate) fn encode_type0(mc: u8) -> Vec<u8> {
    vec![mc, checksum6(&[mc, 0x00])]
}

/// Encode a Type 1 cyclic request: `[MC=0x00, CKT, PD_out..., OD=0x00]` with the
/// M-sequence type 1_1 in CKT bits 6-7 and the A.1.6 checksum in bits 0-5
/// (A.2.3/A.1.6; the request has no trailing checksum octet).
pub(crate) fn encode_type1_cycle(pd_out: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x00u8, 0x40];
    frame.extend_from_slice(pd_out);
    frame.push(0x00); // OD (1-byte, idle)
    frame[1] |= checksum6(&frame);
    frame
}

/// Parsed device OPERATE response.
#[derive(Debug, Clone)]
pub(crate) struct OperateResponse {
    pub(crate) pd: Vec<u8>,
    pub(crate) pd_valid: bool,
    pub(crate) checksum_ok: bool,
    /// CKS Event flag (bit 7): the device has a diagnostic event pending for
    /// the master to retrieve (A.1.5).
    pub(crate) event_present: bool,
}

/// Decode a device reply `[PD_in..., OD..., CKS]` (Spec A.1.5; length
/// `pd_in_len + od_len + 1`). There is no leading status octet: the checksum
/// is over every data octet and CKS with its checksum bits (0-5) zeroed, the
/// PD status is CKS bit 6 (1 = invalid) and the Event flag is CKS bit 7.
pub(crate) fn decode_operate(data: &[u8], pd_in_len: usize, od_len: usize) -> OperateResponse {
    if data.len() < pd_in_len + od_len + 1 {
        return OperateResponse {
            pd: Vec::new(),
            pd_valid: false,
            checksum_ok: false,
            event_present: false,
        };
    }
    let pd_end = data.len() - od_len - 1;
    let pd = data[..pd_end].to_vec();
    let cks = data[data.len() - 1];
    let mut masked = data.to_vec();
    if let Some(last) = masked.last_mut() {
        *last &= 0xC0;
    }
    let checksum_ok = checksum6(&masked) == cks & 0x3F;
    let pd_valid = cks & 0x40 == 0;
    let event_present = cks & 0x80 != 0;
    OperateResponse {
        pd,
        pd_valid,
        checksum_ok,
        event_present,
    }
}

// ─── M-sequence control and ISDU/Diagnosis framing (A.1.2, A.5, Table 52) ───

// The ISDU request encoders below are exercised by unit tests now and will be
// driven from the scheduler by the follow-on ISDU/parameter-exchange task; the
// pure model currently issues no ISDU reads, so they are not yet referenced
// from non-test code.
/// Communication channel values (A.1.2, Table A.1): bits 5-6 of the MC octet.
#[allow(dead_code)]
pub(crate) const CHANNEL_PROCESS: u8 = 0;
#[allow(dead_code)]
pub(crate) const CHANNEL_PAGE: u8 = 1;
pub(crate) const CHANNEL_DIAGNOSIS: u8 = 2;
/// Direct Parameter Page 1 offset of MinCycleTime (used by the startup probe).
pub(crate) const DPP1_OFF_MIN_CYCLE_TIME: u8 = 0x02;
#[allow(dead_code)]
pub(crate) const CHANNEL_ISDU: u8 = 3;

/// FlowCTRL values (Table 52).
pub(crate) const FLOWCTRL_START: u8 = 0x10;
#[allow(dead_code)]
pub(crate) const FLOWCTRL_IDLE: u8 = 0x11;
#[allow(dead_code)]
pub(crate) const FLOWCTRL_ABORT: u8 = 0x1F;

/// Build an M-sequence control octet (A.1.2): `R/W<<7 | channel<<5 | address`.
/// On the ISDU channel the low 5 address bits carry FlowCTRL.
pub(crate) fn mc(rw_read: bool, channel: u8, address: u8) -> u8 {
    ((rw_read as u8) << 7) | ((channel & 0x03) << 5) | (address & 0x1F)
}

/// CHKPDU (A.5.6): XOR of every ISDU octet, with CHKPDU itself taken as 0.
#[allow(dead_code)]
pub(crate) fn isdu_chkpdu(octets_without_chkpdu: &[u8]) -> u8 {
    octets_without_chkpdu.iter().fold(0u8, |acc, &o| acc ^ o)
}

/// Build an ISDU read request (Table A.13) using the index format from
/// Table A.15: 8-bit index (subindex 0), 8-bit index + subindex, or 16-bit
/// index + subindex. Length counts every ISDU octet including CHKPDU (A.5.3).
#[allow(dead_code)]
pub(crate) fn isdu_read_request(index: u16, subindex: Option<u8>) -> Vec<u8> {
    // Subindex 0 references the whole object (Table A.15): no subindex octet.
    let sub = subindex.filter(|&s| s != 0);
    let (service, body): (u8, Vec<u8>) = if index <= 0xFF {
        match sub {
            Some(s) => (0xA, vec![index as u8, s]),
            None => (0x9, vec![index as u8]),
        }
    } else {
        (0xB, vec![(index >> 8) as u8, index as u8, sub.unwrap_or(0)])
    };
    let total = 1 + body.len() + 1; // I-Service/Length + body + CHKPDU
    let mut out = vec![(service << 4) | total as u8];
    out.extend_from_slice(&body);
    let chk = isdu_chkpdu(&out);
    out.push(chk);
    out
}

/// Split an ISDU octet stream into OD-width chunks with their FlowCTRL value
/// (7.3.6.2, Table 52): the first message uses START, then COUNT increments
/// from 1 and wraps 15 -> 0.
#[allow(dead_code)]
pub(crate) fn isdu_flowctrl_segments(isdu: &[u8], od_len: usize) -> Vec<(u8, Vec<u8>)> {
    let chunk = od_len.max(1);
    let mut out = Vec::new();
    let mut count: u8 = 1;
    for (i, part) in isdu.chunks(chunk).enumerate() {
        let flow = if i == 0 { FLOWCTRL_START } else { count };
        out.push((flow, part.to_vec()));
        if i > 0 {
            count = if count == 15 { 0 } else { count + 1 };
        }
    }
    out
}

/// Encode a TYPE_0 master write message: `[MC, CKT, OD...]` with the A.1.6
/// checksum in CKT bits 0-5 (Figure A.5; no type bits, no trailing checksum).
pub(crate) fn encode_type0_write(mc: u8, od: &[u8]) -> Vec<u8> {
    let mut frame = vec![mc, 0x00];
    frame.extend_from_slice(od);
    frame[1] = checksum6(&frame);
    frame
}

/// Diagnosis-channel event memory read (Table 59 T2/T3): R, DIAGNOSIS, address.
pub(crate) fn diagnosis_read_mc(address: u8) -> u8 {
    mc(true, CHANNEL_DIAGNOSIS, address)
}

/// Diagnosis-channel event confirmation (Table 59 T8): W, DIAGNOSIS, StatusCode.
pub(crate) fn diagnosis_write_mc(address: u8) -> u8 {
    mc(false, CHANNEL_DIAGNOSIS, address)
}

/// Event-readout plan (Table 58/59): StatusCode (address 0), the six event
/// slots (addresses 1..=0x12), then the StatusCode write that clears the Event
/// flag. The boolean marks the one write message.
pub(crate) fn event_readout_plan() -> Vec<(u8, bool)> {
    let mut plan: Vec<(u8, bool)> = (0..=0x12u8)
        .map(|a| (diagnosis_read_mc(a), false))
        .collect();
    plan.push((diagnosis_write_mc(0x00), true));
    plan
}

/// The M-sequence controls of [`event_readout_plan`], in order.
#[allow(dead_code)]
pub(crate) fn event_readout_mcs() -> Vec<u8> {
    event_readout_plan().into_iter().map(|(mc, _)| mc).collect()
}

/// IO-Link COM speed (display/config only in this model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IolinkComSpeed {
    Com1,
    Com2,
    Com3,
}

/// Link state exposed to the inspector panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IolinkLinkState {
    Startup,
    Operate,
}

/// Which frame in the startup/cyclic schedule a trace record came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IolinkFrameKind {
    WakeUp,
    Idle,
    OperateReq,
    Cyclic,
    /// Diagnosis-channel event memory readout (Table 59) triggered by the CKS
    /// Event flag: StatusCode, event slots, then the StatusCode confirmation.
    EventReadout,
}

/// One captured master↔device exchange, decoded where the master already
/// builds requests and parses responses. Serialized to JS as a plain object.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IolinkXfer {
    pub seq: u32,
    pub kind: IolinkFrameKind,
    pub com: IolinkComSpeed,
    pub pd_out: Vec<u8>,
    pub pd_in: Vec<u8>,
    pub od: u8,
    /// `None` for non-cyclic frames (no decodable OPERATE response).
    pub ck_ok: Option<bool>,
    pub pd_valid: Option<bool>,
    pub link_state: IolinkLinkState,
    pub raw_master: Vec<u8>,
    pub raw_device: Vec<u8>,
}

/// In-flight frame: request bytes known at queue time; the device response
/// accumulates until the next frame is queued, then it's finalized.
#[derive(Debug, Clone)]
struct PendingXfer {
    seq: u32,
    kind: IolinkFrameKind,
    pd_out: Vec<u8>,
    link_state: IolinkLinkState,
    raw_master: Vec<u8>,
    raw_device: Vec<u8>,
}

/// Max trace records retained (oldest dropped).
const TRACE_CAP: usize = 256;

/// Default ticks the master waits (one `poll` per UART tick) between frames.
/// The simulated device executes far slower than the UART advances, so frames
/// are paced generously to guarantee the device has fully processed (and
/// replied to) one frame before the next arrives — this is what keeps the
/// device's byte framing aligned. Sized for the `-O0` iolink-dido demo firmware;
/// overridable per device via the `frame_gap_ticks` config (a faster `-O2`
/// device, e.g. the C3 thermal firmware, can run a much smaller gap so many
/// cyclic reads fit the step budget).
const FRAME_GAP_TICKS: u32 = 6000;

/// Native IO-Link master peer. Attaches to the firmware's UART as a
/// `UartStreamDevice`: `poll` drives the master's request bytes onto the firmware
/// RX path, `on_tx_byte` receives the device's response bytes from the firmware
/// TX path.
///
/// Drives a **deterministic, tick-paced** startup schedule rather than reacting
/// to response timing: wake-up pulse → Type-0 READ of the Direct Parameter page
/// MinCycleTime octet (spec transition T1, → PREOPERATE) → Type-0 WRITE of the
/// DeviceOperate MasterCommand (→ ESTAB_COM) → cyclic Type 1 requests
/// (→ OPERATE). Process data input is captured from the cyclic responses.
/// Selects which protocol engine backs an [`IolinkMaster`]. The hand-rolled
/// engine is always available; the `Native` variant drives the real
/// `iolinki-master` C stack and only exists under the `iolink-native` feature.
/// Trace/UART behavior still runs on the hand-rolled path for now — this enum
/// currently records the chosen backend so the swap can land incrementally.
///
/// `allow(dead_code)`: under `iolink-native` only `Native` is constructed (so
/// `HandRolled` looks unused) and the native port is held but not yet read
/// because trace/UART still run hand-rolled. Removing it would force a
/// premature migration. See plan Task 5.
#[allow(dead_code)]
#[derive(Debug)]
enum IolinkMasterBackend {
    HandRolled,
    #[cfg(feature = "iolink-native")]
    Native(super::iolink_native::NativeIolinkMasterPort),
}

#[derive(Debug, serde::Serialize)]
pub struct IolinkMaster {
    pd_in_len: usize,
    od_len: usize,
    com: IolinkComSpeed,
    pub link_state: IolinkLinkState,
    /// Bytes still to send onto the firmware RX path (one frame at a time).
    #[serde(skip)]
    tx_queue: VecDeque<u8>,
    /// Device-response bytes accumulated since the current frame was queued.
    #[serde(skip)]
    rx_accum: Vec<u8>,
    /// Schedule position (0 = wake-up, then IDLEs, transition, cyclic Type 1).
    step: u32,
    /// UART ticks elapsed since the current frame finished sending.
    #[serde(skip)]
    gap_ticks: u32,
    /// Latest valid process-data input bytes received from the device.
    latest_pd: Vec<u8>,
    /// Latches true on the first valid cyclic frame and is intentionally sticky.
    pub pd_valid: bool,
    /// Bounded ring of completed transactions (oldest→newest), for the analyzer.
    #[serde(skip)]
    trace: VecDeque<IolinkXfer>,
    /// The frame currently in flight (request sent, response accumulating).
    #[serde(skip)]
    current: Option<PendingXfer>,
    /// Monotonic per-frame sequence number.
    #[serde(skip)]
    frame_seq: u32,
    /// Protocol engine backing this master (hand-rolled, or the real native
    /// `iolinki-master` stack under the `iolink-native` feature).
    #[serde(skip)]
    backend: IolinkMasterBackend,
    /// Optional capture sink the master writes a human-readable record of what
    /// it received into: `MASTER PD=<hex>`, `MASTER VERDICT ...` (decoded
    /// thermal-fingerprint verdict for the 9-byte PD schema), and `MASTER EVENT
    /// ...` when the device sets the CKS Event flag (A.1.5). Wired to the same
    /// captured UART-TX buffer the test runner reads, so a test can assert on
    /// what the MASTER observed over IO-Link (not just the device console). When
    /// `None`, the master is silent (UI/default path unchanged).
    #[serde(skip)]
    log_sink: Option<Arc<Mutex<Vec<u8>>>>,
    /// Whether the previous decoded cyclic reply carried the CKS Event flag, so
    /// a `MASTER EVENT` line is emitted once per event rising edge (not per frame).
    #[serde(skip)]
    event_latched: bool,
    /// Pending diagnosis-channel event memory readout (Table 59): `(MC, is_write)`
    /// in order; empty when no event readout is in progress.
    #[serde(skip)]
    event_readout: VecDeque<(u8, bool)>,
    /// Inter-frame gap in UART ticks (overridable per device via config).
    frame_gap_ticks: u32,
}

impl IolinkMaster {
    pub fn new(pd_in_len: usize, od_len: usize, com: IolinkComSpeed) -> Self {
        Self::new_with_gap(pd_in_len, od_len, com, FRAME_GAP_TICKS)
    }

    /// Like [`new`], but with an explicit inter-frame gap (UART ticks). A faster
    /// device can use a small gap so many cyclic reads fit the step budget.
    pub fn new_with_gap(
        pd_in_len: usize,
        od_len: usize,
        com: IolinkComSpeed,
        frame_gap_ticks: u32,
    ) -> Self {
        #[cfg(feature = "iolink-native")]
        let backend = IolinkMasterBackend::Native(
            super::iolink_native::NativeIolinkMasterPort::new_type2_com3(pd_in_len as u8, 0),
        );
        #[cfg(not(feature = "iolink-native"))]
        let backend = IolinkMasterBackend::HandRolled;

        let mut m = Self {
            pd_in_len,
            od_len,
            com,
            link_state: IolinkLinkState::Startup,
            tx_queue: VecDeque::new(),
            rx_accum: Vec::new(),
            step: 0,
            gap_ticks: 0,
            latest_pd: vec![0u8; pd_in_len.max(1)],
            pd_valid: false,
            trace: VecDeque::new(),
            current: None,
            frame_seq: 0,
            backend,
            log_sink: None,
            event_latched: false,
            event_readout: VecDeque::new(),
            frame_gap_ticks: frame_gap_ticks.max(1),
        };
        m.queue_next_frame(); // queue the wake-up immediately
        m
    }

    /// Wire a capture sink so the master records what it received over IO-Link
    /// (`MASTER PD=`, `MASTER VERDICT`, `MASTER EVENT`) into a test-observable
    /// channel. Typically the same `Arc<Mutex<Vec<u8>>>` the runner attaches as
    /// the UART-TX capture sink, so `uart_contains` assertions can key on it.
    pub fn set_log_sink(&mut self, sink: Arc<Mutex<Vec<u8>>>) {
        self.log_sink = Some(sink);
    }

    /// Append a line (CRLF-terminated) to the capture sink, if one is wired.
    fn log_line(&self, line: &str) {
        if let Some(sink) = &self.log_sink {
            if let Ok(mut guard) = sink.lock() {
                guard.extend_from_slice(line.as_bytes());
                guard.extend_from_slice(b"\r\n");
            }
        }
    }

    /// Decode the device's 9-byte thermal-fingerprint process-data frame and
    /// log the master-observed verdict + raw hex. The PD schema is the device's
    /// published process data:
    ///   [int16 temp_x100][int16 heatrate_x100][u8 state][u8 health]
    ///   [u16 time_to_limit_s][u8 event_flags]   (big-endian on the wire)
    /// For other PD lengths only the raw hex is logged (no thermal schema).
    fn log_received_pd(&self, pd: &[u8]) {
        if self.log_sink.is_none() {
            return;
        }
        let mut hex = String::with_capacity(pd.len() * 2);
        for b in pd {
            hex.push_str(&format!("{b:02X}"));
        }
        self.log_line(&format!("MASTER PD={hex}"));

        if pd.len() == 9 {
            let state = match pd[4] {
                0 => "IDLE",
                1 => "WARMUP",
                2 => "STABLE",
                3 => "FAULT",
                _ => "UNKNOWN",
            };
            // pd[8] high nibble carries the device's fault classification (the
            // tfs_fault_t enum); the low nibble carries the 5-bit event flags.
            // This is the device's published PD schema — the master decodes the
            // verdict the device actually computed, it does not re-derive it.
            let fault = match pd[8] >> 4 {
                0 => "NONE",
                1 => "OVERTEMP",
                2 => "COOLING_FAILURE",
                3 => "HOTSPOT_EMERGENCE",
                _ => "UNKNOWN",
            };
            let health = pd[5];
            self.log_line(&format!(
                "MASTER VERDICT state={state} health={health} fault={fault}"
            ));
        }
    }

    /// Name of the protocol engine currently backing this master. Used by the
    /// native-backend gating test; not part of the stable component API.
    pub fn backend_name_for_test(&self) -> &'static str {
        match &self.backend {
            IolinkMasterBackend::HandRolled => "hand-rolled",
            #[cfg(feature = "iolink-native")]
            IolinkMasterBackend::Native(_) => "iolinki-master",
        }
    }

    /// First process-data input byte (channel bitmap for a DI hub).
    pub fn input_byte(&self) -> u8 {
        self.latest_pd.first().copied().unwrap_or(0)
    }

    pub fn com_speed(&self) -> IolinkComSpeed {
        self.com
    }

    /// Snapshot of captured transactions, oldest→newest. Cloned for the UI.
    pub fn trace_snapshot(&self) -> Vec<IolinkXfer> {
        self.trace.iter().cloned().collect()
    }

    /// Clear the trace ring (the analyzer's "Clear" control).
    pub fn trace_clear(&mut self) {
        self.trace.clear();
    }

    fn operate_response_len(&self) -> usize {
        self.pd_in_len + self.od_len + 1
    }

    /// Turn a completed in-flight frame into a trace record, decoding the
    /// device response only for cyclic (OPERATE) frames.
    fn finalize_xfer(&self, p: PendingXfer) -> IolinkXfer {
        let (pd_in, ck_ok, pd_valid) = if matches!(p.kind, IolinkFrameKind::Cyclic) {
            let n = self.operate_response_len();
            if p.raw_device.len() >= n {
                let r = decode_operate(&p.raw_device[..n], self.pd_in_len, self.od_len);
                (r.pd, Some(r.checksum_ok), Some(r.pd_valid))
            } else {
                (Vec::new(), None, Some(false))
            }
        } else {
            (Vec::new(), None, None)
        };
        IolinkXfer {
            seq: p.seq,
            kind: p.kind,
            com: self.com,
            pd_out: p.pd_out,
            pd_in,
            od: 0x00,
            ck_ok,
            pd_valid,
            link_state: p.link_state,
            raw_master: p.raw_master,
            raw_device: p.raw_device,
        }
    }

    /// Queue the next frame in the startup/cyclic schedule and advance `step`.
    /// Also finalizes the previous in-flight frame into the trace ring.
    fn queue_next_frame(&mut self) {
        // Finalize the previous frame (its response accumulated during the gap).
        if let Some(p) = self.current.take() {
            let x = self.finalize_xfer(p);
            if self.trace.len() >= TRACE_CAP {
                self.trace.pop_front();
            }
            self.trace.push_back(x);
        }
        self.rx_accum.clear();

        let (frame, kind): (Vec<u8>, IolinkFrameKind) = if self.step == 0 {
            (vec![0x55], IolinkFrameKind::WakeUp) // wake-up pulse (once)
        } else if self.step == 1 {
            // Spec startup probe (transition T1): Type-0 READ of the Direct
            // Parameter page MinCycleTime octet. The device answers OD + CKS and
            // moves to PREOPERATE.
            (
                encode_type0(mc(true, CHANNEL_PAGE, DPP1_OFF_MIN_CYCLE_TIME)),
                IolinkFrameKind::Idle,
            )
        } else if self.step == 2 {
            // Spec transition to OPERATE: Type-0 WRITE of MasterCommand
            // DeviceOperate (0x99) to Direct Parameter page address 0. The device
            // answers with the CKS octet alone and moves to ESTAB_COM; the first
            // cyclic frame then completes the move to OPERATE.
            (
                encode_type0_write(mc(false, CHANNEL_PAGE, 0x00), &[0x99]),
                IolinkFrameKind::OperateReq,
            )
        } else {
            self.link_state = IolinkLinkState::Operate;
            // A pending event readout (Table 59) takes precedence over cyclic
            // process data: read StatusCode, the event slots, then write
            // StatusCode to clear the Event flag.
            if let Some((mc, is_write)) = self.event_readout.pop_front() {
                let frame = if is_write {
                    encode_type0_write(mc, &[0x00])
                } else {
                    encode_type0(mc)
                };
                (frame, IolinkFrameKind::EventReadout)
            } else {
                (encode_type1_cycle(&[]), IolinkFrameKind::Cyclic) // cyclic Type 1
            }
        };

        let pd_out: Vec<u8> = Vec::new(); // DI device: master sends no PD out
        for &b in &frame {
            self.tx_queue.push_back(b);
        }
        self.current = Some(PendingXfer {
            seq: self.frame_seq,
            kind,
            pd_out,
            link_state: self.link_state,
            raw_master: frame,
            raw_device: Vec::new(),
        });
        self.frame_seq = self.frame_seq.wrapping_add(1);

        // Hold `step` at the first cyclic index so it keeps repeating Type 1.
        if self.step <= 2 {
            self.step += 1;
        }
    }
}

impl UartStreamDevice for IolinkMaster {
    /// The C/Q line carries binary IO-Link M-sequences, not console text. This
    /// used to be asserted by a downcast inside `attach_uart_tx_sink`; stating
    /// it here lets every UART model apply the same rule.
    fn carries_protocol_octets(&self) -> bool {
        true
    }

    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        if let Some(byte) = self.tx_queue.pop_front() {
            return Some(byte);
        }
        // Frame fully sent: wait the inter-frame gap, then queue the next one.
        self.gap_ticks = self.gap_ticks.saturating_add(1);
        if self.gap_ticks < self.frame_gap_ticks {
            return None;
        }
        self.gap_ticks = 0;
        self.queue_next_frame();
        self.tx_queue.pop_front()
    }

    fn on_tx_byte(&mut self, byte: u8) {
        // Accumulate the device's reply to the current frame. Once a cyclic
        // (OPERATE) response is complete, decode and latch the process data.
        if self.rx_accum.len() < 64 {
            self.rx_accum.push(byte);
        }
        if let Some(p) = self.current.as_mut() {
            if p.raw_device.len() < 64 {
                p.raw_device.push(byte);
            }
        }
        if self.link_state == IolinkLinkState::Operate
            && self
                .current
                .as_ref()
                .is_some_and(|p| matches!(p.kind, IolinkFrameKind::Cyclic))
            && self.rx_accum.len() >= self.operate_response_len()
        {
            let n = self.operate_response_len();
            let resp = decode_operate(&self.rx_accum[..n], self.pd_in_len, self.od_len);
            if resp.checksum_ok && resp.pd_valid {
                // Log only when the verdict changed, so the capture stays
                // readable (one line per distinct verdict the master received).
                if resp.pd != self.latest_pd {
                    self.log_received_pd(&resp.pd);
                }
                self.latest_pd = resp.pd;
                self.pd_valid = true;
            }
            // The CKS Event flag (A.1.5) is the device's initiative to have the
            // master retrieve the event memory over the diagnosis channel.
            // Surface it once per rising edge and start the readout (Table 59).
            if resp.checksum_ok {
                if resp.event_present && !self.event_latched {
                    self.log_line("MASTER EVENT pending (device diagnostic event)");
                    self.event_readout = event_readout_plan().into_iter().collect();
                }
                self.event_latched = resp.event_present;
            }
            self.rx_accum.clear();
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct IolinkMasterKit;
pub static IOLINK_MASTER_KIT: IolinkMasterKit = IolinkMasterKit;

static IOLINK_MASTER_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(&[]),
    device_type: std::borrow::Cow::Borrowed("iolink-master"),
    label: std::borrow::Cow::Borrowed("IO-Link Master"),
    summary: std::borrow::Cow::Borrowed("IO-Link master state machine over UART."),
    detail: std::borrow::Cow::Borrowed("Drives wake-up / startup / operate cycles, m-sequence types, process-data \
             exchange. The IO-Link DI/DO device demo uses this to host two digital-input channels."),
    transport: Transport::Uart,
    category: Category::Uart,
    config_keys: std::borrow::Cow::Borrowed(&[
        ConfigKey {
            name: std::borrow::Cow::Borrowed("pd_in_len"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Process-data input length in bytes. Defaults to 1 (single-byte DI device)."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("m_seq_type"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("M-sequence type (1..6). Used to derive od_len: one OD octet, or two for TYPE_2_V (Table A.10)."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("com"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("Communication speed: \"COM1\" (4.8 kbaud), \"COM2\" (38.4 kbaud, default), or \"COM3\" (230.4 kbaud)."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("frame_gap_ticks"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Inter-frame gap in UART ticks (default 6000). A faster -O2 device can use a small gap so many cyclic reads fit the step budget."),
        },
    ]),
    labs: std::borrow::Cow::Borrowed(&[LabRef {
        board_id: std::borrow::Cow::Borrowed("iolink-dido"),
        chip: std::borrow::Cow::Borrowed("stm32l476"),
        example_dir: std::borrow::Cow::Borrowed("iolink-dido"),
        demo_elf: std::borrow::Cow::Borrowed("demo-iolink-dido.elf"),
    }]),
};

impl PeripheralKit for IolinkMasterKit {
    fn metadata(&self) -> &'static KitMetadata {
        &IOLINK_MASTER_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let pd_in_len = ctx.config_i64("pd_in_len").unwrap_or(1) as usize;
        let m_seq_type = ctx.config_i64("m_seq_type").unwrap_or(1);
        /* Table A.10: only TYPE_2_V (6) carries two OD octets; every other
        M-sequence type carries one. */
        let od_len: usize = if m_seq_type == 6 { 2 } else { 1 };
        let com = match ctx
            .config_str("com")
            .unwrap_or("COM2")
            .to_ascii_uppercase()
            .as_str()
        {
            "COM1" => IolinkComSpeed::Com1,
            "COM3" => IolinkComSpeed::Com3,
            _ => IolinkComSpeed::Com2,
        };
        let frame_gap_ticks = ctx
            .config_i64("frame_gap_ticks")
            .map(|v| v.max(1) as u32)
            .unwrap_or(FRAME_GAP_TICKS);
        let uart = ctx.uart()?;
        uart.attach_stream(Box::new(IolinkMaster::new_with_gap(
            pd_in_len,
            od_len,
            com,
            frame_gap_ticks,
        )));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pump ticks and return the bytes of exactly the next frame: skip any
    /// leading inter-frame gap, collect the frame's bytes, stop at the next gap.
    fn drain(m: &mut IolinkMaster) -> Vec<u8> {
        let mut out = Vec::new();
        let mut started = false;
        for _ in 0..(FRAME_GAP_TICKS * 2 + 16) {
            match m.poll(1000) {
                Some(b) => {
                    out.push(b);
                    started = true;
                }
                None => {
                    if started {
                        break;
                    }
                }
            }
        }
        out
    }

    /// Vectors are derived from the A.1.6 formula independently of the Rust
    /// implementation (see the design doc C1 and the Python oracle).
    #[test]
    fn checksum6_matches_spec_vectors() {
        assert_eq!(checksum6(&[0x00, 0x00]), 0x2D);
        assert_eq!(checksum6(&[0xA2, 0x00]), 0x00);
        assert_eq!(checksum6(&[0x20, 0x00, 0x99]), 0x06);
        // TYPE_1 write: CKT type bits 0-7 included, checksum bits zeroed.
        assert_eq!(checksum6(&[0x00, 0x40, 0xA5, 0x5A]), 0x35);
        // TYPE_2 read.
        assert_eq!(checksum6(&[0x80, 0x80]), 0x2D);
        assert_eq!(checksum6(&[0x00, 0x00, 0x0A]), 0x2E);
    }

    #[test]
    fn encodes_type0_reads_with_ckt_checksum() {
        assert_eq!(encode_type0(0x00), vec![0x00, 0x2D]); // IDLE read
        assert_eq!(encode_type0(0x0F), vec![0x0F, 0x2D]);
        // Startup probe: R, PAGE, DPP MinCycleTime.
        assert_eq!(encode_type0(0xA2), vec![0xA2, 0x00]);
    }

    #[test]
    fn encodes_type0_device_operate_write() {
        // DeviceOperate (A.1.2 page write, MC=0x20) data 0x99, checksum 0x06.
        assert_eq!(checksum6(&[0x20, 0x00, 0x99]), 0x06);
        assert_eq!(encode_type0_write(0x20, &[0x99]), vec![0x20, 0x06, 0x99]);
    }

    #[test]
    fn encodes_type1_di_cycle_with_no_output_pd() {
        // A.2.3/A.1.6: CKT carries type bits 0x40 and ck6([00, 40, 00]) = 0x35.
        assert_eq!(encode_type1_cycle(&[]), vec![0x00, 0x75, 0x00]);
    }

    #[test]
    fn decodes_operate_response_and_extracts_pd() {
        // Reply is `[PD_in..., OD..., CKS]` with no leading status octet.
        let resp = decode_operate(&[0xA5, 0x00, 0x22], 1, 1);
        assert!(resp.checksum_ok);
        assert!(resp.pd_valid);
        assert!(!resp.event_present);
        assert_eq!(resp.pd, vec![0xA5]);
    }

    #[test]
    fn decode_operate_flags_event_and_pd_invalid_in_cks() {
        // PD valid, Event flag set: CKS = 0x80 | ck6([0xA5, 0x80]) = 0x8A.
        let ev = decode_operate(&[0xA5, 0x00, 0x8A], 1, 1);
        assert!(ev.checksum_ok);
        assert!(ev.pd_valid);
        assert!(ev.event_present);

        // PD invalid (CKS bit 6), no event: the PD-status bit participates in
        // the checksum, so CKS = 0x40 | ck6([0xA5, 0x40]) = 0x7A.
        let inv = decode_operate(&[0xA5, 0x00, 0x7A], 1, 1);
        assert!(inv.checksum_ok);
        assert!(!inv.pd_valid);
        assert!(!inv.event_present);
    }

    #[test]
    fn decode_operate_accepts_plan_reply_vectors() {
        // Plan oracle (A.1.6 formula, computed independently):
        //   reply `[OD=0x10] CKS` with no PD -> 10 39
        //   reply `[PD=0xA5] CKS` valid      -> A5 22
        //   reply `[PD=0xA5] CKS` + Event    -> A5 8A
        //   reply `[PD=0xA5] CKS` invalid    -> A5 7A
        // The invalid vector follows C1: Event and PD-status bits of CKS are
        // part of the checked message (only bits 0-5 are zeroed), so
        // CKS = 0x40 | ck6([0xA5, 0x40]) = 0x7A.
        let od = decode_operate(&[0x10, 0x39], 0, 1);
        assert!(od.checksum_ok);
        assert!(od.pd_valid);
        assert!(od.pd.is_empty());

        let valid = decode_operate(&[0xA5, 0x22], 1, 0);
        assert!(valid.checksum_ok);
        assert!(valid.pd_valid);
        assert_eq!(valid.pd, vec![0xA5]);

        let event = decode_operate(&[0xA5, 0x8A], 1, 0);
        assert!(event.checksum_ok);
        assert!(event.pd_valid);
        assert!(event.event_present);

        let invalid = decode_operate(&[0xA5, 0x7A], 1, 0);
        assert!(invalid.checksum_ok);
        assert!(!invalid.pd_valid);
        assert!(!invalid.event_present);
    }

    #[test]
    fn builds_isdu_read_requests_by_index_format() {
        // Vectors computed from the A.5 rules independently of the code.
        // 8-bit index, subindex 0: I-Service 0x9, length 3 -> 93 10 83.
        assert_eq!(isdu_read_request(0x10, None), vec![0x93, 0x10, 0x83]);
        assert_eq!(isdu_read_request(0x10, Some(0)), vec![0x93, 0x10, 0x83]);
        // 8-bit index + subindex: I-Service 0xA, length 4 -> A4 10 01 B5.
        assert_eq!(
            isdu_read_request(0x10, Some(1)),
            vec![0xA4, 0x10, 0x01, 0xB5]
        );
        // Index 0x25 is still in the 8-bit range (Table A.15).
        assert_eq!(isdu_read_request(0x0025, Some(0)), vec![0x93, 0x25, 0xB6]);
        // 16-bit index + subindex: I-Service 0xB, length 5 -> B5 01 23 04 93.
        assert_eq!(
            isdu_read_request(0x0123, Some(4)),
            vec![0xB5, 0x01, 0x23, 0x04, 0x93]
        );
    }

    #[test]
    fn isdu_request_segments_use_flowctrl_start_then_count() {
        // A read of index 0x10 is `93 10 83`; over TYPE_0 (one OD octet per
        // message) it becomes START, COUNT 1, COUNT 2 in the MC address.
        let isdu = isdu_read_request(0x10, None);
        let segments = isdu_flowctrl_segments(&isdu, 1);
        assert_eq!(
            segments,
            vec![
                (FLOWCTRL_START, vec![0x93]),
                (1, vec![0x10]),
                (2, vec![0x83]),
            ]
        );
        // MC = W(0), channel ISDU (0x60) | FlowCTRL.
        assert_eq!(mc(false, CHANNEL_ISDU, FLOWCTRL_START), 0x70);
        assert_eq!(mc(false, CHANNEL_ISDU, 1), 0x61);
        assert_eq!(mc(false, CHANNEL_ISDU, 2), 0x62);
        // The poll is R(1), channel ISDU: START, COUNT and IDLE/ABORT.
        assert_eq!(mc(true, CHANNEL_ISDU, FLOWCTRL_START), 0xF0);
        assert_eq!(mc(true, CHANNEL_ISDU, 1), 0xE1);
        assert_eq!(mc(true, CHANNEL_ISDU, FLOWCTRL_IDLE), 0xF1);
        assert_eq!(mc(true, CHANNEL_ISDU, FLOWCTRL_ABORT), 0xFF);
    }

    #[test]
    fn encode_type0_write_puts_checksum_in_ckt() {
        // Figure A.5: `[MC=0x70, CKT=ck6, OD=0x93]`, no trailing checksum octet.
        let frame = encode_type0_write(0x70, &[0x93]);
        assert_eq!(frame.len(), 3);
        assert_eq!(frame, vec![0x70, checksum6(&[0x70, 0x00, 0x93]), 0x93]);
    }

    #[test]
    fn diagnosis_channel_event_readout_mcs_match_table_58() {
        // R, DIAGNOSIS, address 0 = 0xC0; W, DIAGNOSIS, 0 = 0x40.
        assert_eq!(diagnosis_read_mc(0x00), 0xC0);
        assert_eq!(diagnosis_write_mc(0x00), 0x40);
        let mcs = event_readout_mcs();
        // StatusCode + the six 3-octet event slots (addresses 0..=0x12),
        // then the confirmation write at address 0.
        assert_eq!(mcs.len(), 0x13 + 1);
        assert_eq!(mcs[0], 0xC0);
        assert_eq!(mcs[0x12], 0xD2);
        assert_eq!(*mcs.last().unwrap(), 0x40);
    }

    #[test]
    fn finalize_cyclic_decodes_response_and_marks_ck() {
        let m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        let resp = [0xA5u8, 0x00, 0x22];
        let p = PendingXfer {
            seq: 7,
            kind: IolinkFrameKind::Cyclic,
            pd_out: vec![],
            link_state: IolinkLinkState::Operate,
            raw_master: encode_type1_cycle(&[]),
            raw_device: resp.to_vec(),
        };
        let x = m.finalize_xfer(p);
        assert_eq!(x.seq, 7);
        assert_eq!(x.kind, IolinkFrameKind::Cyclic);
        assert_eq!(x.pd_in, vec![0xA5]);
        assert_eq!(x.ck_ok, Some(true));
        assert_eq!(x.pd_valid, Some(true));
    }

    #[test]
    fn finalize_startup_frame_has_no_crc_verdict() {
        let m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        let p = PendingXfer {
            seq: 0,
            kind: IolinkFrameKind::WakeUp,
            pd_out: vec![],
            link_state: IolinkLinkState::Startup,
            raw_master: vec![0x55],
            raw_device: vec![],
        };
        let x = m.finalize_xfer(p);
        assert_eq!(x.ck_ok, None);
        assert_eq!(x.pd_valid, None);
        assert!(x.pd_in.is_empty());
    }

    #[test]
    fn finalize_incomplete_cyclic_frame_has_no_crc_verdict() {
        let m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        let p = PendingXfer {
            seq: 1,
            kind: IolinkFrameKind::Cyclic,
            pd_out: vec![0],
            link_state: IolinkLinkState::Operate,
            raw_master: encode_type1_cycle(&[0]),
            raw_device: vec![0x20],
        };
        let x = m.finalize_xfer(p);
        assert_eq!(x.ck_ok, None);
        assert_eq!(x.pd_valid, Some(false));
        assert!(x.pd_in.is_empty());
    }

    #[test]
    fn decode_operate_handles_two_byte_pd() {
        let mut frame = vec![0xAAu8, 0xBB, 0x00];
        let ck = checksum6(&frame);
        frame.push(ck);
        let resp = decode_operate(&frame, 2, 1);
        assert!(resp.checksum_ok);
        assert!(resp.pd_valid);
        assert_eq!(resp.pd, vec![0xAA, 0xBB]);
    }

    #[test]
    fn schedule_walks_wakeup_probe_operate_write_then_cyclic_type1() {
        let mut m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);

        // Step 0: wake-up pulse.
        assert_eq!(drain(&mut m), vec![0x55]);
        assert_eq!(m.link_state, IolinkLinkState::Startup);

        // Step 1: startup probe — Type-0 READ of the DPP MinCycleTime octet.
        assert_eq!(drain(&mut m), vec![0xA2, 0x00]);
        assert_eq!(m.link_state, IolinkLinkState::Startup);

        // Step 2: DeviceOperate Type-0 WRITE to the page channel.
        assert_eq!(drain(&mut m), vec![0x20, 0x06, 0x99]);
        assert_eq!(m.link_state, IolinkLinkState::Startup);

        // Then cyclic Type 1 requests, repeating forever.
        assert_eq!(drain(&mut m), vec![0x00, 0x75, 0x00]);
        assert_eq!(m.link_state, IolinkLinkState::Operate);
        assert_eq!(drain(&mut m), vec![0x00, 0x75, 0x00]);
    }

    #[test]
    fn trace_ring_captures_startup_then_cyclic() {
        let mut m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        for _ in 0..(FRAME_GAP_TICKS as u64 * 10 + 64) {
            let _ = m.poll(1000);
        }
        let trace = m.trace_snapshot();
        assert!(
            trace.len() >= 5,
            "expected several frames, got {}",
            trace.len()
        );
        assert_eq!(trace[0].kind, IolinkFrameKind::WakeUp);
        assert!(
            trace
                .iter()
                .any(|x| x.kind == IolinkFrameKind::Cyclic
                    && x.link_state == IolinkLinkState::Operate),
            "expected a cyclic OPERATE frame in the trace"
        );
        for w in trace.windows(2) {
            assert!(w[1].seq > w[0].seq);
        }
    }

    #[test]
    fn trace_clear_empties_ring() {
        let mut m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        for _ in 0..(FRAME_GAP_TICKS as u64 * 3 + 16) {
            let _ = m.poll(1000);
        }
        assert!(!m.trace_snapshot().is_empty());
        m.trace_clear();
        assert!(m.trace_snapshot().is_empty());
    }

    #[test]
    fn decode_operate_surfaces_event_bit() {
        // CKS with the Event flag (0x80): PD valid, event present.
        let mut frame = vec![0xAAu8, 0x00];
        let mut cks = 0x80 | checksum6(&[0xAA, 0x80]);
        frame.push(cks);
        let resp = decode_operate(&frame, 1, 1);
        assert!(resp.checksum_ok);
        assert!(resp.pd_valid);
        assert!(resp.event_present, "EVENT flag (0x80) must be decoded");

        // PD valid, no event.
        cks = checksum6(&[0xAA, 0x00]);
        let f2 = vec![0xAAu8, 0x00, cks];
        let r2 = decode_operate(&f2, 1, 1);
        assert!(!r2.event_present);
    }

    #[test]
    fn master_logs_decoded_thermal_verdict_and_event() {
        // A 9-byte thermal-fingerprint PD frame: a FAULT/OVERTEMP verdict.
        // [temp][temp][rate][rate][state=03][health=00][ttl][ttl][fault<<4|flags]
        // fault=1 (OVERTEMP) in the high nibble of the last byte.
        let pd = [0x1Cu8, 0xC5, 0x00, 0xBB, 0x03, 0x00, 0xFF, 0xFF, 0x17];
        let mut frame = Vec::new();
        frame.extend_from_slice(&pd);
        frame.push(0x00); // OD
                          // CKS with the Event flag set (PD valid): bit 7 plus the checksum.
        frame.push(0x80 | checksum6(&[pd.as_slice(), &[0x00, 0x80]].concat()));

        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut m = IolinkMaster::new(9, 1, IolinkComSpeed::Com2);
        m.set_log_sink(sink.clone());
        while m.link_state != IolinkLinkState::Operate {
            drain(&mut m);
        }
        for b in frame {
            m.on_tx_byte(b);
        }
        let log = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
        assert!(
            log.contains("MASTER PD=1CC500BB0300FFFF17"),
            "raw PD logged: {log}"
        );
        assert!(
            log.contains("MASTER VERDICT state=FAULT health=0 fault=OVERTEMP"),
            "decoded verdict logged: {log}"
        );
        assert!(log.contains("MASTER EVENT"), "event surfaced: {log}");
    }

    #[test]
    fn event_flag_starts_diagnosis_channel_readout() {
        let mut m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        while m.link_state != IolinkLinkState::Operate {
            drain(&mut m);
        }
        // The device raises the CKS Event flag on a cyclic reply.
        let mut mcs = Vec::new();
        for b in [0xA5u8, 0x00, 0x80 | checksum6(&[0xA5, 0x80])] {
            m.on_tx_byte(b);
        }
        // The next frames are the Table 59 readout: StatusCode 0xC0, the event
        // slots 0xC1..=0xD2, then the StatusCode confirmation write 0x40.
        for _ in 0..event_readout_plan().len() {
            let frame = drain(&mut m);
            mcs.push(frame[0]);
        }
        assert_eq!(mcs, event_readout_mcs());
    }

    #[test]
    fn frame_gap_override_paces_faster() {
        // With a small configured gap, a full frame + the inter-frame wait
        // completes in far fewer ticks than the 6000-tick default — proving the
        // per-device override drives the master's pacing.
        let mut m = IolinkMaster::new_with_gap(1, 1, IolinkComSpeed::Com2, 8);
        let mut ticks = 0u32;
        let mut frames = 0u32;
        let mut prev_was_none = true;
        for _ in 0..200 {
            ticks += 1;
            match m.poll(1000) {
                Some(_) => {
                    if prev_was_none {
                        frames += 1;
                    }
                    prev_was_none = false;
                }
                None => prev_was_none = true,
            }
            if frames >= 3 {
                break;
            }
        }
        assert!(frames >= 3, "expected several frames quickly, got {frames}");
        assert!(
            ticks < 100,
            "small gap should reach 3 frames in <100 ticks, took {ticks}"
        );
    }

    #[test]
    fn captures_process_data_from_cyclic_response() {
        let mut m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        // Advance the schedule to the cyclic (OPERATE) phase.
        while m.link_state != IolinkLinkState::Operate {
            drain(&mut m);
        }
        // Device replies to the cyclic request with PD = 0xA5, valid.
        for b in [0xA5u8, 0x00, 0x22] {
            m.on_tx_byte(b);
        }
        assert_eq!(m.input_byte(), 0xA5);
        assert!(m.pd_valid);
    }
}
