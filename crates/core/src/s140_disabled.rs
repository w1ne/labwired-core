// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! nRF52840 "S140 present but disabled" SoftDevice shim.
//!
//! Some nRF52 Arduino images are linked as SoftDevice S140 applications
//! (vector table at [`APP_BASE`]) and issue SoftDevice SVCs, but never
//! enable BLE. The existing nRF51 S110 HLE does not cover that layout.
//!
//! This path loads only the application region, sets VTOR to [`APP_BASE`],
//! and intercepts SoftDevice SVCs at the application's SVCall vector.
//! FreeRTOS's `SVC #0` is left to the application's handler.
//!
//! SVC numbers and the calls this shim implements were taken from the
//! Spooky Keychain Arcade ELF (`tests/fixtures/s140-disabled/`), not from
//! Nordic SoftDevice sources or binaries. Return codes use the same public
//! error numbers already recorded in `nrf-softdevice-hle` facts (S110/S130
//! family). Unknown SVCs return `NRF_ERROR_SVC_HANDLER_MISSING`. Calls that
//! require an enabled SoftDevice return `NRF_ERROR_SOFTDEVICE_NOT_ENABLED`.
//! `sd_softdevice_is_enabled` writes 0 (not enabled) and returns success.
//! `sd_flash_write` / `sd_flash_page_erase` are honoured (the firmware
//! always issues them, even when the SoftDevice is not enabled) by writing
//! the application flash image directly.

use crate::bus::SystemBus;
use crate::cpu::Cpu;
use crate::{Bus, Machine, SimResult};

/// S140 v7.3.0 application vector table / load base observed on this firmware.
pub const APP_BASE: u32 = 0x27000;

/// nRF52840 code flash page size, as used by this firmware (`lsrs rd, #12`).
pub const FLASH_PAGE_SIZE: u32 = 0x1000;

/// Observed in the ELF: `sd_softdevice_is_enabled` → `svc 18`.
pub const SVC_IS_ENABLED: u8 = 0x12;
/// Observed: `sd_flash_page_erase` → `svc 40`.
pub const SVC_FLASH_PAGE_ERASE: u8 = 0x28;
/// Observed: `sd_flash_write` → `svc 41`.
pub const SVC_FLASH_WRITE: u8 = 0x29;
/// Observed: `sd_app_evt_wait` → `svc 65`.
pub const SVC_APP_EVT_WAIT: u8 = 0x41;
/// Observed: `sd_clock_hfclk_request` → `svc 66`.
pub const SVC_CLOCK_HFCLK_REQUEST: u8 = 0x42;
/// Observed: `sd_clock_hfclk_release` → `svc 67`.
pub const SVC_CLOCK_HFCLK_RELEASE: u8 = 0x43;
/// Observed: `sd_clock_hfclk_is_running` → `svc 68`.
pub const SVC_CLOCK_HFCLK_IS_RUNNING: u8 = 0x44;

/// FreeRTOS `vPortStartFirstTask` issues `svc 0`. Not a SoftDevice call.
pub const SVC_FREERTOS: u8 = 0;

pub const NRF_SUCCESS: u32 = 0x0;
pub const NRF_ERROR_SVC_HANDLER_MISSING: u32 = 0x1;
pub const NRF_ERROR_SOFTDEVICE_NOT_ENABLED: u32 = 0x2;
pub const NRF_ERROR_NULL: u32 = 0xE;
pub const NRF_ERROR_INVALID_ADDR: u32 = 0x10;

/// Attached shim state.
pub struct S140DisabledSlot {
    pub app_base: u32,
    pub svc_handler: u32,
    pub svc_count: u64,
    pub is_enabled_count: u64,
}

/// Keep only bytes at or above [`APP_BASE`] from a program image, so a
/// 64 KiB-aligned ELF PT_LOAD that starts at 0x20000 cannot paint the ELF
/// header into the SoftDevice hole.
pub fn clip_app_image(image: &crate::memory::ProgramImage) -> crate::memory::ProgramImage {
    let mut out = crate::memory::ProgramImage::new(image.entry_point, image.arch);
    let base = APP_BASE as u64;
    for seg in &image.segments {
        let start = seg.start_addr;
        let end = start + seg.data.len() as u64;
        if end <= base {
            continue;
        }
        if start >= base {
            out.add_segment(start, seg.data.clone());
            continue;
        }
        let skip = (base - start) as usize;
        out.add_segment(base, seg.data[skip..].to_vec());
    }
    out
}

/// Dispatch one SoftDevice SVC. `args` are the stacked r0-r3.
pub fn dispatch_svc(num: u8, args: [u32; 4], bus: &mut SystemBus) -> Option<u32> {
    if num == SVC_FREERTOS {
        return None;
    }
    Some(match num {
        SVC_IS_ENABLED => svc_is_enabled(args, bus),
        SVC_FLASH_PAGE_ERASE => svc_flash_page_erase(args, bus),
        SVC_FLASH_WRITE => svc_flash_write(args, bus),
        SVC_APP_EVT_WAIT
        | SVC_CLOCK_HFCLK_REQUEST
        | SVC_CLOCK_HFCLK_RELEASE
        | SVC_CLOCK_HFCLK_IS_RUNNING => NRF_ERROR_SOFTDEVICE_NOT_ENABLED,
        _ => NRF_ERROR_SVC_HANDLER_MISSING,
    })
}

fn svc_is_enabled(args: [u32; 4], bus: &mut SystemBus) -> u32 {
    let ptr = args[0];
    if ptr == 0 {
        return NRF_ERROR_NULL;
    }
    match bus.write_u8(ptr as u64, 0) {
        Ok(()) => NRF_SUCCESS,
        Err(_) => NRF_ERROR_INVALID_ADDR,
    }
}

fn in_app_flash(addr: u32, len: u32) -> bool {
    let Some(end) = addr.checked_add(len) else {
        return false;
    };
    addr >= APP_BASE && end <= 0x0010_0000
}

fn svc_flash_write(args: [u32; 4], bus: &mut SystemBus) -> u32 {
    let dst = args[0];
    let src = args[1];
    let words = args[2];
    if dst == 0 || src == 0 || dst % 4 != 0 || src % 4 != 0 {
        return NRF_ERROR_INVALID_ADDR;
    }
    let Some(bytes) = words.checked_mul(4) else {
        return NRF_ERROR_INVALID_ADDR;
    };
    if !in_app_flash(dst, bytes) {
        return NRF_ERROR_INVALID_ADDR;
    }
    for i in 0..bytes {
        let Ok(incoming) = bus.read_u8(src as u64 + i as u64) else {
            return NRF_ERROR_INVALID_ADDR;
        };
        let a = dst as u64 + i as u64;
        let existing = bus.flash.read_u8(a).unwrap_or(0xFF);
        bus.flash.write_u8(a, existing & incoming);
    }
    NRF_SUCCESS
}

fn svc_flash_page_erase(args: [u32; 4], bus: &mut SystemBus) -> u32 {
    let page = args[0];
    let Some(addr) = page.checked_mul(FLASH_PAGE_SIZE) else {
        return NRF_ERROR_INVALID_ADDR;
    };
    if !in_app_flash(addr, FLASH_PAGE_SIZE) {
        return NRF_ERROR_INVALID_ADDR;
    }
    for i in 0..FLASH_PAGE_SIZE {
        bus.flash.write_u8(addr as u64 + i as u64, 0xFF);
    }
    NRF_SUCCESS
}

impl<C: Cpu> Machine<C> {
    /// Attach the disabled-S140 shim. Call after the application image is
    /// loaded. Sets VTOR, SP and PC from the table at `app_base`.
    pub fn attach_s140_disabled(&mut self, app_base: u32) -> SimResult<()> {
        if !self.bus.vector_pair_valid(app_base as u64) {
            return Err(crate::SimulationError::Other(
                "S140-disabled attach needs a valid vector table at the application base".into(),
            ));
        }
        let handler = self.bus.read_u32(app_base as u64 + 0x2C)? & !1;
        let sp = self.bus.read_u32(app_base as u64)?;
        let pc = self.bus.read_u32(app_base as u64 + 4)? & !1;
        if !self.cpu.set_vector_table_base(app_base) {
            return Err(crate::SimulationError::Other(
                "the S140-disabled shim needs a Cortex-M core".into(),
            ));
        }
        let _ = self.bus.write_u32(0xE000_ED08, app_base);
        self.cpu.set_sp(sp);
        self.cpu.set_pc(pc);
        if self.scb_index.is_some() {
            self.boot_vtor = Some(app_base);
        }
        self.s140_disabled = Some(Box::new(S140DisabledSlot {
            app_base,
            svc_handler: handler,
            svc_count: 0,
            is_enabled_count: 0,
        }));
        Ok(())
    }

    pub(crate) fn service_s140_disabled(&mut self) -> SimResult<()> {
        let Some(mut slot) = self.s140_disabled.take() else {
            return Ok(());
        };
        let r = self.service_s140_disabled_inner(&mut slot);
        self.s140_disabled = Some(slot);
        r
    }

    fn service_s140_disabled_inner(&mut self, slot: &mut S140DisabledSlot) -> SimResult<()> {
        let Some(frame) = self.cpu.svcall_frame_at(slot.svc_handler) else {
            return Ok(());
        };
        let mut args = [0u32; 4];
        for (i, v) in args.iter_mut().enumerate() {
            *v = self.bus.read_u32(frame as u64 + 4 * i as u64)?;
        }
        let ret_pc = self.bus.read_u32(frame as u64 + 24)?;
        let num = self.bus.read_u8(ret_pc as u64 - 2)?;
        let Some(result) = dispatch_svc(num, args, &mut self.bus) else {
            return Ok(());
        };
        if num == SVC_IS_ENABLED {
            slot.is_enabled_count += 1;
        }
        self.bus.write_u32(frame as u64, result)?;
        slot.svc_count += 1;
        self.cpu.hle_return_from_exception(&mut self.bus)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::memory::LinearMemory;

    fn flash_bus() -> SystemBus {
        let mut bus = SystemBus::empty();
        bus.flash = LinearMemory::new_erased(0x0010_0000, 0);
        bus.ram = LinearMemory::new(0x0004_0000, 0x2000_0000);
        bus
    }

    #[test]
    fn is_enabled_writes_zero_and_returns_success() {
        let mut bus = flash_bus();
        let ptr = 0x2000_1000;
        let rc = dispatch_svc(SVC_IS_ENABLED, [ptr, 0, 0, 0], &mut bus).unwrap();
        assert_eq!(rc, NRF_SUCCESS);
        assert_eq!(bus.read_u8(ptr as u64).unwrap(), 0);
    }

    #[test]
    fn is_enabled_null_pointer_is_documented_error() {
        let mut bus = flash_bus();
        let rc = dispatch_svc(SVC_IS_ENABLED, [0, 0, 0, 0], &mut bus).unwrap();
        assert_eq!(rc, NRF_ERROR_NULL);
    }

    #[test]
    fn freertos_svc_is_not_intercepted() {
        let mut bus = flash_bus();
        assert!(dispatch_svc(SVC_FREERTOS, [0, 0, 0, 0], &mut bus).is_none());
    }

    #[test]
    fn clock_and_event_wait_require_an_enabled_softdevice() {
        let mut bus = flash_bus();
        for num in [
            SVC_APP_EVT_WAIT,
            SVC_CLOCK_HFCLK_REQUEST,
            SVC_CLOCK_HFCLK_RELEASE,
            SVC_CLOCK_HFCLK_IS_RUNNING,
        ] {
            let rc = dispatch_svc(num, [0, 0, 0, 0], &mut bus).unwrap();
            assert_eq!(rc, NRF_ERROR_SOFTDEVICE_NOT_ENABLED, "svc {num}");
        }
    }

    #[test]
    fn unknown_svc_returns_handler_missing() {
        let mut bus = flash_bus();
        let rc = dispatch_svc(0x20, [0, 0, 0, 0], &mut bus).unwrap();
        assert_eq!(rc, NRF_ERROR_SVC_HANDLER_MISSING);
    }

    #[test]
    fn flash_write_ands_and_erase_restores_ones() {
        let mut bus = flash_bus();
        let dst = APP_BASE + 0x100;
        let src = 0x2000_2000;
        bus.write_u32(src as u64, 0x0000_FFFF).unwrap();
        let rc = dispatch_svc(SVC_FLASH_WRITE, [dst, src, 1, 0], &mut bus).unwrap();
        assert_eq!(rc, NRF_SUCCESS);
        assert_eq!(bus.flash.read_u32(dst as u64), Some(0x0000_FFFF));
        bus.write_u32(src as u64, 0x00FF_0000).unwrap();
        let rc = dispatch_svc(SVC_FLASH_WRITE, [dst, src, 1, 0], &mut bus).unwrap();
        assert_eq!(rc, NRF_SUCCESS);
        assert_eq!(bus.flash.read_u32(dst as u64), Some(0x0000_0000));
        let page = dst / FLASH_PAGE_SIZE;
        let rc = dispatch_svc(SVC_FLASH_PAGE_ERASE, [page, 0, 0, 0], &mut bus).unwrap();
        assert_eq!(rc, NRF_SUCCESS);
        assert_eq!(bus.flash.read_u32(dst as u64), Some(0xFFFF_FFFF));
    }

    #[test]
    fn flash_write_below_app_base_is_rejected() {
        let mut bus = flash_bus();
        let src = 0x2000_2000;
        bus.write_u32(src as u64, 0x1234_5678).unwrap();
        let rc = dispatch_svc(SVC_FLASH_WRITE, [0x20000, src, 1, 0], &mut bus).unwrap();
        assert_eq!(rc, NRF_ERROR_INVALID_ADDR);
    }

    #[test]
    fn clip_app_image_drops_bytes_below_the_application_base() {
        let mut image = crate::memory::ProgramImage::new(0x51DBD, crate::Arch::Arm);
        image.add_segment(0x20000, vec![0xAA; 0x8000]);
        let clipped = clip_app_image(&image);
        assert_eq!(clipped.segments.len(), 1);
        assert_eq!(clipped.segments[0].start_addr, APP_BASE as u64);
        assert_eq!(clipped.segments[0].data.len(), 0x1000);
        assert!(clipped.segments[0].data.iter().all(|&b| b == 0xAA));
    }
}
