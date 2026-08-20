//! Diagnostic (not a gate): the ESP32-S3 bus answers the analog-I2C-master
//! FSM status register at 0x6000_E050 with 0, so the mask-ROM PHY routine
//! `rom_pkdet_vol_start` (0x4003_6A18) spins forever at 0x4003_6A41.
//!
//! ROM disassembly of the loop (esp32s3_rev0_rom.elf):
//!   40036a3e: l32r  a2, ->0x6000e050
//!   40036a41: memw
//!   40036a44: l32i.n a8, a2, 0        ; read 0x6000_E050
//!   40036a46: extui  a8, a8, 24, 3    ; bits[26:24]
//!   40036a49: bnei   a8, 7, 40036a41  ; loop until == 0b111 (FSM idle/done)
//!
//! The C3 models exactly this (peripherals/esp32c3/ana_i2c.rs); the S3 never
//! got the model, so the address falls through to the catch-all `rtc_cntl`
//! stub at 0x6000_8000 (+0x8000) whose read is `words.get(off).unwrap_or(0)`.

use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::Bus;

/// I2C_ANA_MST (analog I²C master / ANA_CONFIG) block on the ESP32-S3.
/// Undocumented in reg_base.h; regi2c_defs.h pins it via
/// I2C_MST_ANA_CONF0_REG = 0x6000E040 / ANA_CONFIG_REG = 0x6000E044.
const ANA_MST_BASE: u64 = 0x6000_E000;
/// Analog-master FSM status word. ROM waits for bits[26:24] == 7.
const ANA_MST_STATUS: u64 = ANA_MST_BASE + 0x50;
/// I2C_MST_ANA_CONF0_REG — bit 24 = I2C_MST_BBPLL_CAL_DONE.
const ANA_MST_CONF0: u64 = ANA_MST_BASE + 0x40;

#[test]
fn s3_ana_i2c_status_never_reaches_fsm_idle() {
    // No FASTBOOT: exercise the faithful vendored-ROM path the hosted runner uses.
    let mut bus = labwired_core::bus::SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

    // Which peripheral owns the address the ROM polls?
    let owner = bus
        .peripherals
        .iter()
        .filter(|p| p.base <= ANA_MST_STATUS && ANA_MST_STATUS < p.base + p.size)
        .map(|p| format!("{}@0x{:08X}+0x{:X}", p.name, p.base, p.size))
        .collect::<Vec<_>>()
        .join(", ");
    let owner = if owner.is_empty() {
        "<unmapped>".to_string()
    } else {
        owner
    };
    let v = bus.read_u32(ANA_MST_STATUS).unwrap();
    eprintln!(
        "0x{ANA_MST_STATUS:08X} owner={owner} value=0x{v:08X} bits[26:24]={}",
        (v >> 24) & 7
    );
    eprintln!(
        "0x{ANA_MST_CONF0:08X} value=0x{:08X} (bit24 BBPLL_CAL_DONE = {})",
        bus.read_u32(ANA_MST_CONF0).unwrap(),
        (bus.read_u32(ANA_MST_CONF0).unwrap() >> 24) & 1
    );

    // The bug: the ROM's exit condition can never be met.
    assert_ne!(
        (v >> 24) & 7,
        7,
        "if this now reads 7 the S3 grew an ana-i2c model — update this diag"
    );

    // And the write the ROM does just before the poll (0x6000_E05C) does not
    // move it either: the stub is pure storage.
    bus.write_u32(ANA_MST_STATUS + 0xC, 0x0088_0000).unwrap();
    let v2 = bus.read_u32(ANA_MST_STATUS).unwrap();
    assert_ne!((v2 >> 24) & 7, 7, "still not idle after the launch write");
}

/// How much of the radio register surface the S3 ROM/libphy touches does the
/// S3 bus actually model? Window list derived from the l32r literal pools of
/// the genuine esp32s3_rev0_rom.elf (see the ROM disassembly in the report):
/// every peripheral base in [0x6000_0000, 0x6005_0000) referenced by ROM code.
#[test]
fn s3_radio_window_coverage() {
    let mut bus = labwired_core::bus::SystemBus::new();
    let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    // (base, what the C3 needed there)
    let radio: &[(u64, &str)] = &[
        (
            0x6000_6000,
            "FE — RF front-end / PLL-lock status (C3: radio_fe_pll_lock)",
        ),
        (
            0x6000_E000,
            "I2C_ANA_MST — analog master FSM (C3: rtc_i2c_ana)",
        ),
        (0x6001_1000, "FE2/PHY gain tables (C3: wifi_fe)"),
        (0x6001_C000, "NRX — receiver/AGC (C3: wifi_bb)"),
        (0x6001_D000, "BB — baseband"),
        (0x6003_1000, "BT/WiFi shared"),
        (0x6003_3000, "WiFi MAC / WDEV (C3: wifi_mac behavioural)"),
        (0x6003_4000, "WiFi MAC cont."),
        (0x6003_5000, "analog master / MAC"),
        (0x6004_2000, "radio clock enable"),
    ];
    for (base, what) in radio {
        let owner = bus
            .peripherals
            .iter()
            .filter(|p| p.base <= *base && *base < p.base + p.size)
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
            .join("+");
        let owner = if owner.is_empty() {
            "<UNMAPPED>".into()
        } else {
            owner
        };
        eprintln!("0x{base:08X}  owner={owner:<18} {what}");
    }
}
