//! Live run of Arduino Nano golden sketch (not a unit micro-test — prints what ran).
use labwired_core::cpu::avr::Avr;
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

#[test]
fn live_run_arduino_nano_blinky_sketch() {
    let elf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/avr/arduino-nano-blinky.elf");
    assert!(elf.is_file(), "missing {elf:?}");
    let bytes = std::fs::read(&elf).unwrap();
    let image = labwired_loader::load_elf_bytes(&bytes).unwrap();
    assert_eq!(image.arch, labwired_core::Arch::Avr);

    let mut cpu = Avr::new();
    let sink = Arc::new(Mutex::new(Vec::new()));
    cpu.set_serial_sink(sink.clone());
    cpu.load_program_image(&image);
    let mut bus = MockBus::new();
    let cfg = SimulationConfig::default();

    let mut toggles = 0u32;
    let mut last_pb5 = false;
    let mut saw_high = false;
    let mut saw_low = false;

    for step in 0..2_000_000u32 {
        match cpu.step(&mut bus, &[], &cfg) {
            Ok(()) => {}
            Err(SimulationError::Halt) => break,
            Err(e) => panic!("step {step} pc={:#x} err={e:?}", cpu.get_pc()),
        }
        let pb5 = cpu.portb() & (1 << 5) != 0;
        if pb5 {
            saw_high = true;
        } else if step > 500 {
            saw_low = true;
        }
        if pb5 != last_pb5 && step > 500 {
            toggles += 1;
            last_pb5 = pb5;
        }
        let serial = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
        if serial.contains("nano-ok") && toggles >= 2 {
            eprintln!("=== Arduino Nano live run SUCCESS ===");
            eprintln!("steps={step}");
            eprintln!("serial={serial:?}");
            eprintln!(
                "portb={:#04x} toggles={toggles} high={saw_high} low={saw_low}",
                cpu.portb()
            );
            eprintln!(
                "pc={:#x} SP={:04x} cycles={}",
                cpu.get_pc(),
                cpu.sp,
                cpu.cycles
            );
            return;
        }
    }
    let serial = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
    panic!(
        "live run failed serial={serial:?} toggles={toggles} portb={:#x} pc={:#x}",
        cpu.portb(),
        cpu.get_pc()
    );
}
