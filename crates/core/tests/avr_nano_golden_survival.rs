// Arduino Nano golden survival: PlatformIO nanoatmega328 blink + Serial.
// Fixture: tests/fixtures/avr/arduino-nano-blinky.elf
// Bar: sim-smoke — GPIO (D13/PB5) toggles and serial contains "nano-ok".

use labwired_core::cpu::avr::{Avr, UCSRA_UDRE};
use labwired_core::{Bus, Cpu, DmaRequest, SimulationConfig, SimulationError};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct MockBus {
    mem: HashMap<u64, u8>,
    config: SimulationConfig,
}

impl MockBus {
    fn new() -> Self {
        Self {
            mem: HashMap::new(),
            config: SimulationConfig::default(),
        }
    }
}

impl Bus for MockBus {
    fn read_u8(&self, addr: u64) -> labwired_core::SimResult<u8> {
        Ok(*self.mem.get(&addr).unwrap_or(&0))
    }
    fn write_u8(&mut self, addr: u64, value: u8) -> labwired_core::SimResult<()> {
        self.mem.insert(addr, value);
        Ok(())
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }
    fn execute_dma(&mut self, _requests: &[DmaRequest]) -> labwired_core::SimResult<()> {
        Ok(())
    }
    fn config(&self) -> &SimulationConfig {
        &self.config
    }
}

fn fixture_elf() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/avr/arduino-nano-blinky.elf")
}

#[test]
fn arduino_nano_golden_prints_and_blinks() {
    let elf_path = fixture_elf();
    assert!(
        elf_path.is_file(),
        "missing golden ELF at {elf_path:?}; build examples/arduino-nano-blinky"
    );
    let bytes = std::fs::read(&elf_path).expect("read elf");
    let image = labwired_loader::load_elf_bytes(&bytes).expect("parse avr elf");
    assert_eq!(image.arch, labwired_core::Arch::Avr);

    let mut cpu = Avr::new();
    let sink = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(sink.clone());
    cpu.load_program_image(&image);
    let mut bus = MockBus::new();
    let cfg = SimulationConfig::default();
    let mut saw_high = false;
    let mut saw_low = false;
    let mut last_err: Option<String> = None;

    // Generous step budget: Arduino init + println + a few delay loops.
    for step in 0..5_000_000u32 {
        match cpu.step(&mut bus, &[], &cfg) {
            Ok(()) => {}
            Err(SimulationError::DecodeError(pc)) => {
                let i = pc as usize;
                let w = if i + 1 < cpu.flash.len() {
                    u16::from_le_bytes([cpu.flash[i], cpu.flash[i + 1]])
                } else {
                    0
                };
                last_err = Some(format!(
                    "DecodeError at pc={pc:#x} word={w:#06x} after {step} steps"
                ));
                break;
            }
            Err(SimulationError::Halt) => break,
            Err(e) => {
                last_err = Some(format!("{e:?} after {step} steps"));
                break;
            }
        }
        let pb = cpu.portb();
        if pb & (1 << 5) != 0 {
            saw_high = true;
        } else if step > 1000 {
            // after init, low is meaningful
            saw_low = true;
        }
        let serial = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
        if serial.contains("nano-ok") && saw_high && saw_low {
            return;
        }
    }

    let serial = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    panic!(
        "golden survival failed.\n  serial={serial:?}\n  saw_high={saw_high} saw_low={saw_low} portb={:#x}\n  err={last_err:?}\n  pc={:#x}",
        cpu.portb(),
        cpu.get_pc()
    );
}

#[test]
fn golden_elf_is_avr_and_loads() {
    let bytes = std::fs::read(fixture_elf()).expect("elf");
    let image = labwired_loader::load_elf_bytes(&bytes).expect("parse");
    assert_eq!(image.arch, labwired_core::Arch::Avr);
    assert!(!image.segments.is_empty());
    let mut cpu = Avr::new();
    cpu.load_program_image(&image);
    assert_eq!(cpu.get_pc() & 1, 0);
    assert_eq!(cpu.ucsr0a & UCSRA_UDRE, UCSRA_UDRE);
}

/// Fast path: golden reaches Serial + does not reboot during init.
#[test]
fn golden_reaches_serial_without_stack_collapse() {
    let bytes = std::fs::read(fixture_elf()).unwrap();
    let image = labwired_loader::load_elf_bytes(&bytes).unwrap();
    let mut cpu = Avr::new();
    cpu.load_program_image(&image);
    let mut bus = MockBus::new();
    let cfg = SimulationConfig::default();
    let mut min_sp = 0xffffu16;
    let mut resets = 0u32;
    for step in 0..100_000u32 {
        let pb = cpu.get_pc();
        if let Err(e) = cpu.step(&mut bus, &[], &cfg) {
            panic!("step {step} pc={pb:#x} SP={:04x} err={e:?}", cpu.sp);
        }
        min_sp = min_sp.min(cpu.sp);
        if cpu.get_pc() == 0 && pb > 4 {
            resets += 1;
        }
        // Stack must stay in SRAM (never clobber SPH via bad STD Y/Z).
        assert!(
            cpu.sp >= 0x100 || step < 20,
            "stack collapsed at step {step} SP={:04x} pc={pb:#x}",
            cpu.sp
        );
        if String::from_utf8_lossy(&cpu.serial_tx).contains("nano-ok") {
            assert_eq!(resets, 0, "unexpected reset during run");
            assert!(min_sp >= 0x0800, "min SP too low: {min_sp:#x}");
            return;
        }
    }
    panic!(
        "no success; serial={:?} pc={:#x} min_sp={:04x} resets={resets}",
        String::from_utf8_lossy(&cpu.serial_tx),
        cpu.get_pc(),
        min_sp
    );
}
