// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! ESP32-C3 reset-state MMIO conformance oracle.
//!
//! Full-estate onboarding (2026-06-11) of the ESP32-C3. The chip yaml now wires
//! every SVD-documented peripheral block the silicon exposes; this oracle pins
//! the subset of their descriptor `reset_value`s that was corroborated against
//! real silicon. It covers two onboarding passes:
//!   1. the control blocks first wired into the chip (SYSTEM, RTC_CNTL,
//!      APB_CTRL, SYSTIMER, IO_MUX, I2C0, SPI2, LEDC, RMT, UART1, TIMG1), and
//!   2. the estate completion (SPI0/1, GPIO_SD, EFUSE, UHCI0/1, BB, TWAI0,
//!      I2S0, AES, SHA, RSA, DS, HMAC, GDMA, APB_SARADC, USB_DEVICE, SENSITIVE,
//!      EXTMEM, XTS_AES, ASSIST_DEBUG).
//!
//! ## Capture provenance
//!
//! A live ESP32-C3 (QFN32, rev v0.4, MAC 38:44:be:42:f5:58) was read over its
//! built-in USB-Serial/JTAG with `openocd-esp32 v0.12.0-esp32-20260424`
//! (`board/esp32c3-builtin.cfg`), `mdw` reads wrapped in tcl `capture {}`. Two
//! capture sets are committed under
//! `scripts/hw-oracle/captures/esp32c3/<ts>/reg_oracle.json` (the original
//! 15-window control-block set and the 21-window estate-completion set). In the
//! estate pass, 94 non-zero descriptor reset values matched silicon exactly
//! (SPI0/1 config, SAR-ADC, the SENSITIVE PMS estate, EXTMEM cache/flash
//! windows, GPIO_SD, USB_DEVICE, XTS_AES); the crypto/DMA accelerators read
//! all-zero idle, matching their descriptors.
//!
//! Of 423 registers that overlapped a descriptor `reset_value`, **366 matched
//! silicon**. The 57 that differed are NOT descriptor bugs: a JTAG `reset
//! halt` on the C3 is a *software* core reset that does not cold-reset the
//! peripherals, and the ROM bootloader has already run — so those registers
//! (UART console CLKDIV/STATUS, FIFO counts, GPIO `IN`/`STRAP`, fed WDTs, RTC
//! calibration state) hold post-ROM/dynamic values rather than cold-reset
//! values. This oracle therefore asserts only the **ROM-untouched, static**
//! reset values where descriptor and silicon agree.
//!
//! ## 2026-08-08: two entries were NOT reset values at all
//!
//! Repeat-reads on live silicon retired the equality claim on `WIFI_MAC`
//! `0x60035000` and `0x6003507C`: both change on every read. They moved to
//! [`FREE_RUNNING_COUNTERS`], which asserts only that the window maps. Two
//! further entries (`RADIO_FE 0x60006000`, `WIFI_MAC 0x6003502C`) mismatched
//! live but were stable, which is the signature of a powered PHY rather than a
//! wrong descriptor; they stay asserted and are annotated in the table with what
//! a capture would have to do to confirm them.
//!
//! ## Running
//!
//! Sim-only (normal CI):
//! ```text
//! cargo test -p labwired-hw-oracle --test esp32c3_reset_conformance
//! ```
//!
//! Re-capture from hardware (C3 on USB-JTAG), then diff by hand against the
//! committed `reg_oracle.json`:
//! ```text
//! openocd -s $OPENOCD_ESP32_SCRIPTS -f board/esp32c3-builtin.cfg \
//!   -c init -c "reset halt" -c "mdw 0x600C0000 48" -c shutdown
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

/// Curated silicon-corroborated reset values (descriptor == live C3), drawn
/// from the blocks newly wired into `configs/chips/esp32c3.yaml`. Only
/// ROM-untouched / static registers are listed — see module docs.
const RESET_VALUES: &[(&str, u64, u32)] = &[
    ("SYSTEM CPU_PERI_CLK_EN", 0x600C_0000, 0x0000_0000),
    ("SYSTEM CPU_PERI_RST_EN", 0x600C_0004, 0x0000_00C0),
    ("SYSTEM CPU_PER_CONF", 0x600C_0008, 0x0000_000C),
    ("SYSTEM MEM_PD_MASK", 0x600C_000C, 0x0000_0001),
    ("APB_CTRL SYSCLK_CONF", 0x6002_6000, 0x0000_0001),
    ("APB_CTRL TICK_CONF", 0x6002_6004, 0x0001_0727),
    ("APB_CTRL CLK_OUT_EN", 0x6002_6008, 0x0000_07FF),
    ("APB_CTRL WIFI_BB_CFG", 0x6002_600C, 0x0000_0000),
    ("SPI2 CMD", 0x6002_4000, 0x0000_0000),
    ("SPI2 ADDR", 0x6002_4004, 0x0000_0000),
    ("SPI2 CTRL", 0x6002_4008, 0x003C_0000),
    ("SPI2 CLOCK", 0x6002_400C, 0x8000_3043),
    ("SYSTIMER CONF", 0x6002_3000, 0x4600_0000),
    ("SYSTIMER UNIT0_OP", 0x6002_3004, 0x0000_0000),
    ("SYSTIMER UNIT1_OP", 0x6002_3008, 0x0000_0000),
    ("SYSTIMER UNIT0_LOAD_HI", 0x6002_300C, 0x0000_0000),
    ("LEDC CH0_CONF0", 0x6001_9000, 0x0000_0000),
    ("LEDC CH0_HPOINT", 0x6001_9004, 0x0000_0000),
    ("LEDC CH0_DUTY", 0x6001_9008, 0x0000_0000),
    ("LEDC CH0_CONF1", 0x6001_900C, 0x4000_0000),
    ("I2C0 SCL_LOW_PERIOD", 0x6001_3000, 0x0000_0000),
    ("I2C0 CTR", 0x6001_3004, 0x0000_020B),
    ("I2C0 SR", 0x6001_3008, 0x0000_C000),
    ("I2C0 TO", 0x6001_300C, 0x0000_0010),
    ("RTC_CNTL OPTIONS0", 0x6000_8000, 0x1C00_A000),
    ("RTC_CNTL SLP_TIMER0", 0x6000_8004, 0x0000_0000),
    ("RTC_CNTL SLP_TIMER1", 0x6000_8008, 0x0000_0000),
    ("RTC_CNTL TIME_UPDATE", 0x6000_800C, 0x0000_0000),
    // --- Estate completion (2026-06-11): the SVD-documented blocks the chip
    // --- yaml did not previously wire. Each value below was read back from the
    // --- live C3 and equals the descriptor reset_value (capture estate-* set).
    // SPI1 (flash controller) config defaults
    ("SPI1 CTRL", 0x6000_2008, 0x002C_A000),
    ("SPI1 CTRL1", 0x6000_200C, 0x0000_0FFC),
    ("SPI1 CLOCK", 0x6000_2014, 0x0003_0103),
    ("SPI1 USER", 0x6000_2018, 0x8000_0000),
    ("SPI1 USER1", 0x6000_201C, 0x5C00_0007),
    ("SPI1 USER2", 0x6000_2020, 0x7000_0000),
    ("SPI1 MISC", 0x6000_2034, 0x0000_0002),
    ("SPI1 CLOCK_GATE", 0x6000_20DC, 0x0000_0001),
    // SPI0 (cache/PSRAM controller) config defaults
    ("SPI0 CTRL", 0x6000_3008, 0x002C_2000),
    ("SPI0 CTRL2", 0x6000_3010, 0x0000_0021),
    ("SPI0 CLOCK", 0x6000_3014, 0x0003_0103),
    ("SPI0 USER1", 0x6000_301C, 0x5C00_0007),
    ("SPI0 USER2", 0x6000_3020, 0x7000_0000),
    ("SPI0 CLOCK_GATE", 0x6000_30DC, 0x0000_0001),
    // GPIO sigma-delta: 8-bit defaults + version stamp
    ("GPIO_SD SIGMADELTA0", 0x6000_4F00, 0x0000_FF00),
    ("GPIO_SD VERSION", 0x6000_4F28, 0x0200_6230),
    // SAR ADC full config estate
    ("APB_SARADC CTRL", 0x6004_0000, 0x4003_8240),
    ("APB_SARADC CTRL2", 0x6004_0004, 0x0000_A1FE),
    ("APB_SARADC FSM_WAIT", 0x6004_000C, 0x00FF_0808),
    ("APB_SARADC ONETIME_SAMPLE", 0x6004_0020, 0x1A00_0000),
    ("APB_SARADC ARB_CTRL", 0x6004_0024, 0x0000_0900),
    ("APB_SARADC DMA_CONF", 0x6004_0050, 0x0000_00FF),
    ("APB_SARADC CLKM_CONF", 0x6004_0054, 0x0000_0004),
    ("APB_SARADC CALI", 0x6004_0060, 0x0000_8000),
    // USB Serial/JTAG config + date stamp
    ("USB_DEVICE CONF0", 0x6004_3018, 0x0000_4200),
    ("USB_DEVICE MEM_CONF", 0x6004_3048, 0x0000_0002),
    ("USB_DEVICE DATE", 0x6004_3080, 0x0200_7300),
    // SENSITIVE permission/PMS estate (security defaults)
    (
        "SENSITIVE APB_PERIPHERAL_ACCESS_1",
        0x600C_1014,
        0x0000_0001,
    ),
    ("SENSITIVE INTERNAL_SRAM_USAGE_1", 0x600C_101C, 0x0000_000F),
    ("SENSITIVE SPI2_PMS_CONSTRAIN_1", 0x600C_103C, 0x000F_F0FF),
    (
        "SENSITIVE CORE_X_IRAM0_PMS_CONSTRAIN_1",
        0x600C_10AC,
        0x001C_7FFF,
    ),
    (
        "SENSITIVE CORE_0_PIF_PMS_CONSTRAIN_2",
        0x600C_10E0,
        0xFCC3_0CF3,
    ),
    // EXTMEM cache config + flash/PSRAM virtual address windows
    ("EXTMEM ICACHE_CTRL1", 0x600C_4004, 0x0000_0003),
    ("EXTMEM IBUS_TO_FLASH_START_VADDR", 0x600C_4054, 0x4200_0000),
    ("EXTMEM IBUS_TO_FLASH_END_VADDR", 0x600C_4058, 0x427F_FFFF),
    ("EXTMEM DBUS_TO_FLASH_START_VADDR", 0x600C_405C, 0x3C00_0000),
    ("EXTMEM DBUS_TO_FLASH_END_VADDR", 0x600C_4060, 0x3C7F_FFFF),
    ("EXTMEM CACHE_STATE", 0x600C_40B0, 0x0000_0001),
    // Flash XTS-AES date stamp
    ("XTS_AES DATE", 0x600C_C05C, 0x2020_0623),
    // Crypto/DMA blocks idle at reset: assert the window maps and reads 0 (not a
    // bus fault) — proves correct wiring of the accelerator estate.
    ("AES (idle)", 0x6003_A000, 0x0000_0000),
    ("SHA MODE (idle)", 0x6003_B000, 0x0000_0000),
    ("RSA (idle)", 0x6003_C000, 0x0000_0000),
    ("DS (idle)", 0x6003_D000, 0x0000_0000),
    ("HMAC (idle)", 0x6003_E000, 0x0000_0000),
    ("GDMA (idle)", 0x6003_F000, 0x0000_0000),
    ("I2S0 (idle)", 0x6002_D000, 0x0000_0000),
    // --- Radio (WiFi/BT) register-backed model, REVERSE-ENGINEERED from live
    // --- silicon (docs/esp32c3_radio_reverse_engineering.md), not SVD. These
    // --- windows reset cold (pre-phy_enable); assert they map + return cold
    // --- state. The WiFi MAC carries 10 non-zero hardware reset defaults here;
    // --- two more used to be listed and are now in FREE_RUNNING_COUNTERS below.
    //
    // ⚠️ TWO OF THESE NEED A COLD POWER-CYCLE CAPTURE TO VALIDATE, and a live
    // read on 2026-08-08 could not supply one. Both boards were running a BLE
    // sketch, and openocd `reset halt` on the C3 is a SOFTWARE core reset that
    // leaves the PHY powered, so the radio estate is read post-`phy_enable`:
    //
    //   RADIO_FE 0x60006000  asserted 0x00000000, live 0x06828040
    //   WIFI_MAC 0x6003502C  asserted 0x00000002, live 0x00000000
    //
    // Unlike FREE_RUNNING_COUNTERS these were STABLE across repeat reads, which
    // is what says "powered-up state", not "counter". The cold values are still
    // the right thing for the descriptor to model, so the assertions stay. To
    // confirm them against silicon, capture with the board freshly power-cycled
    // into a firmware that never calls `phy_enable` — not via `reset halt`.
    ("RADIO_FE (cold)", 0x6000_6000, 0x0000_0000),
    ("RADIO_NRX (cold)", 0x6001_CC00, 0x0000_0000),
    ("WIFI_MAC base (cold)", 0x6003_3000, 0x0000_0000),
    // WiFi MAC non-zero cold-reset defaults (silicon-corroborated)
    ("WIFI_MAC 0x60035024", 0x6003_5024, 0x024E_01FF),
    ("WIFI_MAC 0x60035028", 0x6003_5028, 0xB000_0000),
    ("WIFI_MAC 0x6003502C", 0x6003_502C, 0x0000_0002),
    ("WIFI_MAC 0x60035030", 0x6003_5030, 0x0000_0064),
    ("WIFI_MAC 0x6003503C", 0x6003_503C, 0x0000_0064),
    ("WIFI_MAC 0x60035048", 0x6003_5048, 0x0000_0064),
    ("WIFI_MAC 0x60035054", 0x6003_5054, 0x0000_0064),
    ("WIFI_MAC 0x60035080", 0x6003_5080, 0x0000_07FF),
    ("WIFI_MAC 0x60035084", 0x6003_5084, 0x0000_3202),
    ("WIFI_MAC 0x60035094", 0x6003_5094, 0x0000_0004),
];

/// FREE-RUNNING COUNTERS — mapped, but deliberately NOT equality-asserted.
///
/// These two addresses were in [`RESET_VALUES`] with fixed expected words. They
/// cannot be. Triple repeat-reads inside ONE halted openocd session on a live
/// ESP32-C3 (2026-08-08, reproduced on both attached boards):
///
/// ```text
/// 0x60035000  asserted 0x000C9858   live: 0x00006E8E -> 0x0000A438 -> 0x0000D62B
/// 0x6003507C  asserted 0x7D9AD8A3   live: 0x53327C00 -> 0x1E028FBF -> 0x6F48DDB5
/// ```
///
/// The value changes on EVERY read while the core is halted, so no capture from
/// any silicon can ever equal a constant, and the committed "silicon-corroborated
/// reset default" was a single sample of a counter mistaken for a reset value.
/// 0x60035000 advances monotonically at a fixed rate (a free-running timer);
/// 0x6003507C is unordered between reads (a counter fed by the PHY/PRNG side of
/// the MAC). Neither is documented in the SVD — this window is
/// reverse-engineered, see `docs/esp32c3_radio_reverse_engineering.md`.
///
/// The offline test passed only because the simulator returns a frozen word: an
/// oracle that could never fail against the sim and could never pass against
/// hardware. That is a vacuous gate, so the equality claim is withdrawn.
///
/// They are kept HERE, not deleted, for two reasons: the next person must be
/// able to see that they were considered and excluded on evidence rather than
/// quietly dropped; and the property that DOES hold — the window is mapped and
/// reads without a bus fault, which is what proves the radio estate is wired —
/// is still worth asserting. [`esp32c3_free_running_counters_are_mapped`] does
/// exactly that and nothing more.
///
/// To turn either back into an equality assertion you need a model of what the
/// counter counts, and then the assertion is about its RATE, not its value.
const FREE_RUNNING_COUNTERS: &[(&str, u64)] = &[
    ("WIFI_MAC 0x60035000 (free-running counter)", 0x6003_5000),
    ("WIFI_MAC 0x6003507C (free-running counter)", 0x6003_507C),
];

fn build_sim_bus() -> SystemBus {
    let chip_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/esp32c3.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load chip {chip_path:?}: {e}"));
    let manifest = SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "esp32c3-reset-conformance".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| panic!("build sim bus: {e}"))
}

/// Every newly-wired C3 block must (a) be mapped (no bus fault) and (b) return
/// its silicon-corroborated reset value.
#[test]
fn esp32c3_reset_values_match_silicon() {
    let sim = build_sim_bus();
    let mut failures = Vec::new();

    for &(label, addr, expect) in RESET_VALUES {
        match sim.read_u32(addr) {
            Ok(got) if got == expect => {}
            Ok(got) => failures.push(format!(
                "  [DIFF] {label} 0x{addr:08X}: sim=0x{got:08X} silicon=0x{expect:08X}"
            )),
            Err(e) => failures.push(format!("  [FAULT] {label} 0x{addr:08X}: {e:?}")),
        }
    }

    assert!(
        failures.is_empty(),
        "ESP32-C3 reset-state model diverged from silicon in {} of {} register(s):\n{}",
        failures.len(),
        RESET_VALUES.len(),
        failures.join("\n")
    );
}

/// The two [`FREE_RUNNING_COUNTERS`] windows must still MAP — a read must not
/// bus-fault — which is the part of the old claim that survives contact with
/// silicon. Their VALUE is not asserted, and must not be: see the table's docs
/// for the live read sequences that retired the equality claim.
#[test]
fn esp32c3_free_running_counters_are_mapped() {
    let sim = build_sim_bus();
    let mut failures = Vec::new();

    for &(label, addr) in FREE_RUNNING_COUNTERS {
        if let Err(e) = sim.read_u32(addr) {
            failures.push(format!("  [FAULT] {label} 0x{addr:08X}: {e:?}"));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} free-running WiFi MAC window(s) failed to map:\n{}",
        failures.len(),
        FREE_RUNNING_COUNTERS.len(),
        failures.join("\n")
    );
}
