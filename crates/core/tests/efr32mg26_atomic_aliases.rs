// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! EFR32 Series-2 peripheral bit set/clear — the alias window every Silicon
//! Labs driver writes through.
//!
//! On Series 2 each peripheral occupies a 16 KiB block whose 4 KiB register
//! view is aliased three more times: `+0x1000` SET, `+0x2000` CLR, `+0x3000`
//! TGL (EFR32xG26 Reference Manual rev 1.0, "Peripheral Bit Set and Clear").
//! `emlib` and the Gecko SDK reach for them constantly — `CMU->CLKEN0_SET`,
//! `GPIO->P[port].DOUT_SET`, `USART1->CMD_SET` — so a stock Silicon Labs image
//! cannot bring up a single peripheral without them. Before this decode, every
//! one of those stores landed in unmapped MMIO and faulted the bus.
//!
//! The tests below drive the alias addresses the way firmware does and assert
//! the effect on the BASE register, through the ordinary bus read path.
//!
//! ⚠️ The RP2040 uses the same stride with a different order (`+0x1000` XOR,
//! `+0x2000` SET, `+0x3000` CLR). The overlap is the danger: an RP2040 SET is
//! an EFR32 CLR, so getting the family wrong turns a clock enable into a clock
//! disable and every later access to that block reads zero — a fault nobody
//! traces back to an alias. [`clr_alias_is_not_the_rp2040_set_alias`] pins the
//! difference.

use labwired_config::{AtomicAliasFlavour, ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

/// GPIOC port struct: `GPIO_S_BASE + 0x30 + 2*0x30` (`efr32mg26_gpio.h`
/// `GPIO_TypeDef.P[4]`, `efr32mg26_gpio_port.h`). LED0/LED1 are PC08/PC09 on
/// BRD2709A (UG594), which is why this port is the one worth proving.
const GPIOC: u64 = 0x4003_C090;
/// Within a port struct: MODEH @ +0x0C (pins 8..15), DOUT @ +0x10, DIN @ +0x14.
const DOUT: u64 = 0x10;
const MODEH: u64 = 0x0C;

/// The three Series-2 alias strides.
const SET: u64 = 0x1000;
const CLR: u64 = 0x2000;
const TGL: u64 = 0x3000;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// `CMU_CLKEN0` (absolute) and the GPIO bit in it. Every GPIO probe below
/// enables this first: since the CMU became a real model the ports are clock-
/// gated, so an un-clocked DOUT write lands nowhere — which is the point of
/// the gate, and not what these tests are measuring.
const CMU_CLKEN0: u64 = 0x4000_8064;
const CLKEN0_GPIO: u32 = 1 << 26;

/// A bus with the GPIO clock already on, ready for a port probe.
fn bus_with_gpio_clocked() -> SystemBus {
    let mut bus = bus_for("efr32mg26");
    bus.write_u32(CMU_CLKEN0, CLKEN0_GPIO).unwrap();
    bus
}

fn bus_for(chip_name: &str) -> SystemBus {
    let abs = root(&format!("configs/chips/{chip_name}.yaml"));
    let chip = ChipDescriptor::from_file(&abs).expect("load chip descriptor");
    let manifest = SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "efr32mg26-atomic-aliases".to_string(),
        chip: abs.to_string_lossy().to_string(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
        cpu_hz: None,
    };
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

/// The descriptor must name the FAMILY, not just "yes". A bare `true` would
/// silently mean RP2040 — see the module header.
#[test]
fn descriptor_declares_the_series2_flavour() {
    let chip = ChipDescriptor::from_file(root("configs/chips/efr32mg26.yaml")).expect("load chip");
    assert_eq!(chip.atomic_register_aliases, AtomicAliasFlavour::Efr32s2);
}

#[test]
fn set_alias_ors_bits_into_the_base_register() {
    let mut bus = bus_with_gpio_clocked();
    // Drive PC08/PC09 (LED0/LED1) push-pull so DOUT is meaningful, the same way
    // the demo firmware does — MODEH nibble 0x4 = PUSHPULL.
    bus.write_u32(GPIOC + MODEH, 0x0000_0044).unwrap();

    bus.write_u32(GPIOC + DOUT, 0).unwrap();
    // `GPIO->P[2].DOUT_SET = 1 << 8` — light LED0 without touching LED1.
    bus.write_u32(GPIOC + SET + DOUT, 1 << 8).unwrap();
    assert_eq!(bus.read_u32(GPIOC + DOUT).unwrap(), 1 << 8);

    // A second SET is additive, not a store: LED1 joins LED0.
    bus.write_u32(GPIOC + SET + DOUT, 1 << 9).unwrap();
    assert_eq!(bus.read_u32(GPIOC + DOUT).unwrap(), (1 << 8) | (1 << 9));

    // Zero bits in a SET write are untouched, not cleared.
    bus.write_u32(GPIOC + SET + DOUT, 0).unwrap();
    assert_eq!(bus.read_u32(GPIOC + DOUT).unwrap(), (1 << 8) | (1 << 9));
}

#[test]
fn clr_alias_and_nots_bits_out_of_the_base_register() {
    let mut bus = bus_with_gpio_clocked();
    bus.write_u32(GPIOC + MODEH, 0x0000_0044).unwrap();
    bus.write_u32(GPIOC + DOUT, (1 << 8) | (1 << 9)).unwrap();

    // `GPIO->P[2].DOUT_CLR = 1 << 8` — LED0 off, LED1 untouched.
    bus.write_u32(GPIOC + CLR + DOUT, 1 << 8).unwrap();
    assert_eq!(bus.read_u32(GPIOC + DOUT).unwrap(), 1 << 9);

    // Clearing a bit that is already clear is a no-op, not a fault.
    bus.write_u32(GPIOC + CLR + DOUT, 1 << 8).unwrap();
    assert_eq!(bus.read_u32(GPIOC + DOUT).unwrap(), 1 << 9);
}

#[test]
fn tgl_alias_xors_bits_in_the_base_register() {
    let mut bus = bus_with_gpio_clocked();
    bus.write_u32(GPIOC + MODEH, 0x0000_0044).unwrap();
    bus.write_u32(GPIOC + DOUT, 1 << 8).unwrap();

    // The blink idiom: `DOUT_TGL = LED0 | LED1` flips both, whatever they were.
    bus.write_u32(GPIOC + TGL + DOUT, (1 << 8) | (1 << 9))
        .unwrap();
    assert_eq!(bus.read_u32(GPIOC + DOUT).unwrap(), 1 << 9);

    bus.write_u32(GPIOC + TGL + DOUT, (1 << 8) | (1 << 9))
        .unwrap();
    assert_eq!(bus.read_u32(GPIOC + DOUT).unwrap(), 1 << 8);
}

/// Reading an alias returns the register, not zero and not a fault. Drivers do
/// this by accident constantly — a read-modify-write macro expanded on the
/// `_SET` pointer, or a debugger walking the block.
#[test]
fn every_alias_reads_back_the_base_register() {
    let mut bus = bus_with_gpio_clocked();
    bus.write_u32(GPIOC + MODEH, 0x0000_0044).unwrap();
    bus.write_u32(GPIOC + DOUT, 1 << 9).unwrap();

    for alias in [SET, CLR, TGL] {
        assert_eq!(
            bus.read_u32(GPIOC + alias + DOUT).unwrap(),
            1 << 9,
            "alias +{alias:#x} must read the base register"
        );
    }
}

/// The whole point of naming the family. `+0x2000` is SET on an RP2040 and CLR
/// here; if the flavour were ever "unified" onto the RP2040 order, this test is
/// the one that fires, and it fires on the CLR/SET confusion specifically
/// rather than on some downstream driver hanging.
#[test]
fn clr_alias_is_not_the_rp2040_set_alias() {
    let mut bus = bus_with_gpio_clocked();
    bus.write_u32(GPIOC + MODEH, 0x0000_0044).unwrap();
    bus.write_u32(GPIOC + DOUT, 0).unwrap();

    bus.write_u32(GPIOC + 0x2000 + DOUT, 1 << 8).unwrap();
    assert_eq!(
        bus.read_u32(GPIOC + DOUT).unwrap(),
        0,
        "+0x2000 is CLR on Series 2; reading {:#x} back means the RP2040 order \
         (SET) is in force and every emlib clock-enable is inverted",
        1u32 << 8
    );
}

/// The alias decode must not reach a chip that does not implement it: on an
/// STM32 those addresses are ordinary (mostly unmapped) MMIO, and folding them
/// onto a real register would invent a write the silicon never performs.
#[test]
fn a_chip_without_the_feature_does_not_fold_aliases() {
    let chip =
        ChipDescriptor::from_file(root("configs/chips/stm32f103.yaml")).expect("load stm32f103");
    assert_eq!(chip.atomic_register_aliases, AtomicAliasFlavour::None);
    assert!(
        chip.atomic_register_aliases.op_for_index(1).is_none(),
        "a chip with no alias flavour must decode no alias index"
    );
}

/// Both orders are pinned here in one place, so the table is readable without
/// two reference manuals open.
#[test]
fn the_two_families_disagree_and_the_table_says_how() {
    use labwired_core::bus::AtomicAliasOp;

    // RP2040: +0x1000 XOR, +0x2000 SET, +0x3000 CLR (pico-sdk address_mapped.h).
    assert_eq!(
        AtomicAliasFlavour::Rp2040.op_for_index(1),
        Some(AtomicAliasOp::Xor)
    );
    assert_eq!(
        AtomicAliasFlavour::Rp2040.op_for_index(2),
        Some(AtomicAliasOp::Set)
    );
    assert_eq!(
        AtomicAliasFlavour::Rp2040.op_for_index(3),
        Some(AtomicAliasOp::Clr)
    );

    // EFR32 Series 2: +0x1000 SET, +0x2000 CLR, +0x3000 TGL (an XOR).
    assert_eq!(
        AtomicAliasFlavour::Efr32s2.op_for_index(1),
        Some(AtomicAliasOp::Set)
    );
    assert_eq!(
        AtomicAliasFlavour::Efr32s2.op_for_index(2),
        Some(AtomicAliasOp::Clr)
    );
    assert_eq!(
        AtomicAliasFlavour::Efr32s2.op_for_index(3),
        Some(AtomicAliasOp::Xor)
    );

    // Index 0 is the register itself on both, and never an alias op.
    for flavour in [AtomicAliasFlavour::Rp2040, AtomicAliasFlavour::Efr32s2] {
        assert_eq!(flavour.op_for_index(0), None);
    }
}
