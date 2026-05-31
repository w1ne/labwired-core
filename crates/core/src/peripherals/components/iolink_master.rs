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

/// Native IO-Link master peer. Attaches to the firmware's UART as a
/// `UartStreamDevice`: `poll` drives the master's request bytes onto the
/// firmware RX path, `on_tx_byte` receives the device's response bytes from the
/// firmware TX path. Runs a minimal cyclic master (wake-up → IDLE → OPERATE
/// transition → cyclic Type 1 requests) ported from the reference virtual master.
#[derive(Debug, serde::Serialize)]
pub struct IolinkMaster {
    pd_in_len: usize,
    od_len: usize,
    com: IolinkComSpeed,
    pub link_state: IolinkLinkState,
    /// Bytes still to send onto the firmware RX path.
    #[serde(skip)]
    tx_queue: VecDeque<u8>,
    /// Accumulated device-response bytes from the firmware TX path.
    #[serde(skip)]
    rx_accum: Vec<u8>,
    /// Response length expected for the in-flight request (0 = not awaiting).
    expected_resp: usize,
    /// True once a request is fully sent and we are waiting for its response.
    awaiting: bool,
    /// Latest valid process-data input bytes received from the device.
    latest_pd: Vec<u8>,
    pub pd_valid: bool,
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
            expected_resp: 0,
            awaiting: false,
            latest_pd: vec![0u8; pd_in_len.max(1)],
            pd_valid: false,
        };
        m.queue_startup();
        m
    }

    /// First process-data input byte (channel bitmap for a DI hub).
    pub fn input_byte(&self) -> u8 {
        self.latest_pd.first().copied().unwrap_or(0)
    }

    pub fn com_speed(&self) -> IolinkComSpeed {
        self.com
    }

    fn operate_response_len(&self) -> usize {
        1 + self.pd_in_len + self.od_len + 1
    }

    fn queue_startup(&mut self) {
        self.tx_queue.push_back(0x55); // wake-up
        for b in encode_type0(0x00) {
            self.tx_queue.push_back(b); // Type 0 IDLE
        }
        self.expected_resp = 2;
        self.awaiting = false;
        self.link_state = IolinkLinkState::Startup;
    }

    fn queue_first_operate(&mut self) {
        // Fire-and-forget OPERATE transition (no response), then first cyclic request.
        for b in encode_type0(0x0F) {
            self.tx_queue.push_back(b);
        }
        for b in encode_type1_cycle(&[]) {
            self.tx_queue.push_back(b);
        }
        self.expected_resp = self.operate_response_len();
        self.awaiting = false;
        self.link_state = IolinkLinkState::Operate;
    }

    fn queue_next_operate(&mut self) {
        for b in encode_type1_cycle(&[]) {
            self.tx_queue.push_back(b);
        }
        self.expected_resp = self.operate_response_len();
        self.awaiting = false;
    }

    /// Called when a full response has been received; advance the state machine.
    fn handle_response(&mut self) {
        match self.link_state {
            IolinkLinkState::Startup => {
                // PREOPERATE acknowledged → move to OPERATE.
                self.queue_first_operate();
            }
            IolinkLinkState::Operate => {
                let resp = decode_operate(&self.rx_accum, self.pd_in_len, self.od_len);
                if resp.checksum_ok && resp.pd_valid {
                    self.latest_pd = resp.pd;
                    self.pd_valid = true;
                }
                self.queue_next_operate();
            }
        }
        self.rx_accum.clear();
    }
}

impl UartStreamDevice for IolinkMaster {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        if self.awaiting || self.tx_queue.is_empty() {
            return None;
        }
        let byte = self.tx_queue.pop_front();
        if self.tx_queue.is_empty() && self.expected_resp > 0 {
            self.awaiting = true;
        }
        byte
    }

    fn on_tx_byte(&mut self, byte: u8) {
        if !self.awaiting {
            return;
        }
        self.rx_accum.push(byte);
        if self.rx_accum.len() >= self.expected_resp {
            self.handle_response();
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // DI hub: pd_out_len = 0, od_len = 1 → [MC, CKT, OD, CK]
        assert_eq!(encode_type1_cycle(&[]), vec![0x00, 0x00, 0x00, 0x09]);
    }

    #[test]
    fn decodes_operate_response_and_extracts_pd() {
        // [status=0x20 (PD valid), PD=0xA5, OD=0x00, CK=0x0D]
        let resp = decode_operate(&[0x20, 0xA5, 0x00, 0x0D], 1, 1);
        assert!(resp.checksum_ok);
        assert!(resp.pd_valid);
        assert_eq!(resp.pd, vec![0xA5]);
    }

    #[test]
    fn drives_startup_then_cyclic_operate_and_captures_pd() {
        // DI hub: pd_in_len = 1, od_len = 1, COM2.
        let mut m = IolinkMaster::new(1, 1, IolinkComSpeed::Com2);

        // Startup: master emits wake-up + Type 0 IDLE, then awaits a 2-byte response.
        let mut req = Vec::new();
        while let Some(b) = m.poll(1000) {
            req.push(b);
        }
        assert_eq!(req, vec![0x55, 0x00, 0x24]);
        assert_eq!(m.link_state, IolinkLinkState::Startup);

        // Device acknowledges with any 2-byte PREOPERATE response.
        m.on_tx_byte(0x00);
        m.on_tx_byte(0x24);

        // Master fires OPERATE transition then the first cyclic request.
        let mut req2 = Vec::new();
        while let Some(b) = m.poll(1000) {
            req2.push(b);
        }
        assert_eq!(req2, vec![0x0F, 0x0D, 0x00, 0x00, 0x00, 0x09]);
        assert_eq!(m.link_state, IolinkLinkState::Operate);

        // Device responds with PD = 0xA5, valid.
        for b in [0x20u8, 0xA5, 0x00, 0x0D] {
            m.on_tx_byte(b);
        }
        assert_eq!(m.input_byte(), 0xA5);
        assert!(m.pd_valid);

        // Next cyclic request is queued.
        let mut req3 = Vec::new();
        while let Some(b) = m.poll(1000) {
            req3.push(b);
        }
        assert_eq!(req3, vec![0x00, 0x00, 0x00, 0x09]);
    }

    #[test]
    fn master_reaches_operate_through_real_uart() {
        use crate::peripherals::uart::Uart;
        use crate::Peripheral;

        let mut uart = Uart::new(); // Stm32F1
        uart.set_sink(None, false);
        uart.attach_stream(Box::new(IolinkMaster::new(1, 1, IolinkComSpeed::Com2)));

        // Helper: simulate firmware transmitting a byte (Stm32F1 DR alias = 0x00).
        fn fw_tx(uart: &mut Uart, b: u8) {
            uart.write(0x00, b).unwrap();
        }
        // Helper: simulate firmware reading one RX byte if available.
        fn fw_rx(uart: &mut Uart) -> Option<u8> {
            // status offset 0x00 for F1; RXNE = bit 5
            let status = Peripheral::read(uart, 0x00).unwrap();
            if status & (1 << 5) != 0 {
                Some(Peripheral::read(uart, 0x04).unwrap())
            } else {
                None
            }
        }

        let mut master_request = Vec::new();
        let mut acked_preop = false;
        let mut answered_operate = false;

        for _ in 0..200 {
            // 1) advance UART one tick → master.poll pushes a request byte into RX
            let _ = Peripheral::tick(&mut uart);

            // 2) firmware drains any RX byte and records the master's request
            while let Some(b) = fw_rx(&mut uart) {
                master_request.push(b);
            }

            // 3) fake firmware/device logic: respond once each request completes
            if !acked_preop && master_request == vec![0x55, 0x00, 0x24] {
                fw_tx(&mut uart, 0x00);
                fw_tx(&mut uart, 0x24);
                acked_preop = true;
                master_request.clear();
            } else if acked_preop
                && !answered_operate
                && master_request == vec![0x0F, 0x0D, 0x00, 0x00, 0x00, 0x09]
            {
                for b in [0x20u8, 0xA5, 0x00, 0x0D] {
                    fw_tx(&mut uart, b);
                }
                answered_operate = true;
            }
        }

        let master = uart.attached_streams[0]
            .as_any()
            .unwrap()
            .downcast_ref::<IolinkMaster>()
            .unwrap();
        assert_eq!(master.link_state, IolinkLinkState::Operate);
        assert_eq!(master.input_byte(), 0xA5);
        assert!(master.pd_valid);
    }
}
