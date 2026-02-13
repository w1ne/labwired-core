// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#[cfg(test)]
mod integration_tests {
    use crate::cpu::CortexM;
    use crate::decoder::arm::{self as decoder, Instruction};
    use crate::peripherals::nvic::NvicState;
    use crate::{Bus, Cpu, DebugControl, Machine, Peripheral, SimResult, StopReason};
    use labwired_config::{Arch, ChipDescriptor, MemoryRange, PeripheralConfig, SystemManifest};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
    use std::sync::{Arc, Mutex};

    fn create_machine() -> VariableMachine {
        // Placeholder name collision? No.
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        Machine::new(cpu, bus)
    }
    type VariableMachine = Machine<CortexM>;

    #[derive(Debug)]
    struct RecordingPeripheral {
        regs: [u8; 16],
        last_read: AtomicU64,
        last_write: AtomicU64,
        last_write_value: AtomicU8,
        tick_next: bool,
    }

    impl RecordingPeripheral {
        fn new() -> Self {
            Self {
                regs: [0; 16],
                last_read: AtomicU64::new(u64::MAX),
                last_write: AtomicU64::new(u64::MAX),
                last_write_value: AtomicU8::new(0),
                tick_next: false,
            }
        }

        fn with_tick(tick_next: bool) -> Self {
            Self {
                tick_next,
                ..Self::new()
            }
        }
    }

    impl Peripheral for RecordingPeripheral {
        fn read(&self, offset: u64) -> SimResult<u8> {
            self.last_read.store(offset, Ordering::SeqCst);
            Ok(self.regs.get(offset as usize).copied().unwrap_or(0))
        }

        fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
            self.last_write.store(offset, Ordering::SeqCst);
            self.last_write_value.store(value, Ordering::SeqCst);
            if let Some(byte) = self.regs.get_mut(offset as usize) {
                *byte = value;
            }
            Ok(())
        }

        fn tick(&mut self) -> crate::PeripheralTickResult {
            let tick_now = self.tick_next;
            self.tick_next = false;
            crate::PeripheralTickResult {
                irq: tick_now,
                cycles: 0,
                dma_requests: Vec::new(),
                explicit_irqs: Vec::new(),
            }
        }
    }

    #[test]
    fn test_decoder_mov() {
        // 0x202A => MOV R0, #42
        // 0010 0000 0010 1010
        let instr = decoder::decode_thumb_16(0x202A);
        assert_eq!(instr, Instruction::MovImm { rd: 0, imm: 42 });
    }

    #[test]
    fn test_cpu_execute_mov() {
        let mut machine = create_machine();
        // Use RAM address because Flash via Bus is read-only
        let base_addr: u64 = 0x2000_0000;
        machine.cpu.pc = base_addr as u32;

        // Write opcode to memory
        // 0x202A -> Little Endian: 2A 20
        machine.bus.write_u8(base_addr, 0x2A).unwrap();
        machine.bus.write_u8(base_addr + 1, 0x20).unwrap();

        // Step
        machine.step().unwrap();

        assert_eq!(machine.cpu.r0, 42);
        assert_eq!(machine.cpu.pc, (base_addr + 2) as u32);
    }

    #[test]
    fn test_cpu_execute_branch() {
        let mut machine = create_machine();
        let base_addr: u64 = 0x2000_0000;
        machine.cpu.pc = base_addr as u32;

        // Unconditional Branch: B <offset>
        // We want to skip over a NOP.
        // 0x2000_0000: B +4 (Offset=2 instructions -> +4 bytes)
        // 0x2000_0002: NOP (Skipped)
        // 0x2000_0004: Target

        // Encoding for B +2 (instructions): 0xE002
        // Little Endian: 02 E0
        machine.bus.write_u8(base_addr, 0x02).unwrap();
        machine.bus.write_u8(base_addr + 1, 0xE0).unwrap();

        // Step
        machine.step().unwrap();

        // Expected PC: Base + 4 + (2<<1) = Base + 8
        // Wait, my decoder test says:
        // Encoding: i=2 -> 0xE002
        // Target = PC + 4 + (2 << 1) = PC + 8
        // So valid target is 0x2000_0008.

        assert_eq!(machine.cpu.pc, (base_addr + 8) as u32);
    }
    #[test]
    fn test_cpu_execute_ldr_str() {
        let mut machine = create_machine();
        let base_addr: u64 = 0x2000_0000;
        machine.cpu.pc = base_addr as u32;

        // 1. STR R0, [R1, #0]
        // R0 = 0xDEADBEEF
        // R1 = 0x2000_0010 (Target RAM)
        machine.cpu.r0 = 0xDEADBEEF;
        machine.cpu.r1 = 0x2000_0010;

        // Opcode STR R0, [R1, #0] -> 0x6008
        // 0110 0 00000 001 000
        machine.bus.write_u8(base_addr, 0x08).unwrap();
        machine.bus.write_u8(base_addr + 1, 0x60).unwrap();

        machine.step().unwrap();

        // precise verify RAM
        let val = machine.bus.read_u32(0x2000_0010).unwrap();
        assert_eq!(val, 0xDEADBEEF);

        // 2. LDR R2, [R1, #0]
        // Should load 0xDEADBEEF into R2
        // Opcode LDR R2, [R1, #0] -> 0x680A
        // 0110 1 00000 001 010
        machine.bus.write_u8(base_addr + 2, 0x0A).unwrap();
        machine.bus.write_u8(base_addr + 3, 0x68).unwrap();

        machine.step().unwrap();

        assert_eq!(machine.cpu.r2, 0xDEADBEEF);
    }

    #[test]
    fn test_uart_write() {
        let mut machine = create_machine();
        // Base PC = RAM
        let base_addr: u64 = 0x2000_0000;
        machine.cpu.pc = base_addr as u32;

        // Code:
        // MOV R0, #72 ('H')
        // STR R0, [R1] (where R1 points to UART)

        // Manual setup for simplicity
        machine.cpu.r0 = 72; // 'H'
        machine.cpu.r1 = 0x4000_C000;

        // STR R0, [R1, #0] -> 0x6008
        // 0110 0 00000 001 000
        machine.bus.write_u8(base_addr, 0x08).unwrap();
        machine.bus.write_u8(base_addr + 1, 0x60).unwrap();

        // Capture stdout? Rust test harness captures it.
        // We mainly verify it doesn't crash.
        // Ideally we would mock stdout, but for this level of sim,
        // ensuring it runs without MemoryViolation is enough.
        machine.step().unwrap();
    }

    #[test]
    fn test_bus_routes_peripheral_reads_writes() {
        let mut bus = crate::bus::SystemBus::new();
        let base = 0x5000_0000;
        bus.peripherals.push(crate::bus::PeripheralEntry {
            name: "recording".to_string(),
            base,
            size: 0x10,
            irq: None,
            dev: Box::new(RecordingPeripheral::new()),
        });

        bus.write_u8(base + 2, 0xAB).unwrap();
        let read = bus.read_u8(base + 2).unwrap();
        assert_eq!(read, 0xAB);
    }

    #[test]
    fn test_bus_u32_roundtrip_peripheral() {
        let mut bus = crate::bus::SystemBus::new();
        let base = 0x5000_1000;
        bus.peripherals.push(crate::bus::PeripheralEntry {
            name: "recording32".to_string(),
            base,
            size: 0x10,
            irq: None,
            dev: Box::new(RecordingPeripheral::new()),
        });

        let value = 0xA1B2_C3D4;
        bus.write_u32(base, value).unwrap();
        let read_back = bus.read_u32(base).unwrap();
        assert_eq!(read_back, value);
    }

    #[test]
    fn test_write_does_not_probe_peripheral_read_path() {
        #[derive(Debug)]
        struct ReadSideEffectPeripheral {
            reg: AtomicU8,
            reads: Arc<AtomicU64>,
        }

        impl Peripheral for ReadSideEffectPeripheral {
            fn read(&self, _offset: u64) -> SimResult<u8> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                Ok(self.reg.swap(0, Ordering::SeqCst))
            }

            fn write(&mut self, _offset: u64, value: u8) -> SimResult<()> {
                self.reg.store(value, Ordering::SeqCst);
                Ok(())
            }

            fn peek(&self, _offset: u64) -> Option<u8> {
                Some(self.reg.load(Ordering::SeqCst))
            }
        }

        let base = 0x5000_2000;
        let reads = Arc::new(AtomicU64::new(0));

        let mut bus = crate::bus::SystemBus::new();
        bus.peripherals.push(crate::bus::PeripheralEntry {
            name: "read_side_effect".to_string(),
            base,
            size: 0x10,
            irq: None,
            dev: Box::new(ReadSideEffectPeripheral {
                reg: AtomicU8::new(0xF0),
                reads: reads.clone(),
            }),
        });

        bus.write_u8(base, 0xAA).unwrap();

        assert_eq!(
            reads.load(Ordering::SeqCst),
            0,
            "write path should not invoke peripheral read()"
        );
        assert_eq!(bus.read_u8(base).unwrap(), 0xAA);
    }

    #[test]
    fn test_tick_peripheral_sets_nvic_pending() {
        let mut bus = crate::bus::SystemBus::new();
        let nvic_state = Arc::new(NvicState::default());
        bus.nvic = Some(nvic_state.clone());

        bus.peripherals.push(crate::bus::PeripheralEntry {
            name: "tick_irq".to_string(),
            base: 0x5000_2000,
            size: 0x10,
            irq: Some(16),
            dev: Box::new(RecordingPeripheral::with_tick(true)),
        });

        let irqs = bus.tick_peripherals();
        assert!(
            irqs.is_empty(),
            "IRQ should be pended in NVIC, not returned"
        );

        let ispr0 = nvic_state.ispr[0].load(Ordering::SeqCst);
        assert_eq!(ispr0 & 0x1, 0x1, "NVIC ISPR0 bit 0 should be set");
    }

    #[test]
    fn test_tick_peripheral_without_nvic_returns_irq() {
        let mut bus = crate::bus::SystemBus::new();
        bus.nvic = None;

        bus.peripherals.push(crate::bus::PeripheralEntry {
            name: "tick_irq_legacy".to_string(),
            base: 0x5000_3000,
            size: 0x10,
            irq: Some(16),
            dev: Box::new(RecordingPeripheral::with_tick(true)),
        });

        let irqs = bus.tick_peripherals();
        assert_eq!(irqs, vec![16]);
    }

    #[test]
    fn test_machine_step_error_does_not_tick_peripherals() {
        #[derive(Debug)]
        struct TickCounterPeripheral {
            tick_count: Arc<AtomicU64>,
        }

        impl Peripheral for TickCounterPeripheral {
            fn read(&self, _offset: u64) -> SimResult<u8> {
                Ok(0)
            }

            fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
                Ok(())
            }

            fn tick(&mut self) -> crate::PeripheralTickResult {
                self.tick_count.fetch_add(1, Ordering::SeqCst);
                crate::PeripheralTickResult {
                    irq: true,
                    cycles: 1,
                    dma_requests: Vec::new(),
                    explicit_irqs: Vec::new(),
                }
            }
        }

        let mut machine = create_machine();
        let tick_count = Arc::new(AtomicU64::new(0));

        machine.bus.peripherals.push(crate::bus::PeripheralEntry {
            name: "tick_counter".to_string(),
            base: 0x5000_4000,
            size: 0x10,
            irq: Some(16),
            dev: Box::new(TickCounterPeripheral {
                tick_count: tick_count.clone(),
            }),
        });

        // Force instruction fetch to fail with a memory violation.
        machine.cpu.pc = 0xDEAD_BEEF;
        let step = machine.step();

        assert!(
            matches!(step, Err(crate::SimulationError::MemoryViolation(_))),
            "expected memory violation on fetch"
        );
        assert_eq!(
            tick_count.load(Ordering::SeqCst),
            0,
            "peripherals should not tick when CPU step fails"
        );
    }

    #[test]
    fn test_from_config_skips_unsupported_peripherals() {
        let chip = ChipDescriptor {
            name: "test-chip".to_string(),
            arch: Arch::Arm,
            flash: MemoryRange {
                base: 0x0,
                size: "128KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![
                PeripheralConfig {
                    id: "uart1".to_string(),
                    r#type: "uart".to_string(),
                    base_address: 0x4000_C000,
                    size: None,
                    irq: None,
                    config: HashMap::new(),
                },
                PeripheralConfig {
                    id: "mystery".to_string(),
                    r#type: "unknown".to_string(),
                    base_address: 0x5000_0000,
                    size: None,
                    irq: None,
                    config: HashMap::new(),
                },
            ],
        };

        let manifest = SystemManifest {
            name: "test-system".to_string(),
            chip: "test-chip".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            board_io: Vec::new(),
        };

        let bus = crate::bus::SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(bus.peripherals.len(), 1);
        assert_eq!(bus.peripherals[0].name, "uart1");
        assert_eq!(bus.peripherals[0].base, 0x4000_C000);
    }

    #[test]
    fn test_gpio_bsrr_brr_buffered_writes() {
        let mut bus = crate::bus::SystemBus::new();
        let gpioa_base = 0x4001_0800;
        let odr = gpioa_base + 0x0C;
        let bsrr = gpioa_base + 0x10;

        bus.write_u32(bsrr, 0x0000_0005).unwrap();
        assert_eq!(bus.read_u32(odr).unwrap() & 0xFFFF, 0x0005);

        bus.write_u32(bsrr, 0x0005_0000).unwrap();
        assert_eq!(bus.read_u32(odr).unwrap() & 0xFFFF, 0x0000);

        bus.write_u16(bsrr, 0x0003).unwrap();
        assert_eq!(bus.read_u32(odr).unwrap() & 0xFFFF, 0x0003);

        bus.write_u16(bsrr + 2, 0x0003).unwrap();
        assert_eq!(bus.read_u32(odr).unwrap() & 0xFFFF, 0x0000);
    }

    #[test]
    fn test_from_config_defaults_size_irq_and_base() {
        let chip = ChipDescriptor {
            name: "test-chip-2".to_string(),
            arch: Arch::Arm,
            flash: MemoryRange {
                base: 0x0,
                size: "128KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![
                PeripheralConfig {
                    id: "systick".to_string(),
                    r#type: "systick".to_string(),
                    base_address: 0xE000_E010,
                    size: None,
                    irq: None,
                    config: HashMap::new(),
                },
                PeripheralConfig {
                    id: "gpioa".to_string(),
                    r#type: "gpio".to_string(),
                    base_address: 0x4001_0800,
                    size: None,
                    irq: None,
                    config: HashMap::new(),
                },
            ],
        };

        let manifest = SystemManifest {
            name: "test-system-2".to_string(),
            chip: "test-chip-2".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            board_io: Vec::new(),
        };

        let bus = crate::bus::SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(bus.peripherals.len(), 2);

        let systick = bus
            .peripherals
            .iter()
            .find(|p| p.name == "systick")
            .unwrap();
        assert_eq!(systick.base, 0xE000_E010);
        assert_eq!(systick.size, 0x1000);
        assert_eq!(systick.irq, Some(15));

        let gpioa = bus.peripherals.iter().find(|p| p.name == "gpioa").unwrap();
        assert_eq!(gpioa.base, 0x4001_0800);
        assert_eq!(gpioa.size, 0x1000);
        assert_eq!(gpioa.irq, None);
    }

    #[test]
    fn test_from_config_honors_size_and_irq() {
        let chip = ChipDescriptor {
            name: "test-chip-3".to_string(),
            arch: Arch::Arm,
            flash: MemoryRange {
                base: 0x0,
                size: "128KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![PeripheralConfig {
                id: "uart1".to_string(),
                r#type: "uart".to_string(),
                base_address: 0x4000_C000,
                size: Some("1KB".to_string()),
                irq: Some(37),
                config: HashMap::new(),
            }],
        };

        let manifest = SystemManifest {
            name: "test-system-3".to_string(),
            chip: "test-chip-3".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            board_io: Vec::new(),
        };

        let bus = crate::bus::SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(bus.peripherals.len(), 1);

        let uart1 = &bus.peripherals[0];
        assert_eq!(uart1.name, "uart1");
        assert_eq!(uart1.base, 0x4000_C000);
        assert_eq!(uart1.size, 1024);
        assert_eq!(uart1.irq, Some(37));
    }

    #[test]
    fn test_from_config_gpio_profile_stm32v2() {
        let mut gpio_config = HashMap::new();
        gpio_config.insert(
            "profile".to_string(),
            serde_yaml::Value::String("stm32v2".to_string()),
        );

        let chip = ChipDescriptor {
            name: "test-chip-gpio-v2".to_string(),
            arch: Arch::Arm,
            flash: MemoryRange {
                base: 0x0,
                size: "128KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![PeripheralConfig {
                id: "gpioa".to_string(),
                r#type: "gpio".to_string(),
                base_address: 0x4001_0800,
                size: None,
                irq: None,
                config: gpio_config,
            }],
        };

        let manifest = SystemManifest {
            name: "test-system-gpio-v2".to_string(),
            chip: "test-chip-gpio-v2".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            board_io: Vec::new(),
        };

        let mut bus = crate::bus::SystemBus::from_config(&chip, &manifest).unwrap();
        let base = 0x4001_0800;

        // MODER @ 0x00: set pin 0 to output mode.
        bus.write_u32(base, 0x0000_0001).unwrap();
        assert_eq!(bus.read_u32(base).unwrap() & 0x3, 0x1);

        // ODR @ 0x14 and BSRR @ 0x18 should use STM32v2 offsets.
        bus.write_u32(base + 0x14, 0x0000_0002).unwrap();
        assert_eq!(bus.read_u32(base + 0x14).unwrap() & 0xFFFF, 0x0002);

        bus.write_u32(base + 0x18, 0x0001_0000).unwrap(); // reset pin 0
        bus.write_u32(base + 0x18, 0x0000_0001).unwrap(); // set pin 0
        assert_eq!(bus.read_u32(base + 0x14).unwrap() & 0x0001, 0x0001);
    }

    #[test]
    fn test_from_config_uart_profile_stm32v2() {
        let mut uart_config = HashMap::new();
        uart_config.insert(
            "profile".to_string(),
            serde_yaml::Value::String("stm32v2".to_string()),
        );

        let chip = ChipDescriptor {
            name: "test-chip-uart-v2".to_string(),
            arch: Arch::Arm,
            flash: MemoryRange {
                base: 0x0,
                size: "128KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![PeripheralConfig {
                id: "uart3".to_string(),
                r#type: "uart".to_string(),
                base_address: 0x4000_4800,
                size: None,
                irq: None,
                config: uart_config,
            }],
        };

        let manifest = SystemManifest {
            name: "test-system-uart-v2".to_string(),
            chip: "test-chip-uart-v2".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            board_io: Vec::new(),
        };

        let mut bus = crate::bus::SystemBus::from_config(&chip, &manifest).unwrap();
        let sink = Arc::new(Mutex::new(Vec::new()));
        bus.attach_uart_tx_sink(sink.clone(), false);

        let base = 0x4000_4800;
        bus.write_u8(base + 0x04, b'X').unwrap(); // legacy DR offset should not TX in v2 mode
        bus.write_u8(base + 0x28, b'Y').unwrap(); // TDR
        assert_eq!(bus.read_u8(base + 0x1C).unwrap(), 0xC0); // ISR ready flags

        let data = sink.lock().unwrap().clone();
        assert_eq!(data, vec![b'Y']);
    }

    #[test]
    fn test_from_config_rcc_profile_stm32v2() {
        let mut rcc_config = HashMap::new();
        rcc_config.insert(
            "profile".to_string(),
            serde_yaml::Value::String("stm32v2".to_string()),
        );

        let chip = ChipDescriptor {
            name: "test-chip-rcc-v2".to_string(),
            arch: Arch::Arm,
            flash: MemoryRange {
                base: 0x0,
                size: "128KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![PeripheralConfig {
                id: "rcc".to_string(),
                r#type: "rcc".to_string(),
                base_address: 0x4402_0C00,
                size: None,
                irq: None,
                config: rcc_config,
            }],
        };

        let manifest = SystemManifest {
            name: "test-system-rcc-v2".to_string(),
            chip: "test-chip-rcc-v2".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            board_io: Vec::new(),
        };

        let mut bus = crate::bus::SystemBus::from_config(&chip, &manifest).unwrap();
        let base = 0x4402_0C00;

        bus.write_u32(base + 0xA4, 0xA5A5_0001).unwrap();
        bus.write_u32(base + 0x9C, 0x5A5A_0002).unwrap();

        assert_eq!(bus.read_u32(base + 0xA4).unwrap(), 0xA5A5_0001);
        assert_eq!(bus.read_u32(base + 0x9C).unwrap(), 0x5A5A_0002);
        assert_eq!(bus.read_u32(base + 0x18).unwrap(), 0);
    }

    #[test]
    fn test_from_config_profile_register_layout_alias_still_supported() {
        let mut gpio_config = HashMap::new();
        gpio_config.insert(
            "register_layout".to_string(),
            serde_yaml::Value::String("stm32v2".to_string()),
        );

        let chip = ChipDescriptor {
            name: "test-chip-gpio-v2-alias".to_string(),
            arch: Arch::Arm,
            flash: MemoryRange {
                base: 0x0,
                size: "128KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            peripherals: vec![PeripheralConfig {
                id: "gpioa".to_string(),
                r#type: "gpio".to_string(),
                base_address: 0x4001_0800,
                size: None,
                irq: None,
                config: gpio_config,
            }],
        };

        let manifest = SystemManifest {
            name: "test-system-gpio-v2-alias".to_string(),
            chip: "test-chip-gpio-v2-alias".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            board_io: Vec::new(),
        };

        let mut bus = crate::bus::SystemBus::from_config(&chip, &manifest).unwrap();
        let base = 0x4001_0800;
        bus.write_u32(base + 0x14, 0x0000_0002).unwrap();
        assert_eq!(bus.read_u32(base + 0x14).unwrap() & 0xFFFF, 0x0002);
    }

    #[test]
    fn test_cpu_execute_sp_rel() {
        let mut machine = create_machine();
        let base_addr: u64 = 0x2000_0000;
        machine.cpu.pc = base_addr as u32;

        // Setup Stack Pointer
        let stack_top = 0x2000_1000;
        machine.cpu.sp = stack_top;

        // 1. STR R0, [SP, #4]
        // R0 = 0xCAFEBABE
        machine.cpu.r0 = 0xCAFEBABE;

        // Opcode: 1001 0 000 00000001 (STR R0, [SP, 4]) -> 0x9001
        machine.bus.write_u8(base_addr, 0x01).unwrap();
        machine.bus.write_u8(base_addr + 1, 0x90).unwrap();

        machine.step().unwrap();

        // Verify Memory at SP+4
        let val = machine.bus.read_u32((stack_top + 4) as u64).unwrap();
        assert_eq!(val, 0xCAFEBABE);

        // 2. LDR R1, [SP, #4]
        // Opcode: 1001 1 001 00000001 (LDR R1, [SP, 4]) -> 0x9901
        machine.bus.write_u8(base_addr + 2, 0x01).unwrap();
        machine.bus.write_u8(base_addr + 3, 0x99).unwrap();

        machine.step().unwrap();

        assert_eq!(machine.cpu.r1, 0xCAFEBABE);
    }

    #[test]
    fn test_cpu_execute_cond_branch() {
        let mut machine = create_machine();
        let base_addr: u64 = 0x2000_0000;
        machine.cpu.pc = base_addr as u32;

        // 1. CMP R0, #0 -> Z=1
        // MOV R0, #0
        machine.cpu.r0 = 0;
        // CMP R0, #0 -> 0x2800 (0010 1000 0000 0000)

        // Manual store of CMP R0, #0
        machine.bus.write_u8(base_addr, 0x00).unwrap();
        machine.bus.write_u8(base_addr + 1, 0x28).unwrap();

        machine.step().unwrap();

        // Check Z flag in XPSR (Bit 30)
        assert_eq!(machine.cpu.xpsr & (1 << 30), 1 << 30);

        // 2. BEQ +4 (If Z=1, Branch)
        // Encoding: 0xD002 (Cond=0 EQ, Offset=4)
        machine.bus.write_u8(base_addr + 2, 0x02).unwrap();
        machine.bus.write_u8(base_addr + 3, 0xD0).unwrap();

        // Target should be Base + 2 + 4 + 4 = Base + 10 (Wait)
        // PC during execution is (Base+2). Pipeline PC = (Base+2) + 4.
        // Target = PC + 4 + offset?
        // Thumb Bcc: Target = PC + 4 + (imm8 << 1)
        // My decoder: offset = imm8 << 1 = 4.
        // CPU logic: target = pc + 4 + offset.
        // Wait, standard: Target = PC + 4 + (sign_extended(imm8) << 1)
        // If my decoder returns offset=4, and logic is pc+4+offset => pc+8.
        // Let's verify standard.
        // "Branch target address = PC + 4 + (SignExtended(imm8) << 1)"
        // Correct.

        machine.step().unwrap();

        // PC was 0x2000_0002.
        // PC+4 = 0x2000_0006.
        // Offset = 4.
        // Target = 0x2000_000A.

        assert_eq!(machine.cpu.pc, 0x2000_000A);
    }

    #[test]
    fn test_cpu_execute_shifts() {
        let mut machine = create_machine();
        let base_addr: u64 = 0x2000_0000;
        machine.cpu.pc = base_addr as u32;

        // LSLS R0, R1, #4
        machine.cpu.r1 = 0x0000_0001;
        // 0x0110 -> (000 00 00100 001 000) ?
        // 00000 00100 001 000 -> 0x0108
        machine.bus.write_u8(base_addr, 0x08).unwrap();
        machine.bus.write_u8(base_addr + 1, 0x01).unwrap();

        machine.step().unwrap();
        assert_eq!(machine.cpu.r0, 0x10);

        // LSRS R2, R3, #2
        machine.cpu.r3 = 0x10;
        // 00001 00010 011 010 -> 0x089A
        machine.bus.write_u8(base_addr + 2, 0x9A).unwrap();
        machine.bus.write_u8(base_addr + 3, 0x08).unwrap();

        machine.step().unwrap();
        assert_eq!(machine.cpu.r2, 0x04);
    }

    #[test]
    fn test_cpu_execute_cmp_reg() {
        let mut machine = create_machine();
        let base_addr: u64 = 0x2000_0000;
        machine.cpu.pc = base_addr as u32;

        machine.cpu.r1 = 10;
        machine.cpu.r0 = 5;
        // CMP R1, R0 -> 0x4281
        machine.bus.write_u8(base_addr, 0x81).unwrap();
        machine.bus.write_u8(base_addr + 1, 0x42).unwrap();

        machine.step().unwrap();
        // 10 - 5 = 5. N=0, Z=0, C=1 (no borrow), V=0
        let xpsr = machine.cpu.xpsr >> 28;
        assert_eq!(xpsr & 0b1000, 0); // N
        assert_eq!(xpsr & 0b0100, 0); // Z
        assert_eq!(xpsr & 0b0010, 0b0010); // C
    }

    #[test]
    fn test_cpu_execute_mov_reg() {
        let mut machine = create_machine();
        let base_addr: u64 = 0x2000_0000;
        machine.cpu.pc = base_addr as u32;

        machine.cpu.sp = 0x2002_0000;
        // MOV R7, SP -> 0x466F
        machine.bus.write_u8(base_addr, 0x6F).unwrap();
        machine.bus.write_u8(base_addr + 1, 0x46).unwrap();

        machine.step().unwrap();
        assert_eq!(machine.cpu.r7, 0x2002_0000);
    }

    #[test]
    fn test_cpu_execute_strb_imm() {
        let mut machine = create_machine();
        let base_addr: u64 = 0x2000_0000;
        machine.cpu.pc = base_addr as u32;

        machine.cpu.r1 = 0xAB;
        machine.cpu.r0 = 0x2000_1000;
        // STRB R1, [R0, #0] -> 0x7001
        machine.bus.write_u8(base_addr, 0x01).unwrap();
        machine.bus.write_u8(base_addr + 1, 0x70).unwrap();

        machine.step().unwrap();
        assert_eq!(machine.bus.read_u8(0x2000_1000).unwrap(), 0xAB);
    }

    #[test]
    fn test_systick_timer() {
        let mut machine = create_machine();

        // 1. Configure SysTick
        // RVR = 10 (Reload after 10 ticks)
        machine.bus.write_u32(0xE000_E014, 10).unwrap();
        // CSR = 1 (Enable)
        machine.bus.write_u32(0xE000_E010, 1).unwrap();

        // CVR is initially 0, so first tick should reload and start counting?
        // In my impl: tick() checks ENABLE. If 0, returns false.
        // If cvr == 0, cvr = rvr, sets COUNTFLAG.

        // Step 1: PC=0 (Unknown instruction at 0 is likely, but machine.step will still tick systick)
        let _ = machine.step();
        let cvr = machine.bus.read_u32(0xE000_E018).unwrap();
        assert_eq!(cvr, 10);

        // Step 2-11: Count down to 0
        for _ in 0..10 {
            let _ = machine.step();
        }

        let cvr_final = machine.bus.read_u32(0xE000_E018).unwrap();
        assert_eq!(cvr_final, 0);

        let csr = machine.bus.read_u32(0xE000_E010).unwrap();
        assert_eq!(csr & 0x10000, 0x10000); // COUNTFLAG should be set
    }

    #[test]
    fn test_exception_stacking() {
        let mut machine = create_machine();

        // 1. Setup Vector Table for SysTick (Exception 15)
        // Address = 15 * 4 = 60 (0x3C)
        let isr_addr: u32 = 0x0000_1000;
        machine.bus.write_u32(0x3C, isr_addr | 1).unwrap(); // Thumb address

        // 2. Setup initial state
        machine.cpu.pc = 0x2000_0000;
        machine.cpu.sp = 0x2002_0000;
        machine.cpu.r0 = 0x12345678;

        // 3. Trigger SysTick (Reload=1, Enable=3 [ENABLE|TICKINT])
        machine.bus.write_u32(0xE000_E014, 1).unwrap();
        machine.bus.write_u32(0xE000_E010, 3).unwrap();

        // Step 1: PC=0x2000_0000. Ticks SysTick.
        // SysTick wrap triggers exception 15.
        let _ = machine.step();

        // Step 2: Next step should detect pending exception AND handle it.
        // It should perform stacking and jump to 0x1000.
        let _ = machine.step();

        assert_eq!(machine.cpu.pc, 0x1000);
        assert_eq!(machine.cpu.sp, 0x2002_0000 - 32);
        assert_eq!(machine.cpu.lr, 0xFFFF_FFF9);

        // Check if R0 was stacked correctly at [SP]
        let stacked_r0 = machine.bus.read_u32(machine.cpu.sp as u64).unwrap();
        assert_eq!(stacked_r0, 0x12345678);
    }

    #[test]
    fn test_exception_lifecycle() {
        let mut machine = create_machine();

        // 1. Setup SysTick Vector
        let isr_addr: u32 = 0x0000_1000;
        machine.bus.write_u32(0x3C, isr_addr | 1).unwrap();

        // 2. Setup initial state
        machine.cpu.pc = 0x2000_0000;
        machine.cpu.sp = 0x2002_0000;
        machine.cpu.r0 = 10;
        machine.cpu.r7 = 20;

        // 3. Trigger SysTick
        machine.bus.write_u32(0xE000_E014, 100).unwrap();
        machine.bus.write_u32(0xE000_E010, 3).unwrap();

        // Step 1: Wrap SysTick
        machine.step().unwrap();

        // Step 2: Handle Exception (Entry)
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 0x1000);
        assert_eq!(machine.cpu.lr, 0xFFFF_FFF9);

        // 4. In ISR: Modify R0, then BX LR
        // MOV R0, #42 -> 0x202A
        machine.bus.write_u8(0x1000, 0x2A).unwrap();
        machine.bus.write_u8(0x1001, 0x20).unwrap();
        // BX LR -> 0x4770 (BX R14)
        machine.bus.write_u8(0x1002, 0x70).unwrap();
        machine.bus.write_u8(0x1003, 0x47).unwrap();

        // Step 3: Execute MOV R0, #42 in ISR
        machine.step().unwrap();
        assert_eq!(machine.cpu.r0, 42);

        // Step 4: Execute BX LR (Exception Return)
        machine.step().unwrap();

        // 5. Verify restored state
        assert_eq!(machine.cpu.pc, 0x2000_0002); // Back at original PC + 2
        assert_eq!(machine.cpu.r0, 10); // Original R0 restored!
        assert_eq!(machine.cpu.sp, 0x2002_0000); // SP restored
        assert_eq!(machine.cpu.r7, 20); // R7 was untouched
    }

    #[test]
    fn test_iteration_7_instructions() {
        let mut machine: Machine<CortexM> = create_machine();
        machine.cpu.sp = 0x2000_1000;

        // 1. ADD SP, #12 (3 * 4) -> 0xB003
        machine.bus.write_u16(0, 0xB003).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.sp, 0x2000_100C);

        // 2. SUB SP, #16 (4 * 4) -> 0xB084
        machine.cpu.pc = 2;
        machine.bus.write_u16(2, 0xB084).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.sp, 0x2000_0FFC);

        // 3. ADD R0, R8 (High Reg) -> 0x4440 (Rd=R0, Rm=R8)
        machine.cpu.r0 = 10;
        machine.cpu.r8 = 20;
        machine.cpu.pc = 4;
        machine.bus.write_u16(4, 0x4440).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r0, 30);

        // 4. CPSID i -> 0xB672
        machine.cpu.primask = false;
        machine.cpu.pc = 6;
        machine.bus.write_u16(6, 0xB672).unwrap();
        machine.step().unwrap();
        assert!(machine.cpu.primask);

        // 5. CPSIE i -> 0xB662
        machine.cpu.pc = 8;
        machine.bus.write_u16(8, 0xB662).unwrap();
        machine.step().unwrap();
        assert!(!machine.cpu.primask);
    }

    #[test]
    fn test_iteration_8_instructions() {
        let mut machine: Machine<CortexM> = create_machine();

        // 1. STRH R1, [R0, #4] -> 0x8081 (Rn=R0, Rt=R1, imm5=2 so imm=4)
        machine.cpu.r0 = 0x2000_1000;
        machine.cpu.r1 = 0xABCD;
        machine.bus.write_u16(0, 0x8081).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.bus.read_u16(0x2000_1004).unwrap(), 0xABCD);

        // 2. LDRH R2, [R0, #4] -> 0x8882 (Rn=R0, Rt=R2, imm5=2 so imm=4)
        machine.cpu.pc = 2;
        machine.bus.write_u16(2, 0x8882).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r2, 0xABCD);

        // 3. STM R0!, {R1, R2} -> 0xC006 (Rn=R0, Regs=0x06 (R1,R2))
        machine.cpu.r0 = 0x2000_2000;
        machine.cpu.r1 = 0x11223344;
        machine.cpu.r2 = 0x55667788;
        machine.cpu.pc = 4;
        machine.bus.write_u16(4, 0xC006).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.bus.read_u32(0x2000_2000).unwrap(), 0x11223344);
        assert_eq!(machine.bus.read_u32(0x2000_2004).unwrap(), 0x55667788);
        assert_eq!(machine.cpu.r0, 0x2000_2008); // Rn updated

        // 4. LDM R0!, {R3, R4} -> 0xC818 (Rn=R0, Regs=0x18 (R3,R4))
        // Base is 0x2000_2008 now. Let's write some data there.
        machine.bus.write_u32(0x2000_2008, 0xAAAAAAAA).unwrap();
        machine.bus.write_u32(0x2000_200C, 0xBBBBBBBB).unwrap();
        machine.cpu.pc = 6;
        machine.bus.write_u16(6, 0xC818).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r3, 0xAAAAAAAA);
        assert_eq!(machine.cpu.r4, 0xBBBBBBBB);
        assert_eq!(machine.cpu.r0, 0x2000_2010); // Rn updated

        // 5. MULS R0, R1 -> 0x4348 (Rd=R0, Rn=R1)
        machine.cpu.r0 = 10;
        machine.cpu.r1 = 20;
        machine.cpu.pc = 8;
        machine.bus.write_u16(8, 0x4348).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r0, 200);
    }

    #[test]
    fn test_nvic_external_interrupt() {
        let mut machine: Machine<CortexM> = create_machine();

        // IRQ 0 (Exception 16)
        let irq_num = 16;
        let isr_addr = 0x2000;
        // Vector at VTOR + (16 * 4) = 0x40
        machine.bus.write_u32(0x40, isr_addr | 1).unwrap();

        // 1. Initially disabled. Peripheral ticks but nothing should happen.
        // Mock a peripheral at 0x4000_0000 with IRQ 16
        machine.bus.peripherals.push(crate::bus::PeripheralEntry {
            name: "mock".to_string(),
            base: 0x4000_0000,
            size: 0x10,
            irq: Some(irq_num),
            dev: Box::new(crate::peripherals::stub::StubPeripheral::new(0)),
        });
        // (Note: StubPeripheral::tick returns false. I should use a more active one or just pend manually)

        // Manually pend it in NVIC ISPR
        machine.bus.write_u8(0xE000E100 + 0x100, 1).unwrap(); // ISPR0 bit 0
        machine.step().unwrap();
        assert_ne!(machine.cpu.pc, isr_addr); // Should NOT have jumped (disabled)

        // 2. Enable in NVIC ISER
        machine.bus.write_u8(0xE000E100, 1).unwrap(); // ISER0 bit 0
        machine.step().unwrap(); // Step instruction, collect interrupt
        machine.step().unwrap(); // Handle interrupt
        assert_eq!(machine.cpu.pc, isr_addr); // Should JUMP now
    }

    #[test]
    fn test_vtor_relocation() {
        let mut machine: Machine<CortexM> = create_machine();

        // Relocate VTOR to RAM at 0x2000_0000
        machine.bus.write_u32(0xE000ED08, 0x2000_0000).unwrap();

        // Exception 15 (SysTick) at new VTOR + 0x3C = 0x2000_003C
        let isr_addr = 0x5000;
        machine.bus.write_u32(0x2000_003C, isr_addr | 1).unwrap();

        // Verify reset works from new VTOR
        machine.bus.write_u32(0x2000_0000, 0x2002_0000).unwrap(); // SP
        machine.bus.write_u32(0x2000_0004, 0x1000).unwrap(); // PC
        machine.reset().unwrap();
        assert_eq!(machine.cpu.pc, 0x1000);
        assert_eq!(machine.cpu.sp, 0x2002_0000);

        // Verify exception uses new VTOR
        machine.cpu.set_exception_pending(15);
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, isr_addr);
    }

    #[test]
    fn test_mov_w_instruction() {
        let mut machine: Machine<CortexM> = create_machine();
        machine.cpu.pc = 0;
        machine.cpu.sp = 0x2000_1000;

        // Test MOV.W R0, #0x55
        // Encoding: 0xF04F 0x0055
        // imm12 = 0x055 (i=0, imm3=0, imm8=0x55)
        // Pattern 0, no rotation -> result = 0x55
        machine.bus.write_u16(0, 0xF04F).unwrap();
        machine.bus.write_u16(2, 0x0055).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r0, 0x55, "MOV.W R0, #0x55 failed");
        assert_eq!(machine.cpu.pc, 4, "PC should advance by 4");

        // Test MOV.W R1, #0x42
        // Encoding: 0xF04F 0x0142
        // imm12 = 0x042 (i=0, imm3=0, imm8=0x42)
        // Pattern 0, no rotation -> result = 0x42
        machine.cpu.pc = 4;
        machine.bus.write_u16(4, 0xF04F).unwrap();
        machine.bus.write_u16(6, 0x0142).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r1, 0x42, "MOV.W R1, #0x42 failed");
        assert_eq!(machine.cpu.pc, 8, "PC should advance by 4");
    }

    #[test]
    fn test_mvn_w_instruction() {
        let mut machine: Machine<CortexM> = create_machine();
        machine.cpu.pc = 0;
        machine.cpu.sp = 0x2000_1000;

        // Test MVN.W R0, #0x55
        // Encoding: 0xF06F 0x0055
        // imm12 = 0x055, expands to 0x55, then inverted to 0xFFFFFFAA
        machine.bus.write_u16(0, 0xF06F).unwrap();
        machine.bus.write_u16(2, 0x0055).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r0, !0x55, "MVN.W R0, #0x55 failed");
        assert_eq!(machine.cpu.pc, 4, "PC should advance by 4");
    }

    #[test]
    fn test_division_instructions() {
        let mut machine: Machine<CortexM> = create_machine();
        machine.cpu.pc = 0;
        machine.cpu.sp = 0x2000_1000;

        // Test UDIV: R0 = R1 / R2 (100 / 5 = 20)
        // Encoding: 0xFBB1 0xF0F2 (UDIV R0, R1, R2)
        machine.cpu.r1 = 100;
        machine.cpu.r2 = 5;
        machine.bus.write_u16(0, 0xFBB1).unwrap();
        machine.bus.write_u16(2, 0xF0F2).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r0, 20, "UDIV 100/5 failed");
        assert_eq!(machine.cpu.pc, 4);

        // Test SDIV: R3 = R4 / R5 (-100 / 5 = -20)
        // Encoding: 0xFB94 0xF3F5 (SDIV R3, R4, R5)
        machine.cpu.pc = 4;
        machine.cpu.r4 = (-100i32) as u32;
        machine.cpu.r5 = 5;
        machine.bus.write_u16(4, 0xFB94).unwrap();
        machine.bus.write_u16(6, 0xF3F5).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r3 as i32, -20, "SDIV -100/5 failed");
        assert_eq!(machine.cpu.pc, 8);

        // Test division by zero (should return 0)
        machine.cpu.pc = 8;
        machine.cpu.r6 = 100;
        machine.cpu.r7 = 0;
        machine.bus.write_u16(8, 0xFBB6).unwrap();
        machine.bus.write_u16(10, 0xF8F7).unwrap(); // UDIV R8, R6, R7
        machine.step().unwrap();
        assert_eq!(machine.cpu.r8, 0, "Division by zero should return 0");
    }

    #[test]
    fn test_gpio_basic() {
        let mut machine: Machine<CortexM> = create_machine();

        // GPIOA Base: 0x4001_0800
        // CRL: 0x00, IDR: 0x08, ODR: 0x0C, BSRR: 0x10, BRR: 0x14

        // 1. Check reset values
        let crl = machine.bus.read_u32(0x4001_0800).unwrap();
        assert_eq!(crl, 0x4444_4444, "GPIOA_CRL reset value mismatched");

        // 2. Test ODR write
        machine.bus.write_u32(0x4001_080C, 0x1234).unwrap();
        let odr = machine.bus.read_u32(0x4001_080C).unwrap();
        assert_eq!(odr, 0x1234, "GPIOA_ODR write/read mismatched");

        // 3. Test BSRR Set (Pin 5)
        machine.bus.write_u32(0x4001_0810, 1 << 5).unwrap();
        let odr = machine.bus.read_u32(0x4001_080C).unwrap();
        assert_eq!(odr, 0x1234 | (1 << 5), "GPIOA_BSRR Set failed");

        // 4. Test BSRR Reset (Pin 4)
        machine.bus.write_u32(0x4001_0810, 1 << (16 + 4)).unwrap();
        let odr = machine.bus.read_u32(0x4001_080C).unwrap();
        assert_eq!(
            odr,
            (0x1234 | (1 << 5)) & !(1 << 4),
            "GPIOA_BSRR Reset failed"
        );

        // 5. Test BRR (Pin 5)
        machine.bus.write_u32(0x4001_0814, 1 << 5).unwrap();
        let odr = machine.bus.read_u32(0x4001_080C).unwrap();
        assert_eq!(
            odr,
            (0x1234 | (1 << 5)) & !(1 << 4) & !(1 << 5),
            "GPIOA_BRR failed"
        );
    }

    #[test]
    fn test_metrics_collection() {
        use crate::metrics::PerformanceMetrics;
        let mut machine = create_machine();
        let metrics = std::sync::Arc::new(PerformanceMetrics::new());
        machine.observers.push(metrics.clone());

        // Setup: R0 = 10 (16-bit MOV)
        // Code: 200A (MOV R0, #10)
        machine.bus.write_u16(0x0, 0x200A).unwrap();
        machine.cpu.pc = 0x0;

        machine.step().unwrap();
        assert_eq!(metrics.get_instructions(), 1);
        assert_eq!(metrics.get_cycles(), 1);

        // Setup: BL #0 (32-bit instruction)
        // Code: F000 F800 (BL +0)
        machine.bus.write_u16(0x2, 0xF000).unwrap();
        machine.bus.write_u16(0x4, 0xF800).unwrap();
        machine.cpu.pc = 0x2;

        machine.step().unwrap();
        assert_eq!(metrics.get_instructions(), 2);
        assert_eq!(metrics.get_cycles(), 3); // 1 (MOV) + 2 (BL) = 3
    }

    #[test]
    fn test_peripheral_cycle_accounting_systick() {
        use crate::metrics::PerformanceMetrics;

        let mut machine = create_machine();
        let metrics = std::sync::Arc::new(PerformanceMetrics::new());
        machine.observers.push(metrics.clone());

        // Enable SysTick so it incurs a tick cost each machine step.
        machine.bus.write_u32(0xE000_E010, 1).unwrap(); // CSR = ENABLE

        // MOV R0, #10 (16-bit)
        machine.bus.write_u16(0x0, 0x200A).unwrap();
        machine.cpu.pc = 0x0;

        machine.step().unwrap();

        assert_eq!(metrics.get_instructions(), 1);
        assert_eq!(metrics.get_peripheral_cycles_total(), 1);
        assert_eq!(metrics.get_peripheral_cycles("systick"), 1);
        assert_eq!(metrics.get_cycles(), 2); // 1 (MOV) + 1 (SysTick tick)
    }

    #[test]
    fn test_bit_field_instructions() {
        let mut machine: Machine<CortexM> = create_machine();

        // Test UBFX (Unsigned Bit Field Extract)
        // Extract bits [7:4] from 0xABCD1234
        // UBFX R1, R0, #4, #4
        // Encoding: h1 = 0xF3C0 (1111 0011 1100 0000)
        // h2 = 0imm3 Rd imm2 widthm1
        // lsb = 4 = (imm3<<2)|imm2 = (1<<2)|0 = 4
        // widthm1 = 3 (width-1)
        // h2 = 0001 0001 0000 0011 = 0x1103
        machine.cpu.r0 = 0xABCD1234;
        machine.cpu.pc = 0x0;
        machine.bus.write_u16(0x0, 0xF3C0).unwrap();
        machine.bus.write_u16(0x2, 0x1103).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r1, 0x3); // Bits [7:4] = 0x3

        // Test SBFX (Signed Bit Field Extract)
        // Extract bits [7:4] from 0xFFFFFFF0 (negative)
        // SBFX R2, R0, #4, #4
        // lsb=4: imm3=1, imm2=0 -> (1<<2)|0 = 4
        // widthm1=3 (width-1)
        // h1 = 0xF340, h2 = 0x1203
        machine.cpu.r0 = 0xFFFFFFF0;
        machine.cpu.pc = 0x4;
        machine.bus.write_u16(0x4, 0xF340).unwrap();
        machine.bus.write_u16(0x6, 0x1203).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r2, 0xFFFFFFFF); // Sign-extended 0xF

        // Test BFC (Bit Field Clear)
        // Clear bits [7:4] in R3
        // BFC R3, #4, #4
        // lsb=4: imm3=1, imm2=0
        // msb=7 (lsb+width-1)
        // h1 = 0xF36F (Rn=0xF for BFC), h2 = 0x1307
        machine.cpu.r3 = 0xFFFFFFFF;
        machine.cpu.pc = 0x8;
        machine.bus.write_u16(0x8, 0xF36F).unwrap();
        machine.bus.write_u16(0xA, 0x1307).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r3, 0xFFFFFF0F); // Bits [7:4] cleared

        // Test BFI (Bit Field Insert)
        // Insert bits [3:0] of R0 into bits [7:4] of R4
        // BFI R4, R0, #4, #4
        // lsb=4: imm3=1, imm2=0
        // msb=7
        // h1 = 0xF360 (Rn=0), h2 = 0x1407
        machine.cpu.r0 = 0x0000000A;
        machine.cpu.r4 = 0xFFFFFF0F;
        machine.cpu.pc = 0xC;
        machine.bus.write_u16(0xC, 0xF360).unwrap();
        machine.bus.write_u16(0xE, 0x1407).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r4, 0xFFFFFFAF); // Inserted 0xA into bits [7:4]
    }

    #[test]
    fn test_misc_thumb2_instructions() {
        let mut machine: Machine<CortexM> = create_machine();

        // Test CLZ (Count Leading Zeros)
        // CLZ R1, R0
        // h1 = 0xFAB0, h2 = 0xF180 (for R0, R1)
        machine.cpu.r0 = 0x00000100; // 23 leading zeros
        machine.cpu.pc = 0x0;
        machine.bus.write_u16(0x0, 0xFAB0).unwrap();
        machine.bus.write_u16(0x2, 0xF180).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r1, 23);

        // Test RBIT (Reverse Bits)
        // RBIT R2, R0
        machine.cpu.r0 = 0x12345678;
        machine.cpu.pc = 0x4;
        machine.bus.write_u16(0x4, 0xFA90).unwrap();
        machine.bus.write_u16(0x6, 0xF2A0).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r2, 0x1E6A2C48); // Bit-reversed

        // Test REV (Byte-Reverse Word)
        // REV R3, R0
        machine.cpu.r0 = 0x12345678;
        machine.cpu.pc = 0x8;
        machine.bus.write_u16(0x8, 0xFA90).unwrap();
        machine.bus.write_u16(0xA, 0xF380).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r3, 0x78563412); // Byte-reversed

        // Test REV16 (Byte-Reverse Packed Halfword)
        // REV16 R4, R0
        machine.cpu.r0 = 0x12345678;
        machine.cpu.pc = 0xC;
        machine.bus.write_u16(0xC, 0xFA90).unwrap();
        machine.bus.write_u16(0xE, 0xF490).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r4, 0x34127856); // Halfwords byte-reversed

        // Test REVSH (Reverse byte order in lower halfword, sign extend)
        // REVSH R5, R0
        // h1 = 0xFA90 (Rm=0 in bits 3:0)
        // h2: 1111 dddd 1011 mmmm where dddd=Rd=5, mmmm=Rm=0
        // h2 = 0xF5B0
        machine.cpu.r0 = 0x1234ABCD;
        machine.cpu.pc = 0x10;
        machine.bus.write_u16(0x10, 0xFA90).unwrap();
        machine.bus.write_u16(0x12, 0xF5B0).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r5, 0xFFFFCDAB);
    }

    #[test]
    fn test_bitfield_instructions() {
        let mut machine: Machine<CortexM> = create_machine();

        // 1. BFI R0, R1, #4, #8
        // Rd=0, Rn=1, lsb=4, width=8. msb=11.
        // imm3=1, imm2=0, msb=11 (0x0B). h2 = 0x100B.
        machine.cpu.r0 = 0xFFFF_FFFF;
        machine.cpu.r1 = 0x0000_00AB;
        machine.cpu.pc = 0x0;
        machine.bus.write_u16(0x0, 0xF361).unwrap();
        machine.bus.write_u16(0x2, 0x100B).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r0, 0xFFFFFABF);

        // 2. BFC R2, #8, #16
        // Rd=2, lsb=8, width=16. msb=23 (0x17).
        machine.cpu.r2 = 0xFFFF_FFFF;
        machine.cpu.pc = 0x4;
        machine.bus.write_u16(0x4, 0xF36F).unwrap();
        machine.bus.write_u16(0x6, 0x2217).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r2, 0xFF0000FF);

        // 3. UBFX R3, R4, #4, #8
        machine.cpu.r4 = 0x0000_ABCD;
        machine.cpu.pc = 0x8;
        machine.bus.write_u16(0x8, 0xF3C4).unwrap();
        machine.bus.write_u16(0xA, 0x1307).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r3, 0xBC);

        // 4. SBFX R5, R6, #4, #8
        // R6 binary: ... 1010 1000 0000. Bits 11:4 = 0xA8.
        machine.cpu.r6 = 0x0000_0A80;
        machine.cpu.pc = 0xC;
        machine.bus.write_u16(0xC, 0xF346).unwrap();
        machine.bus.write_u16(0xE, 0x1507).unwrap();
        machine.step().unwrap();
        assert_eq!(machine.cpu.r5, 0xFFFFFFA8);
    }

    #[test]
    fn test_adc_conversion() {
        use crate::peripherals::adc::Adc;

        // 1. Setup Machine with ADC
        let mut bus = crate::bus::SystemBus::new();
        bus.peripherals.push(crate::bus::PeripheralEntry {
            name: "adc1".to_string(),
            base: 0x4001_2400,
            size: 0x400,
            irq: Some(18), // ADC1_2 global interrupt
            dev: Box::new(Adc::new()),
        });

        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);

        // 2. Enable ADC (ADON=1 in CR2)
        // Offset 0x08
        let adc_base = 0x4001_2400;
        let cr2_addr = adc_base + 0x08;
        machine.bus.write_u32(cr2_addr, 1).unwrap(); // ADON=1

        // 3. Start Conversion (SWSTART=1 in CR2)
        // Set SWSTART (bit 30) | ADON (bit 0)
        machine.bus.write_u32(cr2_addr, (1 << 30) | 1).unwrap();

        // 4. Step simulation to process conversion (cycles = 14)
        // We need to execute instructions or just tick.
        // Let's run NOPs.
        machine.bus.write_u16(0x0, 0xBF00).unwrap(); // NOP
        machine.cpu.pc = 0x0;

        // Run enough steps for conversion
        for _ in 0..20 {
            machine.step().unwrap();
        }

        // 5. Verify Result
        let dr_addr = adc_base + 0x4C;
        let sr_addr = adc_base;

        let dr = machine.bus.read_u32(dr_addr).unwrap();
        let sr = machine.bus.read_u32(sr_addr).unwrap();

        assert_ne!(dr, 0, "Data Register should have updated value");
        assert_eq!(
            sr & (1 << 1),
            (1 << 1),
            "EOC bit should be set in Status Register"
        );
    }

    #[test]
    fn test_state_snapshot() {
        use crate::snapshot::MachineSnapshot;

        let mut bus = crate::bus::SystemBus::new();
        // Use default peripherals (Rest of setup matches SystemBus defaults)

        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);

        // Modify CPU state
        machine.cpu.r0 = 42;
        machine.cpu.set_pc(0x0800_0000);

        // Modify Peripheral state (GPIOA ODR)
        machine.bus.write_u32(0x4001_080C, 0xAA).unwrap();

        let val = machine.bus.read_u32(0x4001_080C).unwrap();
        assert_eq!(val, 0xAA, "Readback failed");

        // Take snapshot
        let snap = machine.snapshot();

        // Verify CPU
        if let crate::snapshot::CpuSnapshot::Arm(s) = &snap.cpu {
            assert_eq!(s.registers[0], 42);
            assert_eq!(s.registers[15], 0x0800_0000); // PC is R15
        } else {
            panic!("Expected ARM snapshot");
        }

        // Verify Peripheral via JSON Value inspection
        // snap.peripherals is HashMap<String, serde_json::Value>
        let gpioa = snap.peripherals.get("gpioa").expect("gpioa missing");
        let odr = gpioa
            .get("odr")
            .expect("odr missing")
            .as_u64()
            .expect("odr not u64");
        assert_eq!(odr, 0xAA);

        // Check serialization
        let json_str = serde_json::to_string_pretty(&snap).unwrap();
        println!("Snapshot JSON:\n{}", json_str);

        // Check deserialization
        let _snap_restored: MachineSnapshot = serde_json::from_str(&json_str).unwrap();
    }

    #[test]
    fn test_declarative_peripheral_integration() {
        use crate::peripherals::declarative::GenericPeripheral;
        use labwired_config::PeripheralDescriptor;

        let yaml_path = "../../tests/fixtures/descriptors/mock_timer.yaml";
        let desc = PeripheralDescriptor::from_file(yaml_path).expect("Failed to load YAML");
        let mut p = GenericPeripheral::new(desc);

        // Verify Reset Values
        assert_eq!(p.read(0x00).unwrap(), 0x00, "CTRL reset value");
        assert_eq!(p.read(0x08).unwrap(), 0xFF, "ARR reset value byte 0");

        // Verify Write/Read
        p.write(0x00, 0x01).unwrap();
        assert_eq!(p.read(0x00).unwrap(), 0x01, "CTRL write value");

        // Verify Access Control (COUNT is RO)
        p.write(0x04, 0x55).unwrap();
        assert_eq!(p.read(0x04).unwrap(), 0x00, "COUNT should be RO");
    }

    #[test]
    fn test_breakpoint_sticky_step_over() {
        let mut machine = create_machine();
        let pc = 0x2000_0000;
        machine.cpu.set_pc(pc);

        // Write some NOPs (or MOV R0, R0)
        // 0x4600 is MOV R0, R0 (Thumb)
        machine.bus.write_u16(pc as u64, 0x4600).unwrap();
        machine.bus.write_u16(pc as u64 + 2, 0x4600).unwrap();

        // Add breakpoint at current PC
        machine.add_breakpoint(pc);

        // 1. Run should immediately stop at breakpoint
        let res = machine.run(Some(10)).unwrap();
        assert!(matches!(res, StopReason::Breakpoint(addr) if addr == pc));
        assert_eq!(
            machine.last_breakpoint,
            Some(pc),
            "Should record last hit breakpoint"
        );

        // 2. Run AGAIN should step OVER it and stop at MaxStepsReached (or next instruction)
        let res = machine.run(Some(1)).unwrap();
        assert!(
            matches!(res, StopReason::MaxStepsReached),
            "Should have stepped over and reached limit"
        );
        assert_eq!(
            machine.last_breakpoint, None,
            "Should clear last hit breakpoint after successful step"
        );
        assert_eq!(
            machine.cpu.get_pc(),
            pc + 2,
            "Should have executed one instruction (thumb)"
        );
    }
}
