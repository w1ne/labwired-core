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

/// Encode a Type 1/2 cyclic request:
/// `[MC=0x00, CKT=0x00, PD_out..., OD..., CK]`.
pub(crate) fn encode_type1_cycle(pd_out: &[u8], od_len: usize) -> Vec<u8> {
    let mut frame = vec![0x00u8, 0x00];
    frame.extend_from_slice(pd_out);
    frame.extend(std::iter::repeat(0x00).take(od_len.max(1))); // OD idle bytes
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

#[cfg(feature = "iolink-native")]
mod native {
    use std::ffi::{c_int, c_uint, c_void};
    use std::ptr::NonNull;
    use std::sync::Mutex;

    const IOLINK_MASTER_STATE_STARTUP: c_int = 1;
    const IOLINK_MASTER_STATE_PREOPERATE: c_int = 2;
    const IOLINK_MASTER_STATE_OPERATE: c_int = 3;
    static NATIVE_CALL_LOCK: Mutex<()> = Mutex::new(());

    unsafe extern "C" {
        fn lw_iolm_bridge_new(
            m_seq_type: u8,
            pd_in_len: u8,
            pd_out_len: u8,
            min_cycle_time: u8,
            response_timeout_100us: u8,
        ) -> *mut c_void;
        fn lw_iolm_bridge_free(bridge: *mut c_void);
        fn lw_iolm_bridge_set_pd_out(bridge: *mut c_void, data: *const u8, len: u8) -> c_int;
        fn lw_iolm_bridge_cycle_due(bridge: *mut c_void, now_100us: c_uint) -> c_int;
        fn lw_iolm_bridge_tick_none(bridge: *mut c_void, now_100us: c_uint) -> c_int;
        fn lw_iolm_bridge_feed_rx(bridge: *mut c_void, data: *const u8, len: usize) -> c_int;
        fn lw_iolm_bridge_drain_tx(bridge: *mut c_void, out: *mut u8, out_len: usize) -> usize;
        fn lw_iolm_bridge_state(bridge: *const c_void) -> c_int;
        fn lw_iolm_bridge_get_pd_in(
            bridge: *const c_void,
            out: *mut u8,
            out_len: u8,
            actual_len: *mut u8,
        ) -> c_int;
        #[cfg(test)]
        fn lw_iolm_bridge_wake_count(bridge: *const c_void) -> c_uint;
        #[cfg(test)]
        fn lw_iolm_conformance_run_profile(
            m_seq_type: u8,
            pd_in_len: u8,
            pd_out_len: u8,
            pd_value: u8,
            result: *mut NativeConformanceResult,
        ) -> c_int;
        #[cfg(test)]
        fn lw_iolm_conformance_run_multi_profile(
            port_count: u8,
            m_seq_type: u8,
            pd_in_len: u8,
            pd_out_len: u8,
            first_pd_value: u8,
            results: *mut NativeConformanceResult,
        ) -> c_int;
        #[cfg(test)]
        fn lw_iolm_conformance_run_multi_direct_parameter_isolation(
            values: *mut u8,
            value_count: u8,
        ) -> c_int;
    }

    #[cfg(test)]
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default)]
    pub(crate) struct NativeConformanceResult {
        pub(crate) master_state: c_int,
        pub(crate) pd_in_len: u8,
        pub(crate) pd_out_len: u8,
        pub(crate) pd_in: [u8; 32],
        pub(crate) device_observed_pd_input_len: u8,
        pub(crate) device_observed_pd_input: [u8; 32],
        pub(crate) device_observed_pd_output_len: u8,
        pub(crate) device_observed_pd_output: [u8; 32],
        pub(crate) cycles: u8,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum NativeState {
        Startup,
        Preoperate,
        Operate,
        Error,
    }

    pub(crate) struct NativeIolinkMaster {
        bridge: NonNull<c_void>,
    }

    // The bridge pointer is uniquely owned by this wrapper. Mutating C entry
    // points require `&mut self`; the module-level mutex serializes calls that
    // use the bridge shim's short-lived active-context global.
    unsafe impl Send for NativeIolinkMaster {}

    impl std::fmt::Debug for NativeIolinkMaster {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("NativeIolinkMaster")
                .field("state", &self.state())
                .finish_non_exhaustive()
        }
    }

    impl NativeIolinkMaster {
        pub(crate) fn new(
            m_seq_type: u8,
            pd_in_len: usize,
            pd_out: &[u8],
        ) -> Result<Self, &'static str> {
            if pd_in_len > u8::MAX as usize || pd_out.len() > u8::MAX as usize {
                return Err("IO-Link PD length does not fit native C API");
            }
            let _guard = NATIVE_CALL_LOCK
                .lock()
                .expect("native IO-Link lock poisoned");
            let ptr = unsafe {
                lw_iolm_bridge_new(m_seq_type, pd_in_len as u8, pd_out.len() as u8, 10, 20)
            };
            let bridge = NonNull::new(ptr).ok_or("failed to initialize native IO-Link master")?;
            let mut this = Self { bridge };
            drop(_guard);
            this.set_pd_out(pd_out)?;
            Ok(this)
        }

        pub(crate) fn set_pd_out(&mut self, pd_out: &[u8]) -> Result<(), &'static str> {
            if pd_out.len() > u8::MAX as usize {
                return Err("IO-Link PD out length does not fit native C API");
            }
            let _guard = NATIVE_CALL_LOCK
                .lock()
                .expect("native IO-Link lock poisoned");
            let ret = unsafe {
                lw_iolm_bridge_set_pd_out(self.bridge.as_ptr(), pd_out.as_ptr(), pd_out.len() as u8)
            };
            if ret == 0 {
                Ok(())
            } else {
                Err("native IO-Link set_pd_out failed")
            }
        }

        pub(crate) fn cycle_due(&mut self, now_100us: u32) -> Result<(), &'static str> {
            let _guard = NATIVE_CALL_LOCK
                .lock()
                .expect("native IO-Link lock poisoned");
            let ret = unsafe { lw_iolm_bridge_cycle_due(self.bridge.as_ptr(), now_100us) };
            if ret >= 0 {
                Ok(())
            } else {
                Err("native IO-Link cycle tick failed")
            }
        }

        pub(crate) fn tick_none(&mut self, now_100us: u32) -> Result<i32, &'static str> {
            let _guard = NATIVE_CALL_LOCK
                .lock()
                .expect("native IO-Link lock poisoned");
            let ret = unsafe { lw_iolm_bridge_tick_none(self.bridge.as_ptr(), now_100us) };
            if ret >= 0 {
                Ok(ret)
            } else {
                Err("native IO-Link poll tick failed")
            }
        }

        pub(crate) fn feed_rx(&mut self, data: &[u8]) -> Result<(), &'static str> {
            let ret =
                unsafe { lw_iolm_bridge_feed_rx(self.bridge.as_ptr(), data.as_ptr(), data.len()) };
            if ret == 0 {
                Ok(())
            } else {
                Err("native IO-Link RX queue overflow")
            }
        }

        pub(crate) fn drain_tx(&mut self) -> Vec<u8> {
            let mut out = [0u8; 128];
            let n = unsafe {
                lw_iolm_bridge_drain_tx(self.bridge.as_ptr(), out.as_mut_ptr(), out.len())
            };
            out[..n].to_vec()
        }

        pub(crate) fn state(&self) -> NativeState {
            match unsafe { lw_iolm_bridge_state(self.bridge.as_ptr()) } {
                IOLINK_MASTER_STATE_STARTUP => NativeState::Startup,
                IOLINK_MASTER_STATE_PREOPERATE => NativeState::Preoperate,
                IOLINK_MASTER_STATE_OPERATE => NativeState::Operate,
                _ => NativeState::Error,
            }
        }

        pub(crate) fn pd_in(&self, max_len: usize) -> Option<Vec<u8>> {
            let mut out = vec![0u8; max_len.min(u8::MAX as usize)];
            let mut actual_len = 0u8;
            let ret = unsafe {
                lw_iolm_bridge_get_pd_in(
                    self.bridge.as_ptr(),
                    out.as_mut_ptr(),
                    out.len() as u8,
                    &mut actual_len,
                )
            };
            if ret == 0 {
                out.truncate(actual_len as usize);
                Some(out)
            } else {
                None
            }
        }

        #[cfg(test)]
        pub(crate) fn wake_count(&self) -> u32 {
            unsafe { lw_iolm_bridge_wake_count(self.bridge.as_ptr()) as u32 }
        }
    }

    impl Drop for NativeIolinkMaster {
        fn drop(&mut self) {
            unsafe { lw_iolm_bridge_free(self.bridge.as_ptr()) };
        }
    }

    #[cfg(test)]
    pub(crate) fn run_real_device_stack_profile(
        m_seq_type: u8,
        pd_in_len: u8,
        pd_out_len: u8,
        pd_value: u8,
    ) -> Result<NativeConformanceResult, &'static str> {
        let _guard = NATIVE_CALL_LOCK
            .lock()
            .expect("native IO-Link lock poisoned");
        let mut result = NativeConformanceResult::default();
        let ret = unsafe {
            lw_iolm_conformance_run_profile(
                m_seq_type,
                pd_in_len,
                pd_out_len,
                pd_value,
                &mut result,
            )
        };
        if ret == 0 {
            Ok(result)
        } else {
            Err("native IO-Link real device-stack profile failed")
        }
    }

    #[cfg(test)]
    pub(crate) fn run_real_multi_device_stack_profile<const N: usize>(
        m_seq_type: u8,
        pd_in_len: u8,
        pd_out_len: u8,
        first_pd_value: u8,
    ) -> Result<[NativeConformanceResult; N], &'static str> {
        if N == 0 || N > u8::MAX as usize {
            return Err("invalid native IO-Link port count");
        }
        let _guard = NATIVE_CALL_LOCK
            .lock()
            .expect("native IO-Link lock poisoned");
        let mut results = [NativeConformanceResult::default(); N];
        let ret = unsafe {
            lw_iolm_conformance_run_multi_profile(
                N as u8,
                m_seq_type,
                pd_in_len,
                pd_out_len,
                first_pd_value,
                results.as_mut_ptr(),
            )
        };
        if ret == 0 {
            Ok(results)
        } else {
            Err("native IO-Link multi-device profile failed")
        }
    }

    #[cfg(test)]
    pub(crate) fn run_real_multi_device_direct_parameter_isolation() -> Result<[u8; 2], &'static str>
    {
        let _guard = NATIVE_CALL_LOCK
            .lock()
            .expect("native IO-Link lock poisoned");
        let mut values = [0u8; 2];
        let ret = unsafe {
            lw_iolm_conformance_run_multi_direct_parameter_isolation(
                values.as_mut_ptr(),
                values.len() as u8,
            )
        };
        if ret == 0 {
            Ok(values)
        } else {
            Err("native IO-Link multi-device direct parameter isolation failed")
        }
    }
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
    /// Process-data output bytes sent by the simulated master on cyclic frames.
    pd_out: Vec<u8>,
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
    #[cfg(feature = "iolink-native")]
    #[serde(skip)]
    native: Option<native::NativeIolinkMaster>,
    #[cfg(feature = "iolink-native")]
    #[serde(skip)]
    native_now_100us: u32,
}

impl IolinkMaster {
    pub fn new(pd_in_len: usize, od_len: usize, com: IolinkComSpeed) -> Self {
        Self::new_with_pd_out(pd_in_len, od_len, com, Vec::new())
    }

    pub fn new_with_pd_out(
        pd_in_len: usize,
        od_len: usize,
        com: IolinkComSpeed,
        pd_out: Vec<u8>,
    ) -> Self {
        let mut m = Self {
            pd_in_len,
            od_len: od_len.max(1),
            com,
            link_state: IolinkLinkState::Startup,
            tx_queue: VecDeque::new(),
            rx_accum: Vec::new(),
            step: 0,
            gap_ticks: 0,
            latest_pd: vec![0u8; pd_in_len.max(1)],
            pd_out,
            pd_valid: false,
            trace: VecDeque::new(),
            current: None,
            frame_seq: 0,
            #[cfg(feature = "iolink-native")]
            native: None,
            #[cfg(feature = "iolink-native")]
            native_now_100us: 0,
        };
        m.queue_next_frame(); // queue the wake-up immediately
        m
    }

    #[cfg(feature = "iolink-native")]
    pub fn new_with_native_c_master(
        pd_in_len: usize,
        od_len: usize,
        com: IolinkComSpeed,
        pd_out: Vec<u8>,
        m_seq_type: u8,
    ) -> Self {
        let native = native::NativeIolinkMaster::new(m_seq_type, pd_in_len, &pd_out)
            .expect("initialize native C iolinki-master");
        let mut m = Self::new_with_pd_out(pd_in_len, od_len, com, pd_out);
        m.tx_queue.clear();
        m.rx_accum.clear();
        m.trace.clear();
        m.current = None;
        m.step = 0;
        m.gap_ticks = 0;
        m.frame_seq = 0;
        m.link_state = IolinkLinkState::Startup;
        m.native = Some(native);
        m.native_now_100us = 0;
        m.queue_next_frame();
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

        #[cfg(feature = "iolink-native")]
        if self.native.is_some() {
            self.queue_next_native_frame();
            return;
        }

        let idle_end = 1 + IDLE_FRAMES; // steps [1..=IDLE_FRAMES] are IDLE
        let (frame, kind): (Vec<u8>, IolinkFrameKind) = if self.step == 0 {
            (vec![0x55], IolinkFrameKind::WakeUp) // wake-up pulse (once)
        } else if self.step < idle_end {
            (encode_type0(0x00), IolinkFrameKind::Idle) // Type 0 IDLE → PREOPERATE
        } else if self.step == idle_end {
            (encode_type0(0x0F), IolinkFrameKind::OperateReq) // OPERATE transition
        } else {
            self.link_state = IolinkLinkState::Operate;
            (
                encode_type1_cycle(&self.pd_out, self.od_len),
                IolinkFrameKind::Cyclic,
            ) // cyclic Type 1/2
        };

        let pd_out = if matches!(kind, IolinkFrameKind::Cyclic) {
            self.pd_out.clone()
        } else {
            Vec::new()
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
        });
        self.frame_seq = self.frame_seq.wrapping_add(1);

        // Hold `step` at the first cyclic index so it keeps repeating Type 1.
        if self.step <= idle_end {
            self.step += 1;
        }
    }

    #[cfg(feature = "iolink-native")]
    fn queue_next_native_frame(&mut self) {
        let native = self.native.as_mut().expect("native IO-Link master");
        native
            .cycle_due(self.native_now_100us)
            .expect("native IO-Link cycle due");
        self.native_now_100us = self.native_now_100us.wrapping_add(20);
        let frame = native.drain_tx();
        if frame.is_empty() {
            return;
        }

        self.link_state = match native.state() {
            native::NativeState::Operate => IolinkLinkState::Operate,
            _ => IolinkLinkState::Startup,
        };
        let kind = match frame.as_slice() {
            [0x55] => IolinkFrameKind::WakeUp,
            [0x00, 0x24] => IolinkFrameKind::Idle,
            [0x0F, 0x0D] => IolinkFrameKind::OperateReq,
            _ => IolinkFrameKind::Cyclic,
        };
        let pd_out = if matches!(kind, IolinkFrameKind::Cyclic) {
            self.pd_out.clone()
        } else {
            Vec::new()
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
        });
        self.frame_seq = self.frame_seq.wrapping_add(1);
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
        #[cfg(feature = "iolink-native")]
        if let Some(native) = self.native.as_mut() {
            native.feed_rx(&[byte]).expect("feed native IO-Link RX");
            let _ = native
                .tick_none(self.native_now_100us)
                .expect("poll native IO-Link RX");
            if native.state() == native::NativeState::Operate {
                self.link_state = IolinkLinkState::Operate;
                if let Some(pd) = native.pd_in(self.pd_in_len) {
                    self.latest_pd = pd;
                    self.pd_valid = true;
                    self.rx_accum.clear();
                }
            }
            return;
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
        ConfigKey {
            name: "pd_out_hex",
            ty: ConfigType::Str,
            doc: "Optional process-data output bytes as hex, e.g. \"11 22\".",
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
        let pd_out = parse_pd_out_hex(ctx.config_str("pd_out_hex")).unwrap_or_default();
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
        #[cfg(feature = "iolink-native")]
        {
            uart.attach_stream(Box::new(IolinkMaster::new_with_native_c_master(
                pd_in_len,
                od_len,
                com,
                pd_out,
                m_seq_type as u8,
            )));
            Ok(())
        }
        #[cfg(not(feature = "iolink-native"))]
        {
            uart.attach_stream(Box::new(IolinkMaster::new_with_pd_out(
                pd_in_len, od_len, com, pd_out,
            )));
            Ok(())
        }
    }
}

fn parse_pd_out_hex(value: Option<&str>) -> Option<Vec<u8>> {
    let value = value?.trim();
    if value.is_empty() {
        return Some(Vec::new());
    }

    let mut out = Vec::new();
    for token in value.split(|c: char| c.is_ascii_whitespace() || c == ',' || c == ':') {
        if token.is_empty() {
            continue;
        }
        let token = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
            .unwrap_or(token);
        let byte = u8::from_str_radix(token, 16).ok()?;
        out.push(byte);
    }

    Some(out)
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
        assert_eq!(encode_type1_cycle(&[], 1), vec![0x00, 0x00, 0x00, 0x09]);
    }

    #[test]
    fn parses_pd_out_hex_config() {
        assert_eq!(
            parse_pd_out_hex(Some("0x11 22,33:44")),
            Some(vec![0x11, 0x22, 0x33, 0x44])
        );
        assert_eq!(parse_pd_out_hex(Some("")), Some(Vec::new()));
        assert_eq!(parse_pd_out_hex(Some("not-hex")), None);
        assert_eq!(parse_pd_out_hex(None), None);
    }

    #[test]
    fn cyclic_schedule_includes_configured_pd_out_and_two_byte_od() {
        let mut m = IolinkMaster::new_with_pd_out(2, 2, IolinkComSpeed::Com2, vec![0x11, 0x22]);

        while m.link_state != IolinkLinkState::Operate {
            drain(&mut m);
        }

        let frame = drain(&mut m);
        let expected = encode_type1_cycle(&[0x11, 0x22], 2);
        assert_eq!(frame, expected);
        let trace = m.trace_snapshot();
        let cyclic = trace
            .iter()
            .find(|x| x.kind == IolinkFrameKind::Cyclic)
            .expect("cyclic trace record");
        assert_eq!(cyclic.pd_out, vec![0x11, 0x22]);
        assert_eq!(cyclic.raw_master, expected);
    }

    #[cfg(feature = "iolink-native")]
    #[test]
    fn native_c_master_walks_startup_transition_and_cyclic_exchange() {
        use super::native::{NativeIolinkMaster, NativeState};

        let pd_out = [0x11, 0x22];
        let mut native =
            NativeIolinkMaster::new(4, 4, &pd_out).expect("native C IO-Link master init");

        native.cycle_due(0).expect("wake-up tick");
        assert_eq!(native.drain_tx(), vec![0x55]);
        assert_eq!(native.wake_count(), 1);
        assert_eq!(native.state(), NativeState::Startup);

        native.cycle_due(20).expect("idle tick");
        assert_eq!(native.drain_tx(), vec![0x00, 0x24]);
        native
            .feed_rx(&[0x00, 0x24])
            .expect("type0 startup response");
        native.tick_none(21).expect("startup RX poll");
        assert_eq!(native.state(), NativeState::Preoperate);

        native.cycle_due(40).expect("operate transition tick");
        assert_eq!(native.drain_tx(), vec![0x0F, 0x0D]);
        assert_eq!(native.state(), NativeState::Operate);

        native.cycle_due(60).expect("cyclic tick");
        assert_eq!(native.drain_tx(), encode_type1_cycle(&pd_out, 2));

        let mut response = vec![0x20, 0xAA, 0xBB, 0xCC, 0xDD, 0x00, 0x00];
        response.push(crc6(&response));
        native.feed_rx(&response).expect("operate response");
        assert_eq!(native.tick_none(61).expect("operate RX poll"), 1);
        assert_eq!(native.pd_in(4), Some(vec![0xAA, 0xBB, 0xCC, 0xDD]));
    }

    #[cfg(feature = "iolink-native")]
    #[test]
    fn native_stream_uses_c_master_at_uart_boundary() {
        let pd_out = vec![0x11, 0x22];
        let mut m =
            IolinkMaster::new_with_native_c_master(4, 2, IolinkComSpeed::Com2, pd_out.clone(), 4);

        assert_eq!(drain(&mut m), vec![0x55]);
        assert_eq!(drain(&mut m), vec![0x00, 0x24]);
        for b in [0x00, 0x24] {
            m.on_tx_byte(b);
        }
        assert_eq!(drain(&mut m), vec![0x0F, 0x0D]);

        let cyclic = drain(&mut m);
        assert_eq!(cyclic, encode_type1_cycle(&pd_out, 2));
        assert_eq!(m.link_state, IolinkLinkState::Operate);

        let mut response = vec![0x20, 0xAA, 0xBB, 0xCC, 0xDD, 0x00, 0x00];
        response.push(crc6(&response));
        for b in response {
            m.on_tx_byte(b);
        }
        assert_eq!(m.input_byte(), 0xAA);
        assert!(m.pd_valid);
    }

    #[cfg(feature = "iolink-native")]
    #[test]
    fn native_real_device_stack_profiles_exchange_without_scripted_responses() {
        use super::native::run_real_device_stack_profile;

        let cases = [
            (1u8, 1u8, 0u8, 0x11u8),
            (2u8, 2u8, 1u8, 0x22u8),
            (4u8, 2u8, 2u8, 0x33u8),
            (5u8, 3u8, 2u8, 0x44u8),
            (3u8, 4u8, 1u8, 0x55u8),
            (6u8, 4u8, 3u8, 0x66u8),
        ];

        for (m_seq_type, pd_in_len, pd_out_len, pd_value) in cases {
            let result = run_real_device_stack_profile(m_seq_type, pd_in_len, pd_out_len, pd_value)
                .expect("real iolinki device-stack profile");
            assert_eq!(result.master_state, 3);
            assert_eq!(result.pd_in_len, pd_in_len);
            assert_eq!(result.device_observed_pd_input_len, pd_in_len);
            assert_eq!(result.device_observed_pd_output_len, pd_out_len);
            assert!(
                result.cycles > 0,
                "profile should require at least one real cycle"
            );

            for i in 0..pd_in_len as usize {
                assert_eq!(result.pd_in[i], pd_value.wrapping_add(i as u8));
                assert_eq!(
                    result.device_observed_pd_input[i],
                    pd_value.wrapping_add(i as u8)
                );
            }
            for i in 0..pd_out_len as usize {
                assert_eq!(
                    result.device_observed_pd_output[i],
                    (pd_value ^ 0x55).wrapping_add(i as u8)
                );
            }
        }
    }

    #[cfg(feature = "iolink-native")]
    #[test]
    fn native_real_device_stack_runs_two_isolated_devices() {
        use super::native::run_real_multi_device_stack_profile;

        let results = run_real_multi_device_stack_profile::<2>(4, 2, 2, 0x31)
            .expect("real multi-device native IO-Link profile");

        for (port_idx, result) in results.iter().enumerate() {
            let pd_value = 0x31u8.wrapping_add((port_idx as u8) * 0x10);
            assert_eq!(result.master_state, 3, "port {port_idx} should reach OPERATE");
            assert_eq!(result.pd_in_len, 2);
            assert_eq!(result.pd_out_len, 2);
            assert_eq!(result.device_observed_pd_input_len, 2);
            assert_eq!(result.device_observed_pd_output_len, 2);
            assert!(result.cycles > 0);

            for i in 0..2 {
                assert_eq!(result.pd_in[i], pd_value.wrapping_add(i as u8));
                assert_eq!(
                    result.device_observed_pd_input[i],
                    pd_value.wrapping_add(i as u8)
                );
                assert_eq!(
                    result.device_observed_pd_output[i],
                    (pd_value ^ 0x55).wrapping_add(i as u8)
                );
            }
        }
        assert_ne!(results[0].pd_in[0], results[1].pd_in[0]);
        assert_ne!(
            results[0].device_observed_pd_output[0],
            results[1].device_observed_pd_output[0]
        );
    }

    #[cfg(feature = "iolink-native")]
    #[test]
    fn native_real_device_stack_isolates_direct_parameters_per_device() {
        use super::native::run_real_multi_device_direct_parameter_isolation;

        let values = run_real_multi_device_direct_parameter_isolation()
            .expect("real multi-device direct parameter isolation profile");

        assert_eq!(values, [0xA1, 0xB2]);
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
            raw_master: encode_type1_cycle(&[], 1),
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
            raw_master: encode_type1_cycle(&[0], 1),
            raw_device: vec![0x20],
        };
        let x = m.finalize_xfer(p);
        assert_eq!(x.ck_ok, None);
        assert_eq!(x.pd_valid, Some(false));
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
