// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::components::tiol112::Tiol112;
use crate::peripherals::uart::UartStreamDevice;
use crate::sim_input::{InputChannel, SimInput, SimInputError};
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

/// Legacy LabWired demo envelope. Retained for the shipped DI/thermal fixtures;
/// this mixed PD/OD shape is not the normative TYPE_1_1 interleaved protocol.
pub(crate) fn encode_type1_cycle(pd_out: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x00u8, 0x40];
    frame.extend_from_slice(pd_out);
    frame.push(0x00); // OD (1-byte, idle)
    frame[1] |= checksum6(&frame);
    frame
}

/// TYPE_2_x master message (IO-Link v1.1.4 A.2.4, Figure A.14): MC, CKT,
/// PD-out, then OD for a WRITE. A READ omits the OD octets. The checksum is
/// carried in CKT, with the TYPE_2 bits included in its calculation.
pub(crate) fn encode_type2(mc: u8, pd_out: &[u8], od: &[u8]) -> Vec<u8> {
    let mut frame = vec![mc, 0x80];
    frame.extend_from_slice(pd_out);
    if mc & 0x80 == 0 {
        frame.extend_from_slice(od);
    }
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

/// Decode a cyclic OPERATE reply `[PD_in..., CKS]` (Figures A.9/A.10/A.13/A.14:
/// a WRITE M-sequence is answered without an OD octet, and the kit's cyclic
/// frames are writes, so the reply carries process data only). There is no
/// leading status octet: the checksum is over every data octet and CKS with
/// its checksum bits (0-5) zeroed, the PD status is CKS bit 6 (1 = invalid)
/// and the Event flag is CKS bit 7.
#[cfg(test)]
pub(crate) fn decode_operate(data: &[u8], pd_in_len: usize) -> OperateResponse {
    decode_operate_with_od(data, pd_in_len, 0)
}

/// READ replies include OD before PD; WRITE replies do not. Checksum covers
/// both, while only the configured PD slice reaches the application.
fn decode_operate_with_od(data: &[u8], pd_in_len: usize, od_len: usize) -> OperateResponse {
    if data.len() != od_len + pd_in_len + 1 {
        return OperateResponse {
            pd: Vec::new(),
            pd_valid: false,
            checksum_ok: false,
            event_present: false,
        };
    }
    let pd = data[od_len..od_len + pd_in_len].to_vec();
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
    /// OD octets before PD in a TYPE_2 read reply; zero for writes.
    response_od_len: usize,
}

/// Deliberate bring-up for the 32-in/1-out OD1 evaluation fixture. This is a
/// bounded application profile, not complete IO-Link system management.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvaluationPhase {
    Armed,
    Wake,
    MinCycle,
    Capability,
    PdIn,
    PdOut,
    OperateCommand,
    Operate,
    Fault,
}

#[derive(Debug)]
struct Evaluation {
    phase: EvaluationPhase,
    now_us: u64,
    not_before_us: u64,
    next_byte_us: u64,
    last_start_us: u64,
    reply_deadline_us: u64,
    awaiting_reply: bool,
    cycle_us: u64,
    read_next: bool,
    start_pressed: bool,
    error: Option<String>,
}

impl Evaluation {
    fn armed(com: IolinkComSpeed) -> Self {
        Self {
            phase: EvaluationPhase::Armed,
            now_us: 0,
            not_before_us: 0,
            next_byte_us: 0,
            last_start_us: 0,
            reply_deadline_us: 0,
            awaiting_reply: false,
            cycle_us: if com == IolinkComSpeed::Com3 {
                3_000
            } else {
                15_000
            },
            read_next: false,
            start_pressed: false,
            error: None,
        }
    }
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
/// DeviceOperate MasterCommand (→ ESTAB_COM) → configured cyclic requests.
/// This fixture does not negotiate the full device identity, schedule ISDUs,
/// enforce baud timing, or implement a complete conformant master state machine.
/// The optional native C bridge is a separate API and does not drive this UART.
#[derive(Debug, serde::Serialize)]
pub struct IolinkMaster {
    #[serde(skip)]
    component_id: String,
    #[serde(skip)]
    evaluation: Option<Evaluation>,
    #[serde(skip)]
    wake_phy: Option<Tiol112>,
    #[serde(skip)]
    evaluation_log: Vec<String>,
    pd_in_len: usize,
    pd_out: Vec<u8>,
    pd_out_counter: bool,
    m_seq_type: u8,
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
        Self::build(pd_in_len, 0, 1, od_len, com, frame_gap_ticks.max(1))
    }

    /// Explicit, bounded configuration. Engine enum 6 is TYPE_2_V; it is not
    /// the DPP OPERATE capability code (4/5/6/7 for OD 1/2/8/32 on long PD).
    /// Enum 1 preserves the historical receive-only LabWired demo envelope.
    /// Enums 4/5 provide TYPE_2_1 / TYPE_2_2, and other enums are unsupported.
    pub fn new_configured(
        pd_in_len: usize,
        pd_out_len: usize,
        m_seq_type: u8,
        od_len: usize,
        com: IolinkComSpeed,
        frame_gap_ticks: u32,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            pd_in_len <= 32 && pd_out_len <= 32,
            "IO-Link PD lengths must be 0..32 bytes"
        );
        anyhow::ensure!(
            matches!(od_len, 1 | 2 | 8 | 32),
            "IO-Link od_len must be 1, 2, 8, or 32"
        );
        anyhow::ensure!(
            frame_gap_ticks > 0,
            "IO-Link frame_gap_ticks must be 1..4294967295"
        );
        match m_seq_type {
            1 => anyhow::ensure!(pd_out_len == 0 && od_len == 1, "Legacy demo mode 1 supports pd_out_len=0 and od_len=1 only"),
            4 => anyhow::ensure!(pd_in_len == 1 && pd_out_len == 0 && od_len == 1, "TYPE_2_1 (enum 4) requires PDin=1, PDout=0, OD=1"),
            5 => anyhow::ensure!(pd_in_len == 2 && pd_out_len == 0 && od_len == 1, "TYPE_2_2 (enum 5) requires PDin=2, PDout=0, OD=1"),
            6 => {
                anyhow::ensure!(pd_in_len + pd_out_len > 0, "TYPE_2_V requires nonzero process data");
                if od_len == 1 {
                    // Table A.10: code 4 long-PD forms, plus code 0's former
                    // TYPE_2_6 forms (2 input + 1/2 output, or vice versa).
                    anyhow::ensure!(pd_in_len >= 3 || pd_out_len >= 3
                        || (pd_in_len == 2 && pd_out_len >= 1)
                        || (pd_out_len == 2 && pd_in_len >= 1),
                        "TYPE_2_V OD1 requires long PD or a bidirectional 2-byte direction (Table A.10)");
                }
            }
            _ => anyhow::bail!("Unsupported IO-Link m_seq_type {m_seq_type}; supported engine enums: 1 (legacy demo), 4, 5, 6"),
        }
        Ok(Self::build(
            pd_in_len,
            pd_out_len,
            m_seq_type,
            od_len,
            com,
            frame_gap_ticks,
        ))
    }

    fn build(
        pd_in_len: usize,
        pd_out_len: usize,
        m_seq_type: u8,
        od_len: usize,
        com: IolinkComSpeed,
        frame_gap_ticks: u32,
    ) -> Self {
        let mut m = Self {
            component_id: String::new(),
            evaluation: None,
            wake_phy: None,
            evaluation_log: Vec::new(),
            pd_in_len,
            pd_out: vec![0; pd_out_len],
            pd_out_counter: false,
            m_seq_type,
            od_len,
            com,
            link_state: IolinkLinkState::Startup,
            tx_queue: VecDeque::new(),
            rx_accum: Vec::new(),
            step: 0,
            gap_ticks: 0,
            latest_pd: vec![0u8; pd_in_len],
            pd_valid: false,
            trace: VecDeque::new(),
            current: None,
            frame_seq: 0,
            log_sink: None,
            event_latched: false,
            event_readout: VecDeque::new(),
            frame_gap_ticks: frame_gap_ticks.max(1),
        };
        m.queue_next_frame(); // queue the wake-up immediately
        m
    }

    /// Update the selector/output for subsequently queued frames. An already
    /// queued frame and its trace retain the original snapshot.
    pub fn set_process_data_output(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(
            bytes.len() == self.pd_out.len(),
            "PD output must contain exactly {} bytes",
            self.pd_out.len()
        );
        self.pd_out.copy_from_slice(bytes);
        Ok(())
    }

    pub fn process_data_input(&self) -> &[u8] {
        &self.latest_pd
    }

    fn enable_evaluation_profile(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (
                self.pd_in_len,
                self.pd_out.len(),
                self.m_seq_type,
                self.od_len
            ) == (32, 1, 6, 1)
                && matches!(self.com, IolinkComSpeed::Com2 | IolinkComSpeed::Com3),
            "tiol112-eval requires PDin32/PDout1/TYPE_2_V/OD1 and COM2 or COM3"
        );
        self.tx_queue.clear();
        self.current = None;
        self.rx_accum.clear();
        self.frame_seq = 0;
        self.evaluation = Some(Evaluation::armed(self.com));
        Ok(())
    }

    /// One explicit action: assert the PHY's logical WAKE for 1 ms, release,
    /// then perform the documented DPP/DeviceOperate sequence. This does not
    /// simulate the physical master wake current or silently start on attach.
    pub fn start_evaluation(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.evaluation.is_some(),
            "Start requires startup_profile: tiol112-eval"
        );
        let phy = self
            .wake_phy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("tiol112-eval requires a TIOL112 on the same UART"))?;
        phy.set_wake_asserted(true);
        let e = self.evaluation.as_mut().unwrap();
        e.phase = EvaluationPhase::Wake;
        e.not_before_us = e.now_us + 1_000;
        e.awaiting_reply = false;
        e.read_next = false;
        e.error = None;
        self.tx_queue.clear();
        self.current = None;
        self.rx_accum.clear();
        self.event_readout.clear();
        self.link_state = IolinkLinkState::Startup;
        self.pd_valid = false;
        self.latest_pd.fill(0);
        self.evaluation_message("START: logical WAKE asserted for 1000 us");
        Ok(())
    }

    pub fn evaluation_error(&self) -> Option<&str> {
        self.evaluation.as_ref().and_then(|e| e.error.as_deref())
    }

    fn evaluation_message(&mut self, message: &str) {
        self.log_line(&format!("MASTER {message}"));
        if self.evaluation_log.len() >= TRACE_CAP {
            self.evaluation_log.remove(0);
        }
        self.evaluation_log.push(message.to_string());
    }

    fn character_us(&self) -> u64 {
        let baud = match self.com {
            IolinkComSpeed::Com1 => 4_800,
            IolinkComSpeed::Com2 => 38_400,
            IolinkComSpeed::Com3 => 230_400,
        };
        11_000_000u64.div_ceil(baud)
    }

    /// Optional demo stimulus: increment only the first output octet after
    /// each queued OPERATE frame, wrapping at 255. Disabled by default.
    pub fn set_process_data_output_counter(&mut self, enabled: bool) -> anyhow::Result<()> {
        anyhow::ensure!(
            !enabled || !self.pd_out.is_empty(),
            "PD output counter requires pd_out_len >= 1"
        );
        self.pd_out_counter = enabled;
        Ok(())
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
        "hand-rolled"
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
        self.pd_in_len + 1 + self.current.as_ref().map_or(0, |p| p.response_od_len)
    }

    fn complete_current(&mut self) {
        if let Some(p) = self.current.take() {
            let x = self.finalize_xfer(p);
            if self.trace.len() >= TRACE_CAP {
                self.trace.pop_front();
            }
            self.trace.push_back(x);
        }
        self.rx_accum.clear();
    }

    fn validate_evaluation_reply(&self, e: &mut Evaluation) -> anyhow::Result<()> {
        let p = self
            .current
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing pending request"))?;
        let expected = if e.phase == EvaluationPhase::Operate {
            self.pd_in_len + p.response_od_len + 1
        } else if e.phase == EvaluationPhase::OperateCommand {
            1
        } else {
            2
        };
        anyhow::ensure!(
            p.raw_device.len() == expected,
            "reply length {} != {expected}",
            p.raw_device.len()
        );
        let mut checked = p.raw_device.clone();
        *checked.last_mut().unwrap() &= 0xC0;
        anyhow::ensure!(
            checksum6(&checked) == (p.raw_device[expected - 1] & 0x3F),
            "reply checksum failed"
        );
        match e.phase {
            EvaluationPhase::MinCycle => {
                let raw = p.raw_device[0];
                let required = if self.com == IolinkComSpeed::Com3 {
                    0x1E
                } else {
                    0x56
                };
                anyhow::ensure!(raw == required, "evaluation firmware MinCycleTime {raw:#04x} does not match required {required:#04x}");
                let value = u64::from(raw & 0x3F);
                let minimum = match raw >> 6 {
                    0 => value * 100,
                    1 => 6_400 + value * 400,
                    2 => 32_000 + value * 1_600,
                    _ => anyhow::bail!("reserved MinCycleTime encoding {raw:#04x}"),
                };
                anyhow::ensure!(minimum >= 400, "invalid MinCycleTime {raw:#04x}");
                e.cycle_us = e.cycle_us.max(minimum);
                e.phase = EvaluationPhase::Capability;
            }
            EvaluationPhase::Capability => {
                let code = (p.raw_device[0] >> 1) & 7;
                anyhow::ensure!(
                    code == 4,
                    "device OPERATE capability code {code} does not match OD1 TYPE_2_V code4"
                );
                e.phase = EvaluationPhase::PdIn;
            }
            EvaluationPhase::PdIn => {
                anyhow::ensure!(
                    p.raw_device[0] == 0x9F,
                    "device PDin description must be 32 bytes (0x9F)"
                );
                e.phase = EvaluationPhase::PdOut;
            }
            EvaluationPhase::PdOut => {
                anyhow::ensure!(
                    p.raw_device[0] == 0x08,
                    "device PDout description must be 8 bits (0x08)"
                );
                e.phase = EvaluationPhase::OperateCommand;
            }
            EvaluationPhase::OperateCommand => e.phase = EvaluationPhase::Operate,
            EvaluationPhase::Operate => {
                anyhow::ensure!(
                    p.raw_device[expected - 1] & 0x40 == 0,
                    "device marks process data invalid"
                );
            }
            _ => anyhow::bail!("unexpected startup phase"),
        }
        Ok(())
    }

    fn queue_evaluation_frame(&mut self, e: &mut Evaluation) {
        self.complete_current();
        let (frame, kind) = match e.phase {
            EvaluationPhase::MinCycle => (encode_type0(0xA2), IolinkFrameKind::Idle),
            EvaluationPhase::Capability => (encode_type0(0xA3), IolinkFrameKind::Idle),
            EvaluationPhase::PdIn => (encode_type0(0xA5), IolinkFrameKind::Idle),
            EvaluationPhase::PdOut => (encode_type0(0xA6), IolinkFrameKind::Idle),
            EvaluationPhase::OperateCommand => (
                encode_type0_write(0x20, &[0x99]),
                IolinkFrameKind::OperateReq,
            ),
            EvaluationPhase::Operate => {
                self.link_state = IolinkLinkState::Operate;
                // ISDU IDLE while exchanging cyclic PD. No service is in
                // progress in this profile; IDLE must not interrupt one.
                let control = mc(e.read_next, CHANNEL_ISDU, FLOWCTRL_IDLE);
                e.read_next = !e.read_next;
                (
                    encode_type2(control, &self.pd_out, &[0]),
                    IolinkFrameKind::Cyclic,
                )
            }
            _ => return,
        };
        let cyclic = e.phase == EvaluationPhase::Operate;
        let pd_out = if cyclic {
            self.pd_out.clone()
        } else {
            Vec::new()
        };
        let response_od_len = if cyclic && frame[0] & 0x80 != 0 {
            self.od_len
        } else {
            0
        };
        self.tx_queue.extend(frame.iter().copied());
        self.current = Some(PendingXfer {
            seq: self.frame_seq,
            kind,
            pd_out,
            link_state: self.link_state,
            raw_master: frame,
            raw_device: Vec::new(),
            response_od_len,
        });
        self.frame_seq = self.frame_seq.wrapping_add(1);
        if cyclic && self.pd_out_counter {
            self.pd_out[0] = self.pd_out[0].wrapping_add(1);
        }
        e.last_start_us = e.now_us;
        e.next_byte_us = e.now_us;
        e.reply_deadline_us = e.now_us + 50_000u64.max(e.cycle_us * 2);
        e.awaiting_reply = true;
    }

    fn poll_evaluation(&mut self, elapsed_us: u32) -> Option<u8> {
        let mut e = self.evaluation.take().unwrap();
        e.now_us = e.now_us.saturating_add(u64::from(elapsed_us));
        let character_us = self.character_us();
        let mut result = None;
        if e.phase == EvaluationPhase::Wake && e.now_us >= e.not_before_us {
            self.wake_phy.as_ref().unwrap().set_wake_asserted(false);
            self.evaluation_message(
                "WAKE released; checking DPP MinCycleTime/capability/PD lengths",
            );
            e.phase = EvaluationPhase::MinCycle;
            e.not_before_us = e.now_us + 100;
        }
        if !matches!(
            e.phase,
            EvaluationPhase::Armed | EvaluationPhase::Wake | EvaluationPhase::Fault
        ) && e.now_us >= e.not_before_us
        {
            if self.tx_queue.is_empty() && e.awaiting_reply {
                let n = if e.phase == EvaluationPhase::Operate {
                    self.operate_response_len()
                } else if e.phase == EvaluationPhase::OperateCommand {
                    1
                } else {
                    2
                };
                let received = self.current.as_ref().map_or(0, |p| p.raw_device.len());
                let previous_phase = e.phase;
                let verdict = if received >= n {
                    Some(self.validate_evaluation_reply(&mut e))
                } else if e.now_us >= e.reply_deadline_us {
                    Some(Err(anyhow::anyhow!(
                        "reply timeout in {previous_phase:?}: received {received}/{n} bytes"
                    )))
                } else {
                    None
                };
                if let Some(verdict) = verdict {
                    match verdict {
                        Ok(()) => {
                            e.awaiting_reply = false;
                            // TYPE_0 recovery: at least100 Tbit between master
                            // request starts (v1.1.4 Tables A.7/A.8, A.3.9).
                            let recovery = if self.com == IolinkComSpeed::Com3 {
                                435
                            } else {
                                2_605
                            };
                            let period = if previous_phase == EvaluationPhase::Operate {
                                e.cycle_us
                            } else {
                                recovery
                            };
                            e.not_before_us =
                                (e.last_start_us + period).max(e.now_us + character_us);
                            if previous_phase == EvaluationPhase::OperateCommand {
                                self.evaluation_message(&format!(
                                    "DPP accepted; OPERATE cycle >= {} us",
                                    e.cycle_us
                                ));
                            }
                        }
                        Err(error) => {
                            let error = error.to_string();
                            self.evaluation_message(&format!(
                                "FAULT: {error}; reset the run before retrying"
                            ));
                            e.error = Some(error);
                            e.phase = EvaluationPhase::Fault;
                            e.awaiting_reply = false;
                            self.complete_current();
                        }
                    }
                }
            }
            if !e.awaiting_reply && e.phase != EvaluationPhase::Fault && e.now_us >= e.not_before_us
            {
                self.queue_evaluation_frame(&mut e);
            }
            if e.phase != EvaluationPhase::Fault && e.now_us >= e.next_byte_us {
                if let Some(byte) = self.tx_queue.pop_front() {
                    e.next_byte_us = e.now_us + character_us;
                    result = Some(byte);
                }
            }
        }
        self.evaluation = Some(e);
        result
    }

    /// Turn a completed in-flight frame into a trace record, decoding the
    /// device response only for cyclic (OPERATE) frames.
    fn finalize_xfer(&self, p: PendingXfer) -> IolinkXfer {
        let (pd_in, ck_ok, pd_valid) = if matches!(p.kind, IolinkFrameKind::Cyclic)
            || (self.m_seq_type >= 4 && matches!(p.kind, IolinkFrameKind::EventReadout))
        {
            let n = self.pd_in_len + p.response_od_len + 1;
            if p.raw_device.len() >= n {
                let r =
                    decode_operate_with_od(&p.raw_device[..n], self.pd_in_len, p.response_od_len);
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
        self.complete_current();

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
                let frame = if self.m_seq_type >= 4 {
                    encode_type2(mc, &self.pd_out, &vec![0; self.od_len])
                } else if is_write {
                    encode_type0_write(mc, &[0x00])
                } else {
                    encode_type0(mc)
                };
                (frame, IolinkFrameKind::EventReadout)
            } else {
                let frame = if self.m_seq_type >= 4 {
                    encode_type2(0x00, &self.pd_out, &vec![0; self.od_len])
                } else {
                    encode_type1_cycle(&[])
                };
                (frame, IolinkFrameKind::Cyclic)
            }
        };

        let is_type2_operate = self.m_seq_type >= 4 && self.link_state == IolinkLinkState::Operate;
        let pd_out = if is_type2_operate {
            self.pd_out.clone()
        } else {
            Vec::new()
        };
        let response_od_len = if is_type2_operate && frame[0] & 0x80 != 0 {
            self.od_len
        } else {
            0
        };
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
            response_od_len,
        });
        if is_type2_operate && self.pd_out_counter {
            self.pd_out[0] = self.pd_out[0].wrapping_add(1);
        }
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

    fn poll(&mut self, elapsed_us: u32) -> Option<u8> {
        if self.evaluation.is_some() {
            return self.poll_evaluation(elapsed_us);
        }
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
        if self.rx_accum.len() < 65 {
            self.rx_accum.push(byte);
        }
        if let Some(p) = self.current.as_mut() {
            if p.raw_device.len() < 65 {
                p.raw_device.push(byte);
            }
        }
        if self.link_state == IolinkLinkState::Operate
            && self.current.as_ref().is_some_and(|p| {
                matches!(p.kind, IolinkFrameKind::Cyclic)
                    || (self.m_seq_type >= 4 && matches!(p.kind, IolinkFrameKind::EventReadout))
            })
            && self.rx_accum.len() >= self.operate_response_len()
        {
            let n = self.operate_response_len();
            let od_len = self.current.as_ref().map_or(0, |p| p.response_od_len);
            let resp = decode_operate_with_od(&self.rx_accum[..n], self.pd_in_len, od_len);
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

    fn paced_by_device(&self) -> bool {
        self.evaluation.is_some()
    }

    fn declared_baud(&self) -> Option<u32> {
        self.evaluation.as_ref().map(|_| match self.com {
            IolinkComSpeed::Com1 => 4_800,
            IolinkComSpeed::Com2 => 38_400,
            IolinkComSpeed::Com3 => 230_400,
        })
    }

    fn device_id(&self) -> Option<&str> {
        (!self.component_id.is_empty()).then_some(self.component_id.as_str())
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }

    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        vec![crate::peripheral_log::PeripheralLog::new(
            "evaluation",
            self.evaluation_log.clone(),
        )]
    }
}

static MASTER_INPUTS: &[InputChannel] = &[InputChannel {
    key: std::borrow::Cow::Borrowed("start"),
    label: std::borrow::Cow::Borrowed("Start TIOL112 evaluation (logical WAKE then DPP)"),
    unit: std::borrow::Cow::Borrowed("bool"),
    min: 0.0,
    max: 1.0,
    default: Some(0.0),
}];

impl SimInput for IolinkMaster {
    fn input_channels(&self) -> &[InputChannel] {
        MASTER_INPUTS
    }
    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        let pressed = value >= 0.5;
        let e = self.evaluation.as_mut().ok_or_else(|| {
            SimInputError::Evaluation("Start requires startup_profile: tiol112-eval".into())
        })?;
        let rising = pressed && !e.start_pressed;
        e.start_pressed = pressed;
        if rising {
            self.start_evaluation()
                .map_err(|e| SimInputError::Evaluation(e.to_string()))?;
        }
        Ok(())
    }
    fn component_id(&self) -> Option<&str> {
        self.device_id()
    }
    fn set_component_id(&mut self, id: String) {
        self.component_id = id;
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct IolinkMasterKit;
pub static IOLINK_MASTER_KIT: IolinkMasterKit = IolinkMasterKit;

static IOLINK_MASTER_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(MASTER_INPUTS),
    device_type: std::borrow::Cow::Borrowed("iolink-master"),
    label: std::borrow::Cow::Borrowed("IO-Link Master"),
    summary: std::borrow::Cow::Borrowed("IO-Link master state machine over UART."),
    detail: std::borrow::Cow::Borrowed("Tick-paced IO-Link test peer with bounded process-data framing. \
             Supports TYPE_2_1/2/variable and the legacy LabWired receive-only demo envelope. \
             The opt-in tiol112-eval profile waits for explicit Start, schedules logical WAKE, \
             validates DPP, and alternates TYPE_2_V PD reads/writes at the advertised minimum cycle. \
             This peer does not prove physical baud timing, tA, master conformance, \
             ISDU service execution, or Beckhoff compatibility."),
    transport: Transport::Uart,
    category: Category::Uart,
    config_keys: std::borrow::Cow::Borrowed(&[
        ConfigKey {
            name: std::borrow::Cow::Borrowed("startup_profile"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("legacy (default) or tiol112-eval. Evaluation requires a TIOL112 attached earlier to the same UART, PDin32/PDout1/TYPE_2_V/OD1 and COM2/3. Explicit start input schedules a logical 1ms WAKE, validated DPP reads and DeviceOperate; cyclic ISDU-IDLE read/write PD follows. It does not execute ISDU services."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("pd_in_len"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Process-data input length, 0..32 bytes. Default 1. Must match the selected M-sequence."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("pd_out_len"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Process-data output length, 0..32 bytes. Default 0; initialized to zero and changeable through set_process_data_output."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("m_seq_type"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Engine enum: 1=legacy receive-only demo (default), 4=TYPE_2_1, 5=TYPE_2_2, 6=TYPE_2_V. This is NOT the DPP capability code. Other values are rejected."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("pd_out_pattern"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("Optional demo stimulus: constant (default) or counter. Counter increments the first PD output octet once per queued OPERATE frame; requires pd_out_len >= 1."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("od_len"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("On-request octets: TYPE_2_V permits 1, 2, 8, 32; other supported modes require 1. Default 2 for TYPE_2_V, otherwise 1. Long-PD TYPE_2_V OD1 corresponds to DPP capability code 4."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("com"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed("Communication speed: COM1, COM2 (default), or COM3. Legacy mode treats it as metadata; tiol112-eval paces bytes in simulated time and reports the baud to the UART guard. No physical timing claim."),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("frame_gap_ticks"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("Legacy inter-frame gap in UART polls, 1..4294967295 (default 6000). Not a cycle time in microseconds. Cannot be specified for tiol112-eval, which honors device MinCycleTime."),
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
        let mut master = configured_master(&ctx.ext.config)?;
        master.set_component_id(ctx.ext.id.clone());
        let uart = ctx.uart()?;
        if master.evaluation.is_some() {
            master.wake_phy = Some(uart.tiol112().cloned().ok_or_else(|| {
                anyhow::anyhow!(
                    "tiol112-eval requires a TIOL112 declared before the master on the same UART"
                )
            })?);
        }
        uart.attach_stream(Box::new(master));
        Ok(())
    }
}

/// Validate before narrowing signed YAML integers. Missing values have
/// defaults; malformed or out-of-range supplied values never silently do.
fn configured_master(
    config: &std::collections::HashMap<String, serde_yaml::Value>,
) -> anyhow::Result<IolinkMaster> {
    let integer = |key: &str, default: u64, max: u64| -> anyhow::Result<u64> {
        match config.get(key) {
            None => Ok(default),
            Some(value) => {
                let number = value
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("IO-Link {key} must be an unsigned integer"))?;
                anyhow::ensure!(number <= max, "IO-Link {key} must be <= {max}");
                Ok(number)
            }
        }
    };
    let pd_in_len = integer("pd_in_len", 1, 32)? as usize;
    let pd_out_len = integer("pd_out_len", 0, 32)? as usize;
    let m_seq_type = integer("m_seq_type", 1, 6)? as u8;
    let od_len = integer("od_len", if m_seq_type == 6 { 2 } else { 1 }, 32)? as usize;
    let frame_gap_ticks =
        integer("frame_gap_ticks", FRAME_GAP_TICKS as u64, u32::MAX as u64)? as u32;
    let com_text = match config.get("com") {
        None => "COM2",
        Some(value) => value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("IO-Link com must be COM1, COM2, or COM3"))?,
    };
    let com = match com_text.trim().to_ascii_uppercase().as_str() {
        "COM1" => IolinkComSpeed::Com1,
        "COM2" => IolinkComSpeed::Com2,
        "COM3" => IolinkComSpeed::Com3,
        _ => anyhow::bail!("IO-Link com must be COM1, COM2, or COM3"),
    };
    let mut master = IolinkMaster::new_configured(
        pd_in_len,
        pd_out_len,
        m_seq_type,
        od_len,
        com,
        frame_gap_ticks,
    )?;
    match config.get("pd_out_pattern").map(|v| v.as_str()) {
        None | Some(Some("constant")) => {}
        Some(Some("counter")) => master.set_process_data_output_counter(true)?,
        _ => anyhow::bail!("IO-Link pd_out_pattern must be constant or counter"),
    }
    match config.get("startup_profile").map(|v| v.as_str()) {
        None | Some(Some("legacy")) => {}
        Some(Some("tiol112-eval")) => {
            anyhow::ensure!(
                !config.contains_key("frame_gap_ticks"),
                "tiol112-eval uses advertised MinCycleTime; omit frame_gap_ticks"
            );
            master.enable_evaluation_profile()?;
        }
        _ => anyhow::bail!("IO-Link startup_profile must be legacy or tiol112-eval"),
    }
    Ok(master)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requested_master(od_len: usize) -> IolinkMaster {
        IolinkMaster::new_configured(32, 1, 6, od_len, IolinkComSpeed::Com3, 8).unwrap()
    }

    fn yaml_master(yaml: &str) -> anyhow::Result<IolinkMaster> {
        configured_master(&serde_yaml::from_str(yaml).unwrap())
    }

    #[test]
    fn configured_master_preserves_type2v_profile_and_legacy_defaults() {
        let m = yaml_master("pd_in_len: 32\npd_out_len: 1\nm_seq_type: 6\nod_len: 1\ncom: COM3\nframe_gap_ticks: 250").unwrap();
        assert_eq!(
            (m.pd_in_len, m.pd_out.len(), m.m_seq_type, m.od_len),
            (32, 1, 6, 1)
        );
        assert_eq!(m.com, IolinkComSpeed::Com3);
        assert_eq!(m.frame_gap_ticks, 250);
        assert_eq!(m.backend_name_for_test(), "hand-rolled");
        let legacy = yaml_master("{}").unwrap();
        assert_eq!(
            (
                legacy.pd_in_len,
                legacy.pd_out.len(),
                legacy.m_seq_type,
                legacy.od_len
            ),
            (1, 0, 1, 1)
        );
        assert_eq!(legacy.com, IolinkComSpeed::Com2);
        // Retain shipped thermal fixture compatibility, without pretending its
        // legacy envelope is a conformant TYPE_1_1 process-data exchange.
        assert!(yaml_master("pd_in_len: 9\nm_seq_type: 1").is_ok());
    }

    #[test]
    fn config_rejects_malformed_values_before_integer_narrowing() {
        for yaml in [
            "pd_in_len: -1",
            "pd_in_len: 33",
            "pd_in_len: 1.5",
            "pd_in_len: '32'",
            "pd_out_len: -1",
            "pd_out_len: 33",
            "pd_out_len: true",
            "m_seq_type: 2",
            "m_seq_type: 3",
            "m_seq_type: 256",
            "m_seq_type: -1",
            "od_len: 0",
            "od_len: 3",
            "od_len: 33",
            "com: COM4",
            "com: 3",
            "frame_gap_ticks: 0",
            "frame_gap_ticks: -1",
            "frame_gap_ticks: 4294967296",
            "frame_gap_ticks: 1.5",
        ] {
            assert!(
                yaml_master(yaml).is_err(),
                "accepted malformed config {yaml}"
            );
        }
    }

    #[test]
    fn m_sequence_and_od_width_boundaries_are_checked() {
        let config = |pi, po, typ, od| {
            IolinkMaster::new_configured(pi, po, typ, od, IolinkComSpeed::Com2, 1)
        };
        for od in [1, 2, 8, 32] {
            assert!(config(32, 32, 6, od).is_ok());
            assert!(config(0, 32, 6, od).is_ok());
            assert!(config(32, 0, 6, od).is_ok());
            assert!(config(0, 0, 6, od).is_err());
        }
        assert!(config(1, 0, 6, 2).is_ok());
        assert!(config(0, 1, 6, 32).is_ok());
        assert!(config(2, 1, 6, 1).is_ok()); // Table A.10, code 0 former TYPE_2_6
        assert!(config(1, 2, 6, 1).is_ok());
        assert!(config(1, 1, 6, 1).is_err()); // TYPE_2_5 instead
        assert!(config(1, 0, 6, 1).is_err()); // TYPE_2_1 instead
        assert!(config(1, 0, 4, 1).is_ok());
        assert!(config(2, 0, 5, 1).is_ok());
        assert!(config(32, 1, 4, 1).is_err()); // DPP code 4 is not engine enum 4
        assert!(config(1, 0, 4, 2).is_err());
        assert!(config(2, 1, 5, 1).is_err());
        assert!(config(1, 1, 1, 1).is_err());
        assert!(config(1, 0, 1, 2).is_err());
        assert!(IolinkMaster::new_configured(32, 1, 6, 1, IolinkComSpeed::Com3, u32::MAX).is_ok());
    }

    #[test]
    fn type2_write_encoder_has_pd_before_od_and_checksum_in_ckt() {
        // Independently calculated A.1.6 vectors: ck6(00 80 03 00)=35,
        // ck6(C0 80 A5)=3A. A read contains no OD in its master message.
        assert_eq!(
            encode_type2(0x00, &[0x03], &[0x00]),
            [0x00, 0xB5, 0x03, 0x00]
        );
        assert_eq!(encode_type2(0xC0, &[0xA5], &[0x00; 32]), [0xC0, 0xBA, 0xA5]);
    }

    #[test]
    fn configured_scheduler_sends_selector_and_captures_all_32_pd_bytes() {
        let mut m = requested_master(1);
        m.set_process_data_output(&[0x03]).unwrap();
        assert!(m.set_process_data_output(&[]).is_err());
        assert!(m.set_process_data_output(&[1, 2]).is_err());
        assert_eq!(drain(&mut m), [0x55]);
        assert_eq!(drain(&mut m), [0xA2, 0x00]);
        assert_eq!(drain(&mut m), [0x20, 0x06, 0x99]);
        assert_eq!(drain(&mut m), [0x00, 0xB5, 0x03, 0x00]);
        m.set_process_data_output(&[0xA5]).unwrap();
        let expected: Vec<u8> = (0..32).collect();
        for &b in &expected {
            m.on_tx_byte(b);
        }
        assert!(
            !m.pd_valid,
            "a missing CKS must not validate a partial reply"
        );
        m.on_tx_byte(0x2D); // XOR(00..1F)=0, so ck6 is the seed-only value.
        assert!(m.pd_valid);
        assert_eq!(m.process_data_input(), expected);
        assert_eq!(drain(&mut m), [0x00, 0x8A, 0xA5, 0x00]);
        let trace = m.trace_snapshot();
        let x = trace.last().unwrap();
        assert_eq!(x.kind, IolinkFrameKind::Cyclic);
        assert_eq!(x.pd_out, [0x03]); // output snapshot belongs to the sent frame
        assert_eq!(x.pd_in, expected);
        assert_eq!(x.raw_master.len(), 4);
        assert_eq!(x.raw_device.len(), 33);
        assert_eq!(x.ck_ok, Some(true));
        assert_eq!(x.com, IolinkComSpeed::Com3);
    }

    #[test]
    fn type2_event_reads_preserve_pd_and_accept_full_65_octet_reply() {
        let mut m = requested_master(32);
        m.set_process_data_output(&[0xA5]).unwrap();
        while m.link_state != IolinkLinkState::Operate {
            drain(&mut m);
        }
        assert_eq!(m.current.as_ref().unwrap().raw_master.len(), 35);
        let mut flagged = vec![0; 32];
        flagged.push(0x80);
        let last = flagged.len() - 1;
        flagged[last] |= checksum6(&flagged);
        for b in flagged {
            m.on_tx_byte(b);
        }
        assert_eq!(drain(&mut m), [0xC0, 0xBA, 0xA5]);
        let mut reply = vec![0xEE; 32]; // OD must not be exposed as process data
        reply.extend(0..32);
        reply.push(0);
        let last = reply.len() - 1;
        reply[last] = checksum6(&reply);
        for b in reply {
            m.on_tx_byte(b);
        }
        assert_eq!(m.process_data_input(), &(0..32).collect::<Vec<u8>>());
        drain(&mut m);
        let trace = m.trace_snapshot();
        let x = trace.last().unwrap();
        assert_eq!(x.kind, IolinkFrameKind::EventReadout);
        assert_eq!(x.raw_device.len(), 65);
        assert_eq!(x.ck_ok, Some(true));
        assert_eq!(x.pd_in.len(), 32);
        assert_eq!(x.pd_in[0], 0);
        assert_eq!(x.pd_out, [0xA5]);
    }

    #[test]
    fn invalid_checksum_and_pd_invalid_do_not_latch_configured_payload() {
        let mut m = requested_master(1);
        while m.link_state != IolinkLinkState::Operate {
            drain(&mut m);
        }
        for b in (0..32).chain([0x2C]) {
            m.on_tx_byte(b);
        } // one wrong checksum bit
        assert!(!m.pd_valid);
        drain(&mut m);
        for b in (0..32).chain([0x75]) {
            m.on_tx_byte(b);
        } // correct CK6, PD invalid
        assert!(!m.pd_valid);
        assert_eq!(m.process_data_input(), &[0; 32]);
    }

    #[test]
    fn explicit_output_counter_changes_only_future_operate_frames_and_wraps() {
        let mut m = yaml_master("pd_in_len: 32\npd_out_len: 1\nm_seq_type: 6\nod_len: 1\ncom: COM3\npd_out_pattern: counter\nframe_gap_ticks: 8").unwrap();
        m.set_process_data_output(&[254]).unwrap();
        for _ in 0..3 {
            drain(&mut m);
        }
        assert_eq!(drain(&mut m)[2], 254);
        assert_eq!(drain(&mut m)[2], 255);
        assert_eq!(drain(&mut m)[2], 0);
        let trace = m.trace_snapshot();
        assert_eq!(trace[3].pd_out, [254]);
        assert_eq!(trace[4].pd_out, [255]);
        assert!(yaml_master("pd_out_pattern: counter").is_err());
        assert!(yaml_master("pd_out_pattern: random").is_err());
        assert!(yaml_master("pd_out_pattern: 1").is_err());
    }

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
        // Cyclic WRITE reply is `[PD_in..., CKS]` (no OD octet).
        let resp = decode_operate(&[0xA5, 0x22], 1);
        assert!(resp.checksum_ok);
        assert!(resp.pd_valid);
        assert!(!resp.event_present);
        assert_eq!(resp.pd, vec![0xA5]);
    }

    #[test]
    fn decode_operate_flags_event_and_pd_invalid_in_cks() {
        // PD valid, Event flag set: CKS = 0x80 | ck6([0xA5, 0x80]) = 0x8A.
        let ev = decode_operate(&[0xA5, 0x8A], 1);
        assert!(ev.checksum_ok);
        assert!(ev.pd_valid);
        assert!(ev.event_present);

        // PD invalid (CKS bit 6), no event: the PD-status bit participates in
        // the checksum, so CKS = 0x40 | ck6([0xA5, 0x40]) = 0x7A.
        let inv = decode_operate(&[0xA5, 0x7A], 1);
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
        let od = decode_operate(&[0x10, 0x39], 0);
        assert!(od.checksum_ok);
        assert!(od.pd_valid);
        assert!(od.pd.is_empty());

        let valid = decode_operate(&[0xA5, 0x22], 1);
        assert!(valid.checksum_ok);
        assert!(valid.pd_valid);
        assert_eq!(valid.pd, vec![0xA5]);

        let event = decode_operate(&[0xA5, 0x8A], 1);
        assert!(event.checksum_ok);
        assert!(event.pd_valid);
        assert!(event.event_present);

        let invalid = decode_operate(&[0xA5, 0x7A], 1);
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
        let resp = [0xA5u8, 0x22];
        let p = PendingXfer {
            seq: 7,
            kind: IolinkFrameKind::Cyclic,
            pd_out: vec![],
            link_state: IolinkLinkState::Operate,
            raw_master: encode_type1_cycle(&[]),
            raw_device: resp.to_vec(),
            response_od_len: 0,
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
            response_od_len: 0,
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
            response_od_len: 0,
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
        frame.pop();
        frame.push(ck);
        let resp = decode_operate(&frame, 2);
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
        let mut frame = vec![0xAAu8];
        let cks = 0x80 | checksum6(&[0xAA, 0x80]);
        frame.push(cks);
        let resp = decode_operate(&frame, 1);
        assert!(resp.checksum_ok);
        assert!(resp.pd_valid);
        assert!(resp.event_present, "EVENT flag (0x80) must be decoded");

        // PD valid, no event.
        let cks = checksum6(&[0xAA, 0x00]);
        let f2 = vec![0xAAu8, cks];
        let r2 = decode_operate(&f2, 1);
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
        // CKS with the Event flag set (PD valid): bit 7 plus the checksum.
        frame.push(0x80 | checksum6(&[pd.as_slice(), &[0x80]].concat()));

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
        for b in [0xA5u8, 0x80 | checksum6(&[0xA5, 0x80])] {
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
        for b in [0xA5u8, 0x22] {
            m.on_tx_byte(b);
        }
        assert_eq!(m.input_byte(), 0xA5);
        assert!(m.pd_valid);
    }
}
