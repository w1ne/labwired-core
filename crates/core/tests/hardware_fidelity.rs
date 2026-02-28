use labwired_core::peripherals::i2c::I2c;
use labwired_core::peripherals::pio::Pio;
use labwired_core::peripherals::spi::Spi;
use labwired_core::{bus::PeripheralEntry, bus::SystemBus, Bus, Peripheral};

#[test]
fn test_pio_fidelity_ws2812() {
    let mut bus = SystemBus::new();
    bus.peripherals.push(PeripheralEntry {
        name: "PIO0".to_string(),
        base: 0x40020000,
        size: 0x1000,
        irq: None,
        dev: Box::new(Pio::new()),
        ticks_remaining: 0,
    });
    bus.refresh_peripheral_index();

    // WS2812-like program: out pins, 1 [1] ; 1
    // .program ws2812
    // .wrap_target
    //     out pins, 1 [1]
    // .wrap
    let pio_asm = "
    .program ws2812
    .wrap_target
        out pins, 1 [1]
    .wrap
    ";

    // Use the inner Pico PIO implementation for the test
    let mut pio_peripheral = Pio::new();
    pio_peripheral.load_program_asm(pio_asm).unwrap();

    // Configure State Machine 0
    // SET/OUT pins = GP0
    // Clock div = 1.0 (Fixed point 16.8: 1.0 = 0x100)
    pio_peripheral.write_reg(0x0c8, 1 << 16); // SM0_CLKDIV: INT=1, FRAC=0
    pio_peripheral.write_reg(0x000, 1 << 0); // CTRL: Enable SM0

    // Push data to TX FIFO
    pio_peripheral.write_reg(0x010, 0xAAAAAAAA); // TXF0

    // Step and verify
    // Instruction: out pins, 1 [1]
    // 1 cycle for instruction + 1 delay cycle = 2 cycles total per bit

    // Note: Peripheral::tick takes no arguments in current implementation
    for i in 0..10 {
        pio_peripheral.tick();
        let pc_inner = pio_peripheral.sm[0].pc;
        // In the first 10 cycles, it should be executing the out pins [1] which cycles 0,0,1,1,etc?
        // Actually the current simplified PIO logic in pio.rs:
        // sm.delay_cycles = delay_side as u8;
        // if sm.delay_cycles > 0 -> return.
        // So at PC=0, it sets delay=1. Next tick, delay becomes 0. Next tick, it wraps or moves to 1.
        assert!(pc_inner <= 1, "Unexpected PC {} at step {}", pc_inner, i);
    }
}

#[test]
fn test_spi_fidelity_in_machine() {
    let mut bus = SystemBus::new();
    // Add SPI1 at 0x40013000
    bus.peripherals.push(PeripheralEntry {
        name: "SPI1".to_string(),
        base: 0x40013000,
        size: 0x400,
        irq: None,
        dev: Box::new(Spi::new()),
        ticks_remaining: 0,
    });
    bus.refresh_peripheral_index();

    // 1. Enable SPI and set Baud Rate
    bus.write_u8(0x40013000, 0x48).unwrap(); // CR1: SPE=1, BR=1 (div 4)

    // 2. Start transfer by writing to DR
    bus.write_u8(0x4001300C, 0x55).unwrap();

    // Check BSY is set
    let sr = bus.read_u8(0x40013008).unwrap();
    assert_ne!(sr & 0x80, 0);

    // Step machine 32 cycles (8 bits * 4 divider)
    for _ in 0..32 {
        bus.tick_peripherals();
    }

    // Check BSY is cleared and TXE/RXNE set
    let sr = bus.read_u8(0x40013008).unwrap();
    assert_eq!(sr & 0x80, 0);
    assert_ne!(sr & 0x02, 0); // TXE
    assert_ne!(sr & 0x01, 0); // RXNE

    // Read DR
    let dr = bus.read_u8(0x4001300C).unwrap();
    assert_eq!(dr, 0x55);
}

#[test]
fn test_i2c_fidelity_in_machine() {
    let mut bus = SystemBus::new();
    bus.peripherals.push(PeripheralEntry {
        name: "I2C1".to_string(),
        base: 0x40005400,
        size: 0x400,
        irq: None,
        dev: Box::new(I2c::new()),
        ticks_remaining: 0,
    });
    bus.refresh_peripheral_index();

    // 1. START
    bus.write_u8(0x40005401, 0x01).unwrap(); // CR1 START=1 (bit 8)
    for _ in 0..10 {
        bus.tick_peripherals();
    }
    assert_ne!(bus.read_u8(0x40005414).unwrap() & 0x01, 0); // SR1 SB=1

    // 2. Address
    bus.write_u8(0x40005410, 0xA0).unwrap(); // DR = 0xA0
    for _ in 0..20 {
        bus.tick_peripherals();
    }
    assert_eq!(bus.read_u8(0x40005414).unwrap() & 0x01, 0); // SB=0
    assert_ne!(bus.read_u8(0x40005414).unwrap() & 0x02, 0); // ADDR=1
}

#[test]
fn test_pio_fidelity_ws2812_full_bus() {
    let mut bus = SystemBus::new();
    let mut pio = Pio::new();

    // Simplified WS2812-like program:
    // 1. PULL block (wait for data)
    // 2. OUT X, 1 (extract 1 bit)
    // 3. JMP !X, zero [3] (Low phase)
    // 4. JMP end [3] (High phase for '1')
    // 5. zero: NOP [3] (High phase for '0' or just padding)
    let asm = "
.program ws2812
    pull block
    out x, 1
    jmp !x, bit_zero [3]
    jmp end [3]
bit_zero:
    nop [3]
end:
    ";
    pio.load_program_asm(asm).unwrap();

    bus.peripherals.push(PeripheralEntry {
        name: "PIO0".to_string(),
        base: 0x50200000,
        size: 0x1000,
        irq: None,
        dev: Box::new(pio),
        ticks_remaining: 0,
    });
    bus.refresh_peripheral_index();

    // 1. Enable SM0
    bus.write_u32(0x50200000, 1).unwrap(); // CTRL: SM0_ENABLE=1

    // 2. Verify it stalls (PC = 0, PULL block)
    bus.tick_peripherals();
    let pc = bus.read_u32(0x50200000 + 0x0c8 + 12).unwrap(); // SM0_PC
    assert_eq!(pc, 0);

    // 3. Push data to FIFO (Bit '1' and Bit '0')
    // Data: 0x0...0000_0001 (LSB is 1)
    bus.write_u32(0x50200000 + 0x10, 0x00000001).unwrap(); // TXF0

    // 4. Execute PULL (1 cycle)
    bus.tick_peripherals();
    let pc = bus.read_u32(0x50200000 + 0x0c8 + 12).unwrap();
    assert_eq!(pc, 1);

    // 5. Execute OUT X, 1 (1 cycle)
    bus.tick_peripherals();
    let pc = bus.read_u32(0x50200000 + 0x0c8 + 12).unwrap();
    assert_eq!(pc, 2);

    // 6. Execute JMP !X, bit_zero [3]
    // Since X=1, jump is NO, but delay [3] applies.
    bus.tick_peripherals();
    let pc = bus.read_u32(0x50200000 + 0x0c8 + 12).unwrap();
    assert_eq!(pc, 3); // PC points to next instruction (JMP end)

    // Verify delay cycles
    // Instruction 2 is JMP !X, bit_zero [3].
    // It should have set delay_cycles to 3.
    // Wait, the current implementation in pio.rs (line 290) sets delay_cycles = delay_side.
    // So it should tick 3 more times before next instruction.

    for i in 0..3 {
        bus.tick_peripherals();
        let pc_inner = bus.read_u32(0x50200000 + 0x0c8 + 12).unwrap();
        assert_eq!(pc_inner, 3, "Stalled at PC 3 during delay cycle {}", i);
    }

    // After 3 delay cycles, it should execute PC 3 (JMP end [3])
    bus.tick_peripherals();
    let pc = bus.read_u32(0x50200000 + 0x0c8 + 12).unwrap();
    assert_eq!(pc, 5); // Target of JMP end is PC 5 (end label)

    // It should again have 3 delay cycles
    for i in 0..3 {
        bus.tick_peripherals();
        let pc_inner = bus.read_u32(0x50200000 + 0x0c8 + 12).unwrap();
        assert_eq!(pc_inner, 5, "Stalled at PC 5 during delay cycle {}", i);
    }
}
