//! ESP32-WROOM-32 + SSD1680 tri-color 2.9" e-paper.
//!
//! Drives the Waveshare 2.9" tri-color e-paper module (SSD1680 / GDEM029C90)
//! over VSPI + four GPIO sidebands. Draws three full-width horizontal bands —
//! WHITE on top, BLACK in the middle, RED on the bottom — and triggers one
//! full refresh. Mirrors the byte sequence emitted by the AgentDeck firmware
//! (`GxEPD2_290_C90c`) and the LabWired STM32 e-paper lab, so the simulated
//! SSD1680 model decodes both paths identically.
//!
//! Pin mapping (Waveshare default, AgentDeck-compatible):
//!   GPIO5  — CS                     GPIO output push-pull
//!   GPIO18 — SCK    (VSPI signal)   IO_MUX function 1
//!   GPIO23 — MOSI   (VSPI signal)   IO_MUX function 1
//!   GPIO17 — DC                     GPIO output push-pull
//!   GPIO16 — RST                    GPIO output push-pull
//!   GPIO4  — BUSY                   GPIO input
//!
//! Build: `cargo build --release` from this directory (requires the `esp`
//! Rust toolchain — `espup install` if not present).
//!
//! Run in sim: `labwired run -s system.yaml -f <this ELF>`. The ELF can
//! also be flashed to a real ESP32-WROOM-32 with `espflash flash --monitor
//! target/xtensa-esp32-none-elf/release/esp32-epaper-lab`.

#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

use esp_backtrace as _;

// ESP-IDF app image descriptor — required by espflash 4.x to validate
// the flashed image. The macro emits a tagged section the 2nd-stage
// bootloader (and `espflash flash`) reads to identify the app. Doesn't
// affect sim behaviour; pure metadata.
esp_bootloader_esp_idf::esp_app_desc!();

// STATUS — sim-side WIP:
//   * Firmware builds cleanly for `xtensa-esp32-none-elf` (espflash-ready).
//   * In the LabWired sim, esp-hal's Reset → __pre_init → esp32_init chain
//     touches DPORT / IO_MUX / RTC banks that the v0.6 simulator doesn't
//     yet model with enough fidelity, so the firmware traps to the BROM
//     exception vector (0x40000300) before reaching `main`. The e2e test
//     `e2e_esp32_epaper.rs` documents the failure surface.
//   * On physical ESP32-WROOM hardware the same ELF (via espflash) should
//     run since real silicon has BROM and IO_MUX defaults already in place
//     — that's the verification path for Phase 9.
//   * Follow-up to unblock the sim path: either override esp-hal's
//     __pre_init weak symbol via a separate-section trick, or land
//     ESP32-classic peripheral stubs for DPORT clock-mux + IO_MUX defaults.

// ----- Register addresses (ESP32 TRM v4.6) -------------------------------
//
// GPIO (TRM §4.10) at 0x3FF4_4000.
const GPIO_OUT_W1TS_REG:    *mut u32 = 0x3FF4_4008 as *mut u32;
const GPIO_OUT_W1TC_REG:    *mut u32 = 0x3FF4_400C as *mut u32;
const GPIO_ENABLE_W1TS_REG: *mut u32 = 0x3FF4_4024 as *mut u32;
const GPIO_IN_REG:          *const u32 = 0x3FF4_403C as *const u32;
const GPIO_FUNC_OUT_SEL_CFG_BASE: u64 = 0x3FF4_4530;
const GPIO_FUNC_IN_SEL_CFG_BASE:  u64 = 0x3FF4_4130;

// IO_MUX (TRM §4.11) at 0x3FF4_9000.
const IO_MUX_BASE: u64 = 0x3FF4_9000;
// Per-pin offsets: indirected through a fixed mapping. See ESP32 TRM
// Table 4-2. We only need the pins on the e-paper path.
// Per ESP32 TRM Table 4-1 — IO_MUX register offsets are per-pin and irregular,
// not GPIO-index aligned.
const IO_MUX_GPIO4_REG:  *mut u32 = 0x3FF4_9048 as *mut u32; // BUSY
const IO_MUX_GPIO5_REG:  *mut u32 = 0x3FF4_906C as *mut u32; // CS
const IO_MUX_GPIO16_REG: *mut u32 = 0x3FF4_904C as *mut u32; // RST
const IO_MUX_GPIO17_REG: *mut u32 = 0x3FF4_9050 as *mut u32; // DC
const IO_MUX_GPIO18_REG: *mut u32 = 0x3FF4_9070 as *mut u32; // SCK
const IO_MUX_GPIO23_REG: *mut u32 = 0x3FF4_908C as *mut u32; // MOSI

// SPI3 (VSPI) at 0x3FF6_5000. Offsets per ESP32 TRM v4.6 §7 — USER block
// sits at 0x1C/0x20/0x24 on ESP32-classic (S3 has it 4 bytes higher).
const SPI3_CMD_REG:        *mut u32 = 0x3FF6_5000 as *mut u32;
const SPI3_CLOCK_REG:      *mut u32 = 0x3FF6_5018 as *mut u32;
const SPI3_USER_REG:       *mut u32 = 0x3FF6_501C as *mut u32;
#[allow(dead_code)]
const SPI3_USER2_REG:      *mut u32 = 0x3FF6_5024 as *mut u32;
const SPI3_MOSI_DLEN_REG:  *mut u32 = 0x3FF6_5028 as *mut u32;
const SPI3_PIN_REG:        *mut u32 = 0x3FF6_5034 as *mut u32;
const SPI3_W0_REG:         *mut u32 = 0x3FF6_5080 as *mut u32;

// DPORT — peripheral clock-gate / reset (TRM §3.1.3).
const DPORT_PERIP_CLK_EN_REG: *mut u32 = 0x3FF0_00C0 as *mut u32;
const DPORT_PERIP_RST_EN_REG: *mut u32 = 0x3FF0_00C4 as *mut u32;
const DPORT_PERIP_CLK_SPI3_BIT: u32 = 1 << 4;

// ----- Pin masks ----------------------------------------------------------

const CS_MASK:   u32 = 1 << 5;   // GPIO5
const RST_MASK:  u32 = 1 << 16;  // GPIO16
const DC_MASK:   u32 = 1 << 17;  // GPIO17
const BUSY_MASK: u32 = 1 << 4;   // GPIO4
const SCK_MASK:  u32 = 1 << 18;  // GPIO18
const MOSI_MASK: u32 = 1 << 23;  // GPIO23

// VSPI signal indices (TRM Table 4-15 — input/output signal numbers).
// Output signals — we route the VSPI master signals onto GPIO matrix pins.
const VSPICLK_OUT_IDX: u32 = 8;
const VSPID_OUT_IDX:   u32 = 9;  // VSPI MOSI

// ----- Panel geometry -----------------------------------------------------

const WIDTH: u16 = 128;
const HEIGHT: u16 = 296;
const WIDTH_BYTES: u16 = WIDTH / 8;

// ----- GPIO helpers -------------------------------------------------------

#[inline(always)] fn cs_low()  { unsafe { core::ptr::write_volatile(GPIO_OUT_W1TC_REG, CS_MASK) } }
#[inline(always)] fn cs_high() { unsafe { core::ptr::write_volatile(GPIO_OUT_W1TS_REG, CS_MASK) } }
#[inline(always)] fn dc_low()  { unsafe { core::ptr::write_volatile(GPIO_OUT_W1TC_REG, DC_MASK) } }
#[inline(always)] fn dc_high() { unsafe { core::ptr::write_volatile(GPIO_OUT_W1TS_REG, DC_MASK) } }
#[inline(always)] fn rst_low() { unsafe { core::ptr::write_volatile(GPIO_OUT_W1TC_REG, RST_MASK) } }
#[inline(always)] fn rst_high(){ unsafe { core::ptr::write_volatile(GPIO_OUT_W1TS_REG, RST_MASK) } }

#[inline(always)]
fn busy_high() -> bool {
    unsafe { (core::ptr::read_volatile(GPIO_IN_REG) & BUSY_MASK) != 0 }
}

/// Busy-wait `nops` (each ≈ 1 CPU cycle at the configured clock).
/// At the esp-hal default of 80 MHz CPU, 80_000 ≈ 1 ms.
fn delay(cycles: u32) {
    for _ in 0..cycles {
        unsafe { core::arch::asm!("nop") };
    }
}

/// 20 ms reset hold per SSD1680 datasheet.
#[inline]
fn delay_20ms() {
    delay(1_600_000);
}

/// Bounded BUSY wait — sim never raises BUSY, real silicon takes ~15 s.
fn wait_idle() {
    for _ in 0..4_000_000 {
        if !busy_high() {
            return;
        }
    }
}

// ----- VSPI low-level helpers --------------------------------------------

/// Issue one MOSI-only SPI3 transaction of up to 64 bytes.
/// `bytes` is the full payload — the function packs it into the FIFO,
/// programs the bit length, and starts the user-defined cycle. Returns
/// when the controller clears CMD.USR (sim does this synchronously, real
/// silicon within microseconds at the configured clock).
fn spi_write(bytes: &[u8]) {
    debug_assert!(bytes.len() <= 64);
    if bytes.is_empty() {
        return;
    }
    // Pack bytes into W0..W15 (little-endian within each 32-bit word).
    unsafe {
        let mut word: u32 = 0;
        let mut word_idx = 0usize;
        let mut byte_in_word = 0usize;
        for &b in bytes {
            word |= (b as u32) << (byte_in_word * 8);
            byte_in_word += 1;
            if byte_in_word == 4 {
                core::ptr::write_volatile(SPI3_W0_REG.add(word_idx), word);
                word_idx += 1;
                byte_in_word = 0;
                word = 0;
            }
        }
        if byte_in_word > 0 {
            core::ptr::write_volatile(SPI3_W0_REG.add(word_idx), word);
        }

        // USR_MOSI = 1, all other phases off.
        core::ptr::write_volatile(SPI3_USER_REG, 1 << 27);
        // MOSI bit length minus 1.
        core::ptr::write_volatile(SPI3_MOSI_DLEN_REG, ((bytes.len() as u32) * 8) - 1);
        // CMD.USR = 1 → start. Sim clears synchronously; real silicon
        // takes O(microseconds).
        core::ptr::write_volatile(SPI3_CMD_REG, 1 << 18);
        // Wait for CMD.USR to clear.
        for _ in 0..1_000_000 {
            if core::ptr::read_volatile(SPI3_CMD_REG) & (1 << 18) == 0 {
                break;
            }
        }
    }
}

// ----- SSD1680 protocol ---------------------------------------------------

fn ep_cmd(cmd: u8) {
    dc_low();
    cs_low();
    spi_write(&[cmd]);
    cs_high();
}

fn ep_cmd_data(cmd: u8, data: &[u8]) {
    dc_low();
    cs_low();
    spi_write(&[cmd]);
    dc_high();
    spi_write(data);
    cs_high();
}

fn ep_hw_reset() {
    rst_high();
    delay_20ms();
    rst_low();
    delay_20ms();
    rst_high();
    delay_20ms();
    wait_idle();
}

fn ep_init() {
    ep_hw_reset();
    ep_cmd(0x12); // SWRESET
    wait_idle();
    ep_cmd_data(0x01, &[0x27, 0x01, 0x00]); // Driver output control
    ep_cmd_data(0x11, &[0x03]);             // Data entry mode
    ep_cmd_data(0x3C, &[0x05]);             // Border waveform
    ep_cmd_data(0x18, &[0x80]);             // Temp sensor select
    ep_cmd_data(0x21, &[0x00, 0x80]);       // Display update ctrl 1
    ep_cmd_data(0x44, &[0x00, (WIDTH_BYTES - 1) as u8]);
    ep_cmd_data(
        0x45,
        &[0x00, 0x00, ((HEIGHT - 1) & 0xFF) as u8, ((HEIGHT - 1) >> 8) as u8],
    );
    ep_cmd_data(0x4E, &[0x00]);
    ep_cmd_data(0x4F, &[0x00, 0x00]);
}

fn ep_stream_plane<F: Fn(u16) -> u8>(cmd: u8, byte_for_row: F) {
    ep_cmd_data(0x4E, &[0x00]);
    ep_cmd_data(0x4F, &[0x00, 0x00]);

    dc_low();
    cs_low();
    spi_write(&[cmd]);
    dc_high();
    // Stream the plane in 32-byte chunks (2 rows × 16 bytes-per-row).
    // FIFO max is 64 bytes per spi_write; 32 keeps things simple.
    let mut buf = [0u8; 32];
    let mut row: u16 = 0;
    while row < HEIGHT {
        let mut idx = 0usize;
        let mut next_row = row;
        while idx + (WIDTH_BYTES as usize) <= buf.len() && next_row < HEIGHT {
            let v = byte_for_row(next_row);
            for _ in 0..(WIDTH_BYTES as usize) {
                buf[idx] = v;
                idx += 1;
            }
            next_row += 1;
        }
        spi_write(&buf[..idx]);
        row = next_row;
    }
    cs_high();
}

fn ep_refresh() {
    ep_cmd_data(0x22, &[0xF7]);
    ep_cmd(0x20);
    wait_idle();
}

// ----- Test pattern -------------------------------------------------------

fn black_plane_byte(row: u16) -> u8 {
    match row {
        99..=197 => 0x00,
        _ => 0xFF,
    }
}

fn red_plane_byte(row: u16) -> u8 {
    match row {
        198..=295 => 0x00,
        _ => 0xFF,
    }
}

// ----- IO_MUX / GPIO_MATRIX setup ----------------------------------------
//
// Configures the package pins for our function.
//   IO_MUX_GPIOn_REG bits (ESP32 TRM Table 4-3):
//     [0]    FUN_IE         input enable
//     [1:2]  FUN_DRV        drive strength (0..3)
//     [3:4]  FUN_WPD/WPU    pulldown/pullup
//     [7]    FUN_SEL[2:0]   function select (bits [12:14])
//   FUN_SEL=0 → IO_MUX function (= chip default, e.g. VSPI signals for
//                                 GPIO18/23 if pin is "VSPICLK"/"VSPID")
//   FUN_SEL=2 → GPIO function (i.e. driven by the GPIO matrix)

const IO_MUX_FUN_IE: u32 = 1 << 9;       // Input enable
const fn io_mux_fun_sel(n: u32) -> u32 { n << 12 }
const IO_MUX_FUN_DRV_2: u32 = 0b10 << 10; // medium drive

fn configure_pins() {
    unsafe {
        esp_println::println!("[lab]  · io_mux CS/RST/DC");
        core::ptr::write_volatile(IO_MUX_GPIO5_REG,  io_mux_fun_sel(2) | IO_MUX_FUN_DRV_2);
        core::ptr::write_volatile(IO_MUX_GPIO16_REG, io_mux_fun_sel(2) | IO_MUX_FUN_DRV_2);
        core::ptr::write_volatile(IO_MUX_GPIO17_REG, io_mux_fun_sel(2) | IO_MUX_FUN_DRV_2);

        esp_println::println!("[lab]  · io_mux BUSY");
        core::ptr::write_volatile(IO_MUX_GPIO4_REG, io_mux_fun_sel(2) | IO_MUX_FUN_IE);

        esp_println::println!("[lab]  · io_mux SCK/MOSI");
        core::ptr::write_volatile(IO_MUX_GPIO18_REG, io_mux_fun_sel(2) | IO_MUX_FUN_DRV_2);
        core::ptr::write_volatile(IO_MUX_GPIO23_REG, io_mux_fun_sel(2) | IO_MUX_FUN_DRV_2);

        esp_println::println!("[lab]  · gpio matrix VSPI routing");
        let out_sel18 = (GPIO_FUNC_OUT_SEL_CFG_BASE + 18 * 4) as *mut u32;
        core::ptr::write_volatile(out_sel18, VSPICLK_OUT_IDX);
        let out_sel23 = (GPIO_FUNC_OUT_SEL_CFG_BASE + 23 * 4) as *mut u32;
        core::ptr::write_volatile(out_sel23, VSPID_OUT_IDX);

        esp_println::println!("[lab]  · gpio enable outputs");
        core::ptr::write_volatile(
            GPIO_ENABLE_W1TS_REG,
            CS_MASK | RST_MASK | DC_MASK | SCK_MASK | MOSI_MASK,
        );

        esp_println::println!("[lab]  · initial out state");
        core::ptr::write_volatile(GPIO_OUT_W1TS_REG, CS_MASK | RST_MASK | DC_MASK);
    }
    let _ = GPIO_FUNC_IN_SEL_CFG_BASE;
    let _ = IO_MUX_BASE;
}

/// Disable the RTC watchdog and the two timer-group task watchdogs.
///
/// Each WDT block is protected by a write-key register: write the magic
/// key first, write the disable bits, then write a non-key value to
/// re-lock. Real silicon and our sim's `RtcCntlStub` round-trip writes
/// either way; this just keeps the firmware self-sufficient (no esp-hal
/// runtime dependency for the boot path).
fn disable_watchdogs() {
    unsafe {
        // RTC_CNTL_WDTWPROTECT_REG at 0x3FF480A4, key = 0x50D83AA1.
        // RTC_CNTL_WDTCONFIG0_REG at 0x3FF4808C — clear EN (bit 31).
        core::ptr::write_volatile(0x3FF4_80A4 as *mut u32, 0x50D8_3AA1);
        core::ptr::write_volatile(0x3FF4_808C as *mut u32, 0);
        core::ptr::write_volatile(0x3FF4_80A4 as *mut u32, 0);

        // TIMG0_WDTWPROTECT_REG at 0x3FF5_F064, key = 0x50D83AA1.
        // TIMG0_WDTCONFIG0_REG at 0x3FF5_F048 — clear EN (bit 31).
        core::ptr::write_volatile(0x3FF5_F064 as *mut u32, 0x50D8_3AA1);
        core::ptr::write_volatile(0x3FF5_F048 as *mut u32, 0);
        core::ptr::write_volatile(0x3FF5_F064 as *mut u32, 0);

        // TIMG1_WDTWPROTECT_REG at 0x3FF6_0064.
        core::ptr::write_volatile(0x3FF6_0064 as *mut u32, 0x50D8_3AA1);
        core::ptr::write_volatile(0x3FF6_0048 as *mut u32, 0);
        core::ptr::write_volatile(0x3FF6_0064 as *mut u32, 0);
    }
}

fn enable_spi3_clock() {
    unsafe {
        // Ungate VSPI clock and pull SPI3 out of reset.
        let cur = core::ptr::read_volatile(DPORT_PERIP_CLK_EN_REG);
        core::ptr::write_volatile(DPORT_PERIP_CLK_EN_REG, cur | DPORT_PERIP_CLK_SPI3_BIT);
        let cur = core::ptr::read_volatile(DPORT_PERIP_RST_EN_REG);
        core::ptr::write_volatile(DPORT_PERIP_RST_EN_REG, cur & !DPORT_PERIP_CLK_SPI3_BIT);
    }
}

/// Program SPI3 for SSD1680: mode 0 (CPOL=0/CPHA=0), MSB-first, ~2 MHz.
///
/// SPI_CLOCK_REG fields (TRM §7.7.2):
///   CLKCNT_L[5:0]   low cycles  - 1
///   CLKCNT_H[11:6]  high cycles - 1
///   CLKCNT_N[17:12] total cycles - 1   (period = N+1 APB cycles)
///   CLKDIV_PRE[30:18] pre-divider - 1
///   CLK_EQU_SYSCLK[31] 1 → bypass divider, use APB direct
///
/// For 80 MHz APB / 40 = 2 MHz: PRE=0 (1), N=39 (period 40), L=19, H=20.
const fn spi_clock_div_2mhz() -> u32 {
    let n = 39;            // 0-39 → period of 40 APB cycles
    let l = 19;            // low half of cycle
    let h = 20;            // high half (l + h + 2 = n + 1 — slight rounding ok)
    let pre = 0;
    (pre << 18) | (n << 12) | (h << 6) | l
}

fn configure_spi3() {
    unsafe {
        // Clock divider: 2 MHz. SSD1680 max is 20 MHz; we're being
        // conservative to tolerate longer breadboard jumper wires.
        core::ptr::write_volatile(SPI3_CLOCK_REG, spi_clock_div_2mhz());
        // SPI_PIN_REG: clear CK_IDLE_EDGE (bit 29) so SCK is low when idle
        // (CPOL=0 — matches SSD1680 mode 0).
        core::ptr::write_volatile(SPI3_PIN_REG, 0);
        // USER: USR_MOSI (bit 27) enables MOSI phase. Bit 7 (CK_OUT_EDGE)
        // = 0 so MOSI is launched on the falling edge before SCK rises
        // (= CPHA=0). Bit 25 (USR_DUMMY) cleared.
        core::ptr::write_volatile(SPI3_USER_REG, 1 << 27);
        // USER2: no command phase (cleared).
        core::ptr::write_volatile(SPI3_USER2_REG, 0);
    }
}

// ----- Boot ---------------------------------------------------------------

#[esp_hal::main]
fn main() -> ! {
    esp_println::println!("[lab] boot");

    // Directly disable the RTC + task watchdogs by poking the register
    // banks ourselves — same effect as `esp_hal::init(...)` but without
    // the ROM-call cascade (memset/ets_delay_us/etc) that the sim only
    // models partially. Keeps the same ELF working in both sim and on
    // real silicon.
    disable_watchdogs();
    esp_println::println!("[lab] watchdogs disabled");

    enable_spi3_clock();
    esp_println::println!("[lab] spi3 clock on");

    configure_spi3();
    esp_println::println!("[lab] spi3 configured (~2 MHz, mode 0)");

    configure_pins();
    esp_println::println!("[lab] pins configured");

    ep_init();
    esp_println::println!("[lab] panel init done");

    ep_stream_plane(0x24, black_plane_byte);
    esp_println::println!("[lab] black plane streamed");

    ep_stream_plane(0x26, red_plane_byte);
    esp_println::println!("[lab] red plane streamed");

    ep_refresh();
    esp_println::println!("[lab] refresh done");

    loop {
        unsafe { core::arch::asm!("waiti 0") };
    }
}
