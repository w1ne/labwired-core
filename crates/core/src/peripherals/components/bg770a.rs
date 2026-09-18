// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Quectel BG770A-GL cellular modem (LTE-M / NB-IoT, Cat-M1 / Cat-NB2).
//!
//! The BG770A talks AT commands over a 115200 8N1 UART (default rate). Firmware
//! TXes a line terminated by `\r`; the modem echoes each byte as it arrives
//! (when echo is enabled — ATE1, the default after reset), then emits the
//! response on a new line, terminated by `\r\nOK\r\n` or `\r\nERROR\r\n`.
//!
//! Modelled from two sources:
//!   1. The official **BG77xA-GL & BG95xA-GL AT Commands Manual V1.3** —
//!      response shapes, parameter ranges, error codes, and per-command
//!      "Maximum Response Time" timing all come straight from this PDF
//!      (`core/crates/core/tests/fixtures/quectel_bg770a/datasheet/`).
//!   2. A real-hardware capture from a BG770A-GL EVB running firmware
//!      `BG770AGLAAR01A05`
//!      (`core/crates/core/tests/fixtures/quectel_bg770a/at_harvest.log`).
//!      Identity strings and the small Quectel quirks the manual doesn't
//!      document (e.g. `AT+COPS=?` returning `+CME ERROR: 515` when unattached)
//!      are taken from there.
//!
//! Echo is emitted instantly as bytes arrive. Command **responses are delayed
//! by the documented per-command max response time** (e.g. 300 ms for most
//! commands, 5 s for `AT+CPIN=`, 15 s for `AT+CFUN=`) — firmware that polls
//! the UART before the deadline sees zero bytes, matching the chip. After a
//! `CFUN=0→1` transition the modem emits the boot URC chain (`+CPIN: READY`,
//! `+QUSIM`, `+QIND: SMS DONE`, `+QIND: PB DONE`) with realistic spacing.
//! Power-on boot URCs (starting with `RDY`) are opt-in via [`with_boot_urcs`].
//!
//! Out of scope: full socket/TLS/MQTT/HTTP stack (`AT+QIOPEN`, `AT+QMTOPEN`,
//! `AT+QHTTPGET`), GPS (`AT+QGPS*`), SMS, AT% Sequans-extension surface and
//! AT+VZ Verizon extension. Those commands return `ERROR` so probing firmware
//! sees a deterministic miss instead of a lie.

use crate::network::SimMqttFabric;
use crate::peripherals::rf_medium::{NodePosition, PathLossParams, RfMedium};
use crate::peripherals::uart::UartStreamDevice;
use std::any::Any;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

mod at;

/// Synthetic cell-tower node id in the shared [`RfMedium`] (path-loss peer).
const CELL_TOWER_NODE: &str = "cell";
/// Assumed downlink TX power (dBm) used to map distance → RSSI when the medium
/// has no per-frame TX. Co-located (range 0) → 0 dBm → CSQ 31.
const CELL_TX_POWER_DBM: f64 = 0.0;

/// Map path-loss RSSI (dBm) → AT+CSQ steps (3GPP 27.007: 0 = −113 dBm, 31 = −51 dBm).
fn csq_from_dbm(dbm: f64) -> u8 {
    if !dbm.is_finite() {
        return 99;
    }
    let steps = ((dbm + 113.0) / 2.0).round() as i32;
    steps.clamp(0, 31) as u8
}

/// Default identity for a real BG770A-GL on the bench.
const ID_MANUFACTURER: &str = "Quectel";
const ID_MODEL: &str = "BG770A-GL";
const ID_FIRMWARE: &str = "BG770AGLAAR01A05";
/// Fake IMEI — same digit count and Luhn-valid layout as a real Quectel one,
/// but obviously synthetic so it can't collide with a real device.
const FAKE_IMEI: &str = "860000000000007";
/// Fake ICCID / IMSI for the simulated SIM. ICCID begins `89` per ITU-T E.118.
const FAKE_ICCID: &str = "8900000000000000000F";
const FAKE_IMSI: &str = "001010000000001";

// ---- Per-command "Maximum Response Time" values from the AT Commands Manual.
// These are upper bounds the modem may take to emit a response after the
// terminating \r. The model treats each as the *actual* response delay,
// which matches "deterministic worst case" behaviour for firmware testing.

/// 300 ms — the manual's default for almost every command.
const DELAY_DEFAULT_US: u32 = 300_000;
/// `AT+CPIN=<pin>` and other facility-lock writes (Section 5.x): 5 s.
const DELAY_CPIN_WRITE_US: u32 = 5_000_000;
/// `AT+CFUN=<fun>` (Section 2.21): "15 s, determined by the network."
const DELAY_CFUN_WRITE_US: u32 = 15_000_000;
/// `AT+COPS=<mode>...` (Section 3.x): "180 s, determined by the network."
const DELAY_COPS_WRITE_US: u32 = 180_000_000;
/// `AT+CGATT=<state>` (Section 8.x): "140 s, determined by the network."
const DELAY_CGATT_WRITE_US: u32 = 140_000_000;
/// `AT+CGACT=<state>,<cid>` (Section 8.x): "150 s, determined by the network."
const DELAY_CGACT_WRITE_US: u32 = 150_000_000;

// ---- Boot URC chain timing.
// Real BG770A emits these in roughly this order after power-on or after
// `AT+CFUN=1` from a powered-down state. Inter-event spacing is approximate
// and based on real-hardware observation, not the manual.
const URC_DELAY_RDY_US: u32 = 1_500_000;
const URC_DELAY_CPIN_READY_US: u32 = 1_000_000;
const URC_DELAY_QUSIM_US: u32 = 200_000;
const URC_DELAY_SMS_DONE_US: u32 = 1_800_000;
const URC_DELAY_PB_DONE_US: u32 = 500_000;

/// One scheduled chunk of bytes the modem will emit after `remaining_us`
/// elapses (counted from the moment the previous chunk finishes draining).
#[derive(Debug)]
struct ScheduledChunk {
    remaining_us: u32,
    bytes: Vec<u8>,
}

/// Lifecycle of a Quectel MQTT client (one per `client_idx` 0..=5).
///
/// State machine matches the BG770A's AT-level model:
///   `Closed` —`AT+QMTOPEN`→ `Initialized` —`AT+QMTCONN`→ `Connected`
///   `Connected` —`AT+QMTDISC`→ `Initialized` —`AT+QMTCLOSE`→ `Closed`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
enum MqttState {
    #[default]
    Closed,
    Initialized,
    Connected,
}

/// Distinguishes which HTTP write command put the modem into the `CONNECT`
/// prompt mode, so we know how to react when the firmware has finished
/// streaming bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpPromptKind {
    Url,
    PostBody,
}

/// Quectel raw-socket lifecycle. Real BG770A supports connectIDs 0..=11.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
enum SocketState {
    #[default]
    Closed,
    Open,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct Socket {
    state: SocketState,
    /// "TCP", "UDP", "TCP LISTENER", "UDP SERVICE".
    service_type: String,
    remote_host: String,
    remote_port: u16,
    /// Bytes available to be read via `AT+QIRD`. Test helpers (or future URC
    /// injection) push here; firmware drains with QIRD.
    #[serde(skip)]
    rx_buffer: Vec<u8>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct MqttClient {
    state: MqttState,
    /// Last publish message id allocated to this client; cycles 1..=65535.
    next_msgid: u16,
    /// `+QMTCFG="ssl",<id>,1,<ctxid>` toggles SSL on. When `false`, the
    /// client uses plain MQTT (port 1883 typical); when `true`, MQTT-over-
    /// TLS using SSL context `ssl_ctxid` (port 8883 typical).
    ssl_enabled: bool,
    ssl_ctxid: u8,
    /// Broker host from last successful `AT+QMTOPEN`.
    #[serde(skip)]
    broker_host: String,
    /// Broker port from last successful `AT+QMTOPEN`.
    broker_port: u16,
}

/// Strip a leading `"<key>"` from `s`, then split the rest on commas. Returns
/// the key and the comma-separated args (with leading comma already consumed).
/// Used for `AT+QSSLCFG="key",arg...` and `AT+QMTCFG="key",arg...` parsing
/// where the subkey is quoted and the args may contain numbers or strings.
fn parse_quoted_subkey(s: &str) -> Option<(&str, Vec<&str>)> {
    let s = s.trim();
    let s = s.strip_prefix('"')?;
    let close = s.find('"')?;
    let key = &s[..close];
    let after = s[close + 1..].trim_start_matches(',');
    let args: Vec<&str> = if after.is_empty() {
        Vec::new()
    } else {
        after.split(',').map(|t| t.trim()).collect()
    };
    Some((key, args))
}

/// Per-command delays for the Quectel TCP/IP and MQTT sub-surface.
/// Values come from the BG77xA-GL AT Commands Manual §10 (Internet AT
/// commands) and the dedicated Quectel MQTT Application Note.
const DELAY_QIACT_US: u32 = 150_000_000; // up to 150 s, "determined by network"
const DELAY_QMTOPEN_US: u32 = 75_000_000; // up to 75 s
const DELAY_QMTCONN_US: u32 = 5_000_000; // up to 5 s
/// Delay from `AT+QMTOPEN=...` write OK to the async `+QMTOPEN: <id>,<r>` URC
/// when the broker responds promptly. Real-hardware: ~1.5 s for a healthy
/// broker, longer when DNS or TLS handshake is slow.
const URC_DELAY_QMTOPEN_US: u32 = 1_500_000;
const URC_DELAY_QMTCONN_US: u32 = 800_000;
const URC_DELAY_QMTPUB_US: u32 = 400_000;
const URC_DELAY_QMTDISC_US: u32 = 300_000;
/// `AT+QIOPEN` max response time (datasheet §10.x): 150 s.
const DELAY_QIOPEN_US: u32 = 150_000_000;
/// `AT+QICLOSE` max response time: 10 s.
const DELAY_QICLOSE_US: u32 = 10_000_000;
/// Async `+QIOPEN: <id>,<r>` URC after the sync OK clears.
const URC_DELAY_QIOPEN_US: u32 = 1_500_000;
/// Async `+QIURC: "dnsgip",...` URC after `AT+QIDNSGIP=` is issued.
const URC_DELAY_QIDNS_US: u32 = 3_000_000;

#[derive(Debug, serde::Serialize)]
pub struct QuectelBg770a {
    /// AT command echo. Reset default is ON (ATE1), matching real hardware.
    echo: bool,
    /// Quiet mode (ATQ): when true, the modem suppresses final result codes.
    quiet: bool,
    /// Verbose mode (ATV): true means textual result codes (`OK`/`ERROR`),
    /// false means numeric (`0`/`4`). Default true.
    verbose: bool,
    /// `+CMEE` mode: 0 disabled, 1 numeric, 2 verbose. Default 0.
    cmee_mode: u8,
    /// `+CFUN` level: 0 minimum, 1 full, 4 airplane. Default 1.
    cfun: u8,
    /// Last `+CEREG` n setting (0..=2,4). Default 0.
    cereg_n: u8,
    /// Last `+CREG` n setting. Default 0.
    creg_n: u8,
    /// Simulated registration status reported by `+CEREG?`. 0..=5 per 3GPP 27.007.
    /// Default 2 = "searching" — matches a freshly-powered modem with no antenna context.
    cereg_stat: u8,
    /// Packet-domain attachment state for `+CGATT?` (0 detached, 1 attached).
    cgatt: u8,
    /// PDP context activation for `+CGACT?` (cid 1 only in this model).
    cgact_cid1: u8,
    /// Single simulated PDP context, reported by `+CGDCONT?`.
    pdp_apn: String,
    pdp_type: String,
    /// Set to true after `AT+QPOWD`: the modem is off and ignores everything.
    powered_off: bool,

    /// `AT+QIACT` activation state for cid 1. `0` deactivated, `1` activated.
    qiact_cid1: u8,
    /// One MQTT client per supported id (0..=5). Real BG770A supports 6 clients.
    mqtt: [MqttClient; 6],
    /// One raw socket per connectID (0..=11). State machine: Closed → Open.
    sockets: [Socket; 12],
    /// Per-client "post-publish payload" mode for raw sockets (mirrors the
    /// QMTPUB design): tracks `(connect_id, requested_length)`.
    #[serde(skip)]
    awaiting_qisend_payload: Option<(u8, usize)>,
    #[serde(skip)]
    qisend_payload_buf: Vec<u8>,
    /// `AT+CMGS` prompt mode: firmware sends the SMS body (text mode) or PDU
    /// (PDU mode) until Ctrl-Z; modem replies with `+CMGS: <mr>` + OK.
    #[serde(skip)]
    awaiting_cmgs_payload: bool,
    #[serde(skip)]
    cmgs_payload_buf: Vec<u8>,
    /// Monotonic message-reference counter for `+CMGS: <mr>`. Wraps at 256.
    cmgs_mr: u8,
    /// In-memory user-file-system (UFS) backing for `AT+QFLST` / QFUPL /
    /// QFDWL / QFDEL. Key is the filename (no path on real HW); value is the
    /// raw file body.
    filesystem: BTreeMap<String, Vec<u8>>,
    /// Open file handles from `AT+QFOPEN`. Key is the handle id (1..=N), value
    /// is `(filename, current_offset, mode_flags)`. Handle 0 is reserved as
    /// "invalid"; real HW seems to allocate from 1.
    open_files: BTreeMap<u16, (String, usize, u8)>,
    next_file_handle: u16,
    /// Seed CSQ when no RfMedium is driving quality (or as fallback). Defaults
    /// to 99,99 (no service). Prefer [`Self::effective_csq`] for AT replies.
    csq_rssi: u8,
    csq_ber: u8,
    /// When set, forces CSQ steps and bypasses path-loss (scripted demos).
    /// Cleared by the `range_m` SimInput so geometry wins again.
    csq_override: Option<u8>,
    /// Distance (m) from this UE to the synthetic cell tower in the shared
    /// [`RfMedium`]. Drive with SimInput `range_m` — same physics as air path loss.
    range_m: f64,
    /// Shared medium slot with VirtualAirBus / lab AirBus (path loss + positions).
    #[serde(skip)]
    medium: Arc<Mutex<Option<RfMedium>>>,
    /// RfMedium node id (defaults to external_devices id, else `"modem"`).
    #[serde(skip)]
    rf_node_id: Option<String>,
    /// system.yaml `external_devices` id for SimInput discovery (`modem`, …).
    #[serde(skip)]
    component_id: Option<String>,
    /// QGPSCFG sub-key state. Real HW persists these across reboot; we keep
    /// just the values exposed by the bench-captured read forms.
    qgps_outport: String,
    qgps_outport_baud: u32,
    qgps_autogps: u8,
    qgps_nmeasrc: u8,
    qgps_gnssconfig: u8,
    /// QFUPL CONNECT-prompt mode: tracks `(filename, expected_size)`.
    #[serde(skip)]
    awaiting_qfupl: Option<(String, usize)>,
    #[serde(skip)]
    qfupl_buf: Vec<u8>,
    /// `+CCLK?` real-time clock string in the `yy/MM/dd,hh:mm:ss±zz` shape
    /// the manual documents. Defaults to the "never set" 1970-era value the
    /// real chip emits before NTP sync.
    cclk: String,
    /// HTTP CONNECT-prompt mode: when set, every TX byte goes into
    /// `http_data_buf` until `expected_len` bytes have been received. Then we
    /// either store the URL (for `QHTTPURL`) or send the POST async URC.
    #[serde(skip)]
    awaiting_http_data: Option<(HttpPromptKind, usize)>,
    #[serde(skip)]
    http_data_buf: Vec<u8>,
    /// URL most recently set via `AT+QHTTPURL`. Surfaces in `AT+QHTTPURL?` if
    /// we ever model the read form (currently not in the captured surface).
    http_url: String,
    /// Body the model returns from `AT+QHTTPREAD` after a successful GET.
    /// Tests can override via `set_http_response`.
    http_response_code: u16,
    http_response_body: Vec<u8>,
    /// SSL security level per SSL context (id 0..=5). 0 = no auth, 1 = server
    /// auth, 2 = mutual auth. Default 0.
    ssl_seclevel: [u8; 6],
    /// `+QGPS=1` toggles the GNSS engine on; many GPS commands gate on this.
    gps_active: bool,
    /// `+CMGF` message format (0 = PDU, 1 = text). Default 0 (PDU).
    cmgf: u8,
    /// `+CSCS` character set; default "GSM".
    cscs: String,
    /// `+QSCLK` sleep mode (0 disabled, 1 enabled, 2 enabled-deep). Default 0.
    qsclk: u8,
    /// `+CPSMS` power-saving-mode enable state.
    cpsms_mode: u8,
    /// `+CEDRXS` extended DRX state.
    cedrxs_mode: u8,
    /// Per-client "post-publish payload" mode: when in this mode, the modem
    /// is waiting for the firmware to send the QMTPUB payload (followed by
    /// 0x1A / Ctrl-Z to terminate). Tracks `(client_id, msg_id, topic)`.
    #[serde(skip)]
    awaiting_qmtpub_payload: Option<(u8, u16, String)>,
    /// Accumulator for QMTPUB payload bytes received while in payload mode.
    #[serde(skip)]
    qmtpub_payload_buf: Vec<u8>,
    /// Simulated MQTT topic fabric (AirBus.cellular). Not a real broker.
    #[serde(skip)]
    mqtt_net: SimMqttFabric,

    /// Per-command response delay set by the current handler; cleared between
    /// commands. Drives the `ScheduledChunk` inserted after the line completes.
    #[serde(skip)]
    current_delay_us: u32,
    /// Set by a handler when it wants the post-OK URC chain (e.g. CFUN=0→1)
    /// to be queued *after* this command's response. on_tx_byte consumes it.
    #[serde(skip)]
    pending_cfun_resume_urcs: bool,
    /// Generic post-response URC bursts: queued by handlers that emit `OK`
    /// followed by an async result line (`AT+QMTOPEN`, `AT+QMTCONN`,
    /// `AT+QMTSUB`, `AT+QMTDISC`, `AT+QMTCLOSE`). Drained by `on_tx_byte`
    /// after the response is scheduled so the OK always lands first.
    #[serde(skip)]
    deferred_urcs: Vec<(u32, Vec<u8>)>,
    /// Accumulator for the response to the in-flight command. Drained into a
    /// `ScheduledChunk` once `handle_line` returns.
    #[serde(skip)]
    respond_buf: Vec<u8>,
    /// Bytes accumulated since last line terminator.
    #[serde(skip)]
    line_buf: Vec<u8>,
    /// FIFO of pending delayed emissions (command responses + URCs).
    #[serde(skip)]
    pending: VecDeque<ScheduledChunk>,
    /// Bytes ready to leave on the UART RX line *now*.
    #[serde(skip)]
    out_queue: VecDeque<u8>,
}

impl Default for QuectelBg770a {
    fn default() -> Self {
        Self::new()
    }
}

impl QuectelBg770a {
    pub fn new() -> Self {
        Self {
            echo: true,
            quiet: false,
            verbose: true,
            cmee_mode: 0,
            cfun: 1,
            cereg_n: 0,
            creg_n: 0,
            cereg_stat: 2,
            cgatt: 0,
            cgact_cid1: 0,
            pdp_apn: String::from("internet"),
            pdp_type: String::from("IP"),
            powered_off: false,
            qiact_cid1: 0,
            mqtt: Default::default(),
            sockets: Default::default(),
            awaiting_qisend_payload: None,
            qisend_payload_buf: Vec::new(),
            awaiting_cmgs_payload: false,
            cmgs_payload_buf: Vec::new(),
            cmgs_mr: 0,
            filesystem: BTreeMap::new(),
            open_files: BTreeMap::new(),
            next_file_handle: 1,
            csq_rssi: 99,
            csq_ber: 99,
            csq_override: None,
            range_m: 0.0,
            medium: Arc::new(Mutex::new(None)),
            rf_node_id: None,
            component_id: None,
            qgps_outport: String::from("uartnmea"),
            qgps_outport_baud: 115200,
            qgps_autogps: 0,
            qgps_nmeasrc: 1,
            qgps_gnssconfig: 1,
            awaiting_qfupl: None,
            qfupl_buf: Vec::new(),
            cclk: String::from("70/01/01,00:00:00+00"),
            awaiting_http_data: None,
            http_data_buf: Vec::new(),
            http_url: String::new(),
            http_response_code: 200,
            http_response_body: b"Hello, HTTP!".to_vec(),
            ssl_seclevel: [0; 6],
            gps_active: false,
            cmgf: 0,
            cscs: String::from("GSM"),
            qsclk: 0,
            cpsms_mode: 0,
            cedrxs_mode: 0,
            awaiting_qmtpub_payload: None,
            qmtpub_payload_buf: Vec::new(),
            // Replaced by attach_lab_air / attach_private_lab_air (one AirBus path).
            mqtt_net: SimMqttFabric::new(),
            current_delay_us: DELAY_DEFAULT_US,
            pending_cfun_resume_urcs: false,
            deferred_urcs: Vec::new(),
            respond_buf: Vec::with_capacity(128),
            line_buf: Vec::with_capacity(128),
            pending: VecDeque::new(),
            out_queue: VecDeque::new(),
        }
    }

    /// Schedule the power-on URC chain (`RDY`, `+CPIN: READY`, `+QUSIM`,
    /// `+QIND: SMS DONE`, `+QIND: PB DONE`) on this fresh modem. Without this,
    /// the model is silent until firmware sends a command — useful for tests
    /// that don't want to deal with boot noise.
    pub fn with_boot_urcs(mut self) -> Self {
        self.schedule_boot_urcs();
        self
    }

    /// Set the simulated registration status reported via `+CEREG?`. When
    /// `+CEREG=1` or `+CEREG=2` has been issued, this also schedules a
    /// `+CEREG: <stat>` URC matching the new state, exactly like real hardware.
    pub fn set_registration(&mut self, stat: u8) {
        let changed = self.cereg_stat != stat;
        self.cereg_stat = stat;
        if changed && self.cereg_n >= 1 {
            let urc = format!("\r\n+CEREG: {}\r\n", stat);
            self.schedule(DELAY_DEFAULT_US, urc.into_bytes());
        }
    }

    /// Set the APN reported in `+CGDCONT?` (cid 1, IPv4).
    pub fn set_apn(&mut self, apn: impl Into<String>) {
        self.pdp_apn = apn.into();
    }

    /// Set the simulated RTC string returned by `AT+CCLK?`. Format must match
    /// the 3GPP `yy/MM/dd,hh:mm:ss±zz` shape (17+ chars).
    pub fn set_cclk(&mut self, s: impl Into<String>) {
        self.cclk = s.into();
    }

    /// Pre-populate the in-memory filesystem with a file (visible to
    /// `AT+QFLST`, downloadable via `AT+QFDWL`).
    pub fn put_file(&mut self, name: impl Into<String>, data: Vec<u8>) {
        self.filesystem.insert(name.into(), data);
    }

    /// Inject an incoming MQTT publish for the given client. Emits
    /// `+QMTRECV: <client>,<msgid>,"<topic>","<payload>"` after a short
    /// delay — matches the URC firmware sees when the broker pushes a message.
    /// No-op when the client isn't connected.
    pub fn inject_mqtt_recv(&mut self, client_id: u8, topic: &str, payload: &[u8]) {
        let id = client_id as usize;
        if id >= self.mqtt.len() || self.mqtt[id].state != MqttState::Connected {
            return;
        }
        let msgid = {
            let m = &mut self.mqtt[id];
            m.next_msgid = m.next_msgid.wrapping_add(1);
            m.next_msgid
        };
        let urc = format!(
            "\r\n+QMTRECV: {},{},\"{}\",\"{}\"\r\n",
            client_id,
            msgid,
            topic,
            String::from_utf8_lossy(payload)
        );
        self.schedule(URC_DELAY_QMTPUB_US, urc.into_bytes());
    }

    /// Seed CSQ values (0..=31 or 99). Does **not** force an override —
    /// path-loss wins when an RfMedium is attached. Prefer `range_m` SimInput.
    pub fn set_signal(&mut self, rssi: u8, ber: u8) {
        self.csq_rssi = rssi;
        self.csq_ber = ber;
    }

    /// Force CSQ steps until the next `range_m` drive.
    ///
    /// Used by system-yaml `config.rssi` seed and unit tests — **not** a
    /// playground SimInput channel (UI drives `range_m` only).
    pub fn set_csq_override(&mut self, rssi: u8, ber: u8) {
        self.csq_rssi = rssi;
        self.csq_ber = ber;
        self.csq_override = Some(rssi);
    }

    /// Share the lab AirBus / VirtualAirBus medium slot so path-loss geometry
    /// is the same story as nRF RADIO RSSI.
    pub fn share_medium_slot(&mut self, slot: Arc<Mutex<Option<RfMedium>>>) {
        self.medium = slot;
        self.sync_geometry();
    }

    /// Label this UE in the medium (multi-node `node_id` or device id).
    pub fn set_rf_node_id(&mut self, id: impl Into<String>) {
        self.rf_node_id = Some(id.into());
        self.sync_geometry();
    }

    /// Bind this modem to the lab AirBus MQTT fabric (shared across UEs).
    pub fn set_mqtt_net(&mut self, bus: SimMqttFabric) {
        self.mqtt_net = bus;
    }

    /// Lab AirBus fabric handle (publish log / fan-out). Not a wire broker.
    pub fn mqtt_net(&self) -> &SimMqttFabric {
        &self.mqtt_net
    }

    /// True when radio quality may carry MQTT.
    ///
    /// - CSQ override 99 → no
    /// - [`RfMedium`] present → RSSI must be ≥ floor (same path-loss as CSQ)
    /// - No medium yet (bare unit tests) → allow if PDP active (`qiact`) so
    ///   AT-only tests still exercise QMT* without inventing geometry
    pub fn rf_link_ok(&mut self) -> bool {
        if let Some(o) = self.csq_override {
            return o < 99;
        }
        self.sync_geometry();
        if let Ok(slot) = self.medium.lock() {
            if let Some(m) = slot.as_ref() {
                let d = m.distance_m(CELL_TOWER_NODE, self.rf_node_key());
                let dbm = m.rssi_dbm(CELL_TX_POWER_DBM, d);
                return dbm >= m.params().rssi_floor_dbm;
            }
        }
        self.qiact_cid1 == 1 || self.csq_rssi < 99
    }

    fn mqtt_endpoint_id(&self) -> String {
        // Prefer lab node id (multi-UE World / attach_lab_air) so two boards
        // that both declare external_devices id "modem" still get distinct
        // fabric endpoints for pub/sub fan-out.
        self.rf_node_id
            .clone()
            .or_else(|| self.component_id.clone())
            .unwrap_or_else(|| "modem".into())
    }

    /// Drain fabric → modem `+QMTRECV` URCs (called from UART poll).
    fn drain_mqtt_network(&mut self) {
        let ep = self.mqtt_endpoint_id();
        for d in self.mqtt_net.take_pending(&ep) {
            self.inject_mqtt_recv(d.client_id, &d.topic, &d.payload);
        }
    }

    fn rf_node_key(&self) -> &str {
        self.rf_node_id
            .as_deref()
            .or(self.component_id.as_deref())
            .unwrap_or("modem")
    }

    /// Ensure a local RfMedium exists when nothing has been shared yet so
    /// single-board labs still get path-loss CSQ without an AirBus.
    fn ensure_medium(&mut self) {
        let Ok(mut slot) = self.medium.lock() else {
            return;
        };
        if slot.is_none() {
            *slot = Some(RfMedium::new(1).with_params(PathLossParams::default()));
        }
    }

    /// Write cell + UE positions from `range_m` into the medium (no create).
    fn sync_geometry(&mut self) {
        let ue = self.rf_node_key().to_string();
        let range = self.range_m.max(0.0);
        if let Ok(mut slot) = self.medium.lock() {
            if let Some(m) = slot.as_mut() {
                m.set_node(CELL_TOWER_NODE, NodePosition { x: 0.0, y: 0.0 });
                m.set_node(ue, NodePosition { x: range, y: 0.0 });
            }
        }
    }

    /// Place the UE `range_m` metres from the cell tower and clear CSQ override
    /// so AT+CSQ follows path loss. Spins up a local medium if none is shared yet.
    pub fn set_range_m(&mut self, range_m: f64) {
        self.range_m = range_m.max(0.0);
        self.csq_override = None;
        self.ensure_medium();
        self.sync_geometry();
    }

    /// CSQ (rssi_steps, ber) actually reported on AT+CSQ / AT+QCSQ.
    ///
    /// Priority: explicit override → path-loss from shared/local [`RfMedium`]
    /// (only if a medium is present) → seed `csq_rssi`. Below the medium RSSI
    /// floor → 99 (no service). Does **not** invent a medium just to answer CSQ.
    pub fn effective_csq(&mut self) -> (u8, u8) {
        if let Some(o) = self.csq_override {
            return (o, self.csq_ber);
        }
        self.sync_geometry();
        if let Ok(slot) = self.medium.lock() {
            if let Some(m) = slot.as_ref() {
                let d = m.distance_m(CELL_TOWER_NODE, self.rf_node_key());
                let dbm = m.rssi_dbm(CELL_TX_POWER_DBM, d);
                if dbm < m.params().rssi_floor_dbm {
                    return (99, self.csq_ber);
                }
                return (csq_from_dbm(dbm), self.csq_ber);
            }
        }
        (self.csq_rssi, self.csq_ber)
    }

    /// One-shot helper that flips the modem into a "registered home" state:
    /// `+CGATT: 1`, `+CGACT: 1,1`, `+CEREG: 0,1`, co-located medium (strong CSQ).
    /// If `+CEREG=1` or `+CEREG=2` is in effect, schedules the `+CEREG: 1`
    /// URC the same way real hardware does on attach completion.
    pub fn complete_network_attach(&mut self) {
        self.cgatt = 1;
        self.cgact_cid1 = 1;
        // Quectel PDP context used by QMTOPEN / QIOPEN. auto_attach labs that
        // skip AT+QIACT still need a live context or MQTT open returns result 3.
        self.qiact_cid1 = 1;
        // Co-located on the medium → path-loss CSQ (not a free-floating number).
        self.csq_ber = 99;
        self.csq_override = None;
        self.ensure_medium();
        self.set_range_m(0.0);
        self.csq_rssi = self.effective_csq().0;
        self.set_registration(1);
    }

    /// Inject incoming TCP/UDP data into a socket's RX buffer. Emits a
    /// `+QIURC: "recv",<connect_id>` URC so firmware knows to call `AT+QIRD`.
    /// Silently dropped if the socket isn't open.
    pub fn inject_socket_recv(&mut self, connect_id: u8, data: &[u8]) {
        let id = connect_id as usize;
        if id >= self.sockets.len() || self.sockets[id].state != SocketState::Open {
            return;
        }
        self.sockets[id].rx_buffer.extend_from_slice(data);
        let urc = format!("\r\n+QIURC: \"recv\",{}\r\n", connect_id);
        self.schedule(URC_DELAY_QMTPUB_US, urc.into_bytes());
    }

    fn schedule(&mut self, delay_us: u32, bytes: Vec<u8>) {
        self.pending.push_back(ScheduledChunk {
            remaining_us: delay_us,
            bytes,
        });
    }

    fn schedule_boot_urcs(&mut self) {
        self.schedule(URC_DELAY_RDY_US, b"\r\nRDY\r\n".to_vec());
        self.schedule(URC_DELAY_CPIN_READY_US, b"\r\n+CPIN: READY\r\n".to_vec());
        self.schedule(URC_DELAY_QUSIM_US, b"\r\n+QUSIM: 1\r\n".to_vec());
        self.schedule(URC_DELAY_SMS_DONE_US, b"\r\n+QIND: SMS DONE\r\n".to_vec());
        self.schedule(URC_DELAY_PB_DONE_US, b"\r\n+QIND: PB DONE\r\n".to_vec());
    }

    /// URCs that a real chip emits after `AT+CFUN=1` brings it back from
    /// minimum-functionality (`CFUN=0`). Sequence captured from the bench:
    /// SIM ready signalling and the SMS/PB init notifications repeat.
    fn schedule_cfun_resume_urcs(&mut self) {
        self.schedule(URC_DELAY_CPIN_READY_US, b"\r\n+CPIN: READY\r\n".to_vec());
        self.schedule(URC_DELAY_QUSIM_US, b"\r\n+QUSIM: 1\r\n".to_vec());
        self.schedule(URC_DELAY_SMS_DONE_US, b"\r\n+QIND: SMS DONE\r\n".to_vec());
        self.schedule(URC_DELAY_PB_DONE_US, b"\r\n+QIND: PB DONE\r\n".to_vec());
    }

    /// Append text to the response buffer for the current command. Goes
    /// through delayed-emission, not directly to the wire.
    fn emit(&mut self, s: &str) {
        self.respond_buf.extend(s.bytes());
    }

    fn ok(&mut self) {
        if !self.quiet {
            self.emit(if self.verbose { "\r\nOK\r\n" } else { "0\r" });
        }
    }

    /// Emit `OK` without the leading blank-line separator. Real BG770A uses
    /// this compact form for several Quectel-extended test commands
    /// (`AT+QMTOPEN=?`, `AT+QMTCONN=?`, `AT+QMTPUB=?`, `AT+QMTDISC=?`,
    /// `AT+QMTCLOSE=?`) — captured directly from hardware.
    fn ok_compact(&mut self) {
        if !self.quiet {
            self.emit(if self.verbose { "OK\r\n" } else { "0\r" });
        }
    }

    fn error(&mut self) {
        if !self.quiet {
            self.emit(if self.verbose { "\r\nERROR\r\n" } else { "4\r" });
        }
    }

    /// Verbose text for documented CME error codes. Drawn from Table 27 of
    /// the BG77xA-GL AT Commands Manual V1.3.
    fn cme_verbose(code: u16) -> Option<&'static str> {
        Some(match code {
            3 => "operation not allowed",
            4 => "operation not supported",
            10 => "SIM not inserted",
            11 => "SIM PIN required",
            12 => "SIM PUK required",
            13 => "SIM failure",
            14 => "SIM busy",
            15 => "SIM wrong",
            16 => "incorrect password",
            500 => "unknown error",
            505 => "GPS not active",
            512 => "(U)SIM not ready",
            515 => "ME storage failure",
            516 => "Not fix now",
            _ => return None,
        })
    }

    fn cme_error(&mut self, code: u16) {
        if self.quiet {
            return;
        }
        match self.cmee_mode {
            0 => self.error(),
            1 => self.emit(&format!("\r\n+CME ERROR: {}\r\n", code)),
            _ => {
                let body = Self::cme_verbose(code)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| code.to_string());
                self.emit(&format!("\r\n+CME ERROR: {}\r\n", body));
            }
        }
    }

    /// Dispatch a complete AT line.
    fn handle_line(&mut self, raw: &str) {
        let line = raw.trim();
        if line.is_empty() {
            return;
        }
        if self.powered_off {
            // Real chip post-AT+QPOWD doesn't respond to anything until
            // the host pulses PWRKEY again. Buffer is dropped silently.
            return;
        }
        // Case-insensitive comparison for the command part; arguments keep case.
        let upper = line.to_ascii_uppercase();

        // Bare AT — liveness probe.
        if upper == "AT" {
            return self.ok();
        }

        // ----- V.250 / Hayes basic set ----------------------------------
        // AT&F[0] — factory reset.
        if upper == "AT&F" || upper == "AT&F0" {
            self.echo = true;
            self.quiet = false;
            self.verbose = true;
            self.cmee_mode = 0;
            self.cereg_n = 0;
            self.creg_n = 0;
            return self.ok();
        }
        if upper == "AT&W" || upper == "AT&W0" || upper == "AT&V" {
            return self.ok();
        }
        if let Some(rest) = upper.strip_prefix("ATE") {
            match rest {
                "" | "0" => self.echo = false,
                "1" => self.echo = true,
                _ => return self.error(),
            }
            return self.ok();
        }
        if let Some(rest) = upper.strip_prefix("ATQ") {
            match rest {
                "" | "0" => self.quiet = false,
                "1" => self.quiet = true,
                _ => return self.error(),
            }
            return self.ok();
        }
        if let Some(rest) = upper.strip_prefix("ATV") {
            match rest {
                "" | "1" => self.verbose = true,
                "0" => self.verbose = false,
                _ => return self.error(),
            }
            return self.ok();
        }

        // ----- HAZARD pre-dispatch: SMS no-op fallthrough ---------------------
        // CMGR / CMGL / CMGD: no SMS stored, return bare OK no-op so firmware
        // boot flows that poll storage don't trip an ERROR. Skip when the arg
        // is the literal `?` test form, which has its own explicit handler
        // emitting the documented payload (handled by the table).
        //
        // Kept out of the name+form table because `AT+CNMA` is matched with a
        // bare `starts_with` (so `AT+CNMAXYZ` no-ops too), and the write forms
        // here must be swallowed before their table handlers would error.
        let is_test_form = upper.ends_with("=?");
        if !is_test_form
            && (upper.starts_with("AT+CMGR=")
                || upper.starts_with("AT+CMGL=")
                || upper == "AT+CMGL"
                || upper.starts_with("AT+CMGD=")
                || upper.starts_with("AT+CSCA=")
                || upper.starts_with("AT+CNMA"))
        {
            return self.ok();
        }

        // ----- HAZARD pre-dispatch: phonebook starts_with ---------------------
        // Phonebook (BG770A doesn't support it — match HW errors).
        // Real BG770A returns `+CME ERROR: operation not allowed` for CPBR/W/F
        // and bare `ERROR` for CPBS (even with CMEE=0 the verbose form leaks
        // through, similar to the QGPSEND quirk).
        //
        // Kept out of the table because the original check is a bare
        // `starts_with`, so longer names like `AT+CPBRXYZ` match too.
        if upper.starts_with("AT+CPBR")
            || upper.starts_with("AT+CPBW")
            || upper.starts_with("AT+CPBF")
        {
            self.emit("\r\n+CME ERROR: operation not allowed\r\n");
            return;
        }
        if upper.starts_with("AT+CPBS") {
            return self.error();
        }

        // ----- HAZARD pre-dispatch: Verizon starts_with -----------------------
        // AT+VZ Verizon-extension surface — BG770A-GL isn't on Verizon, these
        // all error. `+VZWRSRP` is a bare `starts_with`, so `AT+VZWRSRPXYZ`
        // matches the CME error too; keep that check out of the table.
        // (`+VZWAPNE?`/`=?` are exact tokens and live in the table.)
        if upper == "AT+VZWRSRP?" || upper == "AT+VZWRSRQ?" || upper.starts_with("AT+VZWRSRP") {
            self.emit("\r\n+CME ERROR: operation not allowed\r\n");
            return;
        }

        // ----- Table dispatch ------------------------------------------------
        // Every remaining command is `name + form`. The parser splits at the
        // first `=`/`?`; `AT_HANDLERS` maps the token to a per-family handler
        // that owns the former block. Unknown names fall to the catch-all.
        let Some(req) = at::parse_at(&upper) else {
            return self.error();
        };
        match at::handler_for(req.name) {
            Some(handler) => handler(self, &req.form, line, &upper),
            None => self.error(),
        }
    }
}

impl UartStreamDevice for QuectelBg770a {
    fn poll(&mut self, elapsed_us: u32) -> Option<u8> {
        // Pull any fabric deliveries into scheduled +QMTRECV URCs.
        self.drain_mqtt_network();
        // Already-drainable bytes take priority.
        if let Some(b) = self.out_queue.pop_front() {
            return Some(b);
        }
        // Advance time over the pending queue. Each chunk's delay is the wait
        // after the previous chunk fully drains; we apply `elapsed_us` to the
        // current head and spill any leftover into successors.
        let mut remaining = elapsed_us;
        while remaining > 0 {
            match self.pending.front_mut() {
                None => break,
                Some(head) => {
                    if head.remaining_us > remaining {
                        head.remaining_us -= remaining;
                        remaining = 0;
                    } else {
                        remaining -= head.remaining_us;
                        let chunk = self.pending.pop_front().unwrap();
                        self.out_queue.extend(chunk.bytes);
                    }
                }
            }
        }
        self.out_queue.pop_front()
    }

    fn on_tx_byte(&mut self, byte: u8) {
        if self.powered_off {
            return;
        }
        // QFUPL CONNECT-prompt mode: store exactly `expected_size` bytes into
        // the in-memory filesystem under the given filename, then emit
        // `+QFUPL: <len>,<crc>` and OK. CRC is faked as 0 — firmware that
        // checks the CRC won't pass against the model, but the surface shape
        // matches the real chip exactly otherwise.
        if let Some((ref name, expected)) = self.awaiting_qfupl.clone() {
            self.qfupl_buf.push(byte);
            if self.qfupl_buf.len() >= expected {
                let data = std::mem::take(&mut self.qfupl_buf);
                self.filesystem.insert(name.clone(), data.clone());
                self.awaiting_qfupl = None;
                let reply = format!("\r\n+QFUPL: {},0\r\n\r\nOK\r\n", data.len());
                self.schedule(DELAY_DEFAULT_US, reply.into_bytes());
            }
            return;
        }

        // CMGS prompt mode: text or PDU body accumulates until Ctrl-Z;
        // modem replies `\r\n+CMGS: <mr>\r\n\r\nOK\r\n`. Esc cancels.
        if self.awaiting_cmgs_payload {
            match byte {
                0x1A => {
                    self.awaiting_cmgs_payload = false;
                    self.cmgs_payload_buf.clear();
                    self.cmgs_mr = self.cmgs_mr.wrapping_add(1);
                    let reply = format!("\r\n+CMGS: {}\r\n\r\nOK\r\n", self.cmgs_mr);
                    self.schedule(URC_DELAY_QMTPUB_US, reply.into_bytes());
                }
                0x1B => {
                    self.awaiting_cmgs_payload = false;
                    self.cmgs_payload_buf.clear();
                    self.schedule(URC_DELAY_QMTPUB_US, b"\r\nERROR\r\n".to_vec());
                }
                _ => self.cmgs_payload_buf.push(byte),
            }
            return;
        }

        // HTTP CONNECT-prompt mode: after `CONNECT\r\n`, the firmware streams
        // exactly `expected_len` bytes (URL or POST body). When the buffer is
        // full, the modem emits `OK\r\n` and (for POST) schedules the async
        // `+QHTTPPOST: 0,<code>,<len>` URC. No echo while in this mode.
        if let Some((kind, expected_len)) = self.awaiting_http_data {
            self.http_data_buf.push(byte);
            if self.http_data_buf.len() >= expected_len {
                let body = std::mem::take(&mut self.http_data_buf);
                self.awaiting_http_data = None;
                match kind {
                    HttpPromptKind::Url => {
                        self.http_url = String::from_utf8_lossy(&body).into_owned();
                        self.schedule(DELAY_DEFAULT_US, b"\r\nOK\r\n".to_vec());
                    }
                    HttpPromptKind::PostBody => {
                        self.schedule(DELAY_DEFAULT_US, b"\r\nOK\r\n".to_vec());
                        let urc = format!(
                            "\r\n+QHTTPPOST: 0,{},{}\r\n",
                            self.http_response_code,
                            self.http_response_body.len()
                        );
                        self.schedule(URC_DELAY_QIDNS_US, urc.into_bytes());
                    }
                }
            }
            return;
        }

        // QISEND payload mode: after `> ` prompt, payload bytes accumulate
        // until either Ctrl-Z (variable-length submit) or the requested
        // fixed length has been received. Modem then emits `SEND OK`.
        if let Some((cid, expected_len)) = self.awaiting_qisend_payload {
            // Fixed-length form: auto-submit when the buffer reaches `expected_len`.
            if expected_len > 0 && self.qisend_payload_buf.len() < expected_len {
                self.qisend_payload_buf.push(byte);
                if self.qisend_payload_buf.len() == expected_len {
                    self.awaiting_qisend_payload = None;
                    self.qisend_payload_buf.clear();
                    self.schedule(URC_DELAY_QMTPUB_US, b"\r\nSEND OK\r\n".to_vec());
                }
                let _ = cid;
                return;
            }
            // Variable-length form: Ctrl-Z submits, Esc cancels.
            match byte {
                0x1A => {
                    self.awaiting_qisend_payload = None;
                    self.qisend_payload_buf.clear();
                    self.schedule(URC_DELAY_QMTPUB_US, b"\r\nSEND OK\r\n".to_vec());
                }
                0x1B => {
                    self.awaiting_qisend_payload = None;
                    self.qisend_payload_buf.clear();
                    self.schedule(URC_DELAY_QMTPUB_US, b"\r\nSEND FAIL\r\n".to_vec());
                }
                _ => self.qisend_payload_buf.push(byte),
            }
            return;
        }

        // QMTPUB payload mode: after `> ` prompt, every byte is payload until
        // 0x1A (Ctrl-Z, "submit") or 0x1B (Esc, "cancel"). No echo in this
        // mode — real HW falls silent until the terminator.
        if let Some((id, msg_id, topic)) = self.awaiting_qmtpub_payload.clone() {
            match byte {
                0x1A => {
                    // Submit: RF must still be up; land on SimMqttFabric if so.
                    let payload = std::mem::take(&mut self.qmtpub_payload_buf);
                    self.awaiting_qmtpub_payload = None;
                    let pub_result: i8 = if !self.rf_link_ok() {
                        2 // packet send failed (no service / floor)
                    } else {
                        let ep = self.mqtt_endpoint_id();
                        self.mqtt_net.publish(&ep, id, &topic, &payload);
                        self.drain_mqtt_network();
                        0
                    };
                    let mut bytes = b"\r\nOK\r\n".to_vec();
                    bytes.extend_from_slice(
                        format!("\r\n+QMTPUB: {},{},{}\r\n", id, msg_id, pub_result).as_bytes(),
                    );
                    self.schedule(URC_DELAY_QMTPUB_US, bytes);
                }
                0x1B => {
                    self.awaiting_qmtpub_payload = None;
                    self.qmtpub_payload_buf.clear();
                    let err = b"\r\nSEND FAIL\r\n".to_vec();
                    self.schedule(URC_DELAY_QMTPUB_US, err);
                }
                _ => self.qmtpub_payload_buf.push(byte),
            }
            return;
        }
        // Echo is instant, matching the chip's UART bridge behaviour.
        if self.echo {
            self.out_queue.push_back(byte);
        }
        match byte {
            b'\r' | b'\n' => {
                if !self.line_buf.is_empty() {
                    let line = String::from_utf8_lossy(&self.line_buf).into_owned();
                    self.line_buf.clear();
                    // Reset per-command delay; handlers override when needed.
                    self.current_delay_us = DELAY_DEFAULT_US;
                    self.handle_line(&line);
                    if !self.respond_buf.is_empty() {
                        let bytes = std::mem::take(&mut self.respond_buf);
                        let delay = self.current_delay_us;
                        self.schedule(delay, bytes);
                    }
                    // Post-response side effects (URC bursts triggered by the
                    // command) must enqueue *after* the response so firmware
                    // sees OK first, then the URCs.
                    if self.pending_cfun_resume_urcs {
                        self.pending_cfun_resume_urcs = false;
                        self.schedule_cfun_resume_urcs();
                    }
                    for (delay, bytes) in std::mem::take(&mut self.deferred_urcs) {
                        self.schedule(delay, bytes);
                    }
                }
            }
            _ => self.line_buf.push(byte),
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn crate::sim_input::SimInput> {
        Some(self)
    }
}

/// Radio-quality channels. ONE table backs BOTH the `SimInput` impl and kit
/// metadata. Drive `range_m` (path loss) — no free-floating CSQ override in UI.
pub const INPUT_CHANNELS: &[crate::sim_input::InputChannel] = &[
    crate::sim_input::InputChannel {
        key: "range_m",
        label: "Range",
        unit: "m",
        // UE ↔ cell distance for path loss. 0 = co-located (strong CSQ).
        min: 0.0,
        max: 50_000.0,
    },
    crate::sim_input::InputChannel {
        key: "ber",
        label: "BER",
        unit: "CSQ",
        min: 0.0,
        max: 99.0,
    },
];

impl crate::sim_input::SimInput for QuectelBg770a {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "range_m" => self.set_range_m(value),
            "ber" => {
                self.csq_ber = value.round().clamp(0.0, 99.0) as u8;
            }
            _ => unreachable!("require_channel validated the key"),
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id.clone());
        if self.rf_node_id.is_none() {
            self.rf_node_id = Some(id);
        }
        self.sync_geometry();
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct QuectelBg770aKit;
pub static BG770A_KIT: QuectelBg770aKit = QuectelBg770aKit;

static BG770A_METADATA: KitMetadata = KitMetadata {
    inputs: INPUT_CHANNELS,
    device_type: "bg770a-cellular",
    label: "Quectel BG770A Cellular",
    summary: "LTE-M / NB-IoT cellular modem with the full Quectel AT command surface.",
    detail: "Byte-exact V.250 + Quectel +QI*/+QMT*/+QHTTP*/+QGPS*/+QSSL* state machines, \
             validated against real BG770A-GL hardware captures. Firmware sends AT commands, \
             modem replies stream back over UART. Radio quality (AT+CSQ / AT+QCSQ) uses the \
             same RfMedium path-loss geometry as VirtualAirBus: drive `range_m` (metres to \
             cell). Seed CSQ via config `rssi` only when no medium is attached.",
    transport: Transport::Uart,
    category: Category::Uart,
    config_keys: &[
        ConfigKey {
            name: "apn",
            ty: ConfigType::Str,
            doc: "APN to set on the PDP context (e.g. \"internet\").",
        },
        ConfigKey {
            name: "rssi",
            ty: ConfigType::Int,
            doc: "YAML seed CSQ (0..99) until `range_m` is driven. Not a UI SimInput — \
                  playground radio quality is path-loss only (`range_m`).",
        },
        ConfigKey {
            name: "ber",
            ty: ConfigType::Int,
            doc: "Initial bit-error-rate reported by AT+CSQ (0..99, defaults to 99). Drive \
                  it at runtime with the `ber` input channel.",
        },
        ConfigKey {
            name: "boot_urcs",
            ty: ConfigType::Bool,
            doc: "If true, the modem emits the cold-boot URC sequence on attach.",
        },
        ConfigKey {
            name: "auto_attach",
            ty: ConfigType::Bool,
            doc: "If true, the modem reports itself already registered + attached at boot.",
        },
    ],
    labs: &[
        LabRef {
            board_id: "quectel-bg770a-lab",
            chip: "stm32f103",
            example_dir: "quectel-bg770a-lab",
            demo_elf: "demo-quectel-bg770a-lab.elf",
        },
        LabRef {
            board_id: "h735-telematics-lab",
            chip: "stm32h735",
            example_dir: "h735-telematics-lab",
            demo_elf: "demo-h735-telematics-lab.elf",
        },
    ],
};

impl PeripheralKit for QuectelBg770aKit {
    fn metadata(&self) -> &'static KitMetadata {
        &BG770A_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let boot_urcs = matches!(ctx.config_bool("boot_urcs"), Some(true));
        let apn = ctx.config_str("apn").map(str::to_string);
        let rssi = ctx.config_i64("rssi");
        let ber = ctx.config_i64("ber");
        let auto_attach = matches!(ctx.config_bool("auto_attach"), Some(true));

        let mut modem = QuectelBg770a::new();
        if boot_urcs {
            modem = modem.with_boot_urcs();
        }
        if let Some(apn) = apn {
            modem.set_apn(&apn);
        }
        if let Some(rssi) = rssi {
            let ber = ber.unwrap_or(99);
            // Config seed only — not exposed as a free-floating SimInput.
            modem.set_csq_override(rssi.clamp(0, 99) as u8, ber.clamp(0, 99) as u8);
        }
        if auto_attach {
            modem.complete_network_attach();
        } else {
            // Local medium so range_m / CSQ physics work without AirBus.
            modem.ensure_medium();
            modem.sync_geometry();
        }
        crate::sim_input::SimInput::set_component_id(&mut modem, ctx.device_id().to_string());
        let uart = ctx.uart()?;
        uart.attach_stream(Box::new(modem));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Send `line\r` and advance enough time for any scheduled response to
    /// drain. Returns everything the modem queued onto RX during that window.
    fn exchange(modem: &mut QuectelBg770a, line: &str) -> String {
        for b in line.bytes() {
            modem.on_tx_byte(b);
        }
        modem.on_tx_byte(b'\r');
        let mut out = String::new();
        // First pass: flush any immediate echo with 0 elapsed.
        while let Some(b) = modem.poll(0) {
            out.push(b as char);
        }
        // Then advance well past any documented max-response time (180 s
        // for COPS write is the longest) to drain the response.
        while let Some(b) = modem.poll(200_000_000) {
            out.push(b as char);
        }
        out
    }

    #[test]
    fn at_returns_ok_with_echo() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT");
        assert_eq!(r, "AT\r\r\nOK\r\n");
    }

    #[test]
    fn ate0_suppresses_echo() {
        let mut m = QuectelBg770a::new();
        let _ = exchange(&mut m, "ATE0");
        let r = exchange(&mut m, "AT");
        assert_eq!(r, "\r\nOK\r\n");
    }

    #[test]
    fn ati_emits_captured_identity_block() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "ATI");
        assert!(r.contains("Quectel"));
        assert!(r.contains("BG770A-GL"));
        assert!(r.contains("Revision: BG770AGLAAR01A05"));
        assert!(r.ends_with("OK\r\n"));
    }

    #[test]
    fn cgmi_returns_quectel() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+CGMI");
        assert!(r.contains("\r\nQuectel\r\n"));
        assert!(r.ends_with("OK\r\n"));
    }

    #[test]
    fn cpin_query_reports_ready() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+CPIN?");
        assert!(r.contains("+CPIN: READY"));
    }

    #[test]
    fn cpin_test_form_returns_bare_ok() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+CPIN=?");
        assert!(r.ends_with("OK\r\n"));
        assert!(!r.contains("+CPIN:"));
        assert!(!r.contains("ERROR"));
    }

    #[test]
    fn cpin_write_is_not_allowed_when_ready() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+CPIN=\"0000\"");
        assert!(r.contains("\r\nERROR\r\n"));
    }

    #[test]
    fn cpin_write_returns_cme3_when_cmee_verbose() {
        let mut m = QuectelBg770a::new();
        let _ = exchange(&mut m, "AT+CMEE=1");
        let r = exchange(&mut m, "AT+CPIN=\"0000\"");
        assert!(r.contains("+CME ERROR: 3"));
    }

    #[test]
    fn cfun_read_returns_default_one() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+CFUN?");
        assert!(r.contains("+CFUN: 1"));
    }

    #[test]
    fn cfun_write_accepts_zero_one_four_only() {
        let mut m = QuectelBg770a::new();
        for ok in ["AT+CFUN=0", "AT+CFUN=1", "AT+CFUN=4"] {
            // CFUN=1 after CFUN=0 trails URCs after the OK, so check for OK
            // anywhere rather than at the end.
            assert!(
                exchange(&mut m, ok).contains("\r\nOK\r\n"),
                "{ok} should OK"
            );
        }
        for bad in ["AT+CFUN=2", "AT+CFUN=7"] {
            assert!(
                exchange(&mut m, bad).contains("ERROR"),
                "{bad} should ERROR"
            );
        }
    }

    #[test]
    fn csq_reports_unknown_when_no_service() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+CSQ");
        assert!(r.contains("+CSQ: 99,99"));
    }

    #[test]
    fn cereg_read_uses_configured_stat() {
        let mut m = QuectelBg770a::new();
        m.set_registration(3);
        let r = exchange(&mut m, "AT+CEREG?");
        assert!(r.contains("+CEREG: 0,3"));
    }

    #[test]
    fn cops_test_form_errors_unattached() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+COPS=?");
        assert!(r.contains("\r\nERROR\r\n"));
    }

    #[test]
    fn unknown_command_returns_bare_error() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+NONEXISTENT");
        assert!(r.contains("\r\nERROR\r\n"));
    }

    // ---- Timing tests -------------------------------------------------

    #[test]
    fn echo_is_emitted_immediately_but_ok_waits_for_max_response_time() {
        // Per manual, AT's max response time is 300 ms.
        let mut m = QuectelBg770a::new();
        for b in b"AT" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(b'\r');
        // Echo must be visible with no time advanced.
        let mut early = String::new();
        while let Some(b) = m.poll(0) {
            early.push(b as char);
        }
        assert_eq!(
            early, "AT\r",
            "echo of AT\\r should appear before any elapsed_us"
        );
        // 100 µs is far below the 300 ms max — response must still be pending.
        let mut still_pending = String::new();
        while let Some(b) = m.poll(100) {
            still_pending.push(b as char);
        }
        assert_eq!(
            still_pending, "",
            "response must not arrive earlier than the documented 300 ms"
        );
        // Past the deadline, the OK comes through.
        let mut after = String::new();
        while let Some(b) = m.poll(1_000_000) {
            after.push(b as char);
        }
        assert_eq!(after, "\r\nOK\r\n");
    }

    #[test]
    fn cfun_write_takes_15_seconds_per_datasheet() {
        let mut m = QuectelBg770a::new();
        for b in b"AT+CFUN=4" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(b'\r');
        // Drain echo at t=0.
        let mut echo = String::new();
        while let Some(b) = m.poll(0) {
            echo.push(b as char);
        }
        assert_eq!(echo, "AT+CFUN=4\r");
        // At t=10s, the OK has NOT arrived yet (max response time is 15s).
        let mut at_10s = String::new();
        while let Some(b) = m.poll(10_000_000) {
            at_10s.push(b as char);
        }
        assert!(
            at_10s.is_empty(),
            "CFUN write should still be in progress at 10s, got {at_10s:?}"
        );
        // At t=16s total, the OK is out.
        let mut at_16s = String::new();
        while let Some(b) = m.poll(6_000_000) {
            at_16s.push(b as char);
        }
        assert_eq!(at_16s, "\r\nOK\r\n");
    }

    #[test]
    fn cfun_zero_then_one_emits_sim_resume_urcs() {
        let mut m = QuectelBg770a::new();
        let _ = exchange(&mut m, "AT+CFUN=0");
        let r = exchange(&mut m, "AT+CFUN=1");
        // The OK must come first; then the URC chain.
        assert!(r.starts_with("AT+CFUN=1\r\r\nOK\r\n"));
        assert!(r.contains("+CPIN: READY"));
        assert!(r.contains("+QUSIM: 1"));
        assert!(r.contains("+QIND: SMS DONE"));
    }

    #[test]
    fn boot_urc_chain_is_emitted_when_requested() {
        let mut m = QuectelBg770a::new().with_boot_urcs();
        // No commands sent; just advance time and collect.
        let mut out = String::new();
        while let Some(b) = m.poll(10_000_000) {
            out.push(b as char);
        }
        assert!(out.contains("RDY"));
        assert!(out.contains("+CPIN: READY"));
        assert!(out.contains("+QUSIM: 1"));
        assert!(out.contains("+QIND: SMS DONE"));
        assert!(out.contains("+QIND: PB DONE"));
        let rdy = out.find("RDY").unwrap();
        let cpin = out.find("+CPIN: READY").unwrap();
        assert!(rdy < cpin, "RDY must precede +CPIN: READY");
    }

    #[test]
    fn qpowd_emits_powered_down_and_then_modem_is_silent() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+QPOWD");
        assert!(r.contains("\r\nOK\r\n"));
        assert!(r.contains("\r\nPOWERED DOWN\r\n"));
        // After power-down, further commands produce nothing.
        let r2 = exchange(&mut m, "AT");
        assert_eq!(r2, "");
    }

    #[test]
    fn cgatt_write_accepts_zero_and_one() {
        let mut m = QuectelBg770a::new();
        assert!(exchange(&mut m, "AT+CGATT=1").ends_with("OK\r\n"));
        assert!(exchange(&mut m, "AT+CGATT?").contains("+CGATT: 1"));
        assert!(exchange(&mut m, "AT+CGATT=2").contains("ERROR"));
    }

    #[test]
    fn cgdcont_write_updates_apn() {
        let mut m = QuectelBg770a::new();
        let _ = exchange(&mut m, "AT+CGDCONT=1,\"IP\",\"iot.truphone.com\"");
        let r = exchange(&mut m, "AT+CGDCONT?");
        assert!(r.contains("iot.truphone.com"), "got: {r}");
    }

    #[test]
    fn set_registration_emits_urc_when_n_is_enabled() {
        let mut m = QuectelBg770a::new();
        let _ = exchange(&mut m, "AT+CEREG=1");
        // The registration change happens externally (network event), so we
        // mutate then poll for the URC.
        m.set_registration(1);
        let mut out = String::new();
        while let Some(b) = m.poll(1_000_000) {
            out.push(b as char);
        }
        assert!(out.contains("+CEREG: 1"), "expected URC, got {out:?}");
    }

    #[test]
    fn mqtt_publish_prompt_mode_accepts_payload_and_emits_qmtpub_urc() {
        let mut m = QuectelBg770a::new();
        // Bring up PDP context + open + connect, draining responses.
        for cmd in [
            "AT+QICSGP=1,1,\"internet\"",
            "AT+QIACT=1",
            "AT+QMTOPEN=0,\"broker\",1883",
            "AT+QMTCONN=0,\"cid\"",
        ] {
            let _ = exchange(&mut m, cmd);
        }
        // Issue QMTPUB and check the prompt arrives.
        for b in b"AT+QMTPUB=0,42,1,0,\"topic\"" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(b'\r');
        let mut prompt = String::new();
        while let Some(b) = m.poll(1_000_000) {
            prompt.push(b as char);
        }
        assert!(prompt.contains("> "), "expected prompt, got {prompt:?}");
        // Send payload + Ctrl-Z.
        for b in b"Hello, world!" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(0x1A);
        let mut tail = String::new();
        while let Some(b) = m.poll(1_000_000) {
            tail.push(b as char);
        }
        assert!(tail.contains("OK"), "missing OK after Ctrl-Z, got {tail:?}");
        assert!(
            tail.contains("+QMTPUB: 0,42,0"),
            "missing publish-result URC, got {tail:?}"
        );
        // Network side: payload lands on the cellular MQTT fabric.
        assert!(
            m.mqtt_net().has_publish_on("topic"),
            "fabric should retain publish on topic"
        );
        assert_eq!(
            m.mqtt_net().last_payload_on("topic").as_deref(),
            Some(b"Hello, world!".as_slice())
        );
    }

    #[test]
    fn mqtt_fabric_loopback_delivers_qmtrecv() {
        use crate::sim_input::SimInput;
        let bus = crate::network::SimMqttFabric::new();
        let mut m = QuectelBg770a::new();
        m.set_mqtt_net(bus.clone());
        m.set_component_id("ue1".into());
        m.complete_network_attach(); // medium + co-located RF
        for cmd in [
            "AT+QMTOPEN=0,\"broker.labwired.local\",1883",
            "AT+QMTCONN=0,\"cid\"",
            "AT+QMTSUB=0,1,\"telematics/#\",0",
        ] {
            let _ = exchange(&mut m, cmd);
        }
        for b in b"AT+QMTPUB=0,1,0,0,\"telematics/location\"" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(b'\r');
        while m.poll(1_000_000).is_some() {}
        for b in br#"{"lat":1}"# {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(0x1A);
        let mut out = String::new();
        while let Some(b) = m.poll(2_000_000) {
            out.push(b as char);
        }
        assert!(out.contains("+QMTPUB: 0,1,0"), "got {out:?}");
        assert!(
            out.contains("+QMTRECV:") && out.contains("telematics/location"),
            "loopback subscriber should get +QMTRECV, got {out:?}"
        );
        assert!(bus.has_publish_on("telematics/location"));
    }

    #[test]
    fn mqtt_open_fails_when_rf_below_floor() {
        use crate::sim_input::SimInput;
        let mut m = QuectelBg770a::new();
        m.complete_network_attach();
        m.set_input("range_m", 50_000.0).unwrap(); // far → no service
        let r = exchange(&mut m, "AT+QMTOPEN=0,\"broker\",1883");
        assert!(
            r.contains("+QMTOPEN: 0,1"),
            "no RF must refuse open, got {r:?}"
        );
        assert!(
            !m.mqtt_net().has_publish_on("x"),
            "fabric must stay empty when open fails"
        );
    }

    #[test]
    fn qsslcfg_seclevel_round_trip_persists_per_context() {
        let mut m = QuectelBg770a::new();
        // Default: seclevel=0 for any context.
        let r = exchange(&mut m, "AT+QSSLCFG=\"seclevel\",2");
        assert!(r.contains("+QSSLCFG: \"seclevel\",2,0"));
        // Write seclevel=2 on ctx 2.
        let _ = exchange(&mut m, "AT+QSSLCFG=\"seclevel\",2,2");
        // Read on ctx 2 reflects the write; ctx 3 is untouched.
        let r2 = exchange(&mut m, "AT+QSSLCFG=\"seclevel\",2");
        assert!(r2.contains("+QSSLCFG: \"seclevel\",2,2"));
        let r3 = exchange(&mut m, "AT+QSSLCFG=\"seclevel\",3");
        assert!(r3.contains("+QSSLCFG: \"seclevel\",3,0"));
    }

    #[test]
    fn qmtcfg_ssl_round_trip_enables_tls_on_client() {
        let mut m = QuectelBg770a::new();
        // Default: read form shows SSL disabled (single value 0, no ctxid).
        let r0 = exchange(&mut m, "AT+QMTCFG=\"ssl\",0");
        assert!(
            r0.contains("+QMTCFG: \"ssl\",0\r\n"),
            "expected disabled form, got {r0:?}"
        );
        // Enable SSL on client 0 with ctxid 2.
        let _ = exchange(&mut m, "AT+QMTCFG=\"ssl\",0,1,2");
        let r1 = exchange(&mut m, "AT+QMTCFG=\"ssl\",0");
        assert!(
            r1.contains("+QMTCFG: \"ssl\",1,2"),
            "expected enabled form, got {r1:?}"
        );
    }

    #[test]
    fn raw_tcp_socket_lifecycle_open_send_read_close() {
        let mut m = QuectelBg770a::new();
        // Bring up PDP.
        for cmd in ["AT+QICSGP=1,1,\"internet\"", "AT+QIACT=1"] {
            let _ = exchange(&mut m, cmd);
        }
        // Open a TCP socket on connectID 3.
        let r = exchange(&mut m, "AT+QIOPEN=1,3,\"TCP\",\"example.com\",80");
        assert!(r.contains("\r\nOK\r\n"), "missing sync OK: {r:?}");
        assert!(r.contains("+QIOPEN: 3,0"), "missing open URC: {r:?}");
        // QISTATE? lists the open socket.
        let s = exchange(&mut m, "AT+QISTATE?");
        assert!(
            s.contains("+QISTATE: 3,\"TCP\",\"example.com\",80"),
            "QISTATE output was: {s:?}"
        );
        // Variable-length send via Ctrl-Z.
        for b in b"AT+QISEND=3" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(b'\r');
        // Drain prompt.
        let mut prompt = String::new();
        while let Some(b) = m.poll(1_000_000) {
            prompt.push(b as char);
        }
        assert!(prompt.contains("> "), "missing prompt: {prompt:?}");
        for b in b"hello server" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(0x1A);
        let mut send_result = String::new();
        while let Some(b) = m.poll(1_000_000) {
            send_result.push(b as char);
        }
        assert!(send_result.contains("SEND OK"));
        // QIRD returns no buffered data by default.
        let q = exchange(&mut m, "AT+QIRD=3,100");
        assert!(q.contains("+QIRD: 0"), "expected empty read, got {q:?}");
        // QICLOSE tears down.
        let c = exchange(&mut m, "AT+QICLOSE=3");
        assert!(c.contains("\r\nOK\r\n"));
        let s2 = exchange(&mut m, "AT+QISTATE?");
        assert!(!s2.contains("+QISTATE: 3"));
    }

    #[test]
    fn http_get_happy_path_returns_response_code_and_body() {
        let mut m = QuectelBg770a::new();
        for cmd in [
            "AT+QHTTPCFG=\"contextid\",1",
            "AT+QICSGP=1,1,\"internet\"",
            "AT+QIACT=1",
        ] {
            let _ = exchange(&mut m, cmd);
        }
        // QHTTPURL: CONNECT prompt, then stream 19 URL bytes.
        for b in b"AT+QHTTPURL=19,30" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(b'\r');
        let mut prompt = String::new();
        while let Some(b) = m.poll(1_000_000) {
            prompt.push(b as char);
        }
        assert!(
            prompt.contains("CONNECT"),
            "missing URL CONNECT: {prompt:?}"
        );
        for b in b"http://example.com/" {
            m.on_tx_byte(*b);
        }
        let mut after_url = String::new();
        while let Some(b) = m.poll(1_000_000) {
            after_url.push(b as char);
        }
        assert!(after_url.contains("OK"), "no OK after URL: {after_url:?}");
        // GET: sync OK then async +QHTTPGET.
        let g = exchange(&mut m, "AT+QHTTPGET=30");
        assert!(g.contains("\r\nOK\r\n"));
        assert!(
            g.contains("+QHTTPGET: 0,200,12"),
            "missing async result: {g:?}"
        );
        // READ: CONNECT then body then OK + +QHTTPREAD: 0.
        let r = exchange(&mut m, "AT+QHTTPREAD=30");
        assert!(r.contains("CONNECT"));
        assert!(r.contains("Hello, HTTP!"));
        assert!(r.contains("+QHTTPREAD: 0"));
    }

    #[test]
    fn gps_engine_toggle_and_location_reporting() {
        let mut m = QuectelBg770a::new();
        // GPS off → QGPSLOC returns CME error.
        let r0 = exchange(&mut m, "AT+QGPSLOC=2");
        assert!(
            r0.contains("\r\nERROR\r\n"),
            "expected error when GPS off, got {r0:?}"
        );
        // Turn on, query, turn off.
        let on = exchange(&mut m, "AT+QGPS=1");
        assert!(on.ends_with("OK\r\n"));
        let status = exchange(&mut m, "AT+QGPS?");
        assert!(status.contains("+QGPS: 1"));
        let loc = exchange(&mut m, "AT+QGPSLOC=2");
        assert!(loc.contains("+QGPSLOC: 120000.0,37.7749N,122.4194W"));
        let off = exchange(&mut m, "AT+QGPSEND");
        assert!(off.contains("\r\nOK\r\n"));
        let r1 = exchange(&mut m, "AT+QGPS?");
        assert!(r1.contains("+QGPS: 0"));
    }

    #[test]
    fn cmgf_round_trip_persists_message_format() {
        let mut m = QuectelBg770a::new();
        assert!(exchange(&mut m, "AT+CMGF?").contains("+CMGF: 0"));
        let _ = exchange(&mut m, "AT+CMGF=1");
        assert!(exchange(&mut m, "AT+CMGF?").contains("+CMGF: 1"));
        // Out-of-range rejected.
        assert!(exchange(&mut m, "AT+CMGF=2").contains("ERROR"));
    }

    #[test]
    fn cmgs_prompt_mode_accepts_body_and_returns_mr() {
        let mut m = QuectelBg770a::new();
        // Switch to text mode so the CMGS argument is a quoted number.
        let _ = exchange(&mut m, "AT+CMGF=1");
        for b in b"AT+CMGS=\"+1234567890\"" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(b'\r');
        let mut prompt = String::new();
        while let Some(b) = m.poll(1_000_000) {
            prompt.push(b as char);
        }
        assert!(prompt.contains("> "), "missing CMGS prompt: {prompt:?}");
        for b in b"Hello from the bench" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(0x1A);
        let mut reply = String::new();
        while let Some(b) = m.poll(1_000_000) {
            reply.push(b as char);
        }
        assert!(
            reply.contains("+CMGS: 1") && reply.contains("\r\nOK\r\n"),
            "expected +CMGS:1 + OK, got {reply:?}"
        );
    }

    #[test]
    fn qsclk_round_trip_persists_sleep_mode() {
        let mut m = QuectelBg770a::new();
        assert!(exchange(&mut m, "AT+QSCLK?").contains("+QSCLK: 0"));
        let _ = exchange(&mut m, "AT+QSCLK=1");
        assert!(exchange(&mut m, "AT+QSCLK?").contains("+QSCLK: 1"));
    }

    #[test]
    fn tls_socket_lifecycle_via_qsslopen_qsslsend_qsslrecv_qsslclose() {
        let mut m = QuectelBg770a::new();
        for cmd in [
            "AT+QSSLCFG=\"seclevel\",2,0",
            "AT+QICSGP=1,1,\"internet\"",
            "AT+QIACT=1",
        ] {
            let _ = exchange(&mut m, cmd);
        }
        // Open TLS socket on connectID 5 using SSL ctx 2.
        let r = exchange(&mut m, "AT+QSSLOPEN=1,2,5,\"secure.example\",443");
        assert!(r.contains("\r\nOK\r\n"));
        assert!(r.contains("+QSSLOPEN: 5,0"), "missing TLS open URC: {r:?}");
        // Inject incoming data → QSSLRECV drains it.
        m.inject_socket_recv(5, b"encrypted payload");
        // Drain the +QIURC URC.
        let mut urc = String::new();
        while let Some(b) = m.poll(1_000_000) {
            urc.push(b as char);
        }
        assert!(urc.contains("+QIURC: \"recv\",5"));
        let rd = exchange(&mut m, "AT+QSSLRECV=5,100");
        assert!(rd.contains("+QSSLRECV: 17"));
        assert!(rd.contains("encrypted payload"));
        let cl = exchange(&mut m, "AT+QSSLCLOSE=5");
        assert!(cl.contains("\r\nOK\r\n"));
    }

    #[test]
    fn qfupl_qfdwl_qfdel_round_trip_via_in_memory_filesystem() {
        let mut m = QuectelBg770a::new();
        // Upload "config.json" with 13 bytes.
        for b in b"AT+QFUPL=\"config.json\",13" {
            m.on_tx_byte(*b);
        }
        m.on_tx_byte(b'\r');
        let mut connect = String::new();
        while let Some(b) = m.poll(1_000_000) {
            connect.push(b as char);
        }
        assert!(
            connect.contains("CONNECT"),
            "missing CONNECT prompt: {connect:?}"
        );
        for b in b"{\"k\":\"v\"}\nXyZ" {
            m.on_tx_byte(*b);
        }
        let mut tail = String::new();
        while let Some(b) = m.poll(1_000_000) {
            tail.push(b as char);
        }
        assert!(tail.contains("+QFUPL: 13,0"));
        assert!(tail.contains("\r\nOK\r\n"));
        // Verify QFLST sees it.
        let lst = exchange(&mut m, "AT+QFLST=\"*\"");
        assert!(lst.contains("\"config.json\",13"));
        // QFDWL roundtrips the bytes.
        let dw = exchange(&mut m, "AT+QFDWL=\"config.json\"");
        assert!(dw.contains("CONNECT"));
        assert!(dw.contains("{\"k\":\"v\"}"));
        assert!(dw.contains("+QFDWL: 13,0"));
        // Delete and confirm absence.
        let del = exchange(&mut m, "AT+QFDEL=\"config.json\"");
        assert!(del.ends_with("OK\r\n"));
        let lst2 = exchange(&mut m, "AT+QFLST=\"*\"");
        assert!(!lst2.contains("config.json"));
    }

    #[test]
    fn cclk_read_and_write_round_trip() {
        let mut m = QuectelBg770a::new();
        let _ = exchange(&mut m, "AT+CCLK=\"26/06/03,15:30:00+00\"");
        let r = exchange(&mut m, "AT+CCLK?");
        assert!(r.contains("+CCLK: \"26/06/03,15:30:00+00\""));
    }

    #[test]
    fn qntp_emits_async_success_urc() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+QNTP=1,\"pool.ntp.org\"");
        assert!(r.contains("\r\nOK\r\n"));
        assert!(r.contains("+QNTP: 0,"), "missing NTP success URC: {r:?}");
    }

    #[test]
    fn mqtt_inject_recv_emits_qmtrecv_urc() {
        let mut m = QuectelBg770a::new();
        for cmd in [
            "AT+QICSGP=1,1,\"internet\"",
            "AT+QIACT=1",
            "AT+QMTOPEN=0,\"broker\",1883",
            "AT+QMTCONN=0,\"cid\"",
        ] {
            let _ = exchange(&mut m, cmd);
        }
        m.inject_mqtt_recv(0, "topic/hello", b"world");
        let mut out = String::new();
        while let Some(b) = m.poll(1_000_000) {
            out.push(b as char);
        }
        assert!(
            out.contains("+QMTRECV: 0,1,\"topic/hello\",\"world\""),
            "missing QMTRECV URC: {out:?}"
        );
    }

    #[test]
    fn qnwinfo_returns_cached_network_info() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+QNWINFO");
        assert!(r.contains("+QNWINFO: \"NBIoT\""));
    }

    #[test]
    fn phonebook_commands_return_documented_errors() {
        let mut m = QuectelBg770a::new();
        // CPBR/W/F → verbose CME error "operation not allowed".
        let r = exchange(&mut m, "AT+CPBR=?");
        assert!(r.contains("+CME ERROR: operation not allowed"));
        // CPBS → bare ERROR.
        let r2 = exchange(&mut m, "AT+CPBS?");
        assert!(r2.contains("\r\nERROR\r\n"));
    }

    #[test]
    fn file_handle_open_read_close_round_trip() {
        let mut m = QuectelBg770a::new();
        m.put_file(
            "certs/ca.pem",
            b"-----BEGIN CERTIFICATE-----\nMOCK\n".to_vec(),
        );
        // Open for read.
        let o = exchange(&mut m, "AT+QFOPEN=\"certs/ca.pem\",1");
        assert!(o.contains("+QFOPEN: 1"), "expected handle 1, got {o:?}");
        // Read all.
        let r = exchange(&mut m, "AT+QFREAD=1,200");
        assert!(r.contains("CONNECT 33"), "expected 33-byte file, got {r:?}");
        assert!(r.contains("-----BEGIN CERTIFICATE-----"));
        // Subsequent read returns 0 bytes (offset past EOF).
        let r2 = exchange(&mut m, "AT+QFREAD=1,200");
        assert!(r2.contains("CONNECT 0"));
        // Close.
        let c = exchange(&mut m, "AT+QFCLOSE=1");
        assert!(c.ends_with("OK\r\n"));
        // Read-only open of missing file → CME error.
        let miss = exchange(&mut m, "AT+QFOPEN=\"missing\",1");
        assert!(miss.contains("\r\nERROR\r\n"));
    }

    #[test]
    fn qgpscfg_subkey_state_persists() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+QGPSCFG=\"autogps\"");
        assert!(r.contains("+QGPSCFG: \"autogps\",0"));
        let _ = exchange(&mut m, "AT+QGPSCFG=\"autogps\",1");
        let r2 = exchange(&mut m, "AT+QGPSCFG=\"autogps\"");
        assert!(r2.contains("+QGPSCFG: \"autogps\",1"));
    }

    #[test]
    fn complete_network_attach_updates_csq_cereg_cgatt() {
        let mut m = QuectelBg770a::new();
        // Before: 99,99, searching, detached.
        assert!(exchange(&mut m, "AT+CSQ").contains("+CSQ: 99,99"));
        assert!(exchange(&mut m, "AT+CEREG?").contains("+CEREG: 0,2"));
        assert!(exchange(&mut m, "AT+CGATT?").contains("+CGATT: 0"));
        // After.
        let _ = exchange(&mut m, "AT+CEREG=2");
        m.complete_network_attach();
        let r = exchange(&mut m, "AT+CSQ");
        // Co-located on RfMedium (0 dBm TX, 0 m) → CSQ 31, not a free-floating 28.
        assert!(r.contains("+CSQ: 31,99"), "got {r:?}");
        assert!(exchange(&mut m, "AT+CEREG?").contains("+CEREG: 2,1"));
        assert!(exchange(&mut m, "AT+CGATT?").contains("+CGATT: 1"));
        // Quectel PDP context is ready so MQTT/TCP open can succeed without AT+QIACT.
        assert!(
            exchange(&mut m, "AT+QIACT?").contains("+QIACT: 1,1,1,"),
            "auto_attach should activate Quectel PDP context"
        );
        // QCSQ should now have populated values, not NOSERVICE.
        let q = exchange(&mut m, "AT+QCSQ");
        assert!(
            q.contains("+QCSQ: \"eMTC\","),
            "expected eMTC entry, got {q:?}"
        );
    }

    #[test]
    fn sim_input_range_drives_csq_via_path_loss() {
        use crate::sim_input::SimInput;
        let mut m = QuectelBg770a::new();
        m.complete_network_attach();
        assert!(
            exchange(&mut m, "AT+CSQ").contains("+CSQ: 31,"),
            "co-located should be strongest CSQ"
        );
        // Far away: path loss drops RSSI → lower CSQ (or 99 below floor).
        m.set_input("range_m", 5_000.0).expect("range");
        let far = exchange(&mut m, "AT+CSQ");
        assert!(
            !far.contains("+CSQ: 31,"),
            "5 km should not stay CSQ 31, got {far:?}"
        );
        // YAML/test override (not a SimInput channel) forces CSQ.
        m.set_csq_override(15, 3);
        let r = exchange(&mut m, "AT+CSQ");
        assert!(r.contains("+CSQ: 15,3"), "got {r:?}");
        // range_m clears override and physics resume.
        m.set_input("range_m", 0.0).expect("range home");
        assert!(
            exchange(&mut m, "AT+CSQ").contains("+CSQ: 31,"),
            "back home should be CSQ 31 again"
        );
    }

    #[test]
    fn shared_air_medium_slot_is_one_story() {
        use crate::peripherals::nrf52::radio::VirtualAirBus;
        use crate::peripherals::rf_medium::{PathLossParams, RfMedium};
        use crate::sim_input::SimInput;
        let air = VirtualAirBus::new();
        air.attach_medium(RfMedium::new(7).with_params(PathLossParams::default()));
        let mut m = QuectelBg770a::new();
        m.share_medium_slot(air.medium_slot());
        m.set_component_id("ue".into());
        m.set_input("range_m", 0.0).unwrap();
        assert_eq!(m.effective_csq().0, 31);
        // Same medium Arc: range_m writes UE pose into the air bus medium.
        m.set_input("range_m", 10_000.0).unwrap();
        let csq = m.effective_csq().0;
        // Weakened CSQ step or no-service (99) if below the floor — not CSQ 31.
        assert_ne!(
            csq, 31,
            "10 km path loss should not stay max CSQ, got {csq}"
        );
        assert!(
            air.medium_slot().lock().unwrap().is_some(),
            "shared slot still holds the medium"
        );
    }

    #[test]
    fn sequans_at_percent_extensions_match_captured_shapes() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT%RATACT?");
        assert!(r.contains("%RATACT: \"NBIOT\",1,0"));
        let r2 = exchange(&mut m, "AT%STATUS");
        assert!(r2.contains("+CME ERROR: Incorrect parameters"));
        let r3 = exchange(&mut m, "AT%PCONI?");
        assert!(r3.contains("+CME ERROR: operation not allowed"));
    }

    #[test]
    fn verizon_extensions_error_as_documented() {
        let mut m = QuectelBg770a::new();
        assert!(exchange(&mut m, "AT+VZWAPNE?").contains("\r\nERROR\r\n"));
        assert!(exchange(&mut m, "AT+VZWRSRP?").contains("+CME ERROR: operation not allowed"));
    }

    #[test]
    fn qiopen_without_active_pdp_returns_failure_urc() {
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+QIOPEN=1,0,\"TCP\",\"example.com\",80");
        assert!(r.contains("\r\nOK\r\n"));
        assert!(
            r.contains("+QIOPEN: 0,565"),
            "expected PDP-failure URC, got {r:?}"
        );
    }

    #[test]
    fn qidnsgip_returns_async_resolution_urc() {
        let mut m = QuectelBg770a::new();
        for cmd in ["AT+QICSGP=1,1,\"internet\"", "AT+QIACT=1"] {
            let _ = exchange(&mut m, cmd);
        }
        let r = exchange(&mut m, "AT+QIDNSGIP=1,\"example.com\"");
        assert!(r.contains("\r\nOK\r\n"));
        assert!(
            r.contains("+QIURC: \"dnsgip\",0,1"),
            "missing dns success URC, got {r:?}"
        );
        assert!(
            r.contains("+QIURC: \"dnsgip\",\"93.184.216.34\""),
            "missing resolved address URC"
        );
    }

    #[test]
    fn qmtopen_without_active_pdp_returns_failure_urc() {
        // No AT+QIACT first → URC result code 3 (PDP activation failed).
        let mut m = QuectelBg770a::new();
        let r = exchange(&mut m, "AT+QMTOPEN=0,\"broker\",1883");
        assert!(r.contains("\r\nOK\r\n"));
        assert!(
            r.contains("+QMTOPEN: 0,3"),
            "expected PDP-failure URC, got {r:?}"
        );
    }

    #[test]
    fn set_registration_does_not_emit_urc_when_n_is_zero() {
        let mut m = QuectelBg770a::new();
        m.set_registration(1);
        let mut out = String::new();
        while let Some(b) = m.poll(1_000_000) {
            out.push(b as char);
        }
        assert!(
            out.is_empty(),
            "expected no URC when CEREG n=0, got {out:?}"
        );
    }
}
