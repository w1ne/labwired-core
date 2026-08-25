// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod adc;
pub mod afio;
pub mod avr_gpio;
pub mod ble_air;
pub mod bxcan;
pub mod can;
pub mod chip_map;
pub mod comp;
pub mod components;
pub mod crc;
pub mod dac;
pub mod dbgmcu;
pub mod declarative;
pub mod dma;
pub mod dwt;
pub mod efr32;
pub mod esp32;
pub mod esp32c3;
pub mod esp32s3;
pub mod esp_gpspi_wire;
pub mod esp_i2c_core;
pub mod esp_uart;
pub mod esp_xtensa_common;
pub mod exti;
pub mod fdcan;
pub mod flash;
pub mod fmc;
pub mod generic_factory;
pub mod gpdma;
pub mod gpio;
pub mod hc_sr04;
pub mod hsem;
pub mod i2c;
pub mod i2c_temp_sensor;
pub mod i2c_waveform;
pub mod iwdg;
pub mod kit;
pub mod lptim;
pub mod mcg;
pub mod noise;
pub mod nrf52;
pub mod nrf54l;
pub mod nvic;
pub mod pad_claims;
pub mod pad_lines;
pub mod pad_routing;
pub mod pio;
pub mod pwr;
pub mod quadspi;
pub mod radio;
pub mod rcc;
pub mod rf_medium;
pub mod rng;
pub mod rp2040;
pub mod rp2040_clocks;
pub mod rsim;
pub mod rtc;
pub mod rtc_f1;
pub mod rtc_v3;
pub mod sai;
pub mod scb;
pub mod sdmmc;
pub mod simctl;
pub mod spi;
pub mod spi_waveform;
pub mod stm32f4_dma;
pub mod stub;
pub mod systick;
pub mod timer;
pub mod tsc;
pub mod uart;
pub mod uart_waveform;
pub mod usb_otg;
pub mod virtual_ble;
pub mod wave_plan;
pub mod wwdg;

/// Record one CAN/CAN-FD frame into the machine's ONE bus trace.
///
/// Shared by [`fdcan::Fdcan`] (H5) and [`bxcan::BxCan`] (F1/F4) so the two
/// controller families cannot drift in how a frame is represented. They used to
/// hold a `VecDeque<FdcanTraceFrame>` each, with a hand-copied 200-entry
/// eviction rule and a private `trace_seq` — two homes for one concept, which
/// is exactly what this module's trace unification exists to end.
pub fn push_can_trace(
    trace: &crate::bus::bus_trace::BusTrace,
    bus: &str,
    direction: &'static str,
    frame: &crate::network::CanFrame,
) {
    use crate::bus::bus_trace::{BusDir, BusPayload};
    let direction = match direction {
        "tx" => BusDir::Tx,
        "rx" => BusDir::Rx,
        other => unreachable!("CAN trace direction is tx or rx, got {other:?}"),
    };
    trace.push(
        bus,
        BusPayload::Can {
            direction,
            id: frame.id,
            data: frame.data.clone(),
            extended: frame.extended,
            fd: frame.fd,
            bitrate_switch: frame.bitrate_switch,
            remote: frame.remote,
        },
    );
}

/// The frames one CAN controller recorded, projected back into the
/// [`fdcan::FdcanTraceFrame`] shape the UDS/CAN decoders already consume.
///
/// `bus` selects this controller's rows in the shared ring; `peripheral` is the
/// name stamped onto the returned rows (the caller's display id). They are
/// separate arguments because a bare controller built in a unit test has no bus
/// name yet, and must still see its own frames.
pub fn can_trace_snapshot(
    trace: &crate::bus::bus_trace::BusTrace,
    bus: &str,
    peripheral: &str,
) -> Vec<fdcan::FdcanTraceFrame> {
    use crate::bus::bus_trace::{BusDir, BusPayload};
    trace
        .snapshot()
        .into_iter()
        .filter(|e| e.bus == bus)
        .filter_map(|e| match e.payload {
            BusPayload::Can {
                direction,
                id,
                data,
                extended,
                fd,
                bitrate_switch,
                remote,
            } => Some(fdcan::FdcanTraceFrame {
                seq: e.seq,
                peripheral: peripheral.to_string(),
                direction: match direction {
                    BusDir::Tx => "tx",
                    BusDir::Rx => "rx",
                }
                .to_string(),
                id,
                data,
                extended,
                fd,
                bitrate_switch,
                remote,
            }),
            _ => None,
        })
        .collect()
}

/// Every CAN frame in a bus-trace snapshot, from every controller, in one flat
/// list — the shape the CAN/UDS instruments consume.
///
/// The browser used to build this by walking peripherals and downcasting to
/// each concrete controller type in turn, so a new CAN family was invisible
/// until someone remembered to add an arm. Reading the shared ring makes
/// membership a consequence of recording.
pub fn can_trace_snapshot_all(
    events: &[crate::bus::bus_trace::BusTraceEvent],
) -> Vec<fdcan::FdcanTraceFrame> {
    use crate::bus::bus_trace::{BusDir, BusPayload};
    events
        .iter()
        .filter_map(|e| match &e.payload {
            BusPayload::Can {
                direction,
                id,
                data,
                extended,
                fd,
                bitrate_switch,
                remote,
            } => Some(fdcan::FdcanTraceFrame {
                seq: e.seq,
                peripheral: e.bus.clone(),
                direction: match direction {
                    BusDir::Tx => "tx",
                    BusDir::Rx => "rx",
                }
                .to_string(),
                id: *id,
                data: data.clone(),
                extended: *extended,
                fd: *fd,
                bitrate_switch: *bitrate_switch,
                remote: *remote,
            }),
            _ => None,
        })
        .collect()
}
