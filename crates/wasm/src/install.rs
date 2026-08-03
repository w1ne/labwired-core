// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WasmSimulator firmware-quirk installers + runtime-snapshot save/restore.
//! Split out of lib.rs.

use crate::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
impl WasmSimulator {
    /// Arduino-ESP32 boot bootstrap (symbol-table autodiscovery).
    ///
    /// Mirrors the CLI's `arduino-esp32` snapshot-capture profile —
    /// resolves Arduino-ESP32 thunk PCs from the ELF symbol table instead
    /// of hand-curated hardcoded addresses. Works for any GxEPD2-class
    /// sketch (labwired-ereader, future user sketches) without needing
    /// to know its binary layout in advance.
    ///
    /// Caller must pass the same ELF bytes that were loaded via
    /// `load_firmware`. The thunks are installed as flash patches over
    /// the resolved PCs; calling this without the matching ELF is a no-op
    /// (symbols don't resolve → no thunks installed).
    ///
    /// Attaches no peripheral of its own: the panel (model, CS, DC) comes
    /// from the board manifest via `attach_esp32_external_devices` at system
    /// load — see the body below. This method used to hardcode a panel here;
    /// that behaviour is gone, and the manifest is the single source of truth.
    ///
    /// For the record, because the deleted comment had it backwards:
    /// `GxEPD2_290_C90c` is an **SSD1680** controller (0x12 SWRESET, 0x11 data
    /// entry, 0x24/0x26 RAM, 0x22+0x20 update), not UC8151D. UC8151D
    /// (`0x00 PSR` / `0x04 PON` / `0x10 DTM1` / `0x12 DRF` / `0x13 DTM2`) is
    /// what `GxEPD2_290_Z13c` emits. `peripherals::kit::registry::TYPE_ALIASES`
    /// owns that mapping.
    #[wasm_bindgen]
    pub fn install_arduino_esp32_quirks(&mut self, elf_bytes: &[u8]) -> Result<(), JsValue> {
        // ONE boot path. This used to be a second, hand-maintained copy of the
        // Arduino-ESP32 bootstrap, and it had drifted 44 symbols away from the
        // CLI's: the browser thunked `esp_log_writev` / `esp_log_timestamp` /
        // `esp_log_impl_*` long after core deleted them, so a sketch's
        // `ESP_LOGI()` printed in `labwired test` and vanished in the lab. Same
        // firmware, two answers, and nothing could tell you which was real.
        //
        // Everything below now comes from
        // `labwired_core::system::xtensa::install_arduino_esp32_profile`. If a
        // boot step is missing for the browser, add a parameter THERE — do not
        // start a third copy here.
        let program = labwired_loader::load_elf_bytes(elf_bytes)
            .map_err(|e| JsValue::from_str(&format!("parse ELF for boot profile: {e}")))?;
        let symbol_addrs = labwired_loader::extract_arduino_esp32_thunks(elf_bytes);

        // Re-install can happen after a soft re-run without a full construct;
        // always start from a clean session-global slate.
        labwired_core::peripherals::esp_xtensa_common::rom_thunks::reset_esp32_session_state();

        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("no machine"))?;

        // NO hardcoded peripheral here. The panel (and any other external
        // device) is attached from the board manifest by
        // attach_esp32_external_devices during system load — the single source
        // of truth for peripheral wiring, model, CS and DC pins.
        let profile = labwired_core::system::xtensa::install_arduino_esp32_profile(
            machine,
            symbol_addrs,
            program.entry_point as u32,
        )
        .map_err(|e| JsValue::from_str(&e))?;

        // The single-core IPI bridge and its handshake keep-alive are the one
        // genuinely browser-side piece: with a real APP_CPU, `step_with_esp32_aids`
        // delegates straight to `Machine::step` (which delivers the DPORT IPI),
        // so the bridge would be dead weight. The profile decides WHETHER the
        // handshake is pre-seeded; this only mirrors that decision into the
        // step loop.
        if profile.preseed_handshake {
            let mut handshake_bytes: Vec<u32> = Vec::new();
            for (base, two_byte) in [
                (profile.s_resume_cores, true),
                (profile.s_cpu_up, true),
                (profile.s_cpu_inited, true),
                (profile.s_system_inited, true),
                (profile.s_other_cpu_startup_done, false),
            ] {
                if base != 0 {
                    handshake_bytes.push(base);
                    if two_byte {
                        handshake_bytes.push(base + 1);
                    }
                }
            }
            self.esp32_ipi = Some(Esp32IpiBridge {
                handshake_bytes,
                ..Esp32IpiBridge::default()
            });
        }
        Ok(())
    }

    /// Apply a binary `MachineRuntimeSnapshot` (LWRS-framed bincode blob,
    /// produced by `labwired-cli snapshot capture` or `Machine::take_runtime_snapshot`)
    /// to the currently-loaded machine. Bypasses the cold boot — the firmware
    /// resumes mid-flight from the captured CPU + peripheral state.
    ///
    /// Must be called after firmware has been loaded onto the same system
    /// manifest (peripheral names + CPU arch must match the snapshot). On
    /// mismatch the call returns an error and the machine state is left
    /// partially overwritten — callers should treat that as a hard reset.
    #[wasm_bindgen]
    pub fn apply_runtime_snapshot(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| JsValue::from_str("no machine"))?;
        let snap = labwired_core::runtime_snapshot::MachineRuntimeSnapshot::from_bytes(bytes)
            .map_err(|e| JsValue::from_str(&format!("snapshot decode: {e}")))?;
        machine
            .apply_runtime_snapshot(&snap)
            .map_err(|e| JsValue::from_str(&format!("snapshot apply: {e}")))?;
        Ok(())
    }

    /// Capture the current machine state as a binary `MachineRuntimeSnapshot`
    /// (LWRS-framed bincode blob). Mirror of `apply_runtime_snapshot` —
    /// returned bytes can be fed back to `apply_runtime_snapshot` on a fresh
    /// `WasmSimulator` with the same firmware + bus topology.
    #[wasm_bindgen]
    pub fn take_runtime_snapshot(&self) -> Result<Vec<u8>, JsValue> {
        let machine = self
            .machine
            .as_ref()
            .ok_or_else(|| JsValue::from_str("no machine"))?;
        Ok(machine.take_runtime_snapshot().to_bytes())
    }

    /// Re-write the dual-core handshake bytes. Call every ~10k steps from JS
    /// — firmware boot code revisits these and we need them to stay 1.
    #[wasm_bindgen]
    pub fn keep_alive_esp32_dual_core(&mut self) {
        let machine = match self.machine.as_mut() {
            Some(m) => m,
            None => return,
        };
        let _ = machine.bus.write_u8(0x3FFC_6F04, 0x01);
        let _ = machine.bus.write_u8(0x3FFC_6F01, 0x01);
        let _ = machine.bus.write_u8(0x3FFC_6F02, 0x01);
        let _ = machine.bus.write_u8(0x3FFC_6FFD, 0x01);
        let _ = machine.bus.write_u8(0x3FFC_6FFE, 0x01);
        let _ = machine.bus.write_u8(0x3FFC_7190, 0x01);
    }
}
