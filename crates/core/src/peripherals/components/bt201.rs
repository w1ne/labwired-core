// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! BT201 dual-mode Bluetooth module (Jieli KT1025A/B), UART AT interface.
//!
//! The BT201 is a commercial Bluetooth 5.0 module: classic Bluetooth (EDR,
//! A2DP audio and SPP) plus BLE with a transparent data service. A host MCU
//! controls it with AT commands on a UART (115200 8N1). While a phone is
//! connected over BLE, the module forwards the host's non-AT bytes to the
//! phone and the phone's bytes to the host ("transparent transmission").
//!
//! Source: "BT201 Module KT1025A/B User Manual V2.3" (Shenzhen Qingyue
//! Electronics), cited below as [M]. Sections: 2 (frame format, `OK` /
//! `ER+n`), 5 (commands), 6.1 (`AT+B4xx` / `AT+B5xx`), 7 (BLE transparent
//! transmission, 128 bytes per packet).
//!
//! What is modelled
//! ================
//! * Frames: `AT+<CC>[param]\r\n` in, `<IND>[param]\r\n` out. A control
//!   command answers `OK`; a query answers with its data only; an unknown
//!   command answers `ER+2`, a bad parameter `ER+4` [M 2].
//! * Settings the module keeps in its own flash: EDR name (`AT+BD`), BLE
//!   name (`AT+BM`), prompt tone (`AT+CN`), EDR on/off (`AT+B5`), BLE on/off
//!   (`AT+B4`). Names and the radio switches take effect at the next power-on
//!   or `AT+CZ` (soft reset) [M 5, 6.1]; until then the queries (`AT+TD`,
//!   `AT+TM`) report the names in use.
//! * Power-on: after `boot_ms` the module sends its start-up block (version
//!   line and the `Q?+` settings) [M 5], then its link status.
//! * Link state: EDR `TS+00` waiting for pairing, `01` connected, `02` music,
//!   `03` call; BLE `TL+02` advertising, `03` connected, `04` disconnected
//!   [M 5]. A change is pushed at once; the EDR status is pushed again every
//!   `status_period_ms` ("every half a second") until `AT+CR00` turns the
//!   pushes off [M 5].
//! * The phone is a test-script stimulus: input channels `edr_link` (0..3,
//!   the `TS` code the phone causes) and `ble_link` (0/1). A link is refused
//!   while the module boots or while that radio is off.
//! * Transparent data: a host line that starts with `AT` is a command; any
//!   other bytes are data. Data goes to the phone as packets of at most 128
//!   bytes; a packet ends after `packet_gap_us` without a byte. Without a BLE
//!   link the module drops the data. Phone data (`inject_remote`) goes to the
//!   host UART only while BLE is connected.
//!
//! What is NOT modelled
//! ====================
//! Audio (A2DP, the I2S output), SPP data, calls, the TF/U-disk player, the
//! `FFF3` AT-over-BLE characteristic, baud-rate change (`AT+CT`), BLE MTU
//! negotiation, and RF. Reply latency and boot time are not in [M]; they are
//! config values (`reply_delay_us`, `boot_ms`) with assumed defaults.
//!
//! Logs (read with `peripheral_log` on the hosting UART):
//! * `at`: one line per command, `12.345000s AT+TM -> TM+BT201-BLE`.
//! * `link`: power, reset and status changes, `12.345000s TL+03 ble connected`.
//! * `air`: one line per packet, `12.345000s phone->mcu aa 55 01 00 00 c8 cf`
//!   and `mcu->phone ...`; a dropped packet says `dropped (no ble link)`.

use std::collections::VecDeque;

use crate::peripheral_log::PeripheralLog;
use crate::peripherals::device::UartStreamDevice;
use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use crate::sim_input::{InputChannel, SimInput, SimInputError};

/// Factory EDR (classic) name [M 5].
pub const DEFAULT_EDR_NAME: &str = "BT201-AUDIO";
/// Factory BLE name [M 5, 7.1].
pub const DEFAULT_BLE_NAME: &str = "BT201-BLE";
/// Firmware version in the start-up block of the manual's example [M 5].
pub const DEFAULT_VERSION: &str = "2.3-20190517";
/// Largest name `AT+BD` accepts [M 5]. `AT+BM` gets the same limit (the
/// manual gives none).
pub const MAX_NAME_BYTES: usize = 32;
/// Largest transparent packet [M 7].
pub const MAX_PACKET_BYTES: usize = 128;

/// Settings the module keeps in its flash.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Settings {
    edr_name: String,
    ble_name: String,
    prompt_tone: bool,
    edr_on: bool,
    ble_on: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            edr_name: DEFAULT_EDR_NAME.to_string(),
            ble_name: DEFAULT_BLE_NAME.to_string(),
            prompt_tone: true,
            edr_on: true,
            ble_on: true,
        }
    }
}

/// Timing and start-up options (system.yaml `config:`).
#[derive(Debug, Clone)]
pub struct Bt201Config {
    /// Power-on (and `AT+CZ`) to ready, µs.
    pub boot_us: u64,
    /// Command to reply, µs.
    pub reply_delay_us: u64,
    /// EDR status repeat period, µs; 0 = only on change.
    pub status_period_us: u64,
    /// Idle time that ends a transparent packet from the host, µs.
    pub packet_gap_us: u64,
    /// Send the start-up block at power-on.
    pub banner: bool,
    pub version: String,
}

impl Default for Bt201Config {
    fn default() -> Self {
        Self {
            boot_us: 500_000,
            reply_delay_us: 1_000,
            status_period_us: 500_000,
            packet_gap_us: 1_000,
            banner: true,
            version: DEFAULT_VERSION.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ble {
    Off,
    Advertising,
    Connected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostRx {
    /// Between units: the next byte decides command or data.
    Idle,
    /// First byte was `A`; the second decides.
    MaybeCommand,
    /// An `AT` line, up to its line end.
    Command,
    /// Transparent data, up to a gap or a full packet.
    Data,
}

const INPUT_CHANNELS: &[InputChannel] = &[
    InputChannel {
        key: std::borrow::Cow::Borrowed("edr_link"),
        label: std::borrow::Cow::Borrowed("Phone EDR link (TS code)"),
        unit: std::borrow::Cow::Borrowed("state"),
        min: 0.0,
        max: 3.0,
    },
    InputChannel {
        key: std::borrow::Cow::Borrowed("ble_link"),
        label: std::borrow::Cow::Borrowed("Phone BLE link"),
        unit: std::borrow::Cow::Borrowed("on/off"),
        min: 0.0,
        max: 1.0,
    },
];

/// The BT201 model. See the module doc.
pub struct Bt201 {
    id: String,
    cfg: Bt201Config,
    saved: Settings,
    active: Settings,
    /// Device time, µs.
    now_us: u64,
    /// Ready (start-up block sent) at this time; `None` = ready.
    boot_done_at: Option<u64>,
    /// EDR `TS` code; meaningful while EDR is on.
    edr: u8,
    ble: Ble,
    status_push: bool,
    next_status_at: Option<u64>,
    host_rx: HostRx,
    line: Vec<u8>,
    data: Vec<u8>,
    data_last_us: u64,
    /// Bytes on their way to the host, one per poll.
    out: VecDeque<u8>,
    /// Replies not due yet: (due µs, bytes), in order.
    pending: VecDeque<(u64, Vec<u8>)>,
    at_log: Vec<String>,
    link_log: Vec<String>,
    air_log: Vec<String>,
    /// Everything the phone received, in order.
    to_phone: Vec<u8>,
}

impl std::fmt::Debug for Bt201 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bt201")
            .field("id", &self.id)
            .field("edr", &self.edr)
            .field("ble", &self.ble)
            .finish()
    }
}

impl Default for Bt201 {
    fn default() -> Self {
        Self::new(Bt201Config::default())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn ts_meaning(code: u8) -> &'static str {
    match code {
        0 => "edr waiting for pairing",
        1 => "edr connected",
        2 => "edr music playing",
        _ => "edr call",
    }
}

impl Bt201 {
    /// A module that powers on at device time 0.
    pub fn new(cfg: Bt201Config) -> Self {
        Self::with_settings(cfg, Settings::default())
    }

    fn with_settings(cfg: Bt201Config, saved: Settings) -> Self {
        let mut m = Self {
            id: String::new(),
            cfg,
            active: saved.clone(),
            saved,
            now_us: 0,
            boot_done_at: None,
            edr: 0,
            ble: Ble::Off,
            status_push: true,
            next_status_at: None,
            host_rx: HostRx::Idle,
            line: Vec::new(),
            data: Vec::new(),
            data_last_us: 0,
            out: VecDeque::new(),
            pending: VecDeque::new(),
            at_log: Vec::new(),
            link_log: Vec::new(),
            air_log: Vec::new(),
            to_phone: Vec::new(),
        };
        m.power_on();
        m
    }

    /// Set the names the module has in its flash (as if set before).
    pub fn set_names(&mut self, edr: Option<&str>, ble: Option<&str>) {
        if let Some(n) = edr {
            self.saved.edr_name = n.to_string();
            self.active.edr_name = n.to_string();
        }
        if let Some(n) = ble {
            self.saved.ble_name = n.to_string();
            self.active.ble_name = n.to_string();
        }
    }

    /// Set the radio switches the module has in its flash.
    pub fn set_radios(&mut self, edr_on: bool, ble_on: bool) {
        self.saved.edr_on = edr_on;
        self.saved.ble_on = ble_on;
        self.active.edr_on = edr_on;
        self.active.ble_on = ble_on;
    }

    pub fn set_id(&mut self, id: impl Into<String>) {
        self.id = id.into();
    }

    /// The EDR name in use (what `AT+TD` reports).
    pub fn edr_name(&self) -> &str {
        &self.active.edr_name
    }

    /// The BLE name in use (what `AT+TM` reports).
    pub fn ble_name(&self) -> &str {
        &self.active.ble_name
    }

    /// True while a phone is connected over BLE.
    pub fn ble_connected(&self) -> bool {
        self.ble == Ble::Connected
    }

    /// Everything the phone has received over BLE.
    pub fn to_phone(&self) -> &[u8] {
        &self.to_phone
    }

    fn stamp(&self) -> String {
        format!(
            "{}.{:06}s",
            self.now_us / 1_000_000,
            self.now_us % 1_000_000
        )
    }

    fn log_link(&mut self, text: impl AsRef<str>) {
        let line = format!("{} {}", self.stamp(), text.as_ref());
        self.link_log.push(line);
    }

    fn log_air(&mut self, text: impl AsRef<str>) {
        let line = format!("{} {}", self.stamp(), text.as_ref());
        self.air_log.push(line);
    }

    fn ready(&self) -> bool {
        self.boot_done_at.is_none()
    }

    /// Start (or restart) the module: apply the saved settings, drop the
    /// links, and become ready after `boot_us`.
    fn power_on(&mut self) {
        self.active = self.saved.clone();
        self.edr = 0;
        self.ble = Ble::Off;
        self.status_push = true;
        self.next_status_at = None;
        self.host_rx = HostRx::Idle;
        self.line.clear();
        self.data.clear();
        self.pending.clear();
        self.boot_done_at = Some(self.now_us + self.cfg.boot_us);
        self.log_link("power on");
        if self.cfg.boot_us == 0 {
            self.finish_boot();
        }
    }

    fn finish_boot(&mut self) {
        self.boot_done_at = None;
        if self.cfg.banner {
            let tone = u8::from(self.active.prompt_tone);
            let block = format!(
                "AT+VER{}\r\nQA+30\r\nQM+00\r\nQN+{tone:02}\r\nQK+01\r\nQG+01\r\nQ1+01\r\n",
                self.cfg.version
            );
            self.out.extend(block.into_bytes());
        }
        self.log_link(format!(
            "ready edr {} ble {}",
            if self.active.edr_on { "on" } else { "off" },
            if self.active.ble_on { "on" } else { "off" }
        ));
        if self.active.edr_on {
            self.push_ts();
            if self.cfg.status_period_us > 0 {
                self.next_status_at = Some(self.now_us + self.cfg.status_period_us);
            }
        }
        if self.active.ble_on {
            self.set_ble(Ble::Advertising);
        }
    }

    fn push_status(&mut self, text: String) {
        if self.status_push {
            self.out.extend(format!("{text}\r\n").into_bytes());
        }
    }

    fn push_ts(&mut self) {
        let text = format!("TS+{:02}", self.edr);
        let meaning = ts_meaning(self.edr);
        self.log_link(format!("{text} {meaning}"));
        self.push_status(text);
    }

    fn set_ble(&mut self, to: Ble) {
        if self.ble == to {
            return;
        }
        let from = self.ble;
        self.ble = to;
        match (from, to) {
            (Ble::Connected, _) => {
                self.log_link("TL+04 ble disconnected");
                self.push_status("TL+04".to_string());
                if to == Ble::Advertising {
                    self.log_link("TL+02 ble advertising");
                    self.push_status("TL+02".to_string());
                }
            }
            (_, Ble::Advertising) => {
                self.log_link("TL+02 ble advertising");
                self.push_status("TL+02".to_string());
            }
            (_, Ble::Connected) => {
                self.log_link("TL+03 ble connected");
                self.push_status("TL+03".to_string());
            }
            (_, Ble::Off) => {}
        }
    }

    fn set_edr(&mut self, code: u8) {
        if self.edr == code {
            return;
        }
        self.edr = code;
        self.push_ts();
    }

    /// Bring device time forward by `us`.
    fn advance(&mut self, us: u64) {
        self.now_us = self.now_us.saturating_add(us);
        if let Some(t) = self.boot_done_at {
            if t <= self.now_us {
                self.finish_boot();
            }
        }
        if let Some(t) = self.next_status_at {
            if t <= self.now_us {
                if self.active.edr_on && self.ready() {
                    self.push_ts_repeat();
                }
                // One push per due time; a long time step does not make a burst.
                let period = self.cfg.status_period_us.max(1);
                let missed = (self.now_us - t) / period;
                self.next_status_at = Some(t + (missed + 1) * period);
            }
        }
        if self.host_rx == HostRx::Data
            && self.now_us.saturating_sub(self.data_last_us) >= self.cfg.packet_gap_us
        {
            self.flush_data();
        }
        while let Some((due, _)) = self.pending.front() {
            if *due > self.now_us {
                break;
            }
            let (_, bytes) = self.pending.pop_front().expect("front was Some");
            self.out.extend(bytes);
        }
    }

    /// The periodic EDR status: sent, not logged (the log keeps changes).
    fn push_ts_repeat(&mut self) {
        let text = format!("TS+{:02}", self.edr);
        self.push_status(text);
    }

    fn reply(&mut self, text: &str) {
        let bytes = format!("{text}\r\n").into_bytes();
        let due = self.now_us + self.cfg.reply_delay_us;
        if self.cfg.reply_delay_us == 0 {
            self.out.extend(bytes);
        } else {
            self.pending.push_back((due, bytes));
        }
    }

    fn flush_data(&mut self) {
        let data = std::mem::take(&mut self.data);
        self.host_rx = HostRx::Idle;
        if data.is_empty() {
            return;
        }
        if self.ble == Ble::Connected {
            self.log_air(format!("mcu->phone {}", hex(&data)));
            self.to_phone.extend_from_slice(&data);
        } else {
            self.log_air(format!("mcu->phone dropped (no ble link) {}", hex(&data)));
        }
    }

    fn data_byte(&mut self, byte: u8) {
        self.host_rx = HostRx::Data;
        self.data.push(byte);
        self.data_last_us = self.now_us;
        if self.data.len() >= MAX_PACKET_BYTES {
            self.flush_data();
        }
    }

    fn host_byte(&mut self, byte: u8) {
        match self.host_rx {
            HostRx::Idle => match byte {
                b'\r' | b'\n' => {}
                b'A' => {
                    self.line.clear();
                    self.line.push(byte);
                    self.host_rx = HostRx::MaybeCommand;
                }
                _ => self.data_byte(byte),
            },
            HostRx::MaybeCommand => {
                if byte == b'T' {
                    self.line.push(byte);
                    self.host_rx = HostRx::Command;
                } else {
                    self.host_rx = HostRx::Idle;
                    self.data_byte(b'A');
                    self.data_byte(byte);
                }
            }
            HostRx::Command => {
                if byte == b'\n' || byte == b'\r' {
                    let line = std::mem::take(&mut self.line);
                    self.host_rx = HostRx::Idle;
                    self.command(&line);
                } else if self.line.len() < 64 {
                    self.line.push(byte);
                }
            }
            HostRx::Data => self.data_byte(byte),
        }
    }

    /// One `AT` line (without its line end).
    fn command(&mut self, line: &[u8]) {
        let text: String = line.iter().map(|&b| b as char).collect();
        let answer = self.answer(&text);
        let entry = format!("{} {text} -> {}", self.stamp(), answer.0);
        self.at_log.push(entry);
        if answer.1 {
            // AT+CZ: the module answers, then restarts.
            self.power_on();
        }
        self.reply(&answer.0);
    }

    /// The reply to `text`, and whether the module resets after it.
    fn answer(&mut self, text: &str) -> (String, bool) {
        const ER_UNKNOWN: &str = "ER+2";
        const ER_PARAM: &str = "ER+4";
        let Some(rest) = text.strip_prefix("AT+") else {
            return (ER_UNKNOWN.to_string(), false);
        };
        if rest.len() < 2 || !rest.is_char_boundary(2) {
            return (ER_UNKNOWN.to_string(), false);
        }
        let (code, param) = rest.split_at(2);
        let ok = || ("OK".to_string(), false);
        let switch = |p: &str| match p {
            "00" => Some(false),
            "01" => Some(true),
            _ => None,
        };
        let name = |p: &str| (!p.is_empty() && p.len() <= MAX_NAME_BYTES).then(|| p.to_string());
        match code {
            "TM" if param.is_empty() => (format!("TM+{}", self.active.ble_name), false),
            "TD" if param.is_empty() => (format!("TD+{}", self.active.edr_name), false),
            "TS" if param.is_empty() => (format!("TS+{:02}", self.edr), false),
            "TL" if param.is_empty() => {
                let code = match self.ble {
                    Ble::Off => 1,
                    Ble::Advertising => 2,
                    Ble::Connected => 3,
                };
                (format!("TL+{code:02}"), false)
            }
            "QN" if param.is_empty() => (
                format!("QN+{:02}", u8::from(self.active.prompt_tone)),
                false,
            ),
            "BD" => match name(param) {
                Some(n) => {
                    self.saved.edr_name = n;
                    ok()
                }
                None => (ER_PARAM.to_string(), false),
            },
            "BM" => match name(param) {
                Some(n) => {
                    self.saved.ble_name = n;
                    ok()
                }
                None => (ER_PARAM.to_string(), false),
            },
            "CN" => match switch(param) {
                Some(on) => {
                    self.saved.prompt_tone = on;
                    self.active.prompt_tone = on;
                    ok()
                }
                None => (ER_PARAM.to_string(), false),
            },
            "B5" => match switch(param) {
                Some(on) => {
                    self.saved.edr_on = on;
                    ok()
                }
                None => (ER_PARAM.to_string(), false),
            },
            "B4" => match switch(param) {
                Some(on) => {
                    self.saved.ble_on = on;
                    ok()
                }
                None => (ER_PARAM.to_string(), false),
            },
            "CR" => match switch(param) {
                Some(on) => {
                    self.status_push = on;
                    ok()
                }
                None => (ER_PARAM.to_string(), false),
            },
            "CZ" if param.is_empty() => ("OK".to_string(), true),
            "TM" | "TD" | "TS" | "TL" | "QN" | "CZ" => (ER_PARAM.to_string(), false),
            _ => (ER_UNKNOWN.to_string(), false),
        }
    }

    /// The phone asks for an EDR state (`TS` code).
    fn phone_edr(&mut self, code: u8) {
        if !self.ready() || !self.active.edr_on {
            self.log_link(format!("edr link {code} refused (edr not ready)"));
            return;
        }
        self.set_edr(code.min(3));
    }

    /// The phone connects (true) or disconnects (false) over BLE.
    fn phone_ble(&mut self, connect: bool) {
        match (connect, self.ble) {
            (true, Ble::Advertising) => self.set_ble(Ble::Connected),
            (true, Ble::Connected) => {}
            (true, Ble::Off) => self.log_link("ble link refused (not advertising)"),
            (false, Ble::Connected) => self.set_ble(Ble::Advertising),
            (false, _) => {}
        }
    }
}

impl UartStreamDevice for Bt201 {
    fn poll(&mut self, elapsed_us: u32) -> Option<u8> {
        if elapsed_us > 0 {
            self.advance(u64::from(elapsed_us));
        }
        self.out.pop_front()
    }

    fn on_tx_byte(&mut self, byte: u8) {
        if !self.ready() {
            // The module's UART is not running yet.
            return;
        }
        self.host_byte(byte);
    }

    /// 115200 baud is about 11.5 characters per millisecond.
    fn max_bytes_per_tick(&self) -> usize {
        12
    }

    fn device_id(&self) -> Option<&str> {
        (!self.id.is_empty()).then_some(self.id.as_str())
    }

    fn inject_remote(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.ble != Ble::Connected {
            self.log_air(format!("phone->mcu dropped (no ble link) {}", hex(bytes)));
            return Ok(());
        }
        for packet in bytes.chunks(MAX_PACKET_BYTES) {
            self.log_air(format!("phone->mcu {}", hex(packet)));
            self.out.extend(packet.iter().copied());
        }
        Ok(())
    }

    fn next_wake_us(&self) -> Option<u64> {
        let mut next: Option<u64> = None;
        let mut take = |t: u64| {
            let d = t.saturating_sub(self.now_us).max(1);
            next = Some(next.map_or(d, |n| n.min(d)));
        };
        if !self.out.is_empty() {
            take(self.now_us);
        }
        if let Some(t) = self.boot_done_at {
            take(t);
        }
        if let Some((t, _)) = self.pending.front() {
            take(*t);
        }
        if let Some(t) = self.next_status_at {
            take(t);
        }
        if self.host_rx == HostRx::Data {
            take(self.data_last_us + self.cfg.packet_gap_us);
        }
        next
    }

    fn logs(&self) -> Vec<PeripheralLog> {
        let mut air = self.air_log.clone();
        if !self.data.is_empty() {
            // A packet still being collected at the end of the run.
            air.push(format!(
                "{} mcu->phone (open packet) {}",
                self.stamp(),
                hex(&self.data)
            ));
        }
        vec![
            PeripheralLog::new("at", self.at_log.clone()),
            PeripheralLog::new("link", self.link_log.clone()),
            PeripheralLog::new("air", air),
        ]
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}

impl SimInput for Bt201 {
    fn input_channels(&self) -> &[InputChannel] {
        INPUT_CHANNELS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        match key {
            "edr_link" => self.phone_edr(value.round() as u8),
            _ => self.phone_ble(value >= 0.5),
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        (!self.id.is_empty()).then_some(self.id.as_str())
    }

    fn set_component_id(&mut self, id: String) {
        self.id = id;
    }
}

// ── Kit ─────────────────────────────────────────────────────────────────────

pub struct Bt201Kit;
pub static BT201_KIT: Bt201Kit = Bt201Kit;

macro_rules! key {
    ($name:literal, $ty:ident, $doc:literal) => {
        ConfigKey {
            name: std::borrow::Cow::Borrowed($name),
            ty: ConfigType::$ty,
            doc: std::borrow::Cow::Borrowed($doc),
        }
    };
}

static BT201_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(INPUT_CHANNELS),
    device_type: std::borrow::Cow::Borrowed("bt201"),
    label: std::borrow::Cow::Borrowed("BT201 Bluetooth module (KT1025A)"),
    summary: std::borrow::Cow::Borrowed(
        "Dual-mode Bluetooth 5.0 audio + BLE module with a UART AT interface and BLE transparent data.",
    ),
    detail: std::borrow::Cow::Borrowed(
        "Jieli KT1025A/B based BT201 module, from the V2.3 user manual: AT commands with OK / \
         ER+n replies, names and radio switches kept in module flash and applied at reset \
         (AT+CZ), start-up block, TS+/TL+ link status pushes, and BLE transparent data in \
         128-byte packets. A test script drives the phone: input channels edr_link (TS code \
         0..3) and ble_link (0/1), and uart_injections with device: <id> for phone data. \
         Logs on the hosting UART: at, link, air. Not modelled: audio, SPP data, calls, the \
         card player, AT over BLE (FFF3), baud change, RF.",
    ),
    transport: Transport::Uart,
    category: Category::Uart,
    config_keys: std::borrow::Cow::Borrowed(&[
        key!(
            "edr_name",
            Str,
            "EDR (classic) name in module flash at power-on. Default BT201-AUDIO."
        ),
        key!(
            "ble_name",
            Str,
            "BLE name in module flash at power-on. Default BT201-BLE."
        ),
        key!("edr", Bool, "EDR on in module flash at power-on. Default true."),
        key!("ble", Bool, "BLE on in module flash at power-on. Default true."),
        key!(
            "boot_ms",
            Int,
            "Power-on to ready, ms. Not in the manual; default 500."
        ),
        key!(
            "reply_delay_us",
            Int,
            "Command to reply, µs. Not in the manual; default 1000."
        ),
        key!(
            "status_period_ms",
            Int,
            "EDR status (TS+) repeat period, ms; 0 = on change only. Default 500."
        ),
        key!(
            "packet_gap_us",
            Int,
            "Idle time that ends a transparent packet from the MCU, µs. Default 1000."
        ),
        key!(
            "banner",
            Bool,
            "Send the start-up block (AT+VER.., QA+.., ...) at power-on. Default true."
        ),
    ]),
    labs: std::borrow::Cow::Borrowed(&[]),
};

/// Build a module from system.yaml `config:` values.
pub fn bt201_from_config(ctx: &AttachCtx<'_>) -> Bt201 {
    let mut cfg = Bt201Config::default();
    let us = |v: i64| u64::try_from(v).unwrap_or(0);
    if let Some(v) = ctx.config_i64("boot_ms") {
        cfg.boot_us = us(v) * 1_000;
    }
    if let Some(v) = ctx.config_i64("reply_delay_us") {
        cfg.reply_delay_us = us(v);
    }
    if let Some(v) = ctx.config_i64("status_period_ms") {
        cfg.status_period_us = us(v) * 1_000;
    }
    if let Some(v) = ctx.config_i64("packet_gap_us") {
        cfg.packet_gap_us = us(v);
    }
    if let Some(v) = ctx.config_bool("banner") {
        cfg.banner = v;
    }
    let mut saved = Settings::default();
    if let Some(n) = ctx.config_str("edr_name") {
        saved.edr_name = n.to_string();
    }
    if let Some(n) = ctx.config_str("ble_name") {
        saved.ble_name = n.to_string();
    }
    if let Some(v) = ctx.config_bool("edr") {
        saved.edr_on = v;
    }
    if let Some(v) = ctx.config_bool("ble") {
        saved.ble_on = v;
    }
    let mut m = Bt201::with_settings(cfg, saved);
    m.set_id(ctx.device_id());
    m
}

impl PeripheralKit for Bt201Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &BT201_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let module = bt201_from_config(ctx);
        ctx.uart_stream_host()?
            .attach_stream_device(Box::new(module));
        Ok(())
    }
}

#[cfg(test)]
#[path = "bt201_tests.rs"]
mod tests;
