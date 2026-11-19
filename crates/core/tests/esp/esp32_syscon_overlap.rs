//! SYSCON must answer the first 0x100 bytes of the APB-CTRL window.
//!
//! `configure_xtensa_esp32` registers two peripherals at base 0x3FF6_6000:
//! `syscon` (0x100 bytes, real model) and then `apb_ctrl` (0x1000 bytes,
//! read-as-ones stub) for the tail. The comment there says registration order
//! wins on overlap. If it does not, every SYSCON register reads 0xFFFFFFFF.
//!
//! That is not a cosmetic difference. SYSCLK_CONF at offset 0x00 carries
//! PRE_DIV_CNT in bits[9:0]. Read as all-ones it is 1023, so ESP-IDF's
//! `rtc_clk_cpu_freq_get_config` computes div = 1024 and reports the CPU at
//! 80/1024 = 0 MHz. Arduino's `getApbFrequency()` then returns 78125 Hz, and
//! `_get_effective_baudrate` evaluates `80 / (78125/1000000)` — an integer
//! divide by zero, Xtensa exception cause 6, which is precisely why the
//! Arduino serial path has been thunked out on this chip.

use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::Bus;

const SYSCLK_CONF: u64 = 0x3FF6_6000;
const TICK_CONF: u64 = 0x3FF6_6004;

#[test]
fn sysclk_conf_reads_its_reset_value_not_the_apb_ctrl_ones_stub() {
    let mut bus = SystemBus::new();
    let _cpu = configure_xtensa_esp32(&mut bus);

    let v = bus.read_u32(SYSCLK_CONF).expect("SYSCLK_CONF is mapped");
    assert_ne!(
        v, 0xFFFF_FFFF,
        "SYSCLK_CONF read as all-ones: the apb_ctrl stub is shadowing syscon, \
         so PRE_DIV_CNT reads 1023 and the CPU-frequency probe divides by zero",
    );
    assert_eq!(
        v & 0x3FF,
        0,
        "PRE_DIV_CNT must reset to 0 (divider 1); anything else scales the \
         reported CPU frequency and breaks Arduino's baudrate maths",
    );
}

#[test]
fn tick_conf_keeps_the_seeded_xtal_tick_divisor() {
    // The other end of the same window. XTAL_TICK_NUM is seeded to 39
    // (40 MHz / 1 MHz - 1); the ones-stub would report 255.
    let mut bus = SystemBus::new();
    let _cpu = configure_xtensa_esp32(&mut bus);

    let v = bus.read_u32(TICK_CONF).expect("TICK_CONF is mapped");
    assert_eq!(
        v & 0xFF,
        39,
        "XTAL_TICK_NUM must read 39; got 0x{v:08x} (all-ones means apb_ctrl won)",
    );
}
