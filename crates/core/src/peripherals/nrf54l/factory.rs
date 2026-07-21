// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Data-driven factory for the Nordic nRF54L peripheral family.
//!
//! [`try_build`] owns the arms for blocks that are new to the nRF54L family,
//! so `bus::from_config` does not carry them (mirrors
//! `peripherals::nrf52::factory`). Peripherals that stayed register-compatible
//! with nRF52 — TIMER, GPIO, TEMP, EGU, GPIOTE — are still built by the nRF52
//! factory at their nRF54L bases and are deliberately not duplicated here.
//! UARTE is *not* one of them: the nRF54L generation moved EasyDMA into a
//! `DMA.{RX,TX}` cluster and renumbered the tasks/events, so it has its own
//! model here (see [`super::uarte`]).

use crate::Peripheral;
use labwired_config::{PeripheralConfig, SystemManifest};

/// Build an nRF54L peripheral model for `canonical_type`, or `None` if this
/// family does not own that type (so `from_config` falls through to the next
/// factory and then the generic match).
pub fn try_build(
    canonical_type: &str,
    p_cfg: &PeripheralConfig,
    manifest: &SystemManifest,
    bus_trace: &crate::bus::bus_trace::BusTrace,
) -> Option<Box<dyn Peripheral>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        "nrf54l_grtc" => {
            // nRF54L15 has 12 CC channels (Zephyr DT `cc-num`, MDK
            // `GRTC_CC_MaxCount`); other family members may differ.
            let num_cc: usize = p_cfg
                .config
                .get("num_cc")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(12);
            // The chip profile's `irq` is GRTC_0's NVIC position; the model
            // routes INTEN group `g` to `irq + g` (GRTC_0..3). nrfx on the
            // secure app core enables the kernel tick in group 2 and expects it
            // on GRTC_2 = irq + 2, so this base MUST reach the model.
            let irq_base = p_cfg
                .irq
                .unwrap_or(crate::peripherals::nrf54l::grtc::GRTC_IRQ_BASE_DEFAULT);
            Box::new(
                crate::peripherals::nrf54l::grtc::Nrf54lGrtc::new_with_cc_and_irq(num_cc, irq_base),
            )
        }
        "nrf54l_uarte" => Box::new(crate::peripherals::nrf54l::uarte::Nrf54lUarte::new()),
        "nrf54l_twim" => {
            let mut twim = crate::peripherals::nrf54l::twim::Nrf54lTwim::new();
            for ext in &manifest.external_devices {
                if ext.connection != p_cfg.id {
                    continue;
                }
                match crate::peripherals::components::build_external_i2c_device(
                    &ext.r#type,
                    &ext.id,
                    &ext.config,
                ) {
                    Some(device) => {
                        tracing::info!(
                            "nrf54l twim attach: '{}' (type={}) -> '{}'",
                            ext.id,
                            ext.r#type,
                            p_cfg.id
                        );
                        twim.push_slave(crate::bus::bus_trace::wrap_i2c(
                            &p_cfg.id, bus_trace, device,
                        ));
                    }
                    None => {
                        tracing::warn!(
                            "nrf54l twim attach skipped: unknown device type '{}' for '{}'",
                            ext.r#type,
                            ext.id
                        );
                    }
                }
            }
            Box::new(twim)
        }
        "nrf54l_clock" => Box::new(crate::peripherals::nrf54l::clock::Nrf54lClock::new()),
        _ => return None,
    };
    Some(dev)
}
