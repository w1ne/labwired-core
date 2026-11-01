// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! What a debugger needs to CHANGE a machine, not only look at it: memory
//! writes, register writes, and a decode of any address (not just the PC).
//!
//! Both writes are journaled like every other mutating call, so a snapshot
//! restore replays them and lands on the same state. A memory write goes
//! straight to the bus, past the core, so the core's decoded/fetched code is
//! dropped afterwards (`Cpu::invalidate_code_caches`) -- otherwise a poke into
//! code would go on executing what the memory used to hold.

use super::*;

/// The instruction at `pc`, decoded for this machine's architecture. The one
/// decoder behind both `get_disassembly` (the PC) and `disassemble_at`.
pub(crate) fn decode_at(sim: &WasmSimulator, pc: u32) -> String {
    let Some(machine) = sim.machine.as_ref() else {
        return "?? (no machine)".to_string();
    };
    match sim.arch {
        // ESP32-C3 / generic RV32: use the RISC-V decoder. The previous path
        // always ran Thumb decode, so C3 Trace showed ARM-looking ops and
        // frequent `Unknown32` against real RISC-V encodings.
        MachineFamily::RiscV => {
            let pc = pc & !1;
            match machine.bus.read_u16(pc as u64) {
                Ok(lo) => {
                    // RV32C: least-significant two bits != 0b11 ⇒ 16-bit.
                    if lo & 0b11 != 0b11 {
                        format!("{:?}", decode_rv32c(lo))
                    } else {
                        match machine.bus.read_u16(pc as u64 + 2) {
                            Ok(hi) => {
                                let word = (u32::from(hi) << 16) | u32::from(lo);
                                format!("{:?}", decode_rv32(word))
                            }
                            Err(_) => "?? (Error reading RV hi half)".to_string(),
                        }
                    }
                }
                Err(_) => "?? (Error reading RV instruction)".to_string(),
            }
        }
        MachineFamily::Xtensa => {
            // Match the LX7 fetch path: length from byte0, then narrow/wide.
            match machine.bus.read_u8(pc as u64) {
                Ok(b0) => {
                    let len = xtensa_length::instruction_length(b0);
                    if len == 2 {
                        match machine.bus.read_u16(pc as u64) {
                            Ok(hw) => format!("{:?}", xtensa_narrow::decode_narrow(hw)),
                            Err(_) => "?? (Error reading Xtensa narrow)".to_string(),
                        }
                    } else {
                        match machine.bus.read_u32(pc as u64) {
                            Ok(w) => format!("{:?}", xtensa::decode(w)),
                            Err(_) => "?? (Error reading Xtensa wide)".to_string(),
                        }
                    }
                }
                Err(_) => "?? (Error reading Xtensa instruction)".to_string(),
            }
        }
        MachineFamily::CortexA => "Cortex-A disassembly unavailable in the browser".to_owned(),
        MachineFamily::CortexM => {
            let pc = pc & !1;
            match machine.bus.read_u16(pc as u64) {
                Ok(h1) => {
                    let is_32bit = (h1 & 0xE000) == 0xE000 && (h1 & 0x1800) != 0;
                    if is_32bit {
                        match machine.bus.read_u16(pc as u64 + 2) {
                            Ok(h2) => format!("{:?}", decode_thumb_32(h1, h2)),
                            Err(_) => "?? (Error reading h2)".to_string(),
                        }
                    } else {
                        format!("{:?}", decode_thumb_16(h1))
                    }
                }
                Err(_) => "?? (Error reading h1)".to_string(),
            }
        }
        // No shared AVR decoder in the wasm Trace panel yet — show the raw
        // opcode word so the pane is never empty / wrong-arch.
        MachineFamily::Avr => match machine.bus.read_u16(pc as u64) {
            Ok(word) => format!("AVR {word:#06x}"),
            Err(_) => "?? (Error reading AVR instruction)".to_string(),
        },
        // PIC24/PIC18/PIC16: instruction words live in the CPU's flash, not
        // the bus.
        MachineFamily::Pic24 => "PIC24 (no wasm disassembly yet)".to_string(),
        MachineFamily::Pic18 => "PIC18 (no wasm disassembly yet)".to_string(),
        MachineFamily::Pic16 => "PIC16 (no wasm disassembly yet)".to_string(),
        MachineFamily::Rx => "RX (no wasm disassembly yet)".to_string(),
    }
}

#[wasm_bindgen]
impl WasmSimulator {
    /// Decode the instruction at `addr` (not only at the PC), in the engine's
    /// own debug form -- the same text `get_disassembly` gives for the PC.
    #[wasm_bindgen]
    pub fn disassemble_at(&self, addr: u32) -> Result<String, JsValue> {
        self.machine_or_err()?;
        Ok(decode_at(self, addr))
    }

    /// Write `data` at `addr` through the bus, as a debugger poke. Journaled;
    /// the core's cached code is dropped so a write into code takes effect.
    /// A byte the bus refuses fails the call, naming the address.
    #[wasm_bindgen]
    pub fn write_memory(&mut self, addr: u32, data: &[u8]) -> Result<(), JsValue> {
        self.record(lab_tools::Op::WriteMemory(addr, data.to_vec()));
        let machine = self.machine_mut_or_err()?;
        for (i, byte) in data.iter().enumerate() {
            let at = addr as u64 + i as u64;
            machine.bus.write_u8(at, *byte).map_err(|error| {
                JsValue::from_str(&format!("memory write failed at {at:#010x}: {error:?}"))
            })?;
        }
        machine.cpu.invalidate_code_caches();
        Ok(())
    }

    /// Write register `id` (the index `get_register_names` gives it).
    /// Journaled. An index the core does not name is refused, not ignored.
    #[wasm_bindgen]
    pub fn set_register(&mut self, id: u8, value: u32) -> Result<(), JsValue> {
        self.record(lab_tools::Op::SetRegister(id, value));
        let machine = self.machine_mut_or_err()?;
        write_named_register(machine.cpu.as_mut(), id, value).map_err(|e| JsValue::from_str(&e))
    }
}

/// The register write behind `set_register`, with a plain error so it can be
/// tested natively (a JsValue cannot be created off wasm32).
pub(crate) fn write_named_register(cpu: &mut dyn Cpu, id: u8, value: u32) -> Result<(), String> {
    let count = cpu.get_register_names().len();
    if usize::from(id) >= count {
        return Err(format!(
            "register {id} does not exist (this core names {count})"
        ));
    }
    cpu.set_register(id, value);
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// A bare Cortex-M machine with the decode cache ON (the case the
    /// invalidation exists for) and `code` at address 0.
    fn arm_with(code: &[u16]) -> WasmSimulator {
        let mut bus = SystemBus::new();
        let mut cpu = labwired_core::cpu::CortexM::new();
        for (i, hw) in code.iter().enumerate() {
            bus.write_u16(i as u64 * 2, *hw).unwrap();
        }
        cpu.set_pc(0);
        let uart_sink = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(uart_sink.clone(), false);
        let uart_rx_bufs = bus.attach_uart_rx_source();
        let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
        machine.config.decode_cache_enabled = true;
        machine.bus.config.decode_cache_enabled = true;
        WasmSimulator {
            machine: Some(machine),
            board_io: Vec::new(),
            uart_sink,
            console: ConsoleCapture::new(HostConsole::Undeclared, HostConsole::UsbSerialJtag),
            uart_rx_bufs,
            arch: MachineFamily::CortexM,
            esp32_ipi: None,
            jit_browser_enabled: false,
            jit_browser_cache: None,
            cosim: None,
            tools: Default::default(),
        }
    }

    const MOVS_R0_1: u16 = 0x2001;
    const MOVS_R0_7: u16 = 0x2007;
    const B_SELF: u16 = 0xE7FE;

    #[test]
    fn a_memory_write_reads_back() {
        let mut sim = arm_with(&[B_SELF]);
        sim.write_memory(0x2000_0010, &[0xDE, 0xAD]).unwrap();
        assert_eq!(sim.read_memory(0x2000_0010, 2).unwrap(), vec![0xDE, 0xAD]);
    }

    #[test]
    fn a_register_write_reads_back_and_an_unnamed_index_is_refused() {
        let mut sim = arm_with(&[B_SELF]);
        sim.set_register(3, 0x1234_5678).unwrap();
        assert_eq!(sim.get_register(3).unwrap(), 0x1234_5678);
        // The refusal, through the plain-error function set_register wraps
        // (a JsValue error cannot be built off wasm32).
        let cpu = sim.machine.as_mut().unwrap().cpu.as_mut();
        let err = write_named_register(cpu, 200, 1).unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
    }

    #[test]
    fn disassemble_at_decodes_an_address_that_is_not_the_pc() {
        let sim = arm_with(&[B_SELF, MOVS_R0_7]);
        let at_pc = sim.get_disassembly();
        let other = sim.disassemble_at(2).unwrap();
        assert_ne!(
            at_pc, other,
            "decoded the PC instead of the address asked for"
        );
        assert!(other.to_lowercase().contains("mov"), "{other}");
    }

    /// The reason for invalidate_code_caches: execute MOVS r0,#1 (now in the
    /// decode cache), poke MOVS r0,#7 over it, re-execute. Without the
    /// invalidation the cached decode runs and r0 is 1.
    #[test]
    fn a_write_into_code_takes_effect_on_the_next_execution() {
        let mut sim = arm_with(&[MOVS_R0_1, B_SELF]);
        sim.step_single().unwrap();
        assert_eq!(sim.get_register(0).unwrap(), 1);
        sim.machine.as_mut().unwrap().cpu.set_pc(0);
        sim.write_memory(0, &MOVS_R0_7.to_le_bytes()).unwrap();
        sim.step_single().unwrap();
        assert_eq!(
            sim.get_register(0).unwrap(),
            7,
            "stale decode survived the write"
        );
    }
}
