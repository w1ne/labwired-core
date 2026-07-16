// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CAN diagnostic / UDS / log-player helpers attached to the bus.
//! MCU-agnostic host-side test devices (not SoC peripheral models).

pub struct CanDiagnosticTester {
    pub id: String,
    pub connection: String,
    pub request_id: u32,
    pub request_data: Vec<u8>,
    pub sent: bool,
}

/// Stateful ISO-TP / UDS tester driving a *multi-frame* SecurityAccess exchange
/// against an emulated ECU's CAN controller running in **normal** mode (not
/// loopback). Unlike [`CanDiagnosticTester`] (a one-shot single-frame injector),
/// this is a real second CAN node: it injects a FirstFrame, waits for the ECU's
/// FlowControl, injects the ConsecutiveFrame, then waits for the ECU's
/// SecurityAccess positive response — exactly the handshake a physical UDS
/// tester would perform over ISO 15765-2.
///
/// The ECU side is driven entirely through the peripheral's *public* API: we
/// drain its `tx_frames` (frames it transmitted in normal mode) and inject our
/// frames via `deliver_rx` (bxCAN) / `receive_frame` (FDCAN). Injection is
/// filter-gated, so a `false` return (filter not yet configured, FIFO full)
/// leaves the FSM parked on the same send to retry next tick.
// -----
/// One step in a UDS tester script: a raw payload to send and the
/// expected response bytes (`None` = `..` wildcard, any byte matches).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdsStep {
    /// Raw bytes to send to the ECU (before ISO-TP framing).
    pub send: Vec<u8>,
    /// Expected response bytes; `None` entries match any byte.
    pub expect: Vec<Option<u8>>,
    /// Optional expected NRC byte (response 0x7F <sid> <nrc>).
    pub expect_nrc: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanUdsTesterState {
    /// Need to inject the FirstFrame.
    Start,
    /// FirstFrame sent; waiting for the ECU's FlowControl frame.
    AwaitFc,
    /// ConsecutiveFrame sent; waiting for the ECU's positive response.
    AwaitResp,
    /// Tester sent FlowControl; collecting ECU ConsecutiveFrames until the
    /// declared PDU length is reached (script-driven multi-frame response path).
    AwaitMultiResp,
    /// SecurityAccess positive response observed — handshake complete.
    Done,
    /// Timed out before completion (broken / silent ECU).
    Failed,
}

pub struct CanUdsTester {
    pub id: String,
    /// Name of the connected CAN peripheral (e.g. `bxcan1` / `fdcan1`).
    pub connection: String,
    /// Tester → ECU request id (ISO-TP single physical address). Default 0x111.
    pub request_id: u32,
    /// ECU → tester response id. Default 0x222.
    pub reply_id: u32,
    /// ISO-TP FirstFrame payload injected in state `Start`.
    pub first_frame: Vec<u8>,
    /// ISO-TP ConsecutiveFrame payload injected on FlowControl.
    pub consecutive_frame: Vec<u8>,
    /// Current FSM state. Exposed for tests.
    pub state: CanUdsTesterState,
    /// Ticks elapsed since the tester started; used for the give-up timeout.
    pub ticks: u64,
    /// Tick budget before declaring `Failed`.
    pub max_ticks: u64,
    /// Scripted exchange steps; empty when using legacy hardcoded payloads.
    pub script: Vec<UdsStep>,
    /// Index of the current step in `script`.
    pub step_idx: usize,
    /// Set when a step fails; describes what went wrong.
    pub failure: Option<String>,
    /// PDU accumulator for the script-driven multi-frame ECU response path.
    /// Cleared at the start of each step.
    pub(crate) resp_buf: Vec<u8>,
    /// Declared PDU length from the ECU's FF header (script path only).
    pub(crate) resp_expected_len: usize,
    /// Remaining ConsecutiveFrames to inject for a multi-frame tester request,
    /// populated after the request FF is accepted and the ECU FlowControl is
    /// received (script path only).
    pub(crate) pending_cfs: Vec<Vec<u8>>,
}

impl CanUdsTester {
    /// Default tester ↔ ECU ids and ISO-TP payloads for the SecurityAccess
    /// SeedRequest exchange the firmware contract expects.
    pub const DEFAULT_REQUEST_ID: u32 = 0x111;
    pub const DEFAULT_REPLY_ID: u32 = 0x222;
    pub const DEFAULT_FIRST_FRAME: [u8; 8] = [0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33];
    pub const DEFAULT_CONSECUTIVE_FRAME: [u8; 8] = [0x21, 0x44, 0x55, 0x66, 0x77, 0x88, 0x55, 0x55];
    const DEFAULT_MAX_TICKS: u64 = 200_000;

    pub fn new(id: String, connection: String) -> Self {
        Self {
            id,
            connection,
            request_id: Self::DEFAULT_REQUEST_ID,
            reply_id: Self::DEFAULT_REPLY_ID,
            first_frame: Self::DEFAULT_FIRST_FRAME.to_vec(),
            consecutive_frame: Self::DEFAULT_CONSECUTIVE_FRAME.to_vec(),
            state: CanUdsTesterState::Start,
            ticks: 0,
            max_ticks: Self::DEFAULT_MAX_TICKS,
            script: Vec::new(),
            step_idx: 0,
            failure: None,
            resp_buf: Vec::new(),
            resp_expected_len: 0,
            pending_cfs: Vec::new(),
        }
    }

    /// Build the ISO-TP request frame(s) for `script[step_idx]`.
    /// Single-frame when `send.len() <= 7`; otherwise a FirstFrame followed by
    /// ConsecutiveFrames. The caller sends the first frame and queues the rest
    /// in `pending_cfs` after FlowControl.
    pub(crate) fn build_request_frames(&self) -> Vec<Vec<u8>> {
        let Some(step) = self.script.get(self.step_idx) else {
            return Vec::new();
        };
        let data = &step.send;
        let len = data.len();
        if len <= 7 {
            // Single-frame: [len, payload...]
            let mut frame = Vec::with_capacity(len + 1);
            frame.push(len as u8);
            frame.extend_from_slice(data);
            return vec![frame];
        }
        // Multi-frame: FF then CFs.
        let mut frames = Vec::new();
        // FirstFrame: [0x10 | (len>>8), len & 0xFF, first 6 bytes]
        let mut ff = Vec::with_capacity(8);
        ff.push(0x10 | ((len >> 8) as u8));
        ff.push((len & 0xFF) as u8);
        ff.extend_from_slice(&data[..6.min(len)]);
        frames.push(ff);
        // ConsecutiveFrames
        let mut seq: u8 = 1;
        let mut offset = 6;
        while offset < len {
            let end = (offset + 7).min(len);
            let mut cf = Vec::with_capacity(8);
            cf.push(0x20 | (seq & 0x0F));
            cf.extend_from_slice(&data[offset..end]);
            frames.push(cf);
            seq = seq.wrapping_add(1);
            offset = end;
        }
        frames
    }

    /// Return `true` when `resp` satisfies the match criteria of `step`.
    /// If `step.expect_nrc` is `Some(nrc)`, matches `[0x7F, send[0], nrc]`.
    /// Otherwise compares against `step.expect` element-wise (`None` = any byte),
    /// allowing `resp` to be longer than the pattern (prefix match).
    pub(crate) fn matches(resp: &[u8], step: &UdsStep) -> bool {
        if let Some(nrc) = step.expect_nrc {
            return resp == [0x7F, step.send.first().copied().unwrap_or(0), nrc];
        }
        let pattern = &step.expect;
        if resp.len() < pattern.len() {
            return false;
        }
        pattern
            .iter()
            .zip(resp.iter())
            .all(|(p, b)| p.is_none_or(|expected| expected == *b))
    }

    /// Observe one ECU frame in the **script-driven** path. Returns the payload
    /// to inject next (FlowControl or first pending CF), or `None`. Sets
    /// `state = Done / Failed` when the exchange concludes.
    pub(crate) fn observe_ecu_frame_script(&mut self, id: u32, data: &[u8]) -> Option<Vec<u8>> {
        if id != self.reply_id {
            return None;
        }
        match self.state {
            CanUdsTesterState::AwaitFc => {
                if data.first().map(|b| b & 0xF0) == Some(0x30) {
                    // FlowControl received: signal the next CF to inject.
                    // Do NOT change state here — the injected block in
                    // service_can_uds_testers advances AwaitFc→AwaitResp only
                    // after the last CF has been successfully accepted, draining
                    // pending_cfs one entry per tick.
                    return self.pending_cfs.first().cloned();
                }
                None
            }
            CanUdsTesterState::AwaitResp => {
                let ptype = data.first().map(|b| b & 0xF0).unwrap_or(0xFF);
                if ptype == 0x00 {
                    // ECU SingleFrame response. Two ISO-TP SF encodings:
                    //   * classic: byte0 = 0x0L, length L (1..=7) in the low
                    //     nibble, payload from byte 1.
                    //   * CAN-FD escape: byte0 = 0x00, real length in byte 1,
                    //     payload from byte 2 (used for SF up to 62 bytes on FD
                    //     frames — the ECU runs ISO-TP in FD mode).
                    let b0 = data.first().copied().unwrap_or(0);
                    let (pdu_len, data_off) = if b0 == 0x00 {
                        // CAN-FD escape SF: a length byte must follow.
                        match data.get(1) {
                            Some(&len) => (len as usize, 2),
                            None => {
                                self.failure = Some(format!(
                                    "step {}: malformed FD escape SingleFrame (no length byte)",
                                    self.step_idx
                                ));
                                self.state = CanUdsTesterState::Failed;
                                return None;
                            }
                        }
                    } else {
                        ((b0 & 0x0F) as usize, 1)
                    };
                    // The frame must actually carry the declared payload bytes; a
                    // short/truncated SF is a protocol error, not an empty match.
                    if data.len() < data_off + pdu_len {
                        self.failure = Some(format!(
                            "step {}: truncated SingleFrame (declared {} payload bytes, frame carries {})",
                            self.step_idx,
                            pdu_len,
                            data.len().saturating_sub(data_off)
                        ));
                        self.state = CanUdsTesterState::Failed;
                        return None;
                    }
                    let payload: Vec<u8> = data[data_off..data_off + pdu_len].to_vec();
                    self.complete_response(payload);
                } else if ptype == 0x10 {
                    // ECU FirstFrame: start reassembly, send FlowControl.
                    let declared = if data.len() >= 2 {
                        (((data[0] & 0x0F) as usize) << 8) | (data[1] as usize)
                    } else {
                        0
                    };
                    self.resp_expected_len = declared;
                    self.resp_buf.clear();
                    if data.len() > 2 {
                        self.resp_buf.extend_from_slice(&data[2..]);
                    }
                    self.state = CanUdsTesterState::AwaitMultiResp;
                    // FlowControl: ContinueToSend, block size 0, ST 0.
                    return Some(vec![0x30, 0x00, 0x00]);
                }
                None
            }
            CanUdsTesterState::AwaitMultiResp => {
                if data.first().map(|b| b & 0xF0) == Some(0x20) {
                    self.resp_buf
                        .extend_from_slice(data.get(1..).unwrap_or(&[]));
                    if self.resp_buf.len() >= self.resp_expected_len {
                        let payload = self.resp_buf[..self.resp_expected_len].to_vec();
                        self.resp_buf.clear();
                        self.complete_response(payload);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Called when a complete PDU has been reassembled. Matches against the
    /// current step and either advances to the next step (or `Done`) or sets
    /// `Failed`.
    pub(crate) fn complete_response(&mut self, payload: Vec<u8>) {
        let Some(step) = self.script.get(self.step_idx) else {
            self.state = CanUdsTesterState::Done;
            return;
        };
        if Self::matches(&payload, step) {
            self.step_idx += 1;
            self.resp_buf.clear();
            self.resp_expected_len = 0;
            if self.step_idx >= self.script.len() {
                self.state = CanUdsTesterState::Done;
            } else {
                // More steps: the driver will send the next request next tick.
                self.state = CanUdsTesterState::Start;
            }
        } else {
            let msg = if let Some(nrc) = step.expect_nrc {
                format!(
                    "step {}: expected NRC 7F {:02X} {:02X}, got {:02X?}",
                    self.step_idx,
                    step.send.first().copied().unwrap_or(0),
                    nrc,
                    payload
                )
            } else {
                format!(
                    "step {}: expected {:02X?}, got {:02X?}",
                    self.step_idx, step.expect, payload
                )
            };
            self.failure = Some(msg);
            self.state = CanUdsTesterState::Failed;
        }
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            CanUdsTesterState::Done | CanUdsTesterState::Failed
        )
    }

    /// Observe one frame the ECU transmitted. Legacy path (empty `script`):
    /// In `AwaitFc` an ISO-TP FlowControl (`(data[0] & 0xF0) == 0x30`) on
    /// `reply_id` returns the ConsecutiveFrame payload to inject; in `AwaitResp`
    /// a SecurityAccess single-frame positive response (`data[0] == 0x06 &&
    /// data[1] == 0x67`) completes the handshake. Returns the payload to inject
    /// next, else `None`.
    ///
    /// When `script` is non-empty, delegates to `observe_ecu_frame_script`
    /// instead so the script-driven logic handles framing and matching.
    pub(crate) fn observe_ecu_frame(&mut self, id: u32, data: &[u8]) -> Option<Vec<u8>> {
        if !self.script.is_empty() {
            return self.observe_ecu_frame_script(id, data);
        }
        if id != self.reply_id {
            return None;
        }
        match self.state {
            CanUdsTesterState::AwaitFc => {
                if data.first().map(|b| b & 0xF0) == Some(0x30) {
                    // FlowControl seen → time to send the ConsecutiveFrame.
                    return Some(self.consecutive_frame.clone());
                }
                None
            }
            CanUdsTesterState::AwaitResp => {
                if data.first() == Some(&0x06) && data.get(1) == Some(&0x67) {
                    self.state = CanUdsTesterState::Done;
                }
                None
            }
            _ => None,
        }
    }
}

/// Deterministic CAN log replay node: an external bus participant that
/// delivers pre-parsed frames into a bxCAN/FDCAN peripheral at scheduled
/// tick offsets. Vendor-neutral by design — candump input only; vendor log
/// formats convert outside core (2026-07-02 replay-showcase spec).
pub struct CanLogPlayer {
    pub id: String,
    /// Name of the connected CAN peripheral (e.g. `bxcan1` / `fdcan1`).
    pub connection: String,
    /// (due_tick, frame), ascending; first frame rebased to tick 0.
    pub frames: Vec<(u64, crate::network::CanFrame)>,
    pub next_idx: usize,
    pub ticks: u64,
    /// Frames accepted by the peripheral.
    pub delivered: u64,
    /// Frames refused (filters closed / FIFO full / CAN not initialized).
    pub dropped: u64,
}

impl CanLogPlayer {
    pub fn from_candump(
        id: String,
        connection: String,
        text: &str,
        ticks_per_second: u64,
    ) -> Result<Self, String> {
        let parsed = crate::network::candump::parse_candump(text)?;
        if parsed.is_empty() {
            return Err(format!("can-player '{id}': log contains no frames"));
        }
        let t0 = parsed[0].0;
        let mut frames: Vec<(u64, crate::network::CanFrame)> = parsed
            .into_iter()
            .map(|(t, f)| {
                (
                    ((t - t0).max(0.0) * ticks_per_second as f64).round() as u64,
                    f,
                )
            })
            .collect();
        frames.sort_by_key(|(t, _)| *t);
        Ok(Self {
            id,
            connection,
            frames,
            next_idx: 0,
            ticks: 0,
            delivered: 0,
            dropped: 0,
        })
    }

    pub fn is_done(&self) -> bool {
        self.next_idx >= self.frames.len()
    }
}
