// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::peripherals::uart::UartStreamDevice;
use std::any::Any;
use std::collections::VecDeque;

/// IO-Link 6-bit checksum (CRC6). Polynomial `0x1D << 2`, initial value `0x15`.
/// Ports `calculate_crc6` from the project's reference virtual-master crc.py.
pub(crate) fn crc6(data: &[u8]) -> u8 {
    let mut crc: u8 = 0x15;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ (0x1D << 2);
            } else {
                crc <<= 1;
            }
        }
    }
    (crc >> 2) & 0x3F
}

/// Encode a Type 0 master frame: `[MC, CK]` with `CK = crc6([MC, CKT=0x00])`.
pub(crate) fn encode_type0(mc: u8) -> Vec<u8> {
    vec![mc, crc6(&[mc, 0x00])]
}

/// Encode a Type 1 cyclic request: `[MC=0x00, CKT=0x00, PD_out..., OD=0x00, CK]`.
pub(crate) fn encode_type1_cycle(pd_out: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x00u8, 0x00];
    frame.extend_from_slice(pd_out);
    frame.push(0x00); // OD (1-byte, idle)
    let ck = crc6(&frame);
    frame.push(ck);
    frame
}

/// Parsed device OPERATE response.
#[derive(Debug, Clone)]
pub(crate) struct OperateResponse {
    pub(crate) pd: Vec<u8>,
    pub(crate) pd_valid: bool,
    pub(crate) checksum_ok: bool,
}

/// Decode `[status, PD_in..., OD..., CK]` (length `1 + pd_in_len + od_len + 1`).
pub(crate) fn decode_operate(data: &[u8], pd_in_len: usize, od_len: usize) -> OperateResponse {
    if data.len() < 2 + pd_in_len + od_len {
        return OperateResponse {
            pd: Vec::new(),
            pd_valid: false,
            checksum_ok: false,
        };
    }
    let status = data[0];
    let pd_end = data.len() - od_len - 1;
    let pd = data[1..pd_end].to_vec();
    let ck = data[data.len() - 1];
    let checksum_ok = crc6(&data[..data.len() - 1]) == ck;
    let pd_valid = status & 0x20 != 0;
    OperateResponse {
        pd,
        pd_valid,
        checksum_ok,
    }
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

/// Ticks the master waits (one `poll` per UART tick) between frames. The
/// simulated device executes far slower than the UART advances, so frames are
/// paced generously to guarantee the device has fully processed (and replied
/// to) one frame before the next arrives — this is what keeps the device's
/// byte framing aligned. Tunable; sized for the `-O0` demo firmware.
const FRAME_GAP_TICKS: u32 = 6000;

/// Number of IDLE frames sent before the OPERATE transition. The device needs
/// one valid frame to leave AWAITING_COMM for PREOPERATE; a few repeats absorb
/// any byte the wake-up detection consumed.
const IDLE_FRAMES: u32 = 4;

/// Native IO-Link master peer. Attaches to the firmware's UART as a
/// `UartStreamDevice`: `poll` drives the master's request bytes onto the firmware
/// RX path, `on_tx_byte` receives the device's response bytes from the firmware
/// TX path.
///
/// Drives a **deterministic, tick-paced** startup schedule rather than reacting
/// to response timing: wake-up (once) → several IDLE frames (→ PREOPERATE) → the
/// OPERATE transition (→ ESTAB_COM) → cyclic Type 1 requests (→ OPERATE). Process
/// data input is captured from the cyclic responses.
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
}

impl IolinkMaster {
    pub fn new(pd_in_len: usize, od_len: usize, com: IolinkComSpeed) -> Self {
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
        };
        m.queue_next_frame(); // queue the wake-up immediately
        m
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
        1 + self.pd_in_len + self.od_len + 1
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
                (Vec::new(), Some(false), Some(false))
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

        let idle_end = 1 + IDLE_FRAMES; // steps [1..=IDLE_FRAMES] are IDLE
        let (frame, kind): (Vec<u8>, IolinkFrameKind) = if self.step == 0 {
            (vec![0x55], IolinkFrameKind::WakeUp) // wake-up pulse (once)
        } else if self.step < idle_end {
            (encode_type0(0x00), IolinkFrameKind::Idle) // Type 0 IDLE → PREOPERATE
        } else if self.step == idle_end {
            (encode_type0(0x0F), IolinkFrameKind::OperateReq) // OPERATE transition
        } else {
            self.link_state = IolinkLinkState::Operate;
            (encode_type1_cycle(&[]), IolinkFrameKind::Cyclic) // cyclic Type 1
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
        if self.step <= idle_end {
            self.step += 1;
        }
    }
}

impl UartStreamDevice for IolinkMaster {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        if let Some(byte) = self.tx_queue.pop_front() {
            return Some(byte);
        }
        // Frame fully sent: wait the inter-frame gap, then queue the next one.
        self.gap_ticks = self.gap_ticks.saturating_add(1);
        if self.gap_ticks < FRAME_GAP_TICKS {
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
            && self.rx_accum.len() >= self.operate_response_len()
        {
            let n = self.operate_response_len();
            let resp = decode_operate(&self.rx_accum[..n], self.pd_in_len, self.od_len);
            if resp.checksum_ok && resp.pd_valid {
                self.latest_pd = resp.pd;
                self.pd_valid = true;
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
    device_type: "iolink-master",
    label: "IO-Link Master",
    summary: "IO-Link master state machine over UART.",
    detail: "Drives wake-up / startup / operate cycles, m-sequence types, process-data \
             exchange. The AL2205 DI device demo uses this to host two digital-input channels.",
    transport: Transport::Uart,
    category: Category::Uart,
    config_keys: &[
        ConfigKey {
            name: "pd_in_len",
            ty: ConfigType::Int,
            doc: "Process-data input length in bytes. Defaults to 1 (single-byte DI device).",
        },
        ConfigKey {
            name: "m_seq_type",
            ty: ConfigType::Int,
            doc: "M-sequence type (1..6). Used to derive od_len: types ≥ 4 use 2-byte OD frames.",
        },
        ConfigKey {
            name: "com",
            ty: ConfigType::Str,
            doc: "Communication speed: \"COM1\" (4.8 kbaud), \"COM2\" (38.4 kbaud, default), or \"COM3\" (230.4 kbaud).",
        },
    ],
    labs: &[LabRef {
        board_id: "al2205-iolink-dido",
        chip: "stm32l476",
        example_dir: "al2205-iolink-dido",
        demo_elf: "demo-al2205-iolink-dido.elf",
    }],
};

impl PeripheralKit for IolinkMasterKit {
    fn metadata(&self) -> &'static KitMetadata {
        &IOLINK_MASTER_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let pd_in_len = ctx.config_i64("pd_in_len").unwrap_or(1) as usize;
        let m_seq_type = ctx.config_i64("m_seq_type").unwrap_or(1);
        let od_len: usize = if m_seq_type >= 4 { 2 } else { 1 };
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
        let uart = ctx.uart()?;
        uart.attach_stream(Box::new(IolinkMaster::new(pd_in_len, od_len, com)));
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

    #[test]
    fn crc6_matches_iolink_vectors() {
        assert_eq!(crc6(&[0x00, 0x00]), 0x24);
        assert_eq!(crc6(&[0x0F, 0x00]), 0x0D);
        assert_eq!(crc6(&[0x95, 0x00]), 0x1D);
        assert_eq!(crc6(&[0x20, 0xA5, 0x00]), 0x0D);
    }

    #[test]
    fn encodes_type0_idle_and_operate_transition() {
        assert_eq!(encode_type0(0x00), vec![0x00, 0x24]); // IDLE
        assert_eq!(encode_type0(0x0F), vec![0x0F, 0x0D]); // OPERATE transition
    }

    #[test]
    fn encodes_type1_di_cycle_with_no_output_pd() {
        assert_eq!(encode_type1_cycle(&[]), vec![0x00, 0x00, 0x00, 0x09]);
    }

    #[test]
    fn decodes_operate_response_and_extracts_pd() {
        let resp = decode_operate(&[0x20, 0xA5, 0x00, 0x0D], 1, 1);
        assert!(resp.checksum_ok);
        assert!(resp.pd_valid);
        assert_eq!(resp.pd, vec![0xA5]);
    }

    #[test]
    fn finalize_cyclic_decodes_response_and_marks_ck() {
        let m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        let resp = [0x20u8, 0xA5, 0x00, crc6(&[0x20, 0xA5, 0x00])];
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
    fn decode_operate_handles_two_byte_pd() {
        let mut frame = vec![0x20u8, 0xAA, 0xBB, 0x00];
        let ck = crc6(&frame);
        frame.push(ck);
        let resp = decode_operate(&frame, 2, 1);
        assert!(resp.checksum_ok);
        assert!(resp.pd_valid);
        assert_eq!(resp.pd, vec![0xAA, 0xBB]);
    }

    #[test]
    fn schedule_walks_wakeup_idle_transition_then_cyclic_type1() {
        let mut m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);

        // Step 0: wake-up pulse.
        assert_eq!(drain(&mut m), vec![0x55]);
        assert_eq!(m.link_state, IolinkLinkState::Startup);

        // Steps 1..=IDLE_FRAMES: IDLE frames (→ PREOPERATE on the device).
        for _ in 0..IDLE_FRAMES {
            assert_eq!(drain(&mut m), vec![0x00, 0x24]);
        }
        assert_eq!(m.link_state, IolinkLinkState::Startup);

        // Next: the OPERATE transition (MC=0x0F).
        assert_eq!(drain(&mut m), vec![0x0F, 0x0D]);

        // Then cyclic Type 1 requests, repeating forever.
        assert_eq!(drain(&mut m), vec![0x00, 0x00, 0x00, 0x09]);
        assert_eq!(m.link_state, IolinkLinkState::Operate);
        assert_eq!(drain(&mut m), vec![0x00, 0x00, 0x00, 0x09]);
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
    fn captures_process_data_from_cyclic_response() {
        let mut m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);
        // Advance the schedule to the cyclic (OPERATE) phase.
        while m.link_state != IolinkLinkState::Operate {
            drain(&mut m);
        }
        // Device replies to the cyclic request with PD = 0xA5, valid.
        for b in [0x20u8, 0xA5, 0x00, 0x0D] {
            m.on_tx_byte(b);
        }
        assert_eq!(m.input_byte(), 0xA5);
        assert!(m.pd_valid);
    }
}
