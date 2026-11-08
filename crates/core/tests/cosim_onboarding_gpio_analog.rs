// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA devices drive a GPIO input on every imported onboarding
//! profile (`configs/systems/onboarding/*.yaml`) that models one.
//!
//! The ADC sweep in `cosim_onboarding_analog.rs` reaches only the 12 profiles
//! whose converter LabWired can run. This is the other endpoint the coverage
//! inventory accepts, a voltage-thresholded digital input, and it is just as
//! board-agnostic: the port is the first peripheral each built bus reports as
//! a GPIO input, the same Beta NTC divider and Shockley diode load line are
//! routed to the port's first two input pads (`board.gpio_in.<port>.0` / `.1`
//! on most ports; the LiteX CAS block's inputs start at pad 32), the solved
//! voltages are checked
//! against independent references, and the levels are read back twice. Once
//! through `Peripheral::read_gpio_input`, and once with a firmware load from
//! the port's input register.
//!
//! The register is where it gets honest. Most Renode-imported GPIO types have
//! no dedicated LabWired model and run on the STM32F1 placeholder map
//! (`SystemBus::gpio_layout_for`, case 3b), so their input reads back at the
//! placeholder IDR (+0x08). That is NOT the silicon register for that IP, and
//! [`PLACEHOLDER_TYPES`] says so. Only [`SILICON_INPUT_REGISTER`] types are
//! claimed at the silicon offset. GPIO thresholds are simulation assumptions
//! wherever the descriptor declares none, and this is not ADC support.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, GpioInputThresholds, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::{Path, PathBuf};

/// The port's first two input pads, in pad order: the NTC divider drives the
/// first and the diode the second. Most ports start at pin 0; the LiteX CAS
/// block puts LEDs on 0..31 and its first input, switch 0, on pad 32.
#[derive(Clone, Copy)]
struct Pins {
    ntc: u8,
    diode: u8,
}

/// GPIO types whose LabWired model serves the silicon register map, with the
/// input register's offset in that map and its source. The vendor-neutral
/// `gpio` type is keyed by its layout, as `gpio/<config.profile>`.
const SILICON_INPUT_REGISTER: &[(&str, u64, &str)] = &[
    ("stm32f1gpioport", 0x08, "RM0008 §9.2.3 GPIOx_IDR"),
    // STM32W108 (EM357) GPIO_PxIN sits at the F1 IDR offset.
    (
        "gpio/stm32f1",
        0x08,
        "RM0008 §9.2.3 GPIOx_IDR; RM0298 GPIO_PxIN (em357 regs.h GPIO_PAIN)",
    ),
    ("gpio/stm32v2", 0x10, "RM0368 §8.4.5 GPIOx_IDR"),
    // i.MX RT700 RGPIO ports are windowed at port base + 0x40, so PDIR @0x50
    // lands at +0x10 of the window.
    (
        "gpio/kinetis",
        0x10,
        "S32K1xx RM rev 14 §12.3 GPIOx_PDIR; RT700 PERI_GPIO.h PDIR @0x50; RV32M1_ri5cy.h PDIR",
    ),
    ("gpio/sam_port", 0x20, "SAM D5x/E5x DS60001507 §32.7 IN"),
    ("imxrt_gpio", 0x08, "IMXRT1060RM §12.5 GPIO_PSR"),
    (
        "lpc_gpio",
        0x2100,
        "MIMXRT595S_cm33.h GPIO_Type PIN[0] (UM11147)",
    ),
    // A word load at PGPDI1 (0x1740) carries PGPDI0 in its upper halfword;
    // see [`input_bit`].
    ("s32k3_siul2", 0x1740, "S32K388_SIUL2.h PGPDI1/PGPDI0"),
    // GPIODATA is address-masked by PADDR[9:2]; at 0x3FC every bit reads.
    (
        "cc2538_pl061_gpio",
        0x3FC,
        "PL061 TRM DDI0190B §3.3.1 GPIODATA (SWRU319C Table 9-2)",
    ),
    ("nrf52840_gpio", 0x510, "nRF52840 PS §6.9.2 IN"),
    // A declarative port is keyed by its descriptor; its `IN` register is
    // the one the declarative GPIO path stores the routed level in.
    ("declarative/lpc2000/gpio.yaml", 0x00, "UM10114 GPIO IOnPIN"),
    (
        "declarative/tegra/gpio.yaml",
        0x30,
        "Linux gpio-tegra.c GPIO_IN (GPIO_REG + 0x30)",
    ),
    // Byte and halfword ports; see [`load_input`] for the load width.
    (
        "declarative/msp430/port8.yaml",
        0x00,
        "SLAU049F §9.2 / SLAU144J §8.2 PxIN",
    ),
    (
        "declarative/msp430/port16.yaml",
        0x00,
        "SLAU208Q §12.4 PAIN",
    ),
    // sunxi Pn_DAT serves both directions; it is the IN register here.
    (
        "declarative/sunxi/pio.yaml",
        0x10,
        "Linux pinctrl-sunxi.h DATA_REGS_OFFSET (Pn_DAT)",
    ),
    // LiteX GPIOIn: the one `in` CSR at the core base.
    (
        "declarative/litex/gpio_in.yaml",
        0x00,
        "litex/soc/cores/gpio.py GPIOIn `in` (Zephyr gpio_litex.c)",
    ),
    // EOS S3 MISC IO_INPUT, GPIO k at bit k.
    (
        "declarative/eoss3/io_in.yaml",
        0x00,
        "Renode EOSS3_IntrCtrl.cs MiscRegisters.IOInput (misc region)",
    ),
    // JH7100 GPIODIN for GPIO 0..31, inside the pinctrl "gpio" region.
    (
        "declarative/jh7100/gpio_din.yaml",
        0x48,
        "Linux pinctrl-starfive-jh7100.c GPIODIN",
    ),
    // V2M motherboard sysreg SYS_MCI: MMC card detect (bit 0), write protect (bit 1).
    (
        "declarative/vexpress/sysreg_mci.yaml",
        0x48,
        "Linux vexpress-sysreg.c SYS_MCI (sys_mci basic-mmio-gpio dat)",
    ),
    // Window based at the port's P_m byte, so PIN_m is +0x800.
    (
        "declarative/rzg2l/port.yaml",
        0x800,
        "Linux pinctrl-rzg2l.c PIN(off) = 0x0800 + off",
    ),
    // Window based at port 0's P_m byte of the non-safety region.
    (
        "declarative/rzt2m/port.yaml",
        0x800,
        "Zephyr gpio_rzt2m.c PINm_OFFSET + m",
    ),
    (
        "sam4s_pio",
        0x3C,
        "SAM4S DS §31.7 PIO_PDSR; AT91RM9200 DS 1768 PIO_PDSR",
    ),
    ("miv_coregpio", 0x90, "CoreGPIO handbook InputRegister"),
    // Switch pads 32..63 read at Switches; see [`input_bit`].
    (
        "litex_controlandstatus",
        0x04,
        "Renode LiteX_ControlAndStatus Switches (arty_litex_vexriscv.repl)",
    ),
    (
        "efr32s2_gpioport",
        0x14,
        "efr32mg24_gpio_port.h / efr32bg22_gpio_port.h GPIO_P_DIN",
    ),
    // The AVR CPU answers an IN/LDS of PINx with this mirror's input bits.
    (
        "avr_gpio",
        0x00,
        "PINx (DS8271 §14.4, DS2549 §13.4), via the CPU IO mirror",
    ),
];

/// Bit of the input-register word that carries `pin`. Every map here puts
/// pin k at bit k, except SIUL2: `PGPDI0` is the halfword at 0x1742 and holds
/// pin k at bit `15 - k` (Zephyr `gpio_nxp_siul2.c` reverses it with
/// `__RBIT`), so in the word at 0x1740 pin k is bit `31 - k`. The CAS
/// Switches CSR holds switch pad 32 + k at bit k.
fn input_bit(kind: &str, pin: u8) -> u32 {
    match kind {
        "s32k3_siul2" => 1 << (31 - u32::from(pin)),
        "litex_controlandstatus" => {
            assert!((32..64).contains(&pin), "CAS switch pad, got {pin}");
            1 << (pin - 32)
        }
        _ => 1 << pin,
    }
}

/// Firmware load of the input register at the width its firmware uses. The
/// MSP430 ports are byte (`mov.b &P1IN`) and halfword (`mov &PAIN`)
/// registers, and the byte after P1IN is P1OUT, so a word load would read
/// past the port. RZ/G2L and RZ/T2M `PIN_m` are bytes (`readb` in
/// pinctrl-rzg2l.c, a `uint8_t *` in gpio_rzt2m.c) and the next byte is the
/// next port's. Every other map here is word-wide.
fn load_input(bus: &SystemBus, kind: &str, register: u64) -> u32 {
    match kind {
        "declarative/msp430/port8.yaml"
        | "declarative/rzg2l/port.yaml"
        | "declarative/rzt2m/port.yaml" => bus.read_u8(register).map(u32::from),
        "declarative/msp430/port16.yaml" => bus.read_u16(register).map(u32::from),
        _ => bus.read_u32(register),
    }
    .expect("load the port input register")
}

/// Placeholder IDR offset: GPIO types LabWired does not model yet run on the
/// STM32F1 map, so the level is firmware-visible here and nowhere else.
const PLACEHOLDER_INPUT_OFFSET: u64 = 0x08;

/// Imported GPIO types running on that placeholder. A type leaving this list
/// for a real model must move to [`SILICON_INPUT_REGISTER`].
const PLACEHOLDER_TYPES: &[&str] = &[
    "ambiqapollo4_gpio",
    "andesatcgpio100",
    "arm_ahb_gpio",
    "cc2538_gpio",
    "designware_apb_gpio",
    "efmgpioport",
    "efr32_gpioport",
    "gaisler_gpio",
    "max32650_gpio",
    "mpfs_gpio",
    "murax_gpio",
    "npcx_gpio",
    "nxp_imx_gpio",
    "opentitan_gpio",
    "pulp_apb_gpio",
    "quark_gpiocontroller",
    "renesasda14_gpio",
    "renesasra_gpiomisc",
    "samd21_gpio",
    "sifive_gpio",
    "stm32_gpioport",
    "xilinxgpiops",
];

/// Profiles whose port takes the routed level (`read_gpio_input` follows it)
/// but where a firmware load from the port's input register does not see it.
/// Fixing one has to move it out of this list.
///
/// Empty: the i.MX 8M Plus profiles left it once their flash boot alias,
/// which shadowed GPIO1 with erased flash, was turned off.
const INPUT_REGISTER_UNREACHED: &[(&str, &str)] = &[];

/// Absolute input-register address of the covered port, from the vendor
/// header. [`input_register`] derives the address from the descriptor's own
/// base, so it cannot catch a base that is wrong for the silicon; these
/// entries pin it. Each board listed must be covered.
const SILICON_INPUT_ADDRESS: &[(&str, &str, u64, &str)] = &[
    (
        "mimxrt798s",
        "gpio0",
        0x4010_0050,
        "MIMXRT798S_cm33_core0_COMMON.h GPIO0_BASE_NS + PERI_GPIO.h PDIR",
    ),
    (
        "mimxrt700_evk",
        "gpio0",
        0x4010_0050,
        "MIMXRT798S_cm33_core0_COMMON.h GPIO0_BASE_NS + PERI_GPIO.h PDIR",
    ),
    (
        "at91rm9200",
        "pioA",
        0xFFFF_F43C,
        "AT91RM9200 DS 1768 Table 8-1 PIOA + PIO_PDSR",
    ),
    ("vybrid", "gpio0", 0x400F_F010, "vf610.dtsi gpio0 + PDIR"),
    (
        "colibri-vf61",
        "gpio0",
        0x400F_F010,
        "vf610.dtsi gpio0 + PDIR",
    ),
    ("stm32w108", "gpioa", 0x4000_B008, "em357 regs.h GPIO_PAIN"),
    ("lpc2294", "gpio0", 0xE002_8000, "UM10114 IO0PIN"),
    (
        "arty_litex_vexriscv",
        "cas",
        0x6000_6804,
        "Renode arty_litex_vexriscv.repl cas @ sysbus 0x60006800 + Switches",
    ),
    (
        "litex_tock",
        "cas",
        0x6000_6804,
        "Renode litex_tock.repl cas @ sysbus 0x60006800 + Switches",
    ),
    (
        "ri5cy",
        "gpioa",
        0x4802_0010,
        "RV32M1_ri5cy.h GPIOA_BASE + GPIO_Type PDIR",
    ),
    (
        "vegaboard_ri5cy",
        "gpioa",
        0x4802_0010,
        "RV32M1_ri5cy.h GPIOA_BASE + GPIO_Type PDIR",
    ),
    (
        "tegra2",
        "gpioA",
        0x6000_D030,
        "tegra20.dtsi gpio@6000d000 + gpio-tegra.c GPIO_IN",
    ),
    (
        "tegra3",
        "gpioA",
        0x6000_D030,
        "tegra30.dtsi gpio@6000d000 + gpio-tegra.c GPIO_IN",
    ),
    ("mspsim-msp430f149", "p1", 0x20, "SLAS272 P1IN"),
    ("mspsim-msp430f1611", "p1", 0x20, "SLAS368 P1IN"),
    ("mspsim-msp430f2617", "p1", 0x20, "SLAS541 P1IN"),
    ("msp430f2619", "p1", 0x20, "SLAS541 P1IN"),
    ("mspsim-msp430f5437", "pa", 0x200, "SLAS612 PAIN (P1IN)"),
    (
        "a20",
        "pioA",
        0x01C2_0810,
        "sun7i-a20.dtsi pinctrl@1c20800 + pinctrl-sunxi.h PA_DAT",
    ),
    (
        "renesas_rz_g2l",
        "p0",
        0x1103_0810,
        "r9a07g044.dtsi pinctrl@11030000 + pinctrl-rzg2l.c PIN(0x10)",
    ),
    (
        "renesas_rz_t2m",
        "p0",
        0x800A_0800,
        "r9a07g075.dtsi gpio@800a0000 base_ns + gpio_rzt2m.c PINm_OFFSET",
    ),
    (
        "renesas-rz_t2m_rsk",
        "p0",
        0x800A_0800,
        "r9a07g075.dtsi gpio@800a0000 base_ns + gpio_rzt2m.c PINm_OFFSET",
    ),
    (
        "eos-s3",
        "io_in",
        0x4000_5100,
        "Renode platforms/cpus/eos-s3.repl intrCtrl misc region @0x40005100",
    ),
    (
        "eos-s3-qomu",
        "io_in",
        0x4000_5100,
        "Renode platforms/cpus/eos-s3.repl intrCtrl misc region @0x40005100",
    ),
    (
        "eos-s3-quickfeather",
        "io_in",
        0x4000_5100,
        "Renode platforms/cpus/eos-s3.repl intrCtrl misc region @0x40005100",
    ),
    (
        "arvsom",
        "gpio",
        0x1191_0048,
        "Linux jh7100.dtsi pinctrl@11910000 + pinctrl-starfive-jh7100.c GPIODIN",
    ),
    (
        "beaglev_starlight",
        "gpio",
        0x1191_0048,
        "Linux jh7100.dtsi pinctrl@11910000 + pinctrl-starfive-jh7100.c GPIODIN",
    ),
    (
        "starfive-jh7100",
        "gpio",
        0x1191_0048,
        "Linux jh7100.dtsi pinctrl@11910000 + pinctrl-starfive-jh7100.c GPIODIN",
    ),
    (
        "vexpress",
        "sysCtl",
        0x1000_0048,
        "Linux vexpress-v2m.dtsi sysreg@0 (CS7 0x1000_0000) + vexpress-sysreg.c SYS_MCI",
    ),
    (
        "litex_vexriscv",
        "gpio_in",
        0xE000_6000,
        "Zephyr enjoydigital litex_vexriscv.dts gpio_in: gpio@e0006000",
    ),
    (
        "litex_vexriscv_micropython",
        "gpio_in",
        0xE000_6000,
        "Zephyr enjoydigital litex_vexriscv.dts gpio_in: gpio@e0006000",
    ),
    (
        "litex_vexriscv_zephyr",
        "gpio_in",
        0xE000_6000,
        "Zephyr enjoydigital litex_vexriscv.dts gpio_in: gpio@e0006000 (ram0 @0x4000_0000)",
    ),
    (
        "litex_zephyr_vexriscv_i2s",
        "gpio_in",
        0xE000_6000,
        "Zephyr enjoydigital litex_vexriscv.dts gpio_in: gpio@e0006000 (ram0 @0x4000_0000)",
    ),
];

/// Profiles covered here: a port takes the routed level.
const COVERED: usize = 206;
/// Covered profiles read back at a [`SILICON_INPUT_REGISTER`] offset.
const SILICON_REGISTER: usize = 101;
/// Profiles whose descriptor declares no GPIO input. Three of them
/// (`stm32h7`, `stm32h747`, `stm32h747-disco`) are still covered through their
/// ADCs in `cosim_onboarding_analog.rs`; nothing analog reaches the rest yet.
const NO_ANALOG_ENDPOINT: usize = 39;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

/// Independent Shockley load line, including the diode's series resistance.
fn diode_load_line_voltage() -> f64 {
    const SUPPLY_V: f64 = 3.3;
    const LIMIT_OHMS: f64 = 1_000.0;
    const DIODE_IS_A: f64 = 2.52e-9;
    const DIODE_N: f64 = 1.752;
    const DIODE_RS_OHMS: f64 = 0.568;
    const THERMAL_V: f64 = 1.380_649e-23 * 298.15 / 1.602_176_634e-19;
    let current_at_junction_voltage =
        |junction_v: f64| DIODE_IS_A * ((junction_v / (DIODE_N * THERMAL_V)).exp() - 1.0);
    let residual = |junction_v: f64| {
        let current = current_at_junction_voltage(junction_v);
        (SUPPLY_V - junction_v - current * DIODE_RS_OHMS) / LIMIT_OHMS - current
    };
    let (mut low, mut high) = (0.0, 1.2);
    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        if residual(mid) > 0.0 {
            low = mid;
        } else {
            high = mid;
        }
    }
    let junction_v = (low + high) / 2.0;
    junction_v + current_at_junction_voltage(junction_v) * DIODE_RS_OHMS
}

fn signal_voltage(session: &CosimSession, path: &str) -> f64 {
    match session.signals().get(path) {
        Some(CosimSignalValue::F64(volts)) => *volts,
        other => panic!("expected solved voltage at {path}, got {other:?}"),
    }
}

fn onboarding_profiles() -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(root("configs/systems/onboarding"))
        .expect("read onboarding systems")
        .map(|entry| entry.expect("onboarding entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    paths.sort();
    paths
}

fn generic_netlists(port: &str, pins: Pins) -> SystemManifest {
    let mut analog = SystemManifest::from_file(root(
        "examples/ci-fixture-unknown-arch/analog-ntc-diode-system.yaml",
    ))
    .expect("load generic NTC divider and diode netlists");
    for model in &mut analog.cosim_models {
        let (key, pin) = match model.id.as_str() {
            "ntc_divider" => ("v_sense", pins.ntc),
            "diode_load_line" => ("v_diode", pins.diode),
            other => panic!("unexpected fixture model {other}"),
        };
        model
            .outputs
            .insert(key.into(), format!("board.gpio_in.{port}.{pin}"));
    }
    analog
}

/// The first peripheral, in bus order, that reports a GPIO input on two pads,
/// and its first two input pads.
fn first_input_port(bus: &SystemBus) -> Option<(usize, Pins)> {
    bus.peripherals.iter().enumerate().find_map(|(index, p)| {
        let mut inputs = (0..=u8::MAX).filter(|&pin| p.dev.read_gpio_input(pin).is_some());
        let (ntc, diode) = (inputs.next()?, inputs.next()?);
        Some((index, Pins { ntc, diode }))
    })
}

/// Profiles whose port reports input pads the imported board does not wire.
/// They are counted with [`NO_ANALOG_ENDPOINT`], not covered.
const UNWIRED_INPUT_PADS: &[(&str, &str)] = &[(
    "crosslink-nx-evn",
    "Renode crosslink-nx-evn.repl: only LEDs are supported on this LiteX platform's CAS",
)];

/// Absolute address of the port's input register, from its declared type,
/// whether it is the silicon register, and the type key for [`input_bit`].
fn input_register(chip: &ChipDescriptor, port: &str, base: u64) -> (u64, bool, String) {
    let cfg = chip
        .peripherals
        .iter()
        .find(|p| p.id == port)
        .expect("port comes from the descriptor");
    let mut kind = cfg.r#type.to_ascii_lowercase();
    if kind == "gpio" {
        let profile = cfg.config.get("profile").and_then(|v| v.as_str());
        kind = format!("gpio/{}", profile.expect("a `gpio` port names its layout"));
    }
    if kind == "declarative" {
        let path = cfg.config.get("path").and_then(|v| v.as_str());
        let path = path.expect("a declarative port names its descriptor");
        let key = path.rsplit_once("peripherals/").map_or(path, |(_, k)| k);
        kind = format!("declarative/{key}");
    }
    // A window based part-way into its block (Nordic ports at OUT) says so.
    let window = cfg
        .config
        .get("reg_offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if let Some((_, offset, _)) = SILICON_INPUT_REGISTER.iter().find(|(t, ..)| *t == kind) {
        return (base - window + offset, true, kind);
    }
    assert!(
        PLACEHOLDER_TYPES.contains(&kind.as_str()),
        "{}: GPIO type {kind} is unclassified; add it to SILICON_INPUT_REGISTER or PLACEHOLDER_TYPES",
        chip.name
    );
    (base + PLACEHOLDER_INPUT_OFFSET, false, kind)
}

/// Cortex-M3/M4 peripheral bit-band alias window (ARMv7-M ARM §B3.1).
const BIT_BAND_ALIAS: std::ops::Range<u64> = 0x4200_0000..0x4400_0000;

/// A part that decodes real peripherals in the alias window cannot also
/// bit-band it: the bus would translate their registers onto 0x4000_xxxx.
/// The Cortex-M33 (STM32L552, STM32WBA52, RAMN), Cortex-M7 (i.MX RT1064 fast
/// GPIO6..9) and Cortex-M55 (Alif Ensemble LPGPIO) have no bit-band, so their
/// descriptors must name the core. Renode's own `bitbanding` entry is the
/// alias itself and is the one type allowed there.
fn assert_no_peripheral_behind_bit_band(board: &str, chip: &ChipDescriptor, bus: &SystemBus) {
    if !bus.bit_band_enabled {
        return;
    }
    for p in &chip.peripherals {
        if p.r#type.eq_ignore_ascii_case("bitbanding") {
            continue;
        }
        let base = p.base_address;
        assert!(
            !BIT_BAND_ALIAS.contains(&base),
            "{board}: {} ({}) at {base:#x} sits in the bit-band alias window; declare the core",
            p.id,
            p.r#type
        );
    }
}

/// What firmware writes before the pins read as inputs.
fn bring_up_inputs(bus: &mut SystemBus, chip: &ChipDescriptor, port: &str, base: u64, pins: Pins) {
    let kind = chip
        .peripherals
        .iter()
        .find(|p| p.id == port)
        .map(|p| p.r#type.to_ascii_lowercase());
    if kind.as_deref() == Some("miv_coregpio") {
        // CoreGPIO masks InputRegister bits whose CONFIG_X.INREG is clear.
        for pin in [pins.ntc, pins.diode] {
            bus.write_u32(base + 4 * u64::from(pin), 1 << 1)
                .expect("CoreGPIO CONFIG_X.INREG");
        }
    }
    if kind.as_deref() == Some("efr32s2_gpioport") {
        // Series 2 resets every pin DISABLED; emlib's GPIO_PinModeSet writes
        // the pin's MODEL nibble to INPUT (_GPIO_P_MODEL_MODE0_INPUT = 1).
        let model = (1 << (4 * pins.ntc)) | (1 << (4 * pins.diode));
        bus.write_u32(base + 0x04, model)
            .expect("Series-2 GPIO_P_MODEL INPUT");
    }
    if kind.as_deref() == Some("s32k3_siul2") {
        // RTD Siul2_Port_Init writes MSCR[n] with SSS = 0 (GPIO) and IBE
        // (bit 19); GPDI/PGPDI read 0 for a pad whose input buffer is off.
        for pin in [pins.ntc, pins.diode] {
            bus.write_u32(base + 0x240 + 4 * u64::from(pin), 1 << 19)
                .expect("SIUL2 MSCR.IBE");
        }
    }
    set_gpio_mode_fields_to_input(bus, port, base, pins);
    if kind.as_deref() == Some("gpio") && profile_of(chip, port) == Some("sam_port") {
        // ASF/Arduino pinMode(INPUT) sets PINCFGn.INEN (bit 1); PINCFG is
        // one byte per pin from +0x40, so pins 0 and 1 share the first word.
        let pincfg = (0x02 << (8 * pins.ntc)) | (0x02 << (8 * pins.diode));
        bus.write_u32(base + 0x40, pincfg)
            .expect("SAM PORT PINCFG.INEN");
    }
}

/// A port whose direction is a per-pin mode field (`gpio_mode`) gets each
/// pin's field set to the descriptor's first `input` value, as a driver does
/// before it reads the pad: RZ/G2L rzg2l_gpio_direction_input and RZ/T2M
/// rzt2m_gpio_configure write PM 01, a sunxi Pn_CFG nibble is already 0
/// (input) at reset. Read-modify-write over the field's bytes, so pins that
/// share a register keep each other's fields; nothing here names a chip.
fn set_gpio_mode_fields_to_input(bus: &mut SystemBus, port: &str, base: u64, pins: Pins) {
    let Some(descriptor) = bus
        .peripherals
        .iter()
        .find(|p| p.name == port)
        .and_then(|p| p.dev.peripheral_descriptor())
    else {
        return;
    };
    let Some(mode) = descriptor.gpio_mode else {
        return;
    };
    let register = descriptor
        .registers
        .iter()
        .find(|r| r.id == mode.register)
        .expect("gpio_mode names a declared register");
    let width = u32::from(mode.bits_per_pin);
    let input = *mode.input.first().expect("gpio_mode lists an input value");
    for pin in [pins.ntc, pins.diode] {
        let first_bit = u32::from(pin) * width;
        let address = base + register.address_offset + u64::from(first_bit / 8);
        let shift = first_bit % 8;
        let mask = ((1u32 << width) - 1) << shift;
        // A field of at most 8 bits spans at most two bytes.
        let old = u32::from(bus.read_u8(address).unwrap())
            | u32::from(bus.read_u8(address + 1).unwrap()) << 8;
        let new = (old & !mask) | (input << shift);
        bus.write_u8(address, new as u8)
            .expect("mode field low byte");
        bus.write_u8(address + 1, (new >> 8) as u8)
            .expect("mode field high byte");
    }
}

fn profile_of<'a>(chip: &'a ChipDescriptor, port: &str) -> Option<&'a str> {
    chip.peripherals
        .iter()
        .find(|p| p.id == port)
        .and_then(|p| p.config.get("profile"))
        .and_then(|v| v.as_str())
}

struct Outcome {
    silicon_register: bool,
}

fn assert_port_routes_generic_analog(
    board: &str,
    chip: &ChipDescriptor,
    base_profile: &SystemManifest,
    port_index: usize,
    pins: Pins,
    probe: &SystemBus,
) -> Outcome {
    let port = probe.peripherals[port_index].name.clone();
    let base = probe.peripherals[port_index].base;
    let tag = format!("{board}:{port}");
    let mut profile = base_profile.clone();
    profile.cosim_models = generic_netlists(&port, pins).cosim_models;
    let mut bus = SystemBus::from_config(chip, &profile).expect("build onboarding bus");
    bring_up_inputs(&mut bus, chip, &port, base, pins);
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("profile carries both generic devices");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{tag}: both netlists bind to pins {}/{}",
        pins.ntc,
        pins.diode
    );

    let (register, silicon_register, kind) = input_register(chip, &port, base);
    if let Some((_, p, address, source)) = SILICON_INPUT_ADDRESS.iter().find(|(b, ..)| *b == board)
    {
        assert_eq!(
            (port.as_str(), register),
            (*p, *address),
            "{tag}: input register is not at the silicon address ({source})"
        );
    }
    let (ntc_bit, diode_bit) = (input_bit(&kind, pins.ntc), input_bit(&kind, pins.diode));
    let unreached = INPUT_REGISTER_UNREACHED.iter().any(|(b, _)| *b == board);
    let io_v = chip.io_voltage_v.expect("io voltage set before the bus");
    let expected_diode_v = diode_load_line_voltage();
    assert!(
        expected_diode_v < 0.3 * io_v,
        "{tag}: the diode load line sits below VIL"
    );
    let ntc_path = format!("board.gpio_in.{port}.{}", pins.ntc);
    let diode_path = format!("board.gpio_in.{port}.{}", pins.diode);
    let mut register_tracked = true;
    let mut cycles = 0;
    for (temperature_c, expected_ntc_level) in [
        (-10.0_f64, true),
        (25.0, true), // inside the band: holds high
        (80.0, false),
        (25.0, false), // inside the band: holds low
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .expect("set NTC temperature");
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session
            .advance_to(cycles, &mut bus)
            .expect("solve and route both netlists");
        assert_eq!(steps.len(), 2, "{tag}: both netlists solve each step");
        assert!(errors.is_empty(), "{tag}: route errors: {errors:?}");

        let ntc_v = signal_voltage(&session, &ntc_path);
        let r = ntc_resistance_ohms(temperature_c);
        let expected_ntc_v = 3.3 * r / (10_000.0 + r);
        assert!(
            (ntc_v - expected_ntc_v).abs() < 1e-9,
            "{tag}: at {temperature_c} °C the Beta divider should be {expected_ntc_v} V, got {ntc_v} V"
        );
        let diode_v = signal_voltage(&session, &diode_path);
        assert!(
            (diode_v - expected_diode_v).abs() < 2e-5,
            "{tag}: Shockley load line should be {expected_diode_v} V, got {diode_v} V"
        );

        let dev = &bus.peripherals[port_index].dev;
        assert_eq!(
            dev.read_gpio_input(pins.ntc),
            Some(expected_ntc_level),
            "{tag}: NTC pin follows the solved voltage with Schmitt hysteresis at {temperature_c} °C"
        );
        assert_eq!(
            dev.read_gpio_input(pins.diode),
            Some(false),
            "{tag}: diode pin reads low below VIL"
        );

        let word = load_input(&bus, &kind, register);
        register_tracked &= (word & ntc_bit != 0) == expected_ntc_level && word & diode_bit == 0;
    }
    if unreached {
        assert!(
            !register_tracked,
            "{tag}: the input register at {register:#x} now tracks the pins; remove it from INPUT_REGISTER_UNREACHED"
        );
    } else {
        assert!(
            register_tracked,
            "{tag}: firmware load from {register:#x} must track both pins"
        );
    }
    Outcome {
        silicon_register: silicon_register && !unreached,
    }
}

#[test]
fn every_onboarding_profile_with_a_gpio_input_takes_routed_ntc_and_diode_levels() {
    let mut covered = Vec::new();
    let mut silicon = 0;
    let mut no_endpoint = 0;
    for system_path in onboarding_profiles() {
        let board = system_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut profile =
            SystemManifest::from_file(&system_path).unwrap_or_else(|e| panic!("load {board}: {e}"));
        // Anchor the chip path at the system file, as the CLI does, so a
        // declarative port's descriptor `path` resolves next to the chip.
        let chip_path = system_path.parent().unwrap().join(&profile.chip);
        profile.chip = chip_path.to_string_lossy().into_owned();
        let mut chip = ChipDescriptor::from_file(&chip_path)
            .unwrap_or_else(|e| panic!("load {board} descriptor: {e}"));
        // Simulation assumptions where the import records nothing; no
        // silicon threshold is claimed.
        chip.io_voltage_v.get_or_insert(3.3);
        chip.gpio_input_thresholds
            .get_or_insert(GpioInputThresholds { vil: 0.3, vih: 0.7 });
        let probe = SystemBus::from_config(&chip, &profile)
            .unwrap_or_else(|e| panic!("build {board} bus: {e}"));
        assert_no_peripheral_behind_bit_band(&board, &chip, &probe);
        let Some((port_index, pins)) = first_input_port(&probe) else {
            no_endpoint += 1;
            continue;
        };
        if UNWIRED_INPUT_PADS.iter().any(|(b, _)| *b == board) {
            no_endpoint += 1;
            continue;
        }
        let outcome =
            assert_port_routes_generic_analog(&board, &chip, &profile, port_index, pins, &probe);
        silicon += usize::from(outcome.silicon_register);
        covered.push(board);
    }
    for (board, ..) in SILICON_INPUT_ADDRESS {
        assert!(
            covered.iter().any(|b| b == board),
            "{board} has a pinned silicon address but is not covered"
        );
    }
    for (board, _) in INPUT_REGISTER_UNREACHED {
        assert!(
            covered.iter().any(|b| b == board),
            "{board} is listed as unreached but is not covered"
        );
    }
    assert_eq!(
        covered.len(),
        COVERED,
        "onboarding profiles covered through a GPIO input; update docs/mixed_signal_board_coverage.md"
    );
    assert_eq!(
        silicon, SILICON_REGISTER,
        "profiles read back at a silicon input register; update docs/mixed_signal_board_coverage.md"
    );
    assert_eq!(
        no_endpoint, NO_ANALOG_ENDPOINT,
        "onboarding profiles with no GPIO input"
    );
}
