// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `connection: spi_gpio`: any SPI device model on arbitrary GPIO pads,
//! decoded from the pad edges firmware makes, on an ATmega2560 (PORTx through
//! the AVR CPU) and an STM32F103 (GPIO ODR/BSRR/IDR).
//!
//! Expectations come from the SPI mode definitions (CPOL = SCK idle level,
//! CPHA = 0 samples on the leading edge with the first bit on MISO from the
//! CS edge), from the Sitronix ST7920 "Serial Interface" section (sync byte
//! `0xF8 | RW << 2 | RS << 1`, each byte as two nibble bytes, CS active HIGH)
//! and from the SD Physical Layer Simplified Specification, SPI mode: CMD0
//! (`40 00 00 00 00 95`) answers R1 = 0x01 (idle), CMD8 with VHS = 1 and check
//! pattern 0xAA (`48 00 00 01 AA 87`) answers R7 = R1 0x01 then `00 00 01 AA`,
//! each response following NCR filler bytes of 0xFF.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::{Avr, AvrModel};
use labwired_core::inspect::{Artifact, InspectOpts};
use labwired_core::{Bus, Machine};
use std::collections::HashMap;
use std::path::PathBuf;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn try_bus(
    chip: &str,
    ty: &str,
    config: &[(&str, serde_yaml::Value)],
) -> anyhow::Result<SystemBus> {
    let chip_path = repo(chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let cfg: HashMap<String, serde_yaml::Value> = config
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "spi-gpio-rig".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![ExternalDevice {
            id: "dev".to_string(),
            r#type: ty.to_string(),
            connection: "spi_gpio".to_string(),
            channel: None,
            route: Default::default(),
            config: cfg,
        }],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        parts: Default::default(),
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest)
}

fn bus(chip: &str, ty: &str, config: &[(&str, serde_yaml::Value)]) -> SystemBus {
    try_bus(chip, ty, config).expect("build bus")
}

fn artifact(bus: &SystemBus) -> Artifact {
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    bus.display_artifact("dev", &opts)
        .expect("the device publishes a display artifact")
}

// ── ST7920 serial framing (Sitronix "Serial Interface") ──────────────────

fn st7920_cmd(wire: &mut Vec<u8>, b: u8) {
    wire.extend([0xF8, b & 0xF0, b << 4]);
}

fn st7920_data(wire: &mut Vec<u8>, bytes: &[u8]) {
    wire.push(0xFA);
    for &b in bytes {
        wire.extend([b & 0xF0, b << 4]);
    }
}

/// Basic set, display on, clear, then "HI" at the first text line.
fn st7920_hello() -> Vec<u8> {
    let mut w = Vec::new();
    for c in [0x30, 0x0C, 0x01, 0x80] {
        st7920_cmd(&mut w, c);
    }
    st7920_data(&mut w, b"HI");
    w
}

// ── SD SPI-mode init ─────────────────────────────────────────────────────

const CMD0: [u8; 6] = [0x40, 0, 0, 0, 0, 0x95];
const CMD8: [u8; 6] = [0x48, 0, 0, 0x01, 0xAA, 0x87];

/// CMD0, two 0xFF response bytes, CMD8, six 0xFF response bytes.
fn sd_init_wire() -> Vec<u8> {
    let mut w = CMD0.to_vec();
    w.extend([0xFF; 2]);
    w.extend(CMD8);
    w.extend([0xFF; 6]);
    w
}

/// What came back during the response bytes: R1 after CMD0, R7 after CMD8,
/// each preceded by at least one 0xFF filler (NCR).
fn assert_sd_init_response(miso: &[u8]) {
    assert_eq!(miso.len(), 20, "{miso:02X?}");
    let r1 = &miso[6..8];
    let first = r1.iter().position(|&b| b != 0xFF).expect("R1 after CMD0");
    assert!(first >= 1, "NCR >= 1 before R1: {r1:02X?}");
    assert_eq!(r1[first], 0x01, "CMD0 -> R1 idle: {miso:02X?}");
    let r7 = &miso[14..20];
    let first = r7.iter().position(|&b| b != 0xFF).expect("R7 after CMD8");
    assert!(first >= 1, "NCR >= 1 before R7: {r7:02X?}");
    assert_eq!(
        &r7[first..first + 5],
        &[0x01, 0x00, 0x00, 0x01, 0xAA],
        "CMD8 -> R7 echoes VHS and the check pattern: {miso:02X?}"
    );
}

// ── ATmega2560: real AVR code bit-bangs PORTA ────────────────────────────

/// Mode 0, MSB-first SPI bit-banged on PORTA: CS = PA0, SCK = PA1,
/// MOSI = PA2, MISO = PA3. Reads a table at flash byte 0x400
/// (`[cs_active_high, count_lo, count_hi, bytes...]`), sends each byte,
/// stores the byte MISO carried at SRAM 0x0200 + i, deasserts CS and spins.
/// Assembled from this source with avr-gcc -mmcu=atmega2560 -nostdlib
/// -nostartfiles -Wl,-Ttext=0:
///
/// ```text
///     ldi r16,0x21 / out SPH,r16 / ldi r16,0xFF / out SPL,r16
///     cbi PORTA,SCK
///     ldi r16,(1<<CS)|(1<<SCK)|(1<<MOSI) / out DDRA,r16
///     ldi ZL,0x00 / ldi ZH,0x04
///     lpm r20,Z+ / lpm r24,Z+ / lpm r25,Z+
///     rcall cs_off / rcall cs_on
///     ldi XL,0x00 / ldi XH,0x02
/// next_byte: sbiw r24,0 / breq done / lpm r16,Z+ / clr r17 / ldi r18,8
/// bit_loop:  cbi PORTA,MOSI / sbrc r16,7 / sbi PORTA,MOSI / lsl r16
///            sbi PORTA,SCK            ; rising: both ends sample
///            lsl r17 / sbic PINA,MISO / ori r17,1
///            cbi PORTA,SCK            ; falling: the slave shifts
///            dec r18 / brne bit_loop
///            st X+,r17 / sbiw r24,1 / rjmp next_byte
/// done:      rcall cs_off
/// halt:      rjmp halt
/// cs_on:     tst r20 / breq 1f / sbi PORTA,CS / ret / 1: cbi PORTA,CS / ret
/// cs_off:    tst r20 / breq 2f / cbi PORTA,CS / ret / 2: sbi PORTA,CS / ret
/// ```
const AVR_BITBANG: [u16; 49] = [
    0xE201, 0xBF0E, 0xEF0F, 0xBF0D, 0x9811, 0xE007, 0xB901, 0xE0E0, 0xE0F4, 0x9145, 0x9185, 0x9195,
    0xD01E, 0xD017, 0xE0A0, 0xE0B2, 0x9700, 0xF089, 0x9105, 0x2711, 0xE028, 0x9812, 0xFD07, 0x9A12,
    0x0F00, 0x9A11, 0x0F11, 0x9903, 0x6011, 0x9811, 0x952A, 0xF7A9, 0x931D, 0x9701, 0xCFED, 0xD007,
    0xCFFF, 0x2344, 0xF011, 0x9A10, 0x9508, 0x9810, 0x9508, 0x2344, 0xF011, 0x9810, 0x9508, 0x9A10,
    0x9508,
];

/// PORTA pads; `miso` only for a device that answers (the ST7920 has no
/// serial read, so its SID is the only data line).
fn mega_pins(cs_active: &str, miso: bool) -> Vec<(&'static str, serde_yaml::Value)> {
    let mut v = vec![
        ("sck", "PA1".into()),
        ("mosi", "PA2".into()),
        ("cs", "PA0".into()),
        ("cs_active", cs_active.into()),
        ("mode", 0.into()),
        ("bit_order", "msb".into()),
    ];
    if miso {
        v.push(("miso", "PA3".into()));
    }
    v
}

/// Run the bit-bang firmware over `wire` until it spins; return the bus and
/// the MISO byte of every frame.
fn run_avr(bus: SystemBus, cs_active_high: bool, wire: &[u8]) -> (SystemBus, Vec<u8>) {
    let mut cpu = Avr::for_chip_name("atmega2560");
    assert_eq!(cpu.model, AvrModel::Atmega2560);
    cpu.load_words(0, &AVR_BITBANG);
    let mut table = vec![u8::from(cs_active_high)];
    table.extend((wire.len() as u16).to_le_bytes());
    table.extend_from_slice(wire);
    cpu.load_flash(0x400, &table);
    let mut machine = Machine::new(cpu, bus);
    let mut last = u32::MAX;
    let mut spins = 0;
    for _ in 0..400_000 {
        machine.step().expect("step");
        if machine.cpu.pc == last {
            spins += 1;
            if spins > 4 {
                let base = usize::from(0x0200 - machine.cpu.model.sram_start());
                let miso = machine.cpu.sram[base..base + wire.len()].to_vec();
                return (machine.bus, miso);
            }
        } else {
            spins = 0;
            last = machine.cpu.pc;
        }
    }
    panic!("bit-bang firmware never finished");
}

/// End to end: AVR code bit-banging ST7920 serial frames on PORTA pins that
/// are not the hardware SPI pins paints the panel. CS is active high.
#[test]
fn avr_bitbanged_st7920_paints_framebuffer() {
    let bus = bus(
        "configs/chips/atmega2560.yaml",
        "st7920",
        &mega_pins("high", false),
    );
    let (bus, _) = run_avr(bus, true, &st7920_hello());
    let art = artifact(&bus);
    assert_eq!(art.meta["display_on"], true, "{:?}", art.meta);
    assert_eq!(art.meta["framing_errors"], 0, "{:?}", art.meta);
    let line = art.meta["text"][0].as_str().expect("text line 0");
    assert_eq!(&line[..2], "HI", "{line:?}");
    assert!(
        art.meta["ink_bytes"].as_u64().unwrap() > 0,
        "{:?}",
        art.meta
    );
}

/// The same frames with the CS polarity reversed (the firmware holds CS low
/// throughout) never select an active-high ST7920: nothing is painted.
#[test]
fn avr_st7920_ignores_frames_while_cs_is_inactive() {
    let bus = bus(
        "configs/chips/atmega2560.yaml",
        "st7920",
        &mega_pins("high", false),
    );
    let (bus, _) = run_avr(bus, false, &st7920_hello());
    let art = artifact(&bus);
    assert_eq!(art.meta["display_on"], false, "{:?}", art.meta);
    assert_eq!(art.meta["ink_bytes"], 0, "{:?}", art.meta);
}

/// End to end: AVR code bit-bangs SD CMD0 and CMD8 and reads R1 and R7 back
/// through PINA, bit by bit. CS is active low.
#[test]
fn avr_bitbanged_sd_init_reads_r1_and_r7() {
    let bus = bus(
        "configs/chips/atmega2560.yaml",
        "microsd",
        &mega_pins("low", true),
    );
    let (_, miso) = run_avr(bus, false, &sd_init_wire());
    assert_sd_init_response(&miso);
}

// ── STM32F103: a scripted GPIO master on ODR/BSRR, MISO through IDR ──────

const GPIOA: u64 = 0x4001_0800;
const GPIO_IDR: u64 = 0x08;
const GPIO_BSRR: u64 = 0x10;
const RCC_APB2ENR: u64 = 0x4002_1018;

struct F103Master {
    bus: SystemBus,
    sck: u8,
    mosi: u8,
    miso: u8,
    cs: u8,
}

impl F103Master {
    fn new(bus: SystemBus) -> Self {
        let mut m = Self {
            bus,
            sck: 5,
            mosi: 7,
            miso: 6,
            cs: 4,
        };
        let en = m.bus.read_u32(RCC_APB2ENR).unwrap_or(0);
        m.bus.write_u32(RCC_APB2ENR, en | (1 << 2)).unwrap(); // IOPAEN
        m.set(m.cs, true);
        m.set(m.sck, false);
        m
    }

    fn set(&mut self, pin: u8, high: bool) {
        let v = if high {
            1u32 << pin
        } else {
            1u32 << (pin + 16)
        };
        self.bus.write_u32(GPIOA + GPIO_BSRR, v).unwrap();
    }

    fn miso(&self) -> bool {
        self.bus.read_u32(GPIOA + GPIO_IDR).unwrap() & (1 << self.miso) != 0
    }

    /// Mode 0, MSB first.
    fn xfer(&mut self, byte: u8) -> u8 {
        let mut got = 0u8;
        for i in (0..8).rev() {
            self.set(self.mosi, byte & (1 << i) != 0);
            self.set(self.sck, true);
            if self.miso() {
                got |= 1 << i;
            }
            self.set(self.sck, false);
        }
        got
    }

    fn frame(&mut self, cs_active_high: bool, wire: &[u8]) -> Vec<u8> {
        self.set(self.cs, cs_active_high);
        let out = wire.iter().map(|&b| self.xfer(b)).collect();
        self.set(self.cs, !cs_active_high);
        out
    }
}

fn f103_pins(cs_active: &str, miso: bool) -> Vec<(&'static str, serde_yaml::Value)> {
    let mut v = vec![
        ("sck", "PA5".into()),
        ("mosi", "PA7".into()),
        ("cs", "PA4".into()),
        ("cs_active", cs_active.into()),
    ];
    if miso {
        v.push(("miso", "PA6".into()));
    }
    v
}

/// SD init over BSRR stores, R1/R7 read back through IDR.
#[test]
fn stm32_bitbanged_sd_init_reads_r1_and_r7() {
    let mut m = F103Master::new(bus(
        "configs/chips/stm32f103.yaml",
        "microsd",
        &f103_pins("low", true),
    ));
    let miso = m.frame(false, &sd_init_wire());
    assert_sd_init_response(&miso);
}

/// ST7920 framing over BSRR stores paints the panel; CS active high.
#[test]
fn stm32_bitbanged_st7920_paints_framebuffer() {
    let mut m = F103Master::new(bus(
        "configs/chips/stm32f103.yaml",
        "st7920",
        &f103_pins("high", false),
    ));
    m.set(m.cs, false); // CS idle low for an active-high part
    m.frame(true, &st7920_hello());
    let art = artifact(&m.bus);
    assert_eq!(art.meta["display_on"], true, "{:?}", art.meta);
    assert!(
        art.meta["ink_bytes"].as_u64().unwrap() > 0,
        "{:?}",
        art.meta
    );
}

/// A CS release in the middle of a byte drops it and resets the card's
/// command framing: the CMD0 that follows a fresh CS assert still answers.
#[test]
fn stm32_cs_edge_aborts_partial_sd_command() {
    let mut m = F103Master::new(bus(
        "configs/chips/stm32f103.yaml",
        "microsd",
        &f103_pins("low", true),
    ));
    m.set(m.cs, false);
    m.xfer(0x40); // a command start byte
    for _ in 0..3 {
        m.set(m.mosi, true);
        m.set(m.sck, true);
        m.set(m.sck, false);
    }
    m.set(m.cs, true); // abort
    let mut wire = CMD0.to_vec();
    wire.extend([0xFF; 2]);
    let miso = m.frame(false, &wire);
    assert!(
        miso[6..].contains(&0x01),
        "fresh CMD0 answers R1: {miso:02X?}"
    );
}

// ── Hardware SPI reaches the same device ─────────────────────────────────

/// With `controller:`, the hardware path (AVR SPDR clocking the parking
/// SPI) and the GPIO decoder share one model and one CS polarity: an
/// active-high ST7920 selected by CS high receives hardware bytes, and
/// inspect lists the device once.
#[test]
fn controller_path_reaches_the_shared_device() {
    let mut cfg = vec![
        ("sck", "PB5".into()),
        ("mosi", "PB3".into()),
        ("cs", "PB2".into()),
        ("cs_active", "high".into()),
        ("controller", "spi".into()),
    ];
    cfg.push(("mode", 0.into()));
    let mut bus = bus("configs/chips/atmega328p.yaml", "st7920", &cfg);
    let portb = 0x0001_0023u64; // PINB; DDRB = +1, PORTB = +2
    bus.write_u8(portb + 1, 1 << 2).unwrap();
    bus.write_u8(portb + 2, 1 << 2).unwrap(); // CS high
    bus.sync_parking_spi_cs();
    let mut w = Vec::new();
    st7920_cmd(&mut w, 0x0C);
    for b in w {
        bus.clock_parking_spi(b);
    }
    assert_eq!(artifact(&bus).meta["display_on"], true);
    let opts = InspectOpts {
        include_bytes: false,
        peripheral: None,
    };
    let listed = bus
        .inspect_devices(None, &opts)
        .into_iter()
        .filter(|d| d.id == "dev")
        .count();
    assert_eq!(listed, 1, "one model, listed once");
}

// ── Manifest validation ──────────────────────────────────────────────────

#[test]
fn config_errors_name_the_bad_key() {
    let err = |cfg: &[(&str, serde_yaml::Value)]| {
        format!(
            "{:#}",
            try_bus("configs/chips/stm32f103.yaml", "microsd", cfg)
                .err()
                .expect("must be refused")
        )
    };
    let base = f103_pins("low", true);
    let with = |k: &'static str, v: serde_yaml::Value| {
        let mut c: Vec<_> = base.iter().filter(|(n, _)| *n != k).cloned().collect();
        c.push((k, v));
        c
    };
    assert!(err(&with("mode", 4.into())).contains("mode"));
    assert!(err(&with("bit_order", "middle".into())).contains("bit_order"));
    assert!(err(&with("cs_active", "maybe".into())).contains("cs_active"));
    assert!(err(&with("sck", "PZ99".into())).contains("sck"));
    let no_mosi: Vec<_> = base.iter().filter(|(n, _)| *n != "mosi").cloned().collect();
    assert!(err(&no_mosi).contains("mosi"));
}

/// A device that cannot say its next MISO byte before the frame cannot have
/// a read-back pad; it is refused at config time, not silently answered.
#[test]
fn miso_needs_a_device_that_previews_its_answer() {
    let err = try_bus(
        "configs/chips/stm32f103.yaml",
        "st7920",
        &f103_pins("high", true),
    )
    .err()
    .expect("st7920 has no serial read: a MISO pad is refused");
    assert!(format!("{err:#}").contains("next_miso"), "{err:#}");
}
