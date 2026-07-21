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

use crate::peripherals::uart::UartStreamDevice;
use std::any::Any;
use std::collections::{BTreeMap, VecDeque};

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
    /// `+CSQ` last RSSI/BER. Defaults to 99,99 (no service); test code or the
    /// `complete_network_attach` helper updates these.
    csq_rssi: u8,
    csq_ber: u8,
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
    /// 0x1A / Ctrl-Z to terminate). Tracks `(client_id, msg_id)`.
    #[serde(skip)]
    awaiting_qmtpub_payload: Option<(u8, u16)>,
    /// Accumulator for QMTPUB payload bytes received while in payload mode.
    #[serde(skip)]
    qmtpub_payload_buf: Vec<u8>,

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

    /// Update the value reported by `AT+CSQ` (defaults to 99,99 = no service).
    /// RSSI is 0..=31, BER is 0..=7; 99 means "unknown".
    pub fn set_signal(&mut self, rssi: u8, ber: u8) {
        self.csq_rssi = rssi;
        self.csq_ber = ber;
    }

    /// One-shot helper that flips the modem into a "registered home" state:
    /// `+CGATT: 1`, `+CGACT: 1,1`, `+CEREG: 0,1`, `+CSQ: 28,99` (-57 dBm).
    /// If `+CEREG=1` or `+CEREG=2` is in effect, schedules the `+CEREG: 1`
    /// URC the same way real hardware does on attach completion.
    pub fn complete_network_attach(&mut self) {
        self.cgatt = 1;
        self.cgact_cid1 = 1;
        self.set_signal(28, 99);
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

        // ----- Identity -------------------------------------------------
        match upper.as_str() {
            "ATI" | "ATI1" => {
                self.emit(&format!(
                    "\r\n{}\r\n{}\r\nRevision: {}\r\n",
                    ID_MANUFACTURER, ID_MODEL, ID_FIRMWARE
                ));
                return self.ok();
            }
            "AT+CGMI" | "AT+GMI" => {
                self.emit(&format!("\r\n{}\r\n", ID_MANUFACTURER));
                return self.ok();
            }
            "AT+CGMM" | "AT+GMM" => {
                self.emit(&format!("\r\n{}\r\n", ID_MODEL));
                return self.ok();
            }
            "AT+CGMR" | "AT+GMR" => {
                self.emit(&format!("\r\n{}\r\n", ID_FIRMWARE));
                return self.ok();
            }
            "AT+CGSN" => {
                self.emit(&format!("\r\n{}\r\n", FAKE_IMEI));
                return self.ok();
            }
            "AT+CIMI" => {
                self.emit(&format!("\r\n{}\r\n", FAKE_IMSI));
                return self.ok();
            }
            "AT+QCCID" => {
                self.emit(&format!("\r\n+QCCID: {}\r\n", FAKE_ICCID));
                return self.ok();
            }
            _ => {}
        }

        // ----- SIM (CPIN) ----------------------------------------------
        if upper == "AT+CPIN=?" {
            return self.ok();
        }
        if upper == "AT+CPIN?" {
            self.emit("\r\n+CPIN: READY\r\n");
            return self.ok();
        }
        if upper.starts_with("AT+CPIN=") {
            // Datasheet "Maximum Response Time: 5 s" for the write form. Even
            // when we immediately reject it (SIM already READY), the chip
            // still takes the time to validate — model the delay.
            self.current_delay_us = DELAY_CPIN_WRITE_US;
            return self.cme_error(3);
        }

        // ----- CMEE -----------------------------------------------------
        if upper == "AT+CMEE=?" {
            self.emit("\r\n+CMEE: (0-2)\r\n");
            return self.ok();
        }
        if upper == "AT+CMEE?" {
            self.emit(&format!("\r\n+CMEE: {}\r\n", self.cmee_mode));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+CMEE=") {
            return match arg.trim().parse::<u8>() {
                Ok(v) if v <= 2 => {
                    self.cmee_mode = v;
                    self.ok()
                }
                _ => self.error(),
            };
        }

        // ----- CFUN -----------------------------------------------------
        if upper == "AT+CFUN=?" {
            self.emit("\r\n+CFUN: (0-1,4),(0-1)\r\n");
            return self.ok();
        }
        if upper == "AT+CFUN?" {
            self.emit(&format!("\r\n+CFUN: {}\r\n", self.cfun));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+CFUN=") {
            let fun = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u8>().ok());
            return match fun {
                Some(v @ (0 | 1 | 4)) => {
                    let prev = self.cfun;
                    self.cfun = v;
                    self.current_delay_us = DELAY_CFUN_WRITE_US;
                    self.ok();
                    // 0 → 1 transition replays the SIM-init URCs the chip
                    // emits when the radio comes back up. They must enqueue
                    // *after* the OK, so defer the actual scheduling to
                    // on_tx_byte which runs after the response is queued.
                    if prev == 0 && v == 1 {
                        self.pending_cfun_resume_urcs = true;
                    }
                }
                _ => self.error(),
            };
        }

        // ----- Signal quality ------------------------------------------
        if upper == "AT+CSQ=?" {
            self.emit("\r\n+CSQ: (0-31,99),(0-7,99)\r\n");
            return self.ok();
        }
        if upper == "AT+CSQ" {
            self.emit(&format!("\r\n+CSQ: {},{}\r\n", self.csq_rssi, self.csq_ber));
            return self.ok();
        }
        if upper == "AT+QCSQ" {
            if self.csq_rssi >= 99 {
                self.emit("\r\n+QCSQ: \"NOSERVICE\"\r\n");
            } else {
                // Derived RSRP/RSRQ values for the populated CSQ; -113 dBm
                // baseline + 2 dB per CSQ step is the standard mapping.
                let rssi_dbm = -113 + 2 * self.csq_rssi as i16;
                let rsrp = rssi_dbm - 18; // approximate eMTC offset
                self.emit(&format!(
                    "\r\n+QCSQ: \"eMTC\",{},{},{},{}\r\n",
                    rssi_dbm, rsrp, 200, -10
                ));
            }
            return self.ok();
        }

        // ----- CEREG / CREG --------------------------------------------
        if upper == "AT+CEREG=?" {
            self.emit("\r\n+CEREG: (0-2,4)\r\n");
            return self.ok();
        }
        if upper == "AT+CEREG?" {
            self.emit(&format!(
                "\r\n+CEREG: {},{}\r\n",
                self.cereg_n, self.cereg_stat
            ));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+CEREG=") {
            return match arg.trim().parse::<u8>() {
                Ok(v @ (0..=2 | 4)) => {
                    self.cereg_n = v;
                    self.ok()
                }
                _ => self.error(),
            };
        }
        if upper == "AT+CREG=?" {
            self.emit("\r\n+CREG: (0-2)\r\n");
            return self.ok();
        }
        if upper == "AT+CREG?" {
            self.emit(&format!(
                "\r\n+CREG: {},{}\r\n",
                self.creg_n, self.cereg_stat
            ));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+CREG=") {
            return match arg.trim().parse::<u8>() {
                Ok(v @ 0..=2) => {
                    self.creg_n = v;
                    self.ok()
                }
                _ => self.error(),
            };
        }

        // ----- COPS -----------------------------------------------------
        if upper == "AT+COPS?" {
            self.emit("\r\n+COPS: 0\r\n");
            return self.ok();
        }
        if upper == "AT+COPS=?" {
            // Real-hardware quirk: unattached → +CME ERROR: 515.
            return self.cme_error(515);
        }
        if upper.starts_with("AT+COPS=") {
            self.current_delay_us = DELAY_COPS_WRITE_US;
            return self.ok();
        }

        // ----- Packet domain: CGATT / CGACT / CGPADDR ------------------
        // Note: real BG770A reports `(0-1)` (range form), not the
        // comma-list form documented in 3GPP 27.007.
        if upper == "AT+CGATT=?" {
            self.emit("\r\n+CGATT: (0-1)\r\n");
            return self.ok();
        }
        if upper == "AT+CGATT?" {
            self.emit(&format!("\r\n+CGATT: {}\r\n", self.cgatt));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+CGATT=") {
            return match arg.trim().parse::<u8>() {
                Ok(v @ 0..=1) => {
                    self.cgatt = v;
                    self.current_delay_us = DELAY_CGATT_WRITE_US;
                    self.ok()
                }
                _ => self.error(),
            };
        }

        if upper == "AT+CGACT=?" {
            self.emit("\r\n+CGACT: (0-1)\r\n");
            return self.ok();
        }
        if upper == "AT+CGACT?" {
            self.emit(&format!("\r\n+CGACT: 1,{}\r\n", self.cgact_cid1));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+CGACT=") {
            let mut parts = arg.split(',');
            let state = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            return match (state, cid) {
                (Some(s @ 0..=1), Some(1)) => {
                    self.cgact_cid1 = s;
                    self.current_delay_us = DELAY_CGACT_WRITE_US;
                    self.ok()
                }
                // Activation requires attach; deactivation always allowed.
                _ => self.error(),
            };
        }

        // CGPADDR test form narrows to defined cids on real hardware — we
        // only define cid 1, so the chip reports `(1)`, not the manual's
        // generic `(1-15)`.
        if upper == "AT+CGPADDR=?" {
            self.emit("\r\n+CGPADDR: (1)\r\n");
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+CGPADDR=") {
            return match arg.trim().parse::<u8>() {
                Ok(1) => {
                    // When the context isn't activated, real HW omits the
                    // address field entirely — just `+CGPADDR: 1`.
                    if self.cgact_cid1 == 1 {
                        self.emit("\r\n+CGPADDR: 1,10.0.0.2\r\n");
                    } else {
                        self.emit("\r\n+CGPADDR: 1\r\n");
                    }
                    self.ok()
                }
                _ => self.error(),
            };
        }

        // ----- CGDCONT --------------------------------------------------
        if upper == "AT+CGDCONT=?" {
            self.emit(
                "\r\n+CGDCONT: (1-15),\"IP\",,,(0),(0),(0)\r\n\
                 +CGDCONT: (1-15),\"IPV6\",,,(0),(0),(0)\r\n\
                 +CGDCONT: (1-15),\"IPV4V6\",,,(0),(0),(0)\r\n\
                 +CGDCONT: (1-15),\"Non-IP\",,,(0),(0),(0)\r\n",
            );
            return self.ok();
        }
        if upper == "AT+CGDCONT?" {
            self.emit(&format!(
                "\r\n+CGDCONT: 1,\"{}\",\"{}\",\"0.0.0.0\",0,0,0\r\n",
                self.pdp_type, self.pdp_apn
            ));
            return self.ok();
        }
        if let Some(arg) = line.strip_prefix("AT+CGDCONT=") {
            // Datasheet: AT+CGDCONT=<cid>[,<PDP_type>[,<APN>...]]
            // Strings are double-quoted; we only model cid=1 with PDP_type+APN.
            let mut parts = arg.splitn(3, ',');
            let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            let pdp_type = parts.next().map(|s| s.trim().trim_matches('"'));
            let apn = parts.next().and_then(|rest| {
                // APN is the third field; strip the surrounding quotes and stop
                // at the first comma not inside a quote.
                let mut chars = rest.chars().peekable();
                if chars.next()? != '"' {
                    return None;
                }
                let mut out = String::new();
                for c in chars {
                    if c == '"' {
                        return Some(out);
                    }
                    out.push(c);
                }
                None
            });
            return match (cid, pdp_type, apn) {
                (Some(1), Some(pt), Some(apn))
                    if matches!(pt, "IP" | "IPV6" | "IPV4V6" | "Non-IP") =>
                {
                    self.pdp_type = pt.to_string();
                    self.pdp_apn = apn;
                    self.ok()
                }
                _ => self.error(),
            };
        }

        // ----- Quectel TCP/IP context (QICSGP / QIACT / QIDEACT) -------
        // The full set is documented in §10 of the AT Commands Manual.
        // We model the minimum needed for the AT+QMTOPEN happy path:
        // a single PDP context (cid 1) that the MQTT subsystem rides on.
        if upper == "AT+QICSGP=?" {
            self.emit("\r\n+QICSGP: (1-5),(1-3),<APN>,<username>,<password>,(0-2)\r\n");
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+QICSGP=") {
            // AT+QICSGP=<cid>,<ctx_type>,"<apn>","<user>","<pwd>",<auth>
            let first = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u8>().ok());
            return match first {
                Some(1..=5) => self.ok(),
                _ => self.error(),
            };
        }

        if upper == "AT+QIACT=?" {
            self.emit("\r\n+QIACT: (1-5)\r\n");
            return self.ok();
        }
        if upper == "AT+QIACT?" {
            // Real HW: returns no `+QIACT:` lines when no context is active —
            // just bare OK.
            if self.qiact_cid1 == 1 {
                self.emit("\r\n+QIACT: 1,1,1,\"10.0.0.2\"\r\n");
            }
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+QIACT=") {
            return match arg.trim().parse::<u8>() {
                Ok(1) => {
                    self.qiact_cid1 = 1;
                    self.current_delay_us = DELAY_QIACT_US;
                    self.ok()
                }
                _ => self.error(),
            };
        }
        if let Some(arg) = upper.strip_prefix("AT+QIDEACT=") {
            return match arg.trim().parse::<u8>() {
                Ok(1) => {
                    self.qiact_cid1 = 0;
                    self.current_delay_us = DELAY_QIACT_US;
                    self.ok()
                }
                _ => self.error(),
            };
        }

        // ----- Quectel socket query helpers ----------------------------
        if upper == "AT+QIOPEN=?" {
            self.emit(
                "\r\n+QIOPEN: (1-5),(0-11),\"TCP/UDP/TCP LISTENER/UDP SERVICE\",\
                 \"<IP_address>/<domain_name>\",<remote_port>,<local_port>,(0-2)\r\n",
            );
            return self.ok();
        }
        if upper == "AT+QISTATE=?" {
            // Test form: real HW returns bare OK, no payload.
            return self.ok();
        }
        if upper == "AT+QIGETERROR" {
            self.emit("\r\n+QIGETERROR: 0,operate successfully\r\n");
            return self.ok();
        }

        // ----- Quectel MQTT (QMTCFG / QMTOPEN / QMTCONN / QMTPUB / ...)
        // The MQTT engine is async: write commands return OK immediately and
        // the operation outcome comes later as a `+QMTOPEN/CONN/PUB/...` URC.
        // Status codes are from the Quectel MQTT Application Note:
        //   +QMTOPEN: <id>,<r>    0 = success, 3 = PDP activation failed
        //   +QMTCONN: <id>,<r>,<rc>  r=0 success, rc=0 accepted
        //   +QMTPUB:  <id>,<msgid>,<r>  r=0 success
        //   +QMTDISC: <id>,<r>    0 = success
        //   +QMTCLOSE: <id>,<r>   0 = success
        if upper == "AT+QMTCFG=?" {
            self.emit(
                "\r\n+QMTCFG: \"version\",(0-5),(3,4)\r\n\
                 +QMTCFG: \"pdpcid\",(0-5),(1-5)\r\n\
                 +QMTCFG: \"ssl\",(0-5),(0,1),(0-5)\r\n\
                 +QMTCFG: \"keepalive\",(0-5),(0-3600)\r\n\
                 +QMTCFG: \"session\",(0-5),(0,1)\r\n\
                 +QMTCFG: \"timeout\",(0-5),(1-60),(0-10),(0,1)\r\n\
                 +QMTCFG: \"will\",(0-5),(0,1),(0-2),(0,1),<will_topic>,<will_message>\r\n\
                 +QMTCFG: \"recv/mode\",(0-5),(0,1),(0,1)\r\n\
                 +QMTCFG: \"aliauth\",(0-5),<product_key>,<device_name>,<device_secret>\r\n",
            );
            return self.ok();
        }
        // QMTCFG="ssl",<client>[,<enable>,<ssl_ctxid>] — read/write the SSL
        // toggle for an MQTT client. Read with one arg returns:
        //   `+QMTCFG: "ssl",0`        when SSL is disabled
        //   `+QMTCFG: "ssl",1,<ctx>`  when SSL is enabled
        // Write with three args sets enable+ctxid. Captured from real HW.
        if let Some(args) = line.strip_prefix("AT+QMTCFG=") {
            if let Some((key, rest)) = parse_quoted_subkey(args) {
                let key_lower = key.to_ascii_lowercase();
                let nums: Vec<u8> = rest.iter().filter_map(|s| s.parse::<u8>().ok()).collect();
                if key_lower == "ssl" {
                    return match nums.len() {
                        1 => {
                            let id = nums[0] as usize;
                            if id > 5 {
                                return self.error();
                            }
                            if self.mqtt[id].ssl_enabled {
                                self.emit(&format!(
                                    "\r\n+QMTCFG: \"ssl\",1,{}\r\n",
                                    self.mqtt[id].ssl_ctxid
                                ));
                            } else {
                                self.emit("\r\n+QMTCFG: \"ssl\",0\r\n");
                            }
                            self.ok()
                        }
                        3 => {
                            let id = nums[0] as usize;
                            if id > 5 || nums[1] > 1 || nums[2] > 5 {
                                return self.error();
                            }
                            self.mqtt[id].ssl_enabled = nums[1] == 1;
                            self.mqtt[id].ssl_ctxid = nums[2];
                            self.ok()
                        }
                        _ => self.error(),
                    };
                }
                // Any other QMTCFG sub-key: accept the write as a no-op so
                // firmware boot scripts proceed. Real-HW validates the args,
                // but for happy-path simulation a permissive OK is enough.
                return self.ok();
            }
            return self.ok();
        }

        // ----- Quectel SSL/TLS (QSSLCFG / QSSLSTATE / QSSLOPEN) ---------
        if upper == "AT+QSSLCFG=?" {
            self.emit(
                "\r\n+QSSLCFG: \"sslversion\",(0-5),(0-4)\r\n\
                 +QSSLCFG: \"ciphersuite\",(0-5),(0X0035,0X002F,0X0004,0X0005,\
0X000A,0X003D,0XC002,0XC003,0XC004,0XC005,0XC007,0XC008,0XC009,0XC00A,0XC011,\
0XC012,0XC013,0XC014,0XC00C,0XC00D,0XC00E,0XC00F,0XC023,0XC024,0XC025,0XC026,\
0XC027,0XC028,0XC029,0XC02A,0XC02B,0XC02F,0XC0A8,0X00AE,0XFFFF)\r\n\
                 +QSSLCFG: \"cacert\",(0-5),<cacertpath>\r\n\
                 +QSSLCFG: \"clientcert\",(0-5),<clientcertpath>\r\n\
                 +QSSLCFG: \"clientkey\",(0-5),<clientkeypath>\r\n\
                 +QSSLCFG: \"seclevel\",(0-5),(0-3)\r\n\
                 +QSSLCFG: \"session\",(0-5),(0,1)\r\n\
                 +QSSLCFG: \"sni\",(0-5),(0,1)\r\n\
                 +QSSLCFG: \"checkhost\",(0-5),(0,1)\r\n\
                 +QSSLCFG: \"ignorelocaltime\",(0-5),(0,1)\r\n\
                 +QSSLCFG: \"negotiatetime\",(0-5),(10-300)\r\n\
                 +QSSLCFG: \"renegotiation\",(0-5),(0,1)\r\n\
                 +QSSLCFG: \"dtls\",(0-5),(0-1)\r\n\
                 +QSSLCFG: \"dtlsversion\",(0-5),(0-2)\r\n",
            );
            return self.ok();
        }
        if let Some(args) = line.strip_prefix("AT+QSSLCFG=") {
            if let Some((key, rest)) = parse_quoted_subkey(args) {
                let key_lower = key.to_ascii_lowercase();
                let ctxid = rest.first().and_then(|s| s.parse::<u8>().ok());
                let value = rest.get(1);
                return match (ctxid, value) {
                    (Some(c), None) if c <= 5 => {
                        // Read form. Real HW returns the current value for any
                        // sub-key; we only persist `seclevel`, so others get a
                        // captured-default placeholder.
                        if key_lower == "seclevel" {
                            self.emit(&format!(
                                "\r\n+QSSLCFG: \"seclevel\",{},{}\r\n",
                                c, self.ssl_seclevel[c as usize]
                            ));
                        }
                        self.ok()
                    }
                    (Some(c), Some(v)) if c <= 5 => {
                        if key_lower == "seclevel" {
                            if let Ok(n) = v.parse::<u8>() {
                                if n <= 3 {
                                    self.ssl_seclevel[c as usize] = n;
                                }
                            }
                        }
                        self.ok()
                    }
                    _ => self.error(),
                };
            }
            return self.error();
        }
        if upper == "AT+QSSLSTATE?" || upper == "AT+QSSLSTATE=?" {
            return self.ok();
        }
        if upper == "AT+QSSLOPEN=?" {
            self.emit("\r\n+QSSLOPEN: (1-5),(0-5),(0-11),<serveraddr>,<server_port>,(0-2)\r\n");
            return self.ok();
        }

        // The Quectel MQTT/socket test forms emit a SINGLE \r\n between the
        // last `+...` payload line and the final `OK` — not the standard two.
        // We mirror this quirk exactly.
        if upper == "AT+QMTOPEN=?" {
            self.emit("\r\n+QMTOPEN: (0-5),<host_name>,(0-65535)\r\n");
            return self.ok_compact();
        }
        if upper == "AT+QMTOPEN?" {
            // No clients open → bare OK (no payload).
            let open: Vec<usize> = (0..self.mqtt.len())
                .filter(|&i| self.mqtt[i].state != MqttState::Closed)
                .collect();
            for id in open {
                self.emit(&format!("\r\n+QMTOPEN: {},\"broker\",1883\r\n", id));
            }
            return self.ok();
        }
        if upper == "AT+QMTCONN=?" {
            self.emit("\r\n+QMTCONN: (0-5),<clientID>,<username>,<password>\r\n");
            return self.ok_compact();
        }
        if upper == "AT+QMTCONN?" {
            let connected: Vec<usize> = (0..self.mqtt.len())
                .filter(|&i| self.mqtt[i].state == MqttState::Connected)
                .collect();
            for id in connected {
                self.emit(&format!("\r\n+QMTCONN: {},3\r\n", id));
            }
            return self.ok();
        }
        if upper == "AT+QMTPUB=?" {
            self.emit("\r\n+QMTPUB: (0-5),(0-65535),(0-2),(0,1),<topic>,(1-4096)\r\n");
            return self.ok_compact();
        }
        if upper == "AT+QMTDISC=?" {
            self.emit("\r\n+QMTDISC: (0-5)\r\n");
            return self.ok_compact();
        }
        if upper == "AT+QMTCLOSE=?" {
            self.emit("\r\n+QMTCLOSE: (0-5)\r\n");
            return self.ok_compact();
        }

        // QMTOPEN write — open MQTT network. Returns OK immediately, then
        // a `+QMTOPEN: <id>,<result>` URC. With no active PDP the result is 3
        // (PDP failed), matching real HW.
        if let Some(arg) = upper.strip_prefix("AT+QMTOPEN=") {
            let client_id = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u8>().ok());
            return match client_id {
                Some(id @ 0..=5) => {
                    self.current_delay_us = DELAY_QMTOPEN_US;
                    self.ok();
                    let result: i8 = if self.qiact_cid1 == 1 { 0 } else { 3 };
                    if result == 0 {
                        self.mqtt[id as usize].state = MqttState::Initialized;
                    }
                    let urc = format!("\r\n+QMTOPEN: {},{}\r\n", id, result);
                    self.deferred_urcs
                        .push((URC_DELAY_QMTOPEN_US, urc.into_bytes()));
                }
                _ => self.error(),
            };
        }

        // QMTCONN write — connect to broker.
        if let Some(arg) = upper.strip_prefix("AT+QMTCONN=") {
            let client_id = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u8>().ok());
            return match client_id {
                Some(id @ 0..=5) => {
                    if self.mqtt[id as usize].state != MqttState::Initialized {
                        return self.error();
                    }
                    self.current_delay_us = DELAY_QMTCONN_US;
                    self.ok();
                    self.mqtt[id as usize].state = MqttState::Connected;
                    let urc = format!("\r\n+QMTCONN: {},0,0\r\n", id);
                    self.deferred_urcs
                        .push((URC_DELAY_QMTCONN_US, urc.into_bytes()));
                }
                _ => self.error(),
            };
        }

        // QMTPUB write — publish. Real HW: emits `> ` prompt, firmware sends
        // payload + 0x1A, modem emits OK + async `+QMTPUB: <id>,<msgid>,<r>`.
        // We model the prompt mode here.
        if let Some(arg) = line.strip_prefix("AT+QMTPUB=") {
            let mut parts = arg.split(',');
            let client_id = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            let msg_id = parts.next().and_then(|s| s.trim().parse::<u16>().ok());
            return match (client_id, msg_id) {
                (Some(id @ 0..=5), Some(mid)) => {
                    if self.mqtt[id as usize].state != MqttState::Connected {
                        return self.error();
                    }
                    // Enter payload-receive mode; emit "> " prompt.
                    self.emit("\r\n> ");
                    self.awaiting_qmtpub_payload = Some((id, mid));
                }
                _ => self.error(),
            };
        }

        // QMTSUB — subscribe. We acknowledge with a synthetic URC.
        if let Some(arg) = line.strip_prefix("AT+QMTSUB=") {
            let client_id = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u8>().ok());
            return match client_id {
                Some(id @ 0..=5) if self.mqtt[id as usize].state == MqttState::Connected => {
                    self.ok();
                    let urc = format!("\r\n+QMTSUB: {},1,0,0\r\n", id);
                    self.deferred_urcs
                        .push((URC_DELAY_QMTPUB_US, urc.into_bytes()));
                }
                _ => self.error(),
            };
        }

        // QMTDISC — graceful disconnect.
        if let Some(arg) = upper.strip_prefix("AT+QMTDISC=") {
            let client_id = arg.trim().parse::<u8>().ok();
            return match client_id {
                Some(id @ 0..=5) if self.mqtt[id as usize].state == MqttState::Connected => {
                    self.ok();
                    self.mqtt[id as usize].state = MqttState::Initialized;
                    let urc = format!("\r\n+QMTDISC: {},0\r\n", id);
                    self.deferred_urcs
                        .push((URC_DELAY_QMTDISC_US, urc.into_bytes()));
                }
                _ => self.error(),
            };
        }

        // QMTCLOSE — close MQTT network connection.
        if let Some(arg) = upper.strip_prefix("AT+QMTCLOSE=") {
            let client_id = arg.trim().parse::<u8>().ok();
            return match client_id {
                Some(id @ 0..=5) if self.mqtt[id as usize].state != MqttState::Closed => {
                    self.ok();
                    self.mqtt[id as usize].state = MqttState::Closed;
                    let urc = format!("\r\n+QMTCLOSE: {},0\r\n", id);
                    self.deferred_urcs
                        .push((URC_DELAY_QMTDISC_US, urc.into_bytes()));
                }
                _ => self.error(),
            };
        }

        // ----- Raw TCP/UDP sockets (QIOPEN / QISEND / QIRD / QICLOSE / QISTATE / QIDNS)
        if upper == "AT+QISEND=?" {
            self.emit("\r\n+QISEND: (0-11),(0-1460)\r\n");
            return self.ok();
        }
        if upper == "AT+QIRD=?" {
            self.emit("\r\n+QIRD: (0-11),(0-1500)\r\n");
            return self.ok();
        }
        if upper == "AT+QICLOSE=?" {
            self.emit("\r\n+QICLOSE: (0-11),(0-65535)\r\n");
            return self.ok();
        }
        if upper == "AT+QIDNSCFG=?" {
            self.emit("\r\n+QIDNSCFG: (1-5),<pridnsaddr>,<secdnsaddr>\r\n");
            return self.ok();
        }
        if upper == "AT+QIDNSGIP=?" {
            self.emit("\r\n+QIDNSGIP: (1-5),<hostname>\r\n");
            return self.ok();
        }
        // QIDNSCFG read: errors when no context — matches real HW.
        if let Some(arg) = upper.strip_prefix("AT+QIDNSCFG=") {
            return match arg.trim().parse::<u8>() {
                Ok(_) if self.qiact_cid1 == 0 => self.error(),
                Ok(1..=5) => {
                    self.emit("\r\n+QIDNSCFG: 1,\"8.8.8.8\",\"8.8.4.4\"\r\n");
                    self.ok()
                }
                _ => self.error(),
            };
        }

        // QIOPEN write — open a raw TCP/UDP socket. Sync OK, then async URC
        // `+QIOPEN: <connectID>,<err>` (err=0 success, non-zero failure).
        if let Some(arg) = line.strip_prefix("AT+QIOPEN=") {
            let parts: Vec<&str> = arg.split(',').collect();
            // Min args: contextID, connectID, service_type, host, port.
            if parts.len() < 5 {
                return self.error();
            }
            let ctx_id = parts[0].trim().parse::<u8>().ok();
            let connect_id = parts[1].trim().parse::<u8>().ok();
            let service = parts[2].trim().trim_matches('"').to_string();
            let host = parts[3].trim().trim_matches('"').to_string();
            let port = parts[4].trim().parse::<u16>().ok();
            return match (ctx_id, connect_id, port) {
                (Some(1..=5), Some(cid @ 0..=11), Some(port)) => {
                    self.current_delay_us = DELAY_QIOPEN_US;
                    self.ok();
                    let result: i16 = if self.qiact_cid1 == 1 {
                        self.sockets[cid as usize] = Socket {
                            state: SocketState::Open,
                            service_type: service,
                            remote_host: host,
                            remote_port: port,
                            rx_buffer: Vec::new(),
                        };
                        0
                    } else {
                        // 565 (Quectel: "Failed to activate a PDP context") —
                        // standard mapping for no-context attach attempts.
                        565
                    };
                    let urc = format!("\r\n+QIOPEN: {},{}\r\n", cid, result);
                    self.deferred_urcs
                        .push((URC_DELAY_QIOPEN_US, urc.into_bytes()));
                }
                _ => self.error(),
            };
        }

        // QISTATE — list active sockets or query one. Format:
        //   +QISTATE: <conn>,<service>,<host>,<rport>,<lport>,<state>,<ctxid>,
        //             <sring>,<access_mode>
        // Real HW returns bare OK when no sockets are open.
        if upper == "AT+QISTATE?" {
            let open: Vec<usize> = (0..self.sockets.len())
                .filter(|&i| self.sockets[i].state == SocketState::Open)
                .collect();
            for cid in open {
                let s = &self.sockets[cid];
                self.emit(&format!(
                    "\r\n+QISTATE: {},\"{}\",\"{}\",{},0,2,1,0,0,\"usbmodem\"\r\n",
                    cid, s.service_type, s.remote_host, s.remote_port
                ));
            }
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+QISTATE=") {
            // `1,<connectID>` queries by connect_id (the only form we model).
            let mut parts = arg.split(',');
            let kind = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            return match (kind, cid) {
                (Some(1), Some(c @ 0..=11)) => {
                    let s = &self.sockets[c as usize];
                    if s.state == SocketState::Open {
                        let line = format!(
                            "\r\n+QISTATE: {},\"{}\",\"{}\",{},0,2,1,0,0,\"usbmodem\"\r\n",
                            c, s.service_type, s.remote_host, s.remote_port
                        );
                        self.emit(&line);
                    }
                    self.ok()
                }
                _ => self.error(),
            };
        }

        // QISEND write — enter prompt mode like QMTPUB. Two forms:
        //   AT+QISEND=<cid>          → variable length, terminated by Ctrl-Z
        //   AT+QISEND=<cid>,<len>    → fixed length, modem returns SEND OK
        //                               after exactly <len> bytes
        if let Some(arg) = line.strip_prefix("AT+QISEND=") {
            let mut parts = arg.split(',');
            let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            let len = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
            return match cid {
                Some(c @ 0..=11) if self.sockets[c as usize].state == SocketState::Open => {
                    self.emit("\r\n> ");
                    self.awaiting_qisend_payload = Some((c, len.unwrap_or(0)));
                }
                _ => self.error(),
            };
        }

        // QIRD write — read buffered data. Format:
        //   +QIRD: <read_actual_length>\r\n<data>\r\n\r\nOK\r\n
        // If no data is available, returns `+QIRD: 0` + OK.
        if let Some(arg) = upper.strip_prefix("AT+QIRD=") {
            let mut parts = arg.split(',');
            let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            let max_len = parts
                .next()
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(1500);
            return match cid {
                Some(c @ 0..=11) => {
                    let buf = &mut self.sockets[c as usize].rx_buffer;
                    let take = buf.len().min(max_len);
                    let drained: Vec<u8> = buf.drain(..take).collect();
                    let mut payload = format!("\r\n+QIRD: {}\r\n", drained.len()).into_bytes();
                    payload.extend_from_slice(&drained);
                    if !drained.is_empty() {
                        payload.extend_from_slice(b"\r\n");
                    }
                    self.respond_buf.extend(payload);
                    self.ok()
                }
                _ => self.error(),
            };
        }

        // QICLOSE write — close a socket.
        if let Some(arg) = upper.strip_prefix("AT+QICLOSE=") {
            let cid = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u8>().ok());
            return match cid {
                Some(c @ 0..=11) => {
                    self.sockets[c as usize] = Socket::default();
                    self.current_delay_us = DELAY_QICLOSE_US;
                    self.ok()
                }
                _ => self.error(),
            };
        }

        // QIDNSGIP write — async DNS lookup. Sync OK, then `+QIURC: "dnsgip",
        // <err>,<count>,<ip>` URC. We always succeed with a synthetic IP.
        if let Some(arg) = line.strip_prefix("AT+QIDNSGIP=") {
            let mut parts = arg.split(',');
            let ctx_id = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            let host = parts.next().map(|s| s.trim().trim_matches('"').to_string());
            return match (ctx_id, host) {
                (Some(1..=5), Some(_)) => {
                    self.ok();
                    let urc =
                        b"\r\n+QIURC: \"dnsgip\",0,1\r\n+QIURC: \"dnsgip\",\"93.184.216.34\"\r\n"
                            .to_vec();
                    self.deferred_urcs.push((URC_DELAY_QIDNS_US, urc));
                }
                _ => self.error(),
            };
        }

        // ----- HTTP (QHTTPCFG / QHTTPURL / QHTTPGET / QHTTPPOST / QHTTPREAD)
        // Sync command takes the modem into a CONNECT-prompt mode; firmware
        // streams the URL or POST body bytes; modem then returns OK and (for
        // GET/POST) emits an async `+QHTTPGET/POST: <err>,<code>,<len>` URC.
        if upper == "AT+QHTTPCFG=?" {
            self.emit(
                "\r\n+QHTTPCFG: \"contextid\",(1-5)\r\n\
                 +QHTTPCFG: \"requestheader\",(0,1)\r\n\
                 +QHTTPCFG: \"responseheader\",(0,1)\r\n\
                 +QHTTPCFG: \"sslctxid\",(0-5)\r\n\
                 +QHTTPCFG: \"contenttype\",(0-5)\r\n\
                 +QHTTPCFG: \"auth\",(\"username:password\")\r\n\
                 +QHTTPCFG: \"custom_header\",(\"custom_value\")\r\n",
            );
            return self.ok();
        }
        if upper == "AT+QHTTPURL=?" {
            self.emit("\r\n+QHTTPURL: (1-700),(1-65535)\r\n");
            return self.ok();
        }
        if upper == "AT+QHTTPGET=?" {
            self.emit("\r\n+QHTTPGET: (1-65535),(1-2048),(1-65535)\r\n");
            return self.ok();
        }
        if upper == "AT+QHTTPPOST=?" {
            self.emit("\r\n+QHTTPPOST: (1-1024000),(1-65535),(1-65535)\r\n");
            return self.ok();
        }
        if upper == "AT+QHTTPREAD=?" {
            self.emit("\r\n+QHTTPREAD: (1-65535)\r\n");
            return self.ok();
        }
        if upper.starts_with("AT+QHTTPCFG=") {
            // Accept any sub-key (contextid, requestheader, sslctxid, etc.).
            return self.ok();
        }
        // QHTTPURL=<len>,<timeout> — modem emits CONNECT, firmware streams
        // exactly <len> bytes of the URL, then modem replies with OK.
        if let Some(arg) = upper.strip_prefix("AT+QHTTPURL=") {
            let url_len = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<usize>().ok());
            return match url_len {
                Some(n @ 1..=700) => {
                    self.emit("\r\nCONNECT\r\n");
                    self.awaiting_http_data = Some((HttpPromptKind::Url, n));
                    self.http_data_buf.clear();
                }
                _ => self.error(),
            };
        }
        // QHTTPGET=<rsp_timeout>[,...] — sync OK, then async
        //   +QHTTPGET: <err>,<httprspcode>,<content_length>
        if upper.starts_with("AT+QHTTPGET=") {
            self.ok();
            let urc = format!(
                "\r\n+QHTTPGET: 0,{},{}\r\n",
                self.http_response_code,
                self.http_response_body.len()
            );
            self.deferred_urcs
                .push((URC_DELAY_QIDNS_US, urc.into_bytes()));
            return;
        }
        // QHTTPPOST=<bodyLen>,<rspTimeout>,<reqDataTimeout> — CONNECT prompt,
        // firmware streams body, modem returns OK + async URC.
        if let Some(arg) = upper.strip_prefix("AT+QHTTPPOST=") {
            let body_len = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<usize>().ok());
            return match body_len {
                Some(n @ 1..=1_024_000) => {
                    self.emit("\r\nCONNECT\r\n");
                    self.awaiting_http_data = Some((HttpPromptKind::PostBody, n));
                    self.http_data_buf.clear();
                }
                _ => self.error(),
            };
        }
        // QHTTPREAD=<waittime> — modem emits CONNECT, dumps the response
        // body, then `\r\nOK\r\n` and `+QHTTPREAD: 0` URC.
        if upper.starts_with("AT+QHTTPREAD=") {
            let mut payload = b"\r\nCONNECT\r\n".to_vec();
            payload.extend_from_slice(&self.http_response_body);
            payload.extend_from_slice(b"\r\nOK\r\n\r\n+QHTTPREAD: 0\r\n");
            // Replace the auto-OK with this manual emission, so we don't
            // double-up. The handler exits without setting respond_buf and
            // schedules the bytes itself.
            self.schedule(DELAY_DEFAULT_US, payload);
            return;
        }

        // ----- GPS (QGPS / QGPSLOC / QGPSCFG / QGPSEND) ------------------
        if upper == "AT+QGPS=?" {
            self.emit("\r\n+QGPS: (1)[,(1-3)[,(0-1000)[,(1-65535)]\r\n");
            return self.ok();
        }
        if upper == "AT+QGPS?" {
            self.emit(&format!("\r\n+QGPS: {}\r\n", self.gps_active as u8));
            return self.ok();
        }
        if upper == "AT+QGPSLOC=?" {
            self.emit("\r\n+QGPSLOC: (0-5),(0-3600)\r\n");
            return self.ok();
        }
        if upper == "AT+QGPSCFG=?" {
            self.emit(
                "\r\n+QGPSCFG: \"outport\",(\"none\",\"usbnmea\",\"uartnmea\",\"auxnmea\"),\
(4800,9600,19200,38400,57600,115200,230400,460800,921600)\r\n\
                 +QGPSCFG: \"gnssconfig\",(1)\r\n\
                 +QGPSCFG: \"nmeafmt\",(0,1)\r\n\
                 +QGPSCFG: \"gpsnmeatype\",(0-31)\r\n\
                 +QGPSCFG: \"glonassnmeatype\",(0-3)\r\n\
                 +QGPSCFG: \"nmeasrc\",(0,1)\r\n\
                 +QGPSCFG: \"autogps\",(0,1)\r\n\
                 +QGPSCFG: \"priority\",(0,1)[,(0,1)]\r\n\
                 +QGPSCFG: \"xtrafilesize\",(1,3,7)\r\n\
                 +QGPSCFG: \"xtra_info\"\r\n\
                 +QGPSCFG: \"gpsdop\"\r\n\
                 +QGPSCFG: \"estimation_error\"\r\n\
                 +QGPSCFG: \"xtra_download\",<type>\r\n\
                 +QGPSCFG: \"agnssjamming\",(0-4)[,(2-10),(1-65535)]\r\n\
                 +QGPSCFG: \"agnssjammingurcmode\",(0,1)\r\n\
                 +QGPSCFG: \"test_mode\",<mode>\r\n",
            );
            return self.ok();
        }
        if let Some(args) = line.strip_prefix("AT+QGPSCFG=") {
            if let Some((key, rest)) = parse_quoted_subkey(args) {
                let key_lower = key.to_ascii_lowercase();
                return match (key_lower.as_str(), rest.is_empty()) {
                    ("outport", true) => {
                        self.emit(&format!(
                            "\r\n+QGPSCFG: \"outport\",\"{}\",{}\r\n",
                            self.qgps_outport, self.qgps_outport_baud
                        ));
                        self.ok()
                    }
                    ("outport", false) => {
                        let port = rest[0].trim().trim_matches('"').to_string();
                        let baud = rest
                            .get(1)
                            .and_then(|s| s.trim().parse::<u32>().ok())
                            .unwrap_or(115200);
                        self.qgps_outport = port;
                        self.qgps_outport_baud = baud;
                        self.ok()
                    }
                    ("autogps", true) => {
                        self.emit(&format!(
                            "\r\n+QGPSCFG: \"autogps\",{}\r\n",
                            self.qgps_autogps
                        ));
                        self.ok()
                    }
                    ("autogps", false) => {
                        if let Ok(v) = rest[0].trim().parse::<u8>() {
                            self.qgps_autogps = v.min(1);
                        }
                        self.ok()
                    }
                    ("nmeasrc", true) => {
                        self.emit(&format!(
                            "\r\n+QGPSCFG: \"nmeasrc\",{}\r\n",
                            self.qgps_nmeasrc
                        ));
                        self.ok()
                    }
                    ("nmeasrc", false) => {
                        if let Ok(v) = rest[0].trim().parse::<u8>() {
                            self.qgps_nmeasrc = v.min(1);
                        }
                        self.ok()
                    }
                    ("gnssconfig", true) => {
                        self.emit(&format!(
                            "\r\n+QGPSCFG: \"gnssconfig\",{}\r\n",
                            self.qgps_gnssconfig
                        ));
                        self.ok()
                    }
                    _ => self.ok(),
                };
            }
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+QGPS=") {
            return match arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u8>().ok())
            {
                Some(1) => {
                    self.gps_active = true;
                    self.ok()
                }
                _ => self.error(),
            };
        }
        if upper == "AT+QGPSEND" {
            return if self.gps_active {
                self.gps_active = false;
                self.ok()
            } else {
                // Real HW quirk: QGPSEND always emits the verbose `+CME ERROR:
                // 505` (GPS not active) form, even when CMEE=0 — bypassing the
                // usual CMEE-mapping that would otherwise collapse this to a
                // bare `ERROR`. Captured directly from the bench.
                self.emit("\r\n+CME ERROR: 505\r\n");
            };
        }
        if upper == "AT+QGPSLOC" || upper.starts_with("AT+QGPSLOC=") {
            return if self.gps_active {
                // Datasheet format: <UTC>,<lat>,<lon>,<HDOP>,<altitude>,
                // <fix>,<COG>,<spkm>,<spkn>,<date>,<nsat>.
                self.emit(
                    "\r\n+QGPSLOC: 120000.0,37.7749N,122.4194W,1.0,10.0,3,0.0,0.0,0.0,150626,08\r\n",
                );
                self.ok()
            } else {
                // CME 516: "Not fix now" (Quectel-specific).
                self.cme_error(516)
            };
        }

        // ----- SMS (CMGF / CNMI / CMGS / CMGR / CMGL / CMGD / CSCS / CSCA)
        if upper == "AT+CMGF=?" {
            self.emit("\r\n+CMGF: (0,1)\r\n");
            return self.ok();
        }
        if upper == "AT+CMGF?" {
            self.emit(&format!("\r\n+CMGF: {}\r\n", self.cmgf));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+CMGF=") {
            return match arg.trim().parse::<u8>() {
                Ok(v @ 0..=1) => {
                    self.cmgf = v;
                    self.ok()
                }
                _ => self.error(),
            };
        }
        if upper == "AT+CNMI=?" {
            self.emit("\r\n+CNMI: (1-2),(0-2),(0,2),(0-2),(0-1)\r\n");
            return self.ok();
        }
        if upper == "AT+CNMI?" {
            self.emit("\r\n+CNMI: 2,1,0,0,0\r\n");
            return self.ok();
        }
        if upper.starts_with("AT+CNMI=") {
            // Accept any args; we don't surface incoming-SMS URCs anyway.
            return self.ok();
        }
        if upper == "AT+CMGS=?" || upper == "AT+CMGR=?" || upper == "AT+CSCA=?" {
            return self.ok();
        }
        // CMGS write — text mode (CMGF=1): AT+CMGS="number" → `> ` prompt →
        // body + 0x1A → `+CMGS: <mr>` + OK. PDU mode (CMGF=0) uses a length
        // instead of a quoted number; same prompt behaviour.
        if upper.starts_with("AT+CMGS=") {
            self.emit("\r\n> ");
            self.awaiting_cmgs_payload = true;
            self.cmgs_payload_buf.clear();
            return;
        }
        // CMGR / CMGL / CMGD: no SMS stored, return bare OK no-op so firmware
        // boot flows that poll storage don't trip an ERROR. Skip when the arg
        // is the literal `?` test form, which has its own explicit handler
        // emitting the documented payload (handled earlier in this block).
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
        if upper == "AT+CMGL=?" {
            self.emit("\r\n+CMGL: (0-4)\r\n");
            return self.ok();
        }
        if upper == "AT+CMGD=?" {
            self.emit("\r\n+CMGD: (1-50),(0-4)\r\n");
            return self.ok();
        }
        if upper == "AT+CSCS=?" {
            self.emit("\r\n+CSCS: (\"IRA\",\"GSM\",\"UCS2\")\r\n");
            return self.ok();
        }
        if upper == "AT+CSCS?" {
            self.emit(&format!("\r\n+CSCS: \"{}\"\r\n", self.cscs));
            return self.ok();
        }
        if let Some(arg) = line.strip_prefix("AT+CSCS=") {
            let val = arg.trim().trim_matches('"');
            return match val {
                "GSM" | "IRA" | "UCS2" => {
                    self.cscs = val.to_string();
                    self.ok()
                }
                _ => self.error(),
            };
        }
        if upper == "AT+CSCA?" {
            self.emit("\r\n+CSCA: \"+0000000000000\",145\r\n");
            return self.ok();
        }

        // ----- Power save (QSCLK / CPSMS / CEDRXS / QPSMCFG) ------------
        if upper == "AT+QSCLK=?" {
            self.emit("\r\n+QSCLK: (0-2)\r\n");
            return self.ok();
        }
        if upper == "AT+QSCLK?" {
            self.emit(&format!("\r\n+QSCLK: {}\r\n", self.qsclk));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+QSCLK=") {
            return match arg.trim().parse::<u8>() {
                Ok(v @ 0..=2) => {
                    self.qsclk = v;
                    self.ok()
                }
                _ => self.error(),
            };
        }
        if upper == "AT+CPSMS=?" {
            self.emit(
                "\r\n+CPSMS: (0-2),(\"00000000\"-\"10111111\"),(\"00000000\"-\"11111111\"),\
(\"00000000\"-\"10111111\"),(\"00000000\"-\"11111111\")\r\n",
            );
            return self.ok();
        }
        if upper == "AT+CPSMS?" {
            self.emit(&format!(
                "\r\n+CPSMS: {},,,\"00101100\",\"00001010\"\r\n",
                self.cpsms_mode
            ));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+CPSMS=") {
            let first = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u8>().ok());
            return match first {
                Some(v @ 0..=2) => {
                    self.cpsms_mode = v;
                    self.ok()
                }
                _ => self.error(),
            };
        }
        if upper == "AT+CEDRXS=?" {
            self.emit("\r\n+CEDRXS: (0-3),(4,5),(\"0000\"-\"1111\")\r\n");
            return self.ok();
        }
        if upper == "AT+CEDRXS?" {
            self.emit(&format!("\r\n+CEDRXS: {}\r\n", self.cedrxs_mode));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+CEDRXS=") {
            let first = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u8>().ok());
            return match first {
                Some(v @ 0..=3) => {
                    self.cedrxs_mode = v;
                    self.ok()
                }
                _ => self.error(),
            };
        }
        if upper == "AT+QPSMCFG=?" {
            self.emit("\r\n+QPSMCFG: (20-4294967295),(0-15)\r\n");
            return self.ok();
        }
        if upper.starts_with("AT+QPSMCFG=") {
            // Accept the two-arg write form `<threshold>,<version>`; we don't
            // model PSM timing internally yet.
            return self.ok();
        }

        // ----- TLS raw sockets (QSSLOPEN / QSSLSEND / QSSLRECV / QSSLCLOSE)
        // Test forms.
        if upper == "AT+QSSLSEND=?" {
            self.emit("\r\n+QSSLSEND: (0-11)[,(1-1460)]\r\n");
            return self.ok();
        }
        if upper == "AT+QSSLRECV=?" {
            self.emit("\r\n+QSSLRECV: (0-11),(1-1500)\r\n");
            return self.ok();
        }
        if upper == "AT+QSSLCLOSE=?" {
            self.emit("\r\n+QSSLCLOSE: (0-11)\r\n");
            return self.ok();
        }
        // Write forms mirror QIOPEN / QISEND / QIRD / QICLOSE — same socket
        // table is reused so a SSL-opened connectID shows up in QISTATE too.
        if let Some(arg) = line.strip_prefix("AT+QSSLOPEN=") {
            let parts: Vec<&str> = arg.split(',').collect();
            if parts.len() < 5 {
                return self.error();
            }
            let ctx_id = parts[0].trim().parse::<u8>().ok();
            let _ssl_ctx = parts[1].trim().parse::<u8>().ok();
            let connect_id = parts[2].trim().parse::<u8>().ok();
            let host = parts[3].trim().trim_matches('"').to_string();
            let port = parts[4].trim().parse::<u16>().ok();
            return match (ctx_id, connect_id, port) {
                (Some(1..=5), Some(cid @ 0..=11), Some(port)) => {
                    self.current_delay_us = DELAY_QIOPEN_US;
                    self.ok();
                    let result: i16 = if self.qiact_cid1 == 1 {
                        self.sockets[cid as usize] = Socket {
                            state: SocketState::Open,
                            service_type: "SSL".to_string(),
                            remote_host: host,
                            remote_port: port,
                            rx_buffer: Vec::new(),
                        };
                        0
                    } else {
                        565
                    };
                    let urc = format!("\r\n+QSSLOPEN: {},{}\r\n", cid, result);
                    self.deferred_urcs
                        .push((URC_DELAY_QIOPEN_US, urc.into_bytes()));
                }
                _ => self.error(),
            };
        }
        if let Some(arg) = line.strip_prefix("AT+QSSLSEND=") {
            let mut parts = arg.split(',');
            let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            let len = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
            return match cid {
                Some(c @ 0..=11) if self.sockets[c as usize].state == SocketState::Open => {
                    self.emit("\r\n> ");
                    self.awaiting_qisend_payload = Some((c, len.unwrap_or(0)));
                }
                _ => self.error(),
            };
        }
        if let Some(arg) = upper.strip_prefix("AT+QSSLRECV=") {
            let mut parts = arg.split(',');
            let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
            let max_len = parts
                .next()
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(1500);
            return match cid {
                Some(c @ 0..=11) => {
                    let buf = &mut self.sockets[c as usize].rx_buffer;
                    let take = buf.len().min(max_len);
                    let drained: Vec<u8> = buf.drain(..take).collect();
                    let mut payload = format!("\r\n+QSSLRECV: {}\r\n", drained.len()).into_bytes();
                    payload.extend_from_slice(&drained);
                    if !drained.is_empty() {
                        payload.extend_from_slice(b"\r\n");
                    }
                    self.respond_buf.extend(payload);
                    self.ok()
                }
                _ => self.error(),
            };
        }
        if let Some(arg) = upper.strip_prefix("AT+QSSLCLOSE=") {
            let cid = arg
                .split(',')
                .next()
                .and_then(|s| s.trim().parse::<u8>().ok());
            return match cid {
                Some(c @ 0..=11) => {
                    self.sockets[c as usize] = Socket::default();
                    self.current_delay_us = DELAY_QICLOSE_US;
                    self.ok()
                }
                _ => self.error(),
            };
        }

        // ----- Filesystem (QFLDS / QFLST / QFUPL / QFDWL / QFDEL) -------
        // Test forms.
        if upper == "AT+QFLDS=?" || upper == "AT+QFLST=?" {
            return self.ok();
        }
        if upper == "AT+QFUPL=?" {
            self.emit("\r\n+QFUPL: <filename>[,(1-<freesize>)[,(1-65535)[,(0,1)]]]\r\n");
            return self.ok();
        }
        if upper == "AT+QFDWL=?" {
            self.emit("\r\n+QFDWL: <filename>\r\n");
            return self.ok();
        }
        if upper == "AT+QFOPEN=?" {
            self.emit("\r\n+QFOPEN: <filename>[,(0-3)]\r\n");
            return self.ok();
        }
        if upper == "AT+QFREAD=?" {
            self.emit("\r\n+QFREAD: <filehandle>[,<length>]\r\n");
            return self.ok();
        }
        if upper == "AT+QFWRITE=?" {
            self.emit("\r\n+QFWRITE: <filehandle>[,<length>[,<timeout>]]\r\n");
            return self.ok();
        }
        if upper == "AT+QFCLOSE=?" {
            self.emit("\r\n+QFCLOSE: <filehandle>\r\n");
            return self.ok();
        }
        if upper == "AT+QFDEL=?" {
            self.emit("\r\n+QFDEL: <filename>\r\n");
            return self.ok();
        }
        // QFLDS=<storage> — free/total bytes for "UFS" (User File System).
        if let Some(arg) = line.strip_prefix("AT+QFLDS=") {
            let storage = arg.trim().trim_matches('"');
            return match storage {
                "UFS" => {
                    let used: u32 = self.filesystem.values().map(|v| v.len() as u32).sum();
                    let total: u32 = 3_776_512; // matches real-hw default capacity.
                    let free = total.saturating_sub(used);
                    self.emit(&format!("\r\n+QFLDS: {},{}\r\n", free, total));
                    self.ok()
                }
                _ => self.error(),
            };
        }
        // QFLST="<pattern>" — directory listing. We support the bench's
        // default `"*"` wildcard and exact-name matches.
        if let Some(arg) = line.strip_prefix("AT+QFLST=") {
            let pattern = arg.trim().trim_matches('"');
            if pattern == "*" {
                if self.filesystem.is_empty() {
                    self.emit("\r\n+QFLST: \"security/\",0\r\n");
                } else {
                    let snapshot: Vec<(String, usize)> = self
                        .filesystem
                        .iter()
                        .map(|(k, v)| (k.clone(), v.len()))
                        .collect();
                    for (name, len) in snapshot {
                        self.emit(&format!("\r\n+QFLST: \"{}\",{}\r\n", name, len));
                    }
                }
            } else if let Some(len) = self.filesystem.get(pattern).map(|v| v.len()) {
                self.emit(&format!("\r\n+QFLST: \"{}\",{}\r\n", pattern, len));
            }
            return self.ok();
        }
        // QFUPL=<name>,<size>[,<timeout>[,<ack>]] — CONNECT-prompt then bytes.
        if let Some(arg) = line.strip_prefix("AT+QFUPL=") {
            let parts: Vec<&str> = arg.split(',').collect();
            if parts.len() < 2 {
                return self.error();
            }
            let name = parts[0].trim().trim_matches('"').to_string();
            let size = parts[1].trim().parse::<usize>().ok();
            return match size {
                Some(n) if n > 0 && n <= 1_024_000 => {
                    self.emit("\r\nCONNECT\r\n");
                    self.awaiting_qfupl = Some((name, n));
                    self.qfupl_buf.clear();
                }
                _ => self.error(),
            };
        }
        // QFDWL=<name> — emits CONNECT, file bytes, then `+QFDWL: <len>,<crc>`
        // and OK. We always emit a synthetic CRC of 0 for the simulated FS.
        if let Some(arg) = line.strip_prefix("AT+QFDWL=") {
            let name = arg.trim().trim_matches('"').to_string();
            return match self.filesystem.get(&name).cloned() {
                Some(data) => {
                    let mut payload = b"\r\nCONNECT\r\n".to_vec();
                    payload.extend_from_slice(&data);
                    payload.extend_from_slice(
                        format!("\r\nOK\r\n\r\n+QFDWL: {},0\r\n", data.len()).as_bytes(),
                    );
                    self.schedule(DELAY_DEFAULT_US, payload);
                }
                None => self.cme_error(409), // "file does not exist" per manual.
            };
        }
        // QFOPEN=<name>,<mode> — open file handle. Returns `+QFOPEN: <handle>`.
        if let Some(arg) = line.strip_prefix("AT+QFOPEN=") {
            let parts: Vec<&str> = arg.split(',').collect();
            if parts.is_empty() {
                return self.error();
            }
            let name = parts[0].trim().trim_matches('"').to_string();
            let mode = parts
                .get(1)
                .and_then(|s| s.trim().parse::<u8>().ok())
                .unwrap_or(0);
            // Mode 0/2 = read+write (create if missing); mode 1 = read-only
            // (must exist); mode 3 = write-only. We accept all four.
            if mode == 1 && !self.filesystem.contains_key(&name) {
                return self.cme_error(409);
            }
            self.filesystem.entry(name.clone()).or_default();
            let handle = self.next_file_handle;
            self.next_file_handle = self.next_file_handle.wrapping_add(1).max(1);
            self.open_files.insert(handle, (name, 0, mode));
            self.emit(&format!("\r\n+QFOPEN: {}\r\n", handle));
            return self.ok();
        }
        if let Some(arg) = upper.strip_prefix("AT+QFREAD=") {
            let mut parts = arg.split(',');
            let handle = parts.next().and_then(|s| s.trim().parse::<u16>().ok());
            let max_len = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
            return match handle.and_then(|h| {
                let entry = self.open_files.get(&h)?.clone();
                Some((h, entry))
            }) {
                Some((h, (name, offset, _))) => {
                    let data = self.filesystem.get(&name).cloned().unwrap_or_default();
                    let avail = data.len().saturating_sub(offset);
                    let take = max_len.unwrap_or(avail).min(avail);
                    let slice = &data[offset..offset + take];
                    let mut payload = format!("\r\nCONNECT {}\r\n", take).into_bytes();
                    payload.extend_from_slice(slice);
                    payload.extend_from_slice(b"\r\nOK\r\n");
                    self.schedule(DELAY_DEFAULT_US, payload);
                    if let Some(entry) = self.open_files.get_mut(&h) {
                        entry.1 = offset + take;
                    }
                }
                _ => self.cme_error(409),
            };
        }
        if let Some(arg) = upper.strip_prefix("AT+QFCLOSE=") {
            let handle = arg.trim().parse::<u16>().ok();
            return match handle {
                Some(h) if self.open_files.remove(&h).is_some() => self.ok(),
                _ => self.cme_error(409),
            };
        }

        if let Some(arg) = line.strip_prefix("AT+QFDEL=") {
            let name = arg.trim().trim_matches('"');
            return match self.filesystem.remove(name) {
                Some(_) => self.ok(),
                None if name == "*" => {
                    self.filesystem.clear();
                    self.ok()
                }
                None => self.cme_error(409),
            };
        }

        // ----- NTP / Real-time clock -----------------------------------
        if upper == "AT+CCLK=?" {
            return self.ok();
        }
        if upper == "AT+CCLK?" {
            self.emit(&format!("\r\n+CCLK: \"{}\"\r\n", self.cclk));
            return self.ok();
        }
        if let Some(arg) = line.strip_prefix("AT+CCLK=") {
            let val = arg.trim().trim_matches('"').to_string();
            if val.len() >= 17 {
                self.cclk = val;
                return self.ok();
            }
            return self.error();
        }
        if upper == "AT+QLTS=?" {
            self.emit("\r\n+QLTS: (0-2)\r\n");
            return self.ok();
        }
        if upper == "AT+QLTS" || upper.starts_with("AT+QLTS=") {
            // Returns network-derived local time. We reuse the CCLK string.
            self.emit(&format!("\r\n+QLTS: \"{}\",0\r\n", self.cclk));
            return self.ok();
        }
        if upper == "AT+QNTP=?" {
            self.emit("\r\n+QNTP: (1-5),<server>,(1-65535),(0,1)\r\n");
            return self.ok();
        }
        if upper.starts_with("AT+QNTP=") {
            self.ok();
            // Async URC: success result with the current simulated clock.
            let urc = format!("\r\n+QNTP: 0,\"{}\"\r\n", self.cclk);
            self.deferred_urcs
                .push((URC_DELAY_QIDNS_US, urc.into_bytes()));
            return;
        }

        // ----- Network info (QNWINFO / QENG / CEINFO) -------------------
        if upper == "AT+QNWINFO" {
            // Real HW returns `+QNWINFO: "NBIoT","21670","LTE BAND 1",0` even
            // when not attached — the cached "last seen" cell info.
            self.emit("\r\n+QNWINFO: \"NBIoT\",\"21670\",\"LTE BAND 1\",0\r\n");
            return self.ok();
        }
        if upper == "AT+QENG=?" {
            self.emit("\r\n+QENG: (\"servingcell\",\"neighbourcell\")\r\n");
            return self.ok();
        }
        if upper == "AT+CEINFO=?" {
            self.emit("\r\n+CEINFO: (0)\r\n");
            return self.ok();
        }

        // ----- FOTA / version (QFOTADL / QHVN / QKTFOTA) ----------------
        if upper == "AT+QFOTADL=?" || upper == "AT+QKTFOTA=?" {
            return self.ok();
        }
        if upper == "AT+QHVN=?" {
            self.emit("\r\n+QHVN: <hvn>\r\n");
            return self.ok();
        }
        if upper == "AT+QHVN" || upper == "AT+QHVN?" {
            self.emit("\r\n+QHVN: \"BG770AGLAAR01A05_01.001.01.001\"\r\n");
            return self.ok();
        }

        // ----- Misc (QPING / QLBS / QLBSCFG) -----------------------------
        if upper == "AT+QPING=?" {
            self.emit("\r\n+QPING: (1-5),<host>,(1-255),(1-10)\r\n");
            return self.ok();
        }
        if upper == "AT+QLBS=?" {
            return self.ok();
        }
        if upper == "AT+QLBSCFG=?" {
            self.emit(
                "\r\n+QLBSCFG: \"asynch\",(0,1)\r\n\
                 +QLBSCFG: \"timeout\",(10-120)\r\n\
                 +QLBSCFG: \"server\",<server_name>\r\n\
                 +QLBSCFG: \"token\",<token_value>\r\n\
                 +QLBSCFG: \"timeupdate\",(0,1)\r\n\
                 +QLBSCFG: \"withtime\",(0,1)\r\n\
                 +QLBSCFG: \"latorder\",(0,1)\r\n\
                 +QLBSCFG: \"scanband\",(0,1),<scan_band>\r\n\
                 +QLBSCFG: \"singlecell\",(0,1)\r\n",
            );
            return self.ok();
        }
        if upper.starts_with("AT+QLBSCFG=") {
            return self.ok();
        }

        // ----- Phonebook (BG770A doesn't support it — match HW errors) --
        // Real BG770A returns `+CME ERROR: operation not allowed` for CPBR/W/F
        // and bare `ERROR` for CPBS (even with CMEE=0 the verbose form leaks
        // through, similar to the QGPSEND quirk).
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

        // ----- AT% Sequans-extension surface ---------------------------
        // BG770A runs on a Sequans Calliope baseband; the AT% prefix exposes
        // the Sequans extensions. Coverage is partial: the handful of
        // commands that return useful payload on the bench are modelled; the
        // rest hit the catch-all ERROR. Captures from real hardware.
        if upper == "AT%RATACT?" {
            self.emit("\r\n%RATACT: \"NBIOT\",1,0\r\n");
            return self.ok_compact();
        }
        if upper == "AT%RATSW?" {
            self.emit("\r\n%RATSW: 2,1\r\n");
            return self.ok();
        }
        if upper == "AT%MQTTCMD=?" {
            // The real chip drops the leading `\r\n` for `%MQTTCMD` and uses
            // the compact OK form. Mirror exactly.
            self.respond_buf.extend_from_slice(
                b"\n%MQTTCMD: (\"CONNECT\",\"DISCONNECT\",\"SUBSCRIBE\",\"UNSUBSCRIBE\",\"PUBLISH\"),(0-5)\r\n",
            );
            return self.ok_compact();
        }
        if upper == "AT%CERTCMD=?" {
            // Note the trailing space before the final \r\n — captured from
            // hardware verbatim, a Sequans-firmware quirk.
            self.emit(
                "\r\n%CERTCMD: (\"READ\",\"WRITE\",\"DELETE\",\"DIR\",\"COPY\"),(0,1,2,3) \r\n",
            );
            return self.ok_compact();
        }
        if upper == "AT%MEAS=\"8\"" {
            self.emit(
                "\r\n%MEAS: Signal Quality: RSRP = N/A, RSRQ = N/A, SINR = N/A, RSSI = N/A\r\n",
            );
            return self.ok();
        }
        // Bare-OK pass-through commands.
        if upper == "AT%PDNSTAT?"
            || upper == "AT%SCAN=?"
            || upper == "AT%PCOINFO?"
            || upper == "AT%PDNSET=?"
        {
            return self.ok();
        }
        // CME-error-with-verbose-string Sequans commands. Match the exact
        // captured wording even with CMEE=0 — these bypass the filter.
        if upper == "AT%STATEV?"
            || upper == "AT%PCONI?"
            || upper == "AT%SCANCFG?"
            || upper == "AT%MEAS?"
            || upper == "AT%CCID?"
        {
            self.emit("\r\n+CME ERROR: operation not allowed\r\n");
            return;
        }
        if upper == "AT%STATUS" {
            self.emit("\r\n+CME ERROR: Incorrect parameters\r\n");
            return;
        }
        if upper == "AT%PDNRDP?" {
            return self.error();
        }

        // ----- AT+VZ Verizon-extension surface -------------------------
        // BG770A-GL isn't on Verizon — these all error.
        if upper == "AT+VZWAPNE?" || upper == "AT+VZWAPNE=?" {
            return self.error();
        }
        if upper == "AT+VZWRSRP?" || upper == "AT+VZWRSRQ?" || upper.starts_with("AT+VZWRSRP") {
            self.emit("\r\n+CME ERROR: operation not allowed\r\n");
            return;
        }

        // ----- Power -----------------------------------------------------
        if upper == "AT+QPOWD" || upper == "AT+QPOWD=0" || upper == "AT+QPOWD=1" {
            // Manual: emits OK, then `POWERED DOWN` URC ~600-800 ms later,
            // then the modem is silent until PWRKEY toggle.
            self.ok();
            self.schedule(700_000, b"\r\nPOWERED DOWN\r\n".to_vec());
            self.powered_off = true;
            return;
        }

        // ----- Default: unknown command --------------------------------
        self.error();
    }
}

impl UartStreamDevice for QuectelBg770a {
    fn poll(&mut self, elapsed_us: u32) -> Option<u8> {
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
        if let Some((id, msg_id)) = self.awaiting_qmtpub_payload {
            match byte {
                0x1A => {
                    // Submit: response is `\r\nOK\r\n` then async +QMTPUB.
                    self.awaiting_qmtpub_payload = None;
                    self.qmtpub_payload_buf.clear();
                    let mut bytes = b"\r\nOK\r\n".to_vec();
                    bytes.extend_from_slice(
                        format!("\r\n+QMTPUB: {},{},0\r\n", id, msg_id).as_bytes(),
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
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};

pub struct QuectelBg770aKit;
pub static BG770A_KIT: QuectelBg770aKit = QuectelBg770aKit;

static BG770A_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "bg770a-cellular",
    label: "Quectel BG770A Cellular",
    summary: "LTE-M / NB-IoT cellular modem with the full Quectel AT command surface.",
    detail: "Byte-exact V.250 + Quectel +QI*/+QMT*/+QHTTP*/+QGPS*/+QSSL* state machines, \
             validated against real BG770A-GL hardware captures. Firmware sends AT commands, \
             modem replies stream back over UART.",
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
            doc: "Initial signal strength reported by AT+CSQ (0..99).",
        },
        ConfigKey {
            name: "ber",
            ty: ConfigType::Int,
            doc: "Initial bit-error-rate reported by AT+CSQ (0..99, defaults to 99).",
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
    labs: &[LabRef {
        board_id: "quectel-bg770a-lab",
        chip: "stm32f103",
        example_dir: "quectel-bg770a-lab",
        demo_elf: "demo-quectel-bg770a-lab.elf",
    }],
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

        let uart = ctx.uart()?;
        let mut modem = QuectelBg770a::new();
        if boot_urcs {
            modem = modem.with_boot_urcs();
        }
        if let Some(apn) = apn {
            modem.set_apn(&apn);
        }
        if let Some(rssi) = rssi {
            let ber = ber.unwrap_or(99);
            modem.set_signal(rssi.clamp(0, 99) as u8, ber.clamp(0, 99) as u8);
        }
        if auto_attach {
            modem.complete_network_attach();
        }
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
        assert!(r.contains("+CSQ: 28,99"), "got {r:?}");
        assert!(exchange(&mut m, "AT+CEREG?").contains("+CEREG: 2,1"));
        assert!(exchange(&mut m, "AT+CGATT?").contains("+CGATT: 1"));
        // QCSQ should now have populated values, not NOSERVICE.
        let q = exchange(&mut m, "AT+QCSQ");
        assert!(
            q.contains("+QCSQ: \"eMTC\","),
            "expected eMTC entry, got {q:?}"
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
