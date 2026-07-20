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
//! with nRF52 — UARTE, TIMER, GPIO, TEMP, EGU, GPIOTE — are still built by the
//! nRF52 factory at their nRF54L bases and are deliberately not duplicated here.

use crate::Peripheral;
use labwired_config::PeripheralConfig;

/// Build an nRF54L peripheral model for `canonical_type`, or `None` if this
/// family does not own that type (so `from_config` falls through to the next
/// factory and then the generic match).
pub fn try_build(canonical_type: &str, p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
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
            Box::new(crate::peripherals::nrf54l::grtc::Nrf54lGrtc::new_with_cc(
                num_cc,
            ))
        }
        _ => return None,
    };
    Some(dev)
}
