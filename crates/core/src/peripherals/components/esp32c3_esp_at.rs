//! Bounded behavioral ESP-AT subset, not ESP firmware or RF execution.
//! Strict CRLF, 256-byte commands, 1024-byte reply queue, one byte per >=1ms
//! poll. Excess timing credit is discarded; this is functional pacing only.
use super::supply_domain::SupplyGate;
use crate::{
    inspect::{Artifact, InspectOpts},
    peripheral_log::PeripheralLog,
    peripherals::{
        device::UartStreamDevice,
        kit::{AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport},
    },
    sim_input::{InputChannel, SimInput, SimInputError},
};
use std::{borrow::Cow, collections::VecDeque};
const COMMAND_CAP: usize = 256;
const REPLY_CAP: usize = 1024;
const LOG_CAP: usize = 128;
const INPUTS: &[InputChannel] = &[InputChannel {
    key: Cow::Borrowed("mock_connected"),
    label: Cow::Borrowed("Mock connection"),
    unit: Cow::Borrowed("boolean"),
    min: 0.,
    max: 1.,
    default: Some(0.),
}];
pub struct Esp32c3EspAt {
    id: String,
    gate: SupplyGate,
    epoch: Option<u64>,
    command: Vec<u8>,
    reply: VecDeque<u8>,
    draining: bool,
    previous_cr: bool,
    credit_us: u32,
    connected: bool,
    events: VecDeque<String>,
}
impl Esp32c3EspAt {
    pub fn new(id: String, gate: SupplyGate) -> Self {
        let epoch = gate.generation();
        Self {
            id,
            gate,
            epoch,
            command: Vec::with_capacity(COMMAND_CAP),
            reply: VecDeque::with_capacity(REPLY_CAP),
            draining: false,
            previous_cr: false,
            credit_us: 0,
            connected: false,
            events: VecDeque::new(),
        }
    }
    fn live(&self) -> bool {
        self.gate.powered() && self.epoch == self.gate.generation()
    }
    fn reconcile(&mut self) {
        let epoch = self.gate.generation();
        if epoch != self.epoch || !self.gate.powered() {
            self.command.clear();
            self.reply.clear();
            self.draining = false;
            self.previous_cr = false;
            self.credit_us = 0;
            self.connected = false;
            self.epoch = epoch;
        }
    }
    fn log(&mut self, s: String) {
        if self.events.len() == LOG_CAP {
            self.events.pop_front();
        }
        self.events.push_back(s);
    }
    fn append_command(&mut self, b: u8) {
        if self.draining {
            return;
        }
        if self.command.len() < COMMAND_CAP {
            self.command.push(b);
        } else {
            self.command.clear();
            self.draining = true;
        }
    }
    fn answer(&mut self) {
        // Backpressure: consume a complete frame but never exceed the sole reply
        // queue. Reject before changing mock state when even ERROR will not fit.
        if self.reply.len() > REPLY_CAP - 7 {
            self.log("reply capacity: frame dropped".into());
            return;
        }
        let frame = std::str::from_utf8(&self.command).unwrap_or("");
        let reply = if self.draining {
            "ERROR\r\n"
        } else {
            match frame {
                "AT" => "OK\r\n",
                "AT+GMR" => "LabWired behavioral ESP32-C3 ESP-AT mock\r\nOK\r\n",
                "AT+LABMOCKSTATUS?" => {
                    if self.connected {
                        "+LABMOCKSTATUS:CONNECTED\r\nOK\r\n"
                    } else {
                        "+LABMOCKSTATUS:DISCONNECTED\r\nOK\r\n"
                    }
                }
                "AT+LABMOCKCONNECT" => "OK\r\n",
                "AT+LABMOCKDISCONNECT" => "OK\r\n",
                _ => "ERROR\r\n",
            }
        };
        if self.reply.len() + reply.len() > REPLY_CAP {
            self.reply.extend(b"ERROR\r\n");
            self.log("reply capacity: ERROR".into());
            return;
        }
        if !self.draining {
            if frame == "AT+LABMOCKCONNECT" {
                self.connected = true;
            }
            if frame == "AT+LABMOCKDISCONNECT" {
                self.connected = false;
            }
        }
        let text = if self.draining {
            "overflow".to_owned()
        } else {
            String::from_utf8_lossy(&self.command).into_owned()
        };
        self.log(format!("{text} -> {}", reply.trim_end()));
        self.reply.extend(reply.bytes());
    }
}
impl UartStreamDevice for Esp32c3EspAt {
    fn stream_generation(&self) -> Option<u64> {
        self.gate.generation()
    }
    fn device_id(&self) -> Option<&str> {
        Some(&self.id)
    }
    fn carries_protocol_octets(&self) -> bool {
        true
    }
    fn poll(&mut self, us: u32) -> Option<u8> {
        self.reconcile();
        if !self.gate.powered() {
            return None;
        }
        if self.reply.is_empty() {
            self.credit_us = 0;
            return None;
        }
        self.credit_us = self.credit_us.saturating_add(us).min(1000);
        if self.credit_us < 1000 {
            return None;
        }
        self.credit_us = 0;
        self.reply.pop_front()
    }
    fn on_tx_byte(&mut self, b: u8) {
        self.reconcile();
        if !self.gate.powered() {
            return;
        }
        if self.previous_cr && b == b'\n' {
            self.answer();
            self.command.clear();
            self.draining = false;
            self.previous_cr = false;
            return;
        }
        // A pending CR lives in framing state, outside the bounded payload.
        // If it is not followed by LF, it is malformed command payload.
        if self.previous_cr {
            self.append_command(b'\r');
        }
        self.previous_cr = b == b'\r';
        if !self.previous_cr {
            self.append_command(b);
        }
    }
    fn logs(&self) -> Vec<PeripheralLog> {
        vec![PeripheralLog::new(
            "esp_at",
            self.events.iter().cloned().collect(),
        )]
    }
    fn artifacts(&self, id: &str, _: &InspectOpts) -> Vec<Artifact> {
        let live = self.live();
        vec![Artifact {
            kind: "uart_peer".into(),
            id: id.into(),
            meta: serde_json::json!({"behavioral_mock":true,"powered":self.gate.powered(),"mock_connected":live && self.connected,"command_bytes":if live{self.command.len()}else{0},"reply_bytes":if live{self.reply.len()}else{0},"supply_generation":self.gate.generation()}),
            bytes: None,
        }]
    }
    fn as_sim_input_mut(&mut self) -> Option<&mut dyn SimInput> {
        Some(self)
    }
}
impl SimInput for Esp32c3EspAt {
    fn input_channels(&self) -> &[InputChannel] {
        INPUTS
    }
    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
    fn set_component_id(&mut self, id: String) {
        self.id = id;
    }
    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        self.reconcile();
        if self.gate.powered() {
            self.connected = value >= 0.5;
            self.log(format!("stimulus mock_connected={}", self.connected));
        }
        Ok(())
    }
}
pub struct EspAtKit;
pub static ESP_AT_KIT: EspAtKit = EspAtKit;
static META:KitMetadata=KitMetadata{device_type:Cow::Borrowed("esp32c3_esp_at"),label:Cow::Borrowed("ESP32-C3 behavioral ESP-AT"),summary:Cow::Borrowed("Bounded AT peer with explicit mock connection state"),detail:Cow::Borrowed("Behavioral mock only; no ESP firmware or RF. Strict CRLF. AT, AT+GMR, AT+LABMOCKSTATUS?, AT+LABMOCKCONNECT, AT+LABMOCKDISCONNECT; unknown/malformed frames return ERROR. Commands 256 bytes, replies 1024 bytes; saturation drops complete frames before mock-state changes. One reply byte per >=1000us poll; no baud accuracy."),transport:Transport::Uart,category:Category::Uart,config_keys:Cow::Borrowed(&[super::supply::POWERED_CONFIG_KEY,ConfigKey{name:Cow::Borrowed("supply_domain"),ty:ConfigType::Str,doc:Cow::Borrowed("Explicit pre-registered external supply domain; starts off until a finite voltage update.")},ConfigKey{name:Cow::Borrowed("supply_min_v"),ty:ConfigType::Float,doc:Cow::Borrowed("Functional gate threshold, must match domain registration; no silicon threshold claim.")}]),labs:Cow::Borrowed(&[]),inputs:Cow::Borrowed(INPUTS)};
impl PeripheralKit for EspAtKit {
    fn metadata(&self) -> &KitMetadata {
        &META
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let gate = SupplyGate::from_config(ctx)?;
        let routed = gate.generation().is_some();
        let peer = Box::new(Esp32c3EspAt::new(ctx.device_id().into(), gate));
        let host = ctx.uart_stream_host()?;
        if routed {
            host.attach_exclusive_stream_device(peer)
                .map_err(anyhow::Error::msg)?;
        } else {
            host.try_attach_stream_device(peer)
                .map_err(anyhow::Error::msg)?;
        }
        host.detach_console_sink();
        Ok(())
    }
}
