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

    /// Non-consuming UART trace snapshot for instruments such as the logic analyzer.
    #[wasm_bindgen]
    pub fn uart_trace_snapshot(&self) -> JsValue {
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
                let uart = any.downcast_ref::<labwired_core::peripherals::uart::Uart>()?;
                Some(serde_json::json!({
                    "peripheral": p.name,
                    "events": uart.trace_snapshot(),
                }))
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

        let snapshots = machine
            .bus
            .peripherals
            .iter()
            .flat_map(|p| {
                let Some(any) = p.dev.as_any() else {
                    return Vec::new();
                };
                // FDCAN (H5) and bxCAN (F1/F4) both feed the same CAN/UDS
                // trace so the logic analyzer works across controllers.
                if let Some(fdcan) = any.downcast_ref::<labwired_core::peripherals::fdcan::Fdcan>()
                {
                    return fdcan.trace_snapshot(&p.name);
                }
                if let Some(bxcan) = any.downcast_ref::<labwired_core::peripherals::bxcan::BxCan>()
                {
                    return bxcan.trace_snapshot(&p.name);
                }
                Vec::new()
            })
            .collect::<Vec<_>>();

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
}
