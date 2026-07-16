// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CAN diagnostic / UDS / log-player helpers attached to the bus.
//! MCU-agnostic host-side test devices (not SoC peripheral models), plus the
//! per-tick `SystemBus` service methods that drive them.

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

use super::SystemBus;

impl SystemBus {
    pub(crate) fn service_can_diagnostic_testers(&mut self) {
        if self.can_diagnostic_testers.is_empty() {
            return;
        }

        for i in 0..self.can_diagnostic_testers.len() {
            if self.can_diagnostic_testers[i].sent {
                continue;
            }

            let connection = self.can_diagnostic_testers[i].connection.clone();
            let Some(idx) = self.find_peripheral_index_by_name(&connection) else {
                continue;
            };
            let Some(fdcan) = self.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<crate::peripherals::fdcan::Fdcan>())
            else {
                continue;
            };

            let frame = crate::network::CanFrame {
                id: self.can_diagnostic_testers[i].request_id,
                data: self.can_diagnostic_testers[i].request_data.clone(),
                extended: false,
                fd: self.can_diagnostic_testers[i].request_data.len() > 8,
                bitrate_switch: self.can_diagnostic_testers[i].request_data.len() > 8,
                remote: false,
            };
            if fdcan.receive_frame(frame) {
                self.can_diagnostic_testers[i].sent = true;
            }
        }
    }

    /// Per-tick service for the stateful ISO-TP/UDS testers. For each tester:
    /// resolve its peripheral by name, drain the ECU's outbound `tx_frames`,
    /// advance the FSM, and inject the next ISO-TP frame (filter-gated) when due.
    ///
    /// Works against both bxCAN (`deliver_rx`) and FDCAN (`receive_frame`); the
    /// downcast picks whichever is wired. A filtered/dropped injection (return
    /// `false`) leaves the FSM parked on the same send so it retries next tick —
    /// important on the first ticks before the ECU has configured its filter.
    pub(crate) fn service_can_uds_testers(&mut self) {
        if self.can_uds_testers.is_empty() {
            return;
        }

        for i in 0..self.can_uds_testers.len() {
            if self.can_uds_testers[i].is_terminal() {
                continue;
            }

            // Timeout guard so a broken/silent ECU never hangs the sim.
            self.can_uds_testers[i].ticks += 1;
            if self.can_uds_testers[i].ticks > self.can_uds_testers[i].max_ticks {
                self.can_uds_testers[i].state = CanUdsTesterState::Failed;
                continue;
            }

            let connection = self.can_uds_testers[i].connection.clone();
            let Some(idx) = self.find_peripheral_index_by_name(&connection) else {
                continue;
            };

            // Drain the ECU's outbound frames and feed the FSM. `observe_ecu_frame`
            // may return a payload to inject (e.g. the CF unblocked by FlowControl);
            // the actual injection happens below so both peripheral kinds share one
            // filter-gated send path.
            let request_id = self.can_uds_testers[i].request_id;
            let mut pending_inject: Option<Vec<u8>> = None;

            // Resolve the peripheral once; reborrow per phase to satisfy the
            // borrow checker (drain, then inject).
            let drained: Vec<crate::network::CanFrame> = {
                let any = self.peripherals[idx].dev.as_any_mut();
                match any {
                    Some(a) => {
                        if let Some(bx) = a.downcast_mut::<crate::peripherals::bxcan::BxCan>() {
                            bx.tx_frames.drain(..).collect()
                        } else if let Some(fd) =
                            a.downcast_mut::<crate::peripherals::fdcan::Fdcan>()
                        {
                            fd.tx_frames.drain(..).collect()
                        } else {
                            continue;
                        }
                    }
                    None => continue,
                }
            };

            for frame in &drained {
                if let Some(payload) =
                    self.can_uds_testers[i].observe_ecu_frame(frame.id, &frame.data)
                {
                    pending_inject = Some(payload);
                }
            }

            // Decide what (if anything) to inject this tick.
            let has_script = !self.can_uds_testers[i].script.is_empty();
            let to_send: Option<Vec<u8>> = if has_script {
                match self.can_uds_testers[i].state {
                    CanUdsTesterState::Start => {
                        // Build request frames for the current script step.
                        let frames = self.can_uds_testers[i].build_request_frames();
                        if let Some((first, rest)) = frames.split_first() {
                            // Queue any CFs for later (after FlowControl).
                            self.can_uds_testers[i].pending_cfs = rest.to_vec();
                            Some(first.clone())
                        } else {
                            None
                        }
                    }
                    // Use the observe result when an FC arrived this tick, or
                    // the front of pending_cfs when additional CFs remain from
                    // a previous tick's FC (no new ECU frame → pending_inject
                    // is None but the queue is non-empty).
                    CanUdsTesterState::AwaitFc => pending_inject
                        .or_else(|| self.can_uds_testers[i].pending_cfs.first().cloned()),
                    // ECU sent a FirstFrame this tick; observe_ecu_frame_script
                    // already set state=AwaitMultiResp and returned the FlowControl
                    // in pending_inject. Forward it so the ECU can send its CFs.
                    CanUdsTesterState::AwaitMultiResp => pending_inject,
                    _ => None,
                }
            } else {
                match self.can_uds_testers[i].state {
                    CanUdsTesterState::Start => Some(self.can_uds_testers[i].first_frame.clone()),
                    CanUdsTesterState::AwaitFc => pending_inject,
                    _ => None,
                }
            };

            let Some(payload) = to_send else {
                continue;
            };

            let frame = crate::network::CanFrame::classic(request_id, payload);
            let injected = {
                let any = self.peripherals[idx].dev.as_any_mut();
                match any {
                    Some(a) => {
                        if let Some(bx) = a.downcast_mut::<crate::peripherals::bxcan::BxCan>() {
                            bx.deliver_rx(frame)
                        } else if let Some(fd) =
                            a.downcast_mut::<crate::peripherals::fdcan::Fdcan>()
                        {
                            fd.receive_frame(frame)
                        } else {
                            false
                        }
                    }
                    None => false,
                }
            };

            if injected {
                // Advance only on a successful (accepted) injection; otherwise
                // stay parked and retry next tick.
                let has_script = !self.can_uds_testers[i].script.is_empty();
                match self.can_uds_testers[i].state {
                    CanUdsTesterState::Start if has_script => {
                        // SF (no pending CFs) → go straight to AwaitResp.
                        // FF (pending CFs queued) → go to AwaitFc.
                        if self.can_uds_testers[i].pending_cfs.is_empty() {
                            self.can_uds_testers[i].state = CanUdsTesterState::AwaitResp;
                        } else {
                            self.can_uds_testers[i].state = CanUdsTesterState::AwaitFc;
                        }
                    }
                    CanUdsTesterState::Start => {
                        self.can_uds_testers[i].state = CanUdsTesterState::AwaitFc
                    }
                    CanUdsTesterState::AwaitFc if has_script => {
                        // Pop the CF that was just successfully injected.
                        if !self.can_uds_testers[i].pending_cfs.is_empty() {
                            self.can_uds_testers[i].pending_cfs.remove(0);
                        }
                        // Only advance to AwaitResp once all CFs have been sent.
                        if self.can_uds_testers[i].pending_cfs.is_empty() {
                            self.can_uds_testers[i].state = CanUdsTesterState::AwaitResp;
                        }
                    }
                    CanUdsTesterState::AwaitFc => {
                        self.can_uds_testers[i].state = CanUdsTesterState::AwaitResp
                    }
                    _ => {}
                }
            }
        }
    }

    /// Per-tick service for deterministic CAN log replay nodes. For each
    /// player, advance its tick counter and deliver every due frame
    /// (`due_tick < now`) into the connected peripheral, filter-gated the
    /// same way a real bus would drop unmatched frames.
    pub(crate) fn service_can_log_players(&mut self) {
        if self.can_log_players.is_empty() {
            return;
        }
        for i in 0..self.can_log_players.len() {
            self.can_log_players[i].ticks += 1;
            if self.can_log_players[i].is_done() {
                continue;
            }
            let connection = self.can_log_players[i].connection.clone();
            let Some(idx) = self.find_peripheral_index_by_name(&connection) else {
                continue;
            };
            let now = self.can_log_players[i].ticks;
            while !self.can_log_players[i].is_done()
                && self.can_log_players[i].frames[self.can_log_players[i].next_idx].0 < now
            {
                let j = self.can_log_players[i].next_idx;
                let frame = self.can_log_players[i].frames[j].1.clone();
                let accepted = {
                    let any = self.peripherals[idx].dev.as_any_mut();
                    match any {
                        Some(a) => {
                            if let Some(bx) = a.downcast_mut::<crate::peripherals::bxcan::BxCan>() {
                                bx.deliver_rx(frame)
                            } else if let Some(fd) =
                                a.downcast_mut::<crate::peripherals::fdcan::Fdcan>()
                            {
                                fd.receive_frame(frame)
                            } else {
                                false
                            }
                        }
                        None => false,
                    }
                };
                if accepted {
                    self.can_log_players[i].delivered += 1;
                } else {
                    self.can_log_players[i].dropped += 1;
                }
                self.can_log_players[i].next_idx += 1;
            }
        }
    }

    pub(crate) fn yaml_u32(value: Option<&serde_yaml::Value>, default: u32) -> u32 {
        match value {
            Some(serde_yaml::Value::Number(n)) => n.as_u64().map(|v| v as u32).unwrap_or(default),
            Some(serde_yaml::Value::String(s)) => {
                let s = s.trim();
                if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                    u32::from_str_radix(&hex.replace('_', ""), 16).unwrap_or(default)
                } else {
                    s.replace('_', "").parse::<u32>().unwrap_or(default)
                }
            }
            _ => default,
        }
    }

    pub(crate) fn yaml_bytes(value: Option<&serde_yaml::Value>, default: &[u8]) -> Vec<u8> {
        match value {
            Some(serde_yaml::Value::Sequence(seq)) => seq
                .iter()
                .map(|value| Self::yaml_u32(Some(value), 0) as u8)
                .collect(),
            Some(serde_yaml::Value::String(s)) => s
                .split(|c: char| c.is_ascii_whitespace() || c == ',' || c == ':')
                .filter(|part| !part.is_empty())
                .map(|part| {
                    let part = part.trim();
                    if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
                        match u8::from_str_radix(hex, 16) {
                            Ok(b) => b,
                            Err(_) => {
                                tracing::warn!(
                                    "[uds-tester] malformed send byte {:?}, treating as 0x00",
                                    part
                                );
                                0
                            }
                        }
                    } else {
                        u8::from_str_radix(part, 16)
                            .unwrap_or_else(|_| part.parse::<u8>().unwrap_or(0))
                    }
                })
                .collect(),
            _ => default.to_vec(),
        }
    }

    /// Parse an expect string such as `"51 01 .."` into a mask vector.
    /// `".."` becomes `None` (wildcard); any other token is parsed as a hex
    /// byte and becomes `Some(byte)`.
    pub(crate) fn parse_expect(s: &str) -> Vec<Option<u8>> {
        s.split_ascii_whitespace()
            .map(|tok| {
                if tok == ".." {
                    None
                } else {
                    let hex = tok.trim_start_matches("0x").trim_start_matches("0X");
                    match u8::from_str_radix(hex, 16) {
                        Ok(b) => Some(b),
                        Err(_) => {
                            tracing::warn!(
                                "[uds-tester] malformed expect token {:?}, treating as 0x00",
                                tok
                            );
                            Some(0)
                        }
                    }
                }
            })
            .collect()
    }

    /// Parse an optional YAML `script:` sequence into a `Vec<UdsStep>`.
    pub(crate) fn parse_script(value: Option<&serde_yaml::Value>) -> Vec<UdsStep> {
        let seq = match value {
            Some(serde_yaml::Value::Sequence(s)) => s,
            _ => return Vec::new(),
        };
        seq.iter()
            .map(|entry| {
                let send = Self::yaml_bytes(entry.get("send"), &[]);
                let expect_str = entry.get("expect").and_then(|v| v.as_str()).unwrap_or("");
                let expect = Self::parse_expect(expect_str);
                let expect_nrc = entry
                    .get("expect_nrc")
                    .map(|v| Self::yaml_u32(Some(v), 0) as u8);
                UdsStep {
                    send,
                    expect,
                    expect_nrc,
                }
            })
            .collect()
    }
}
