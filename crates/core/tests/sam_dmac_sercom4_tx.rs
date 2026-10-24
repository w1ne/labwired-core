// SPDX-License-Identifier: MIT
//! CPU-authored hardware-request TX guests, not native Arcade or physical timing.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, cycle_clock::event_scheduler_enabled, memory::ProgramImage,
    peripherals::spi::SpiDevice, Arch, Bus, Machine,
};
use std::sync::{Arc, Mutex};

const DMA: u32 = 0x4100a000;
const SPI: u32 = 0x43000000;
const PORT: u32 = 0x41008080;
const AHB: u32 = 0x40000810;
const APBD: u32 = 0x40000820;
const GCLK: u32 = 0x40001c00 + 0x80 + 34 * 4;
const TABLE: u32 = 0x20001000;
const WB: u32 = 0x20001200;
const SOURCE: u32 = 0x20002000;
const LAST_RAM: u32 = 0x2002ffff;
const TX: u32 = (0x0d << 8) | (2 << 20);
const MASTER: u32 = (3 << 2) | (2 << 16);
const MODES: &[&str] = &[
    "complete",
    "noact",
    "primask",
    "nvic",
    "inten",
    "global-off",
    "level-off",
    "channel-off",
    "spi-disabled",
    "dma-clock-off",
    "spi-clock-off",
    "gclk-off",
    "pause-dma",
    "pause-spi",
    "pause-gclk",
    "reset",
    "reset-restart",
    "read-error",
    "writeback-error",
];

struct Slave(Arc<Mutex<Vec<u8>>>);
impl SpiDevice for Slave {
    fn cs_pin(&self) -> &str {
        "PB7"
    }
    fn transfer(&mut self, byte: u8) -> u8 {
        self.0.lock().unwrap().push(byte);
        0xff
    }
}

fn config() -> (ChipDescriptor, SystemManifest) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut chip =
        ChipDescriptor::from_file(root.join("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    chip.peripherals.push(serde_yaml::from_str(
        "id: tx-dma\ntype: sam_dmac_sercom4_tx_byte\nbase_address: 0x4100a000\nsize: 1KB\nirq: 31\nclock: { controller: mclk, reg: AHBMASK, bit: 9 }\n"
    ).unwrap());
    let manifest = SystemManifest::from_file(root.join("configs/systems/pybadge.yaml")).unwrap();
    (chip, manifest)
}

fn board(legacy: bool) -> (SystemBus, Arc<Mutex<Vec<u8>>>) {
    let (chip, manifest) = config();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    if legacy {
        assert!(bus
            .peripherals
            .iter_mut()
            .find(|p| p.name == "sercom4")
            .unwrap()
            .dev
            .force_legacy_walk());
        bus.recompute_walk_deletable();
        bus.refresh_peripheral_index();
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    bus.attach_spi_device("sercom4", Box::new(Slave(seen.clone())))
        .unwrap();
    (bus, seen)
}

// Short aligned literal islands avoid the 1KiB Thumb LDR reach limit. All
// descriptor/data/register stores below are actually executed by the guest.
#[derive(Default)]
struct Guest(Vec<u8>);
impl Guest {
    fn op(&mut self, op: u16) {
        self.0.extend(op.to_le_bytes());
    }
    fn align(&mut self) {
        if self.0.len() & 3 != 0 {
            self.op(0xbf00);
        }
    }
    fn load0(&mut self, value: u32) {
        self.align();
        self.op(0x4800);
        self.op(0xe001);
        self.0.extend(value.to_le_bytes());
    }
    fn store(&mut self, address: u32, value: u32, byte: bool) {
        self.align();
        for op in [0x4801, 0x4902, if byte { 0x7001 } else { 0x6001 }, 0xe003] {
            self.op(op);
        }
        self.0.extend(address.to_le_bytes());
        self.0.extend(value.to_le_bytes());
    }
    fn sample_count(&mut self, address: u32, witness: u32) {
        self.load0(address + 2);
        self.op(0x8801); // LDRH r1,[r0].
        self.load0(witness);
        self.op(0x6001);
    }
    fn wait_for_progress(&mut self, address: u32) {
        self.load0(address + 2);
        // LDRH r1,[r0]; MOVS r2,#4; CMP r1,r2; BHS LDRH.
        // Initial sentinel and untouched count both loop. Only a real beat's
        // writeback below four releases the guest; host step bounds still apply.
        for op in [0x8801, 0x2204, 0x4291, 0xd2fb] {
            self.op(op);
        }
    }
    fn spi(&mut self, enabled: bool) {
        for (address, value, byte) in [
            (PORT + 0x36, 0x20, true),
            (PORT + 0x37, 0x20, true),
            (PORT + 0x4d, 1, true),
            (PORT + 0x4f, 1, true),
            (PORT, (1 << 7) | (1 << 5), false),
            (SPI, MASTER, false),
            (SPI + 4, 0, false), // TX-only; no RX overflow.
            (SPI + 0x0c, 7, true),
            (SPI, MASTER | if enabled { 2 } else { 0 }, false),
        ] {
            self.store(address, value, byte);
        }
    }
    fn arm(&mut self, index: usize, mode: &str) {
        let channel = DMA + 0x40 + index as u32 * 16;
        let descriptor = TABLE + index as u32 * 16;
        let writeback = WB + index as u32 * 16;
        for (address, value) in [
            (
                descriptor,
                4 << 16 | if mode == "noact" { 0x0401 } else { 0x0409 },
            ),
            (
                descriptor + 4,
                if mode == "read-error" {
                    LAST_RAM + 4
                } else {
                    SOURCE + 4
                },
            ),
            (descriptor + 8, SPI + 0x28),
            (descriptor + 12, 0),
            (writeback, 0xaaaaaaaa),
            (writeback + 4, 0xaaaaaaaa),
            (writeback + 8, 0xaaaaaaaa),
            (writeback + 12, 0xaaaaaaaa),
            (DMA + 0x34, TABLE),
            (
                DMA + 0x38,
                if mode == "writeback-error" {
                    0x60000000
                } else {
                    WB
                },
            ),
            (
                DMA,
                match mode {
                    "global-off" => 0x100,
                    "level-off" => 2,
                    _ => 0x102,
                },
            ),
        ] {
            self.store(address, value, false);
        }
        self.store(channel + 13, if mode == "inten" { 0 } else { 3 }, true);
        self.store(
            channel,
            TX | if mode == "channel-off" { 0 } else { 2 },
            false,
        );
        // Deliberately NO SWTRIGCTRL write and NO host request injection.
    }
}

fn guest(index: usize, mode: &str) -> ProgramImage {
    let irq = 31 + index.min(4) as u32;
    let channel = DMA + 0x40 + index as u32 * 16;
    let writeback = WB + index as u32 * 16;
    let mut g = Guest::default();
    g.op(0xb672); // CPSID i, before all configuration.
    for (address, value) in [
        (0xe000ed08, 0x4000),
        (0x20000000, 0),
        (0x20000004, 0),
        (0x20000008, 0),
        (0x2000000c, 0),
        (0x20000010, 0),
        (0x20000014, 0),
        (SOURCE, 0x64534231),
        (AHB, if mode == "dma-clock-off" { 0 } else { 1 << 9 }),
        (APBD, if mode == "spi-clock-off" { 0 } else { 1 }),
        (GCLK, if mode == "gclk-off" { 0 } else { 1 << 6 }),
    ] {
        g.store(address, value, false);
    }
    g.store(LAST_RAM, 0x5a, true);
    g.spi(mode != "spi-disabled");
    if mode != "nvic" {
        g.store(0xe000e100 + (irq / 32) * 4, 1 << (irq % 32), false);
    }
    g.arm(index, mode);
    let pause = match mode {
        "pause-dma" => Some((AHB, 1 << 9)),
        "pause-spi" => Some((APBD, 1)),
        "pause-gclk" => Some((GCLK, 1 << 6)),
        _ => None,
    };
    if let Some((gate, value)) = pause {
        g.wait_for_progress(writeback);
        g.store(gate, 0, false);
        g.sample_count(writeback, 0x20000010);
        for _ in 0..128 {
            g.op(0xbf00);
        }
        g.sample_count(writeback, 0x20000014);
        g.store(gate, value, false);
    }
    if matches!(mode, "reset" | "reset-restart") {
        g.wait_for_progress(writeback);
        g.store(DMA, 0, false); // Required before global SWRST.
        g.sample_count(writeback, 0x20000010);
        g.store(DMA, 1, false);
        g.store(SPI, 1, false); // Discard accepted but unshifted partial packet.
        if mode == "reset-restart" {
            g.spi(true);
            g.arm(index, "complete");
        }
    }
    if mode != "primask" {
        g.op(0xb662);
    }
    g.store(0x20000000, 0xb0055eed, false);
    g.op(0xe7fe);
    let handler = 0x4200 + g.0.len() as u32;
    g.load0(0x20000004);
    for op in [0x6801, 0x3101, 0x6001] {
        g.op(op);
    }
    g.store(0x20000008, irq, false);
    g.load0(channel + 14);
    g.op(0x7801);
    g.load0(0x2000000c);
    g.op(0x6001);
    g.store(channel + 14, 3, true);
    g.op(0x4770);
    let mut bytes = vec![0u8; 0x200];
    bytes[..4].copy_from_slice(&0x20004000u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x4201u32.to_le_bytes());
    let vector = (16 + irq as usize) * 4;
    bytes[vector..vector + 4].copy_from_slice(&(handler | 1).to_le_bytes());
    bytes.extend(g.0);
    let mut image = ProgramImage::new(0x4201, Arch::Arm);
    image.add_segment(0x4000, bytes);
    image
}

#[test]
fn cpu_programmed_sercom4_requests_transfer_pause_reset_and_return_from_dma_irq() {
    for legacy in [true, false] {
        for interval in [1, 64, 1024] {
            if interval > 1 && (legacy || !event_scheduler_enabled()) {
                continue;
            }
            for index in [0, 1, 2, 3, 4, 31] {
                for &mode in MODES {
                    let (mut bus, seen) = board(legacy);
                    let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
                    bus.config.peripheral_tick_interval = interval;
                    let mut machine = Machine::new(cpu, bus);
                    machine.config.peripheral_tick_interval = interval;
                    machine.load_firmware(&guest(index, mode)).unwrap();
                    let irq = 31 + index.min(4) as u32;
                    let mut previous = 0;
                    let mut entries = 0;
                    let mut returns = 0;
                    for _ in 0..6000 {
                        machine.step().unwrap();
                        let active = machine.cpu.active_exception;
                        if active != 0 {
                            assert_eq!(active, 16 + irq);
                        }
                        entries += usize::from(active != 0 && previous == 0);
                        returns += usize::from(active == 0 && previous != 0);
                        previous = active;
                    }
                    let error = mode.ends_with("error");
                    let completed = matches!(
                        mode,
                        "complete"
                            | "noact"
                            | "primask"
                            | "nvic"
                            | "inten"
                            | "pause-dma"
                            | "pause-spi"
                            | "pause-gclk"
                            | "reset-restart"
                    );
                    let delivered = error
                        || matches!(
                            mode,
                            "complete" | "pause-dma" | "pause-spi" | "pause-gclk" | "reset-restart"
                        );
                    assert_eq!(
                        entries,
                        usize::from(delivered),
                        "channel={index} mode={mode} legacy={legacy} interval={interval}"
                    );
                    assert_eq!(returns, entries);
                    assert_eq!(machine.cpu.active_exception, 0);
                    assert_eq!(machine.bus.read_u32(0x20000000).unwrap(), 0xb0055eed);
                    assert_eq!(
                        machine.bus.read_u32(0x20000004).unwrap(),
                        u32::from(delivered)
                    );
                    assert_eq!(
                        machine.bus.read_u32(0x20000008).unwrap(),
                        if delivered { irq } else { 0 }
                    );
                    assert_eq!(
                        machine.bus.read_u32(0x2000000c).unwrap(),
                        if error {
                            1
                        } else if delivered {
                            2
                        } else {
                            0
                        }
                    );
                    let bytes = seen.lock().unwrap().clone();
                    assert_eq!(
                        bytes,
                        if completed {
                            vec![0x31, 0x42, 0x53, 0x64]
                        } else if mode == "read-error" {
                            vec![0x5a]
                        } else if mode == "writeback-error" {
                            vec![0x31]
                        } else {
                            vec![]
                        },
                        "channel={index} mode={mode}"
                    );
                    let writeback = u64::from(WB) + index as u64 * 16;
                    let remaining = machine.bus.read_u16(writeback + 2).unwrap();
                    if completed {
                        assert_eq!(remaining, 0);
                        assert_eq!(machine.bus.read_u16(writeback).unwrap() & 1, 0);
                    } else if mode == "read-error" {
                        assert_eq!(
                            remaining, 3,
                            "failed second read must not commit another beat"
                        );
                    } else if mode == "reset" {
                        assert!((1..4).contains(&remaining));
                    } else {
                        assert_eq!(remaining, 0xaaaa);
                    }
                    let pause_before = machine.bus.read_u32(0x20000010).unwrap();
                    let pause_after = machine.bus.read_u32(0x20000014).unwrap();
                    if mode.starts_with("pause-") {
                        assert!((1..4).contains(&pause_before));
                        assert_eq!(pause_before, pause_after);
                    }
                    if matches!(mode, "reset" | "reset-restart") {
                        assert!((1..4).contains(&pause_before));
                    }
                    let flags = machine
                        .bus
                        .read_u8(u64::from(DMA + 0x40 + index as u32 * 16 + 14))
                        .unwrap();
                    assert_eq!(
                        flags,
                        if matches!(mode, "primask" | "nvic" | "inten") {
                            2
                        } else {
                            0
                        }
                    );
                    assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
                    assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
                    println!("SAM_TX_DMA legacy={legacy} interval={interval} channel={index} mode={mode} entries={entries} returns={returns} bytes={} remaining={remaining} pause_before={pause_before} pause_after={pause_after} flags={flags} exception=0", bytes.len());
                }
            }
        }
    }
}

#[test]
fn explicit_profile_config_channel_modes_and_reset_identity_are_guarded() {
    for mutation in ["missing-clock", "wrong-clock", "wrong-base", "wrong-irq"] {
        let (mut chip, manifest) = config();
        let dma = chip.peripherals.last_mut().unwrap();
        match mutation {
            "missing-clock" => dma.clock = None,
            "wrong-clock" => {
                dma.clock =
                    Some(serde_yaml::from_str("{controller: mclk, reg: AHBMASK, bit: 8}").unwrap())
            }
            "wrong-base" => dma.base_address += 0x1000,
            _ => dma.irq = Some(32),
        }
        assert!(
            SystemBus::from_config(&chip, &manifest).is_err(),
            "{mutation}"
        );
    }
    let (mut bus, _) = board(false);
    bus.write_u32(u64::from(AHB), 1 << 9).unwrap();
    let channel = u64::from(DMA + 0x40);
    for value in [
        2,
        (12 << 8) | (2 << 20),
        TX | (1 << 24),
        TX | (1 << 6),
        (13 << 8) | (3 << 20),
    ] {
        assert!(bus.write_u32(channel, value).is_err());
        assert_eq!(bus.read_u32(channel).unwrap(), 0);
    }
    // Disabled byte-wise setup works; only TRIGSRC is enable-protected.
    bus.write_u8(channel + 1, 13).unwrap();
    bus.write_u8(channel + 2, 0x20).unwrap();
    bus.write_u8(channel, 2).unwrap();
    bus.write_u8(channel + 1, 12).unwrap();
    assert_eq!(bus.read_u32(channel).unwrap(), TX | 2);
    assert!(bus.write_u8(channel + 2, 0x30).is_err());
    assert!(bus.write_u32(u64::from(DMA + 0x10), 1).is_err());
    bus.write_u8(channel, 0).unwrap();
    bus.write_u32(u64::from(DMA), 1).unwrap();
    bus.write_u32(channel, TX | 2).unwrap(); // SWRST must not revert to software profile.
    assert!(bus.write_u32(u64::from(DMA + 0x10), 1).is_err());
}

#[test]
fn unsupported_tx_descriptors_refuse_without_data_count_or_completion() {
    // HOST controls are separate from the actual CPU matrix above.
    let descriptors: [(u16, u16, u32, u32, u32); 11] = [
        (0x0408, 4, SOURCE + 4, SPI + 0x28, 0), // Invalid VALID ambiguity.
        (0x0509, 4, SOURCE + 4, SPI + 0x28, 0), // Halfword beats.
        (0x4409, 4, SOURCE + 4, SPI + 0x28, 0), // Flexible steps.
        (0x0409, 0, SOURCE, SPI + 0x28, 0),
        (0x0409, 4, SOURCE + 4, SPI + 0x28, TABLE), // Link.
        (0x0009, 4, SOURCE, SPI + 0x28, 0),         // Wrong direction/step.
        (0x0c09, 4, SOURCE + 4, SPI + 0x2c, 0),     // Increment destination.
        (0x0409, 4, SOURCE + 4, SPI + 0x29, 0),
        (0x0409, 4, SOURCE + 4, 0x53000028, 0), // Alias, not DATA.
        (0x0409, 4, SOURCE + 4, 0x41000000, 0), // Stub store isn't acceptance.
        (0x0409, 4, 0x104, SPI + 0x28, 0),      // Flash source.
    ];
    for (control, count, source, destination, next) in descriptors {
        let (mut bus, seen) = board(false);
        bus.write_u32(u64::from(AHB), 1 << 9).unwrap();
        for (address, value) in [
            (TABLE, u32::from(control) | (u32::from(count) << 16)),
            (TABLE + 4, source),
            (TABLE + 8, destination),
            (TABLE + 12, next),
            (WB, 0xaaaaaaaa),
            (WB + 4, 0xaaaaaaaa),
            (WB + 8, 0xaaaaaaaa),
            (WB + 12, 0xaaaaaaaa),
            (DMA + 0x34, TABLE),
            (DMA + 0x38, WB),
            (DMA, 0x102),
            (DMA + 0x40, TX | 2),
        ] {
            bus.write_u32(u64::from(address), value).unwrap();
        }
        for _ in 0..10 {
            bus.tick_peripherals();
        }
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(bus.read_u32(u64::from(WB)).unwrap(), 0xaaaaaaaa);
        assert!(bus.read_u8(u64::from(DMA + 0x4e)).is_err());
        let state = bus
            .peripherals
            .iter()
            .find(|p| p.name == "tx-dma")
            .unwrap()
            .dev
            .snapshot();
        assert!(state["refusal"].as_str().is_some());
        assert_eq!(state["channels"][0]["flags"], 0);
    }
}
