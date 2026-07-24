// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Data-driven factory for the Nordic nRF52 peripheral family.
//!
//! [`try_build`] owns the nRF52 / nRF52840 peripheral arms that used to live
//! inline in `bus::from_config`, so that central match stops carrying one
//! family's worth of peripherals (mirrors `peripherals::esp32s3::factory`).

use crate::Peripheral;
use labwired_config::{PeripheralConfig, SystemManifest};

/// Build an nRF52 peripheral model for `canonical_type`, or `None` if this
/// family does not own that type (so `from_config` falls through to the
/// generic match). `manifest` is consulted for external I²C devices attached to
/// a TWIM controller.
pub fn try_build(
    canonical_type: &str,
    p_cfg: &PeripheralConfig,
    manifest: &SystemManifest,
    bus_trace: &crate::bus::bus_trace::BusTrace,
) -> Option<Box<dyn Peripheral>> {
    let dev: Box<dyn Peripheral> = match canonical_type {
        "nrf52840_uart" => Box::new(crate::peripherals::nrf52::uarte::Nrf52Uarte::new()),
        "nrf52840_rtc" | "nrf52_rtc" => {
            // RTC1/RTC2 have 4 CC; RTC0 has 3 (default).
            let num_cc: usize = p_cfg
                .config
                .get("num_cc")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(3);
            Box::new(crate::peripherals::nrf52::rtc::Nrf52Rtc::new_with_cc(
                num_cc,
            ))
        }
        "nrf52840_rng" | "nrf52_rng" => Box::new(crate::peripherals::nrf52::rng::Nrf52Rng::new()),
        "nrf52840_watchdog" | "nrf52_watchdog" | "nrf52_wdt" => {
            Box::new(crate::peripherals::nrf52::wdt::Nrf52Wdt::new())
        }
        "nrf52840_ppi" | "nrf52_ppi" => Box::new(crate::peripherals::nrf52::ppi::Nrf52Ppi::new()),
        "nrf52840_pdm" | "nrf52_pdm" => Box::new(crate::peripherals::nrf52::pdm::Nrf52Pdm::new()),
        "nrf52_gpiote" | "nrf52840_gpiotasksevents" => {
            Box::new(crate::peripherals::nrf52::gpiote::Nrf52Gpiote::new())
        }
        "nrf52840_ecb" | "nrf52_ecb" => Box::new(crate::peripherals::nrf52::ecb::Nrf52Ecb::new()),
        "nrf52_clock" => Box::new(crate::peripherals::nrf52::clock::Nrf52Clock::new()),
        "nrf52840_temp" | "nrf52_temp" => {
            Box::new(crate::peripherals::nrf52::temp::Nrf52Temp::new())
        }
        "nrf52840_adc" | "nrf52840_saadc" | "nrf52_saadc" => {
            Box::new(crate::peripherals::nrf52::saadc::Nrf52Saadc::new())
        }
        "nrf52840_pwm" | "nrf52_pwm" => Box::new(crate::peripherals::nrf52::pwm::Nrf52Pwm::new()),
        "nrf52840_qspi" | "nrf52_qspi" => {
            Box::new(crate::peripherals::nrf52::qspi::Nrf52Qspi::new())
        }
        "nrf52840_nfct" | "nrf52_nfct" => {
            Box::new(crate::peripherals::nrf52::nfct::Nrf52Nfct::new())
        }
        "nrf52840_ficr" | "nrf52_ficr" => {
            Box::new(crate::peripherals::nrf52::ficr::Nrf52Ficr::new())
        }
        "nrf52840_uicr" | "nrf52_uicr" => {
            Box::new(crate::peripherals::nrf52::uicr::Nrf52Uicr::new())
        }
        "nrf52840_nvmc" | "nrf52_nvmc" => {
            Box::new(crate::peripherals::nrf52::nvmc::Nrf52Nvmc::new())
        }
        "nrf52840_egu" | "nrf52_egu" => Box::new(crate::peripherals::nrf52::egu::Nrf52Egu::new()),
        "nrf52840_comp" | "nrf52_comp" => {
            Box::new(crate::peripherals::nrf52::comp::Nrf52Comp::new())
        }
        "nrf52840_lpcomp" | "nrf52_lpcomp" => {
            Box::new(crate::peripherals::nrf52::lpcomp::Nrf52Lpcomp::new())
        }
        "nrf52840_qdec" | "nrf52_qdec" => {
            Box::new(crate::peripherals::nrf52::qdec::Nrf52Qdec::new())
        }
        "nrf52840_i2s" | "nrf52_i2s" => Box::new(crate::peripherals::nrf52::i2s::Nrf52I2s::new()),
        "nrf52840_mwu" | "nrf52_mwu" => Box::new(crate::peripherals::nrf52::mwu::Nrf52Mwu::new()),
        "nrf52840_aar" | "nrf52_aar" => Box::new(crate::peripherals::nrf52::aar::Nrf52Aar::new()),
        "nrf52840_ccm" | "nrf52_ccm" => Box::new(crate::peripherals::nrf52::ccm::Nrf52Ccm::new()),
        "nrf52840_bprot" | "nrf52_bprot" => {
            Box::new(crate::peripherals::nrf52::bprot::Nrf52Bprot::new())
        }
        "nrf52840_radio" | "nrf52_radio" => {
            Box::new(crate::peripherals::nrf52::radio::Nrf52Radio::new())
        }
        "nrf52840_usbd" | "nrf52_usbd" => {
            Box::new(crate::peripherals::nrf52::usbd::Nrf52Usbd::new())
        }
        "nrf52840_acl" | "nrf52_acl" => Box::new(crate::peripherals::nrf52::acl::Nrf52Acl::new()),
        "nrf52840_cryptocell" | "nrf52_cryptocell" => {
            Box::new(crate::peripherals::nrf52::cryptocell::Nrf52Cryptocell::new())
        }
        "nrf52840_usbregulator" | "nrf52_usbregulator" => {
            Box::new(crate::peripherals::nrf52::usbregulator::Nrf52UsbRegulator::new())
        }
        "nrf52840_spis" | "nrf52_spis" => {
            Box::new(crate::peripherals::nrf52::spis::Nrf52Spis::new())
        }
        "nrf52840_twis" | "nrf52_twis" => {
            Box::new(crate::peripherals::nrf52::twis::Nrf52Twis::new())
        }
        "nrf52840_twim" | "nrf52_twim" => {
            let mut twim = crate::peripherals::nrf52::twim::Nrf52Twim::new();
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
                            "twim attach: '{}' (type={}) -> '{}'",
                            ext.id,
                            ext.r#type,
                            p_cfg.id
                        );
                        // Wrap through the single trace helper before the raw
                        // push — same contract as the bus choke point, since the
                        // factory attaches before the peripheral is on the bus.
                        twim.push_slave(crate::bus::bus_trace::wrap_i2c(
                            &p_cfg.id, bus_trace, device,
                        ));
                    }
                    None => {
                        tracing::warn!(
                            "twim attach skipped: unknown device type '{}' \
                                         for external id '{}' on bus '{}'",
                            ext.r#type,
                            ext.id,
                            p_cfg.id
                        );
                    }
                }
            }
            Box::new(twim)
        }
        // nRF52 serial-instance mux: SPIM0/TWIM0 at a single MMIO base.
        // ENABLE=6 selects TWIM, ENABLE=7 selects SPIM (nRF52840 PS §6.31/§6.30).
        "nrf52_serial_instance" => {
            let mut inst = crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance::new();
            for ext in &manifest.external_devices {
                if ext.connection != p_cfg.id {
                    continue;
                }
                // Try I²C device first.
                if let Some(device) = crate::peripherals::components::build_external_i2c_device(
                    &ext.r#type,
                    &ext.id,
                    &ext.config,
                ) {
                    tracing::info!(
                        "serial-instance i2c attach: '{}' (type={}) -> '{}'",
                        ext.id,
                        ext.r#type,
                        p_cfg.id
                    );
                    inst.attach_i2c(crate::bus::bus_trace::wrap_i2c(
                        &p_cfg.id, bus_trace, device,
                    ));
                } else {
                    tracing::warn!(
                        "serial-instance attach skipped: unknown device type '{}' \
                         for external id '{}' on bus '{}'",
                        ext.r#type,
                        ext.id,
                        p_cfg.id
                    );
                }
            }
            Box::new(inst)
        }
        _ => return None,
    };
    Some(dev)
}
