// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WasmSimulator trace-snapshot accessors (UART / WiFi-air / FDCAN / IO-Link),
//! exported via a second #[wasm_bindgen] impl block. Split out of lib.rs.

use crate::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
impl WasmSimulator {
    /// Snapshot of the shared virtual-air TX trace ring buffer (last
    /// ~200 BLE/proprietary frames pushed by any chip in this WASM
    /// instance, most-recent-first). The playground's BLE-on-canvas
    /// visualization polls this to render the packet trace panel; the
    /// underlying state lives in a Rust static, so any WasmSimulator
    /// can return the same snapshot — pick whichever chip is alive.
    #[wasm_bindgen]
    pub fn air_trace_snapshot(&self) -> JsValue {
        let trace = labwired_core::peripherals::nrf52::radio::virtual_air_trace_snapshot();
        serde_wasm_bindgen::to_value(&trace).unwrap_or(JsValue::NULL)
    }

    /// Drain UART TX output bytes accumulated since the last call.
    #[wasm_bindgen]
    pub fn drain_uart_output(&self) -> Vec<u8> {
        if let Ok(mut buf) = self.uart_sink.lock() {
            let data = buf.clone();
            buf.clear();
            data
        } else {
            Vec::new()
        }
    }

    /// Drain SEGGER RTT output bytes accumulated since the last call. Empty for
    /// firmware that does not link the vendor RTT library (nothing is attached).
    ///
    /// Errors when this simulator has no machine. An empty buffer is "the
    /// machine produced no RTT bytes", which a missing machine is not.
    #[wasm_bindgen]
    pub fn drain_rtt_output(&self) -> Result<Vec<u8>, JsValue> {
        Ok(self.machine_or_err()?.bus.drain_rtt_output())
    }

    /// Whether the SEGGER RTT probe model is attached to this machine —
    /// i.e. the firmware ELF resolved a `_SEGGER_RTT` symbol. The console
    /// shows its RTT source only when this is true, so a lab without RTT
    /// never grows a dead toggle.
    ///
    /// Errors when this simulator has no machine. `Ok(false)` means a loaded
    /// machine has no RTT model, which a missing machine is not.
    #[wasm_bindgen]
    pub fn rtt_attached(&self) -> Result<bool, JsValue> {
        Ok(self.machine_or_err()?.bus.segger_rtt_status().is_some())
    }

    /// Push bytes into RTT down-channel 0, the buffer `SEGGER_RTT_GetKey` and
    /// `SEGGER_RTT_Read` drain. No-op success when the machine has no RTT model
    /// is the wrong signal — callers learn that from `rtt_attached`.
    #[wasm_bindgen]
    pub fn feed_rtt_input(&self, data: &[u8]) -> Result<(), JsValue> {
        let machine = self.machine_or_err()?;
        if !machine.bus.write_rtt_input(data) {
            return Err(JsValue::from_str("SEGGER RTT is not attached"));
        }
        Ok(())
    }

    /// Why the Serial pane can be empty while the firmware is talking.
    ///
    /// An ESP32-C3/S3 has two consoles and a board's USB socket is soldered to
    /// exactly one of them, so the twin taps one — the same one the developer's
    /// cable is on. If the firmware prints to the OTHER one, a real board shows
    /// nothing and the twin faithfully shows nothing too. That is correct, and
    /// completely baffling, so this says what happened.
    ///
    /// `null` when nothing was lost. See `labwired_core::console`.
    #[wasm_bindgen]
    pub fn console_mismatch(&self) -> Option<String> {
        self.console.mismatch()
    }

    /// Raw bytes the firmware wrote to the console this board's USB connector is
    /// not wired to. Empty when there are none. Diagnostic only — these bytes
    /// are deliberately NOT merged into the Serial pane, because no real board
    /// would have delivered them.
    #[wasm_bindgen]
    pub fn unheard_console_output(&self) -> Vec<u8> {
        self.console.unheard_output()
    }

    /// Non-consuming UART trace snapshot for instruments such as the logic analyzer.
    ///
    /// Reads the machine's ONE bus trace and groups by bus name. It does NOT
    /// walk peripherals looking for a concrete type, and that is the whole
    /// point: this used to be `downcast_ref::<Uart>()`, which silently found
    /// only the generic STM32-family model. `EspUart` (ESP32-C3 / ESP32-S3),
    /// `Esp32Uart`, `Nrf52Uarte` and `Nrf54lUarte` are all UARTs and none of
    /// them is a `Uart`, so on every ESP and nRF lab this returned `[]` — the
    /// analyzer's UART panel sat empty forever with nothing to indicate an
    /// error. Asking the trace what it recorded, rather than asking the type
    /// system what a UART is, is what makes the answer complete.
    #[wasm_bindgen]
    pub fn uart_trace_snapshot(&self) -> JsValue {
        use labwired_core::bus::bus_trace::{BusDir, BusPayload};

        let Some(machine) = self.machine.as_ref() else {
            return serde_wasm_bindgen::to_value(&Vec::<serde_json::Value>::new())
                .unwrap_or(JsValue::NULL);
        };

        // Bus name → its UART events, in first-seen order so the panel's
        // instrument list is stable across polls.
        let mut order: Vec<String> = Vec::new();
        let mut by_bus: std::collections::HashMap<String, Vec<serde_json::Value>> =
            std::collections::HashMap::new();
        for e in machine.bus.bus_trace_snapshot() {
            let BusPayload::Uart { direction, byte } = e.payload else {
                continue;
            };
            let events = by_bus.entry(e.bus.clone()).or_insert_with(|| {
                order.push(e.bus.clone());
                Vec::new()
            });
            events.push(serde_json::json!({
                "seq": e.seq,
                "cycle": e.cycle,
                "direction": match direction { BusDir::Tx => "tx", BusDir::Rx => "rx" },
                "byte": byte,
            }));
        }

        let snapshots = order
            .into_iter()
            .map(|bus| {
                let events = by_bus.remove(&bus).unwrap_or_default();
                serde_json::json!({ "peripheral": bus, "events": events })
            })
            .collect::<Vec<_>>();

        serde_wasm_bindgen::to_value(&snapshots).unwrap_or(JsValue::NULL)
    }

    /// Non-consuming WiFi 802.11 frame-trace snapshot for the network analyzer
    /// (the WiFi analog of `air_trace_snapshot`). Returns, per ESP32-C3 WiFi MAC,
    /// the recently captured TX/RX frames (most-recent first); the analyzer UI
    /// decodes 802.11 type/addresses and the L3 payload (DHCP/ARP/IP).
    #[wasm_bindgen]
    pub fn wifi_trace_snapshot(&self) -> JsValue {
        let Some(machine) = self.machine.as_ref() else {
            return serde_wasm_bindgen::to_value(&Vec::<serde_json::Value>::new())
                .unwrap_or(JsValue::NULL);
        };

        let snapshots = machine
            .bus
            .peripherals
            .iter()
            .filter_map(|p| {
                let any = p.dev.as_any()?;
                let mac = any
                    .downcast_ref::<labwired_core::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac>(
                    )?;
                Some(serde_json::json!({
                    "peripheral": p.name,
                    "frames": mac.trace_snapshot(),
                }))
            })
            .collect::<Vec<_>>();

        serde_wasm_bindgen::to_value(&snapshots).unwrap_or(JsValue::NULL)
    }

    /// Non-consuming FDCAN frame trace snapshot for CAN/UDS instruments.
    #[wasm_bindgen]
    pub fn fdcan_trace_snapshot(&self) -> JsValue {
        let Some(machine) = self.machine.as_ref() else {
            return serde_wasm_bindgen::to_value(&Vec::<serde_json::Value>::new())
                .unwrap_or(JsValue::NULL);
        };

        // One ring, one read. FDCAN (H5) and bxCAN (F1/F4) both record into it,
        // so a third CAN controller family joins by recording — not by adding a
        // downcast arm here that someone has to remember to write.
        let snapshots =
            labwired_core::peripherals::can_trace_snapshot_all(&machine.bus.bus_trace_snapshot());

        serde_wasm_bindgen::to_value(&snapshots).unwrap_or(JsValue::NULL)
    }

    /// Non-consuming universal bus trace snapshot for logic analyzers.
    /// Returns the shared bus event log (seq, bus, payload) grouped by bus type.
    #[wasm_bindgen]
    pub fn bus_trace_snapshot(&self) -> JsValue {
        let Some(machine) = self.machine.as_ref() else {
            return serde_wasm_bindgen::to_value(&Vec::<serde_json::Value>::new())
                .unwrap_or(JsValue::NULL);
        };
        serde_wasm_bindgen::to_value(&machine.bus.bus_trace_snapshot()).unwrap_or(JsValue::NULL)
    }

    /// Snapshot of the IO-Link master's captured transactions (oldest→newest),
    /// for the IO-Link Analyzer instrument. Empty array if no master is wired.
    #[wasm_bindgen]
    pub fn iolink_trace_snapshot(&self) -> JsValue {
        use labwired_core::peripherals::components::IolinkMaster;
        let Some(machine) = self.machine.as_ref() else {
            return serde_wasm_bindgen::to_value(&Vec::<
                labwired_core::peripherals::components::IolinkXfer,
            >::new())
            .unwrap_or(JsValue::NULL);
        };
        for p in &machine.bus.peripherals {
            let Some(any) = p.dev.as_any() else { continue };
            let Some(uart) = any.downcast_ref::<labwired_core::peripherals::uart::Uart>() else {
                continue;
            };
            for stream in &uart.attached_streams {
                if let Some(m) = stream
                    .as_any()
                    .and_then(|a| a.downcast_ref::<IolinkMaster>())
                {
                    let trace = m.trace_snapshot();
                    return serde_wasm_bindgen::to_value(&trace).unwrap_or(JsValue::NULL);
                }
            }
        }
        serde_wasm_bindgen::to_value(
            &Vec::<labwired_core::peripherals::components::IolinkXfer>::new(),
        )
        .unwrap_or(JsValue::NULL)
    }

    /// Clear the IO-Link master's trace ring.
    #[wasm_bindgen]
    pub fn iolink_trace_clear(&mut self) {
        use labwired_core::peripherals::components::IolinkMaster;
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        for p in &mut machine.bus.peripherals {
            let Some(any) = p.dev.as_any_mut() else {
                continue;
            };
            let Some(uart) = any.downcast_mut::<labwired_core::peripherals::uart::Uart>() else {
                continue;
            };
            for stream in &mut uart.attached_streams {
                if let Some(m) = stream
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<IolinkMaster>())
                {
                    m.trace_clear();
                    return;
                }
            }
        }
    }

    /// Channel table of the in-core analog engine's waveform trace: one entry
    /// per probed model output plus any extra `trace:` expressions, each with
    /// its unit (`"V"` or `"A"`).
    ///
    /// Empty until a co-simulation runner carrying an `adapter: analog` model
    /// is attached to the machine. Empty is the honest answer for a lab with no
    /// circuit in it: the oscilloscope shows no channels rather than a flat
    /// line at zero that nothing measured.
    #[wasm_bindgen]
    pub fn analog_channels(&self) -> Result<JsValue, JsValue> {
        let channels = self
            .machine
            .as_ref()
            .map(|machine| machine.analog_channels())
            .unwrap_or_default();
        serde_wasm_bindgen::to_value(&channels)
            .map_err(|err| JsValue::from_str(&format!("analog_channels: {err}")))
    }

    /// Analog samples newer than `cursor`, plus the cursor to pass next time.
    ///
    /// Cursors are sample sequence numbers, the same contract as
    /// `read_logic_edges`, and are JS numbers for the same reason: sample
    /// counts stay far under 2^53, and a `BigInt` at this boundary would make
    /// the scope panel the only caller in the playground that cannot pass a
    /// plain `0`.
    ///
    /// `dropped` counts samples the ring overwrote before they were read, so
    /// the panel can mark a gap instead of drawing a straight line across one.
    #[wasm_bindgen]
    pub fn analog_trace_snapshot(&self, cursor: f64) -> Result<JsValue, JsValue> {
        let cursor = if cursor.is_finite() && cursor >= 0.0 {
            cursor as u64
        } else {
            return Err(JsValue::from_str(
                "analog_trace_snapshot: cursor must be a non-negative number",
            ));
        };
        serde_wasm_bindgen::to_value(&self.analog_trace_batch(cursor))
            .map_err(|err| JsValue::from_str(&format!("analog_trace_snapshot: {err}")))
    }
}

impl WasmSimulator {
    /// The live analog ring behind [`Self::analog_trace_snapshot`]: the samples
    /// newer than `cursor` that the co-simulation session's analog models wrote,
    /// and an empty batch when no session is attached.
    pub(crate) fn analog_trace_batch(
        &self,
        cursor: u64,
    ) -> labwired_core::analog::AnalogTraceBatch {
        self.machine
            .as_ref()
            .map(|machine| machine.analog_trace_snapshot(cursor))
            .unwrap_or_default()
    }
}
