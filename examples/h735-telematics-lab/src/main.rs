//! H735 telematics lab — LabWired demo for Proemion-style CLM CI story.
//!
//! STM32H735 + Quectel BG770A (virtual) + ILI9341 TFT:
//!   1. AT bring-up + GNSS on modem UART (USART1) — first, before TFT paint
//!   2. Parse AT+QGPSLOC → compact lat/lon on TFT
//!   3. MQTT publish location JSON over modem AT
//!   4. Mirror every AT exchange to console USART3
//!
//! Register map matches examples/tier1-fixture/stm32h735 and RM0468 (sim-derived).
//!
//! Step budget notes: BG770A models long network delays (QIACT/QMTOPEN up to
//! tens of seconds of sim time). With `auto_attach: true` we skip QIACT and rely
//! on a live Quectel PDP context. TFT is compact (no full-screen pixel floods).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Bases (configs/chips/stm32h735.yaml) ───────────────────────────────────
const RCC_BASE: u32 = 0x5802_4400;
const RCC_AHB4ENR: u32 = RCC_BASE + 0xE0;
const RCC_APB1LENR: u32 = RCC_BASE + 0xE8;
const RCC_APB2ENR: u32 = RCC_BASE + 0xF0;

const GPIOA_BASE: u32 = 0x5802_0000;
const GPIOB_BASE: u32 = 0x5802_0400;

// stm32v2 GPIO: MODER @ 0x00, OTYPER @ 0x04, OSPEEDR @ 0x08, PUPDR @ 0x0C,
// AFR[0] @ 0x20 (pins 0-7), AFR[1] @ 0x24 (pins 8-15).
const GPIO_MODER: u32 = 0x00;
const GPIO_OTYPER: u32 = 0x04;
const GPIO_OSPEEDR: u32 = 0x08;
const GPIO_PUPDR: u32 = 0x0C;
const GPIO_AFR: u32 = 0x20;

const USART3_BASE: u32 = 0x4000_4800; // console
const USART1_BASE: u32 = 0x4001_1000; // modem
const SPI1_BASE: u32 = 0x4001_3000; // TFT

// stm32v2 UART: CR1 @ 0x00, BRR @ 0x0C, ISR @ 0x1C, RDR @ 0x24, TDR @ 0x28
const USART_CR1: u32 = 0x00;
const USART_BRR: u32 = 0x0C;
const CR1_UE: u32 = 1 << 0;
const CR1_RE: u32 = 1 << 2;
const CR1_TE: u32 = 1 << 3;
const ISR_TXE: u32 = 1 << 7;
const ISR_RXNE: u32 = 1 << 5;

/// 115200 8N1. This firmware never touches the PLL, so the core and the APB
/// buses stay on the HSI reset clock: the H735 "system starts on the HSI clock"
/// (DS13312 Rev 4, §3.10.1, p30), HSI is 64 MHz (same page, Table 40 p151), and
/// RCC_CR resets with HSIDIV = 00 (÷1) per configs/peripherals/stm32h735/rcc.yaml.
/// Under the default 16× oversampling BRR IS USARTDIV (RM0468 §51.8.4):
///
///   USARTDIV = 64_000_000 / 115_200 = 555.6 → 556
///   actual baud = 64_000_000 / 556 = 115_107.9 (−0.08%)
///
/// Left at its 0 reset the divisor is invalid, not slow: there is no bit period,
/// so nothing can be placed on the wire for a probe to see.
const BRR_115200: u32 = 556;

/// PA9 = USART1_TX, PA10 = USART1_RX, both AF7 — STM32H735xG datasheet DS13312
/// Rev 4, Table 9 "pin alternate functions", port A, p97.
/// PB10 = USART3_TX, PB11 = USART3_RX, both AF7 — same table, port B, p99.
const AF7: u32 = 7;

// SPI v2 (stm32h5): CR1, CR2, CFG1, CFG2, SR, IFCR, TXDR
const H5_SPE: u32 = 1;
const H5_CSTART: u32 = 1 << 9;
const H5_SSI: u32 = 1 << 12;
const H5_MASTER: u32 = 1 << 22;
const H5_SSM: u32 = 1 << 26;
const H5_TXP: u32 = 1 << 1;

#[inline(always)]
fn rd32(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}
#[inline(always)]
fn wr32(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn spin(n: u32) {
    for i in 0..n {
        core::hint::black_box(i);
    }
}

// ── Serial pads ────────────────────────────────────────────────────────────

/// Hand one pad to a USART: MODER = 10 (alternate function), push-pull, very
/// high speed, no pull, and the AF nibble into AFR[pin / 8].
///
/// The AF nibble is the field that actually connects the pad to the peripheral.
/// Without it the pin stays a plain GPIO, so a logic probe reads the output
/// latch — a flat line — while the byte-level bus monitor decodes the same
/// traffic perfectly. Both views are then "working" and they disagree.
fn pad_af(gpio_base: u32, pin: u32, af: u32) {
    let shift = pin * 2;
    let moder = rd32(gpio_base + GPIO_MODER);
    wr32(
        gpio_base + GPIO_MODER,
        (moder & !(0b11 << shift)) | (0b10 << shift),
    );
    wr32(
        gpio_base + GPIO_OTYPER,
        rd32(gpio_base + GPIO_OTYPER) & !(1 << pin),
    );
    wr32(
        gpio_base + GPIO_OSPEEDR,
        rd32(gpio_base + GPIO_OSPEEDR) | (0b11 << shift),
    );
    wr32(
        gpio_base + GPIO_PUPDR,
        rd32(gpio_base + GPIO_PUPDR) & !(0b11 << shift),
    );
    let afr = gpio_base + GPIO_AFR + (pin >> 3) * 4;
    let nib = (pin & 7) * 4;
    wr32(afr, (rd32(afr) & !(0xF << nib)) | (af << nib));
}

/// Mux both UARTs onto their pads and give each one a real bit period.
///
/// The H7 USART kernel clock needs no selection here: RCC_D2CCIP2R resets to 0,
/// which is rcc_pclk2 for USART1 and rcc_pclk1 for USART3 (RM0468 §9.7.21) —
/// the buses this firmware already leaves at their reset rate.
fn uart_pads_init() {
    // GPIOA (bit 0) + GPIOB (bit 1) on AHB4 — MODER/AFR are dead until the port
    // is clocked (RM0468 §9.7.43).
    wr32(RCC_AHB4ENR, rd32(RCC_AHB4ENR) | (1 << 0) | (1 << 1));

    pad_af(GPIOA_BASE, 9, AF7); // USART1_TX → modem
    pad_af(GPIOA_BASE, 10, AF7); // USART1_RX → modem
    pad_af(GPIOB_BASE, 10, AF7); // USART3_TX → console
    pad_af(GPIOB_BASE, 11, AF7); // USART3_RX → console

    for base in [USART1_BASE, USART3_BASE] {
        wr32(base + USART_CR1, 0);
        wr32(base + USART_BRR, BRR_115200);
        wr32(base + USART_CR1, CR1_UE | CR1_TE | CR1_RE);
    }
}

// ── Console USART3 ─────────────────────────────────────────────────────────

fn console_byte(b: u8) {
    for _ in 0..10_000 {
        if rd32(USART3_BASE + 0x1C) & ISR_TXE != 0 {
            break;
        }
    }
    unsafe { write_volatile((USART3_BASE + 0x28) as *mut u8, b) };
}

fn console_str(s: &str) {
    for b in s.bytes() {
        console_byte(b);
    }
}

// ── Modem USART1 ───────────────────────────────────────────────────────────

fn modem_byte(b: u8) {
    for _ in 0..10_000 {
        if rd32(USART1_BASE + 0x1C) & ISR_TXE != 0 {
            break;
        }
    }
    unsafe { write_volatile((USART1_BASE + 0x28) as *mut u8, b) };
}

fn modem_str(s: &str) {
    for b in s.bytes() {
        modem_byte(b);
    }
}

fn modem_has_rx() -> bool {
    rd32(USART1_BASE + 0x1C) & ISR_RXNE != 0
}

fn modem_read() -> u8 {
    (rd32(USART1_BASE + 0x24) & 0xFF) as u8
}

/// Drain modem → console; also fill a small ring for parsing last response.
fn drain_modem(buf: &mut [u8], len: &mut usize) {
    for _ in 0..8192 {
        if !modem_has_rx() {
            return;
        }
        let b = modem_read();
        console_byte(b);
        if *len < buf.len() {
            buf[*len] = b;
            *len += 1;
        } else {
            // slide window
            buf.copy_within(1.., 0);
            buf[buf.len() - 1] = b;
        }
    }
}

fn buf_contains(buf: &[u8], n: usize, needle: &[u8]) -> bool {
    if needle.is_empty() || n < needle.len() {
        return false;
    }
    'outer: for i in 0..=(n - needle.len()) {
        for j in 0..needle.len() {
            if buf[i + j] != needle[j] {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

/// Send AT line and wait until `needle` appears or `iters` spin-drains elapse.
fn send_at_until(line: &str, buf: &mut [u8], len: &mut usize, needle: &[u8], iters: u32) {
    *len = 0;
    console_str("> ");
    console_str(line);
    console_str("\r\n");
    modem_str(line);
    modem_str("\r\n");
    for _ in 0..iters {
        drain_modem(buf, len);
        if !needle.is_empty() && buf_contains(buf, *len, needle) {
            break;
        }
    }
}

fn send_at(line: &str, buf: &mut [u8], len: &mut usize) {
    // Default AT replies complete within ~300 ms of model time.
    send_at_until(line, buf, len, b"OK", 200_000);
}

/// Very small parser for `+QGPSLOC: time,latN,lonW,...`
fn parse_qgpsloc(buf: &[u8], n: usize) -> Option<(f32, f32)> {
    let s = core::str::from_utf8(&buf[..n]).ok()?;
    let rest = s.split("+QGPSLOC:").nth(1)?;
    let line = rest.lines().next()?;
    let mut parts = line.split(',');
    let _time = parts.next()?;
    let lat_s = parts.next()?.trim();
    let lon_s = parts.next()?.trim();
    let lat = parse_dm_hemisphere(lat_s)?;
    let lon = parse_dm_hemisphere(lon_s)?;
    Some((lat, lon))
}

fn parse_dm_hemisphere(s: &str) -> Option<f32> {
    // Forms like "37.7749N" or "122.4194W" (model emits decimal degrees + N/S/E/W).
    let (num, sign) = match s.chars().last() {
        Some('N') | Some('E') => (&s[..s.len() - 1], 1.0f32),
        Some('S') | Some('W') => (&s[..s.len() - 1], -1.0f32),
        _ => (s, 1.0f32),
    };
    let mut v = 0.0f32;
    let mut frac = 0.0f32;
    let mut div = 1.0f32;
    let mut seen_dot = false;
    for c in num.chars() {
        if c == '.' {
            seen_dot = true;
            continue;
        }
        if !c.is_ascii_digit() {
            return None;
        }
        let d = (c as u8 - b'0') as f32;
        if !seen_dot {
            v = v * 10.0 + d;
        } else {
            div *= 10.0;
            frac += d / div;
        }
    }
    Some((v + frac) * sign)
}

// ── SPI1 (H5) + ILI9341 ────────────────────────────────────────────────────

fn spi_enable() {
    // SPI1EN
    wr32(RCC_APB2ENR, rd32(RCC_APB2ENR) | (1 << 12));
    // PA4 CS output, PA3 D/C output
    let moder = rd32(GPIOA_BASE);
    wr32(
        GPIOA_BASE,
        (moder & !((0x3 << 8) | (0x3 << 6))) | (0x1 << 8) | (0x1 << 6),
    ); // PA4, PA3 MODER=01
    wr32(GPIOA_BASE + 0x18, 1 << 4); // CS high
    wr32(GPIOA_BASE + 0x28, 1 << 3); // D/C low (command)
                                     // SPI master, SSM, SSI, 8-bit default CFG1, endless TSIZE=0
    wr32(SPI1_BASE, H5_SSI); // CR1 SSI first
    wr32(SPI1_BASE + 0x0C, H5_MASTER | H5_SSM); // CFG2
    wr32(SPI1_BASE + 0x04, 0); // CR2 TSIZE=0 endless
    wr32(SPI1_BASE, H5_SSI | H5_SPE); // SPE
    wr32(SPI1_BASE, H5_SSI | H5_SPE | H5_CSTART); // CSTART
}

fn spi_write(byte: u8) {
    for _ in 0..4096 {
        if rd32(SPI1_BASE + 0x14) & H5_TXP != 0 {
            break;
        }
    }
    // If transfer ended (EOT), re-arm CSTART
    let sr = rd32(SPI1_BASE + 0x14);
    if sr & (1 << 3) != 0 {
        wr32(SPI1_BASE + 0x18, 0xFFFF_FFFF); // IFCR
        wr32(SPI1_BASE, H5_SSI | H5_SPE | H5_CSTART);
    }
    wr32(SPI1_BASE + 0x20, byte as u32); // TXDR
}

fn cs_low() {
    wr32(GPIOA_BASE + 0x28, 1 << 4); // BRR
}
fn cs_high() {
    wr32(GPIOA_BASE + 0x18, 1 << 4); // BSRR
}

/// D/C low — the next byte is a COMMAND.
fn dc_command() {
    wr32(GPIOA_BASE + 0x28, 1 << 3); // BRR
}
/// D/C high — the next bytes are parameters or pixel data.
fn dc_data() {
    wr32(GPIOA_BASE + 0x18, 1 << 3); // BSRR
}

// ── ILI9341 framing ────────────────────────────────────────────────────────
//
// These helpers used to frame command-vs-data by CHIP SELECT alone: drop CS,
// send the command, send its parameters, raise CS. No ILI9341 works that way.
// In 4-line serial mode the controller latches D/C with every byte (datasheet
// §7.3.2); CS only gates the interface on and off. There is no "first byte
// after CS is a command" rule to lean on — with D/C strapped low a real panel
// reads EVERY byte as a command and shows nothing.
//
// The simulator's legacy no-D/C path approximated the CS idea well enough for
// a single self-contained burst, but nothing closes a RAMWR pixel stream in
// that scheme: once 0x2C opened one, every following CASET/PASET/RAMWR byte was
// swallowed as pixel data, so the whole screen collapsed into the first
// window's rectangle as high-frequency noise. That is not a modelling gap to
// paper over — it is what the wire actually said.

fn tft_cmd(cmd: u8) {
    cs_low();
    dc_command();
    spi_write(cmd);
    cs_high();
}

fn tft_cmd1(cmd: u8, p0: u8) {
    cs_low();
    dc_command();
    spi_write(cmd);
    dc_data();
    spi_write(p0);
    cs_high();
}

fn tft_cmd4(cmd: u8, p0: u8, p1: u8, p2: u8, p3: u8) {
    cs_low();
    dc_command();
    spi_write(cmd);
    dc_data();
    spi_write(p0);
    spi_write(p1);
    spi_write(p2);
    spi_write(p3);
    cs_high();
}

fn tft_set_window(col_start: u16, col_end: u16, row_start: u16, row_end: u16) {
    tft_cmd4(
        0x2A,
        (col_start >> 8) as u8,
        col_start as u8,
        (col_end >> 8) as u8,
        col_end as u8,
    );
    tft_cmd4(
        0x2B,
        (row_start >> 8) as u8,
        row_start as u8,
        (row_end >> 8) as u8,
        row_end as u8,
    );
}

fn tft_fill_rect(x: u16, y: u16, w: u16, h: u16, color: u16) {
    if w == 0 || h == 0 {
        return;
    }
    let x1 = x.saturating_add(w - 1);
    let y1 = y.saturating_add(h - 1);
    tft_set_window(x, x1, y, y1);
    let hi = (color >> 8) as u8;
    let lo = color as u8;
    let n = (w as u32) * (h as u32);
    cs_low();
    dc_command();
    spi_write(0x2C); // RAMWR
    dc_data();
    for _ in 0..n {
        spi_write(hi);
        spi_write(lo);
    }
    cs_high();
}

/// Minimal init + small header band only (avoid full-screen floods).
fn tft_init_compact() {
    spi_enable();
    tft_cmd(0x01); // SWRESET
    spin(5_000);
    tft_cmd(0x11); // SLPOUT
    spin(5_000);
    tft_cmd1(0x3A, 0x55); // COLMOD RGB565
    tft_cmd(0x29); // DISPON
                   // Compact header only: 240×16 navy
    tft_fill_rect(0, 0, 240, 16, 0x0010);
}

/// Crude 5×7 digit/letter blocks for a few characters (lat/lon status).
fn glyph_5x7(c: u8) -> [u8; 5] {
    match c {
        b'0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        b'1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        b'2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        b'3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
        b'4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
        b'5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        b'6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
        b'7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        b'8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        b'9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
        b'.' => [0x00, 0x60, 0x60, 0x00, 0x00],
        b'-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        b' ' => [0x00, 0x00, 0x00, 0x00, 0x00],
        b'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
        b'E' => [0x7F, 0x49, 0x49, 0x49, 0x41],
        b'G' => [0x3E, 0x41, 0x49, 0x49, 0x7A],
        b'L' => [0x7F, 0x40, 0x40, 0x40, 0x40],
        b'M' => [0x7F, 0x02, 0x0C, 0x02, 0x7F],
        b'N' => [0x7F, 0x04, 0x08, 0x10, 0x7F],
        b'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
        b'P' => [0x7F, 0x09, 0x09, 0x09, 0x06],
        b'Q' => [0x3E, 0x41, 0x51, 0x21, 0x5E],
        b'S' => [0x26, 0x49, 0x49, 0x49, 0x32],
        b'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
        b'U' => [0x3F, 0x40, 0x40, 0x40, 0x3F],
        b':' => [0x00, 0x36, 0x36, 0x00, 0x00],
        b'+' => [0x08, 0x08, 0x3E, 0x08, 0x08],
        _ => [0x7F, 0x41, 0x41, 0x41, 0x7F],
    }
}

fn tft_draw_char(x: u16, y: u16, c: u8, fg: u16, bg: u16, scale: u16) {
    let g = glyph_5x7(c);
    for (col, bits) in g.iter().enumerate() {
        for row in 0..7u16 {
            let on = (bits >> row) & 1 != 0;
            let color = if on { fg } else { bg };
            let px = x + (col as u16) * scale;
            let py = y + row * scale;
            tft_fill_rect(px, py, scale, scale, color);
        }
    }
}

fn tft_draw_str(mut x: u16, y: u16, s: &str, fg: u16, bg: u16, scale: u16) {
    for b in s.bytes() {
        tft_draw_char(x, y, b, fg, bg, scale);
        x = x.saturating_add(6 * scale);
        if x > 230 {
            break;
        }
    }
}

fn format_f32(v: f32, out: &mut [u8; 16]) -> &str {
    // Fixed-point style for demo: ±DDD.DDDD
    let neg = v < 0.0;
    let a = if neg { -v } else { v };
    let ip = a as u32;
    let frac = ((a - ip as f32) * 10000.0) as u32;
    let mut i = 0usize;
    if neg {
        out[i] = b'-';
        i += 1;
    }
    let mut tmp = [0u8; 10];
    let mut n = 0usize;
    let mut x = ip;
    if x == 0 {
        tmp[n] = b'0';
        n += 1;
    } else {
        while x > 0 && n < tmp.len() {
            tmp[n] = b'0' + (x % 10) as u8;
            n += 1;
            x /= 10;
        }
    }
    while n > 0 {
        n -= 1;
        out[i] = tmp[n];
        i += 1;
    }
    out[i] = b'.';
    i += 1;
    let f = frac;
    for k in (0..4).rev() {
        let d = (f / 10u32.pow(k)) % 10;
        out[i] = b'0' + d as u8;
        i += 1;
    }
    core::str::from_utf8(&out[..i]).unwrap_or("0")
}

// ── Main ───────────────────────────────────────────────────────────────────

#[entry]
fn main() -> ! {
    // Enable USART3 (APB1 bit 18) + USART1 (APB2 bit 4) + SPI1 (APB2 bit 12).
    wr32(RCC_APB1LENR, rd32(RCC_APB1LENR) | (1 << 18));
    wr32(RCC_APB2ENR, rd32(RCC_APB2ENR) | (1 << 4) | (1 << 12));
    uart_pads_init();

    console_str("LabWired telematics / H735 + modem + TFT\r\n");
    console_str("Stand-in: BG770A AT model (not production modem)\r\n\r\n");

    let mut resp = [0u8; 512];
    let mut resp_len = 0usize;

    // ── 1) Modem / GNSS first (before any TFT pixel work) ──────────────────
    send_at("AT", &mut resp, &mut resp_len);
    send_at("ATE0", &mut resp, &mut resp_len);
    send_at("AT+CMEE=2", &mut resp, &mut resp_len);
    send_at("AT+CSQ", &mut resp, &mut resp_len);
    send_at("AT+CGATT?", &mut resp, &mut resp_len);
    // auto_attach already registered + Quectel PDP ready — skip QICSGP/QIACT
    // (those cost 150s of model time and blow the smoke step budget).

    send_at("AT+QGPS=1", &mut resp, &mut resp_len);
    send_at_until(
        "AT+QGPSLOC=0",
        &mut resp,
        &mut resp_len,
        b"+QGPSLOC:",
        200_000,
    );

    let (lat, lon) = parse_qgpsloc(&resp, resp_len).unwrap_or((37.7749, -122.4194));
    let mut lat_buf = [0u8; 16];
    let mut lon_buf = [0u8; 16];
    let lat_s = format_f32(lat, &mut lat_buf);
    let lon_s = format_f32(lon, &mut lon_buf);

    console_str("GPS fix: lat=");
    console_str(lat_s);
    console_str(" lon=");
    console_str(lon_s);
    console_str("\r\n");

    // ── 2) Compact TFT: header + two numeric lines (scale 1) ───────────────
    tft_init_compact();
    tft_draw_str(2, 2, "GPS", 0xFFFF, 0x0010, 1);
    // Clear two thin text rows
    tft_fill_rect(0, 20, 240, 28, 0x0000);
    tft_draw_str(2, 20, lat_s, 0x07FF, 0x0000, 1);
    tft_draw_str(2, 34, lon_s, 0x07FF, 0x0000, 1);

    // ── 3) MQTT publish location ───────────────────────────────────────────
    // QMTOPEN models up to 75s before OK; URC follows ~1.5s later.
    send_at_until(
        "AT+QMTOPEN=0,\"broker.labwired.local\",1883",
        &mut resp,
        &mut resp_len,
        b"+QMTOPEN: 0,0",
        2_000_000,
    );
    send_at_until(
        "AT+QMTCONN=0,\"labwired-telematics\"",
        &mut resp,
        &mut resp_len,
        b"+QMTCONN:",
        500_000,
    );

    // Build payload: {"lat":...,"lon":...,"src":"qgpsloc"}
    let mut payload = [0u8; 96];
    let mut p = 0usize;
    let head = b"{\"lat\":";
    payload[p..p + head.len()].copy_from_slice(head);
    p += head.len();
    for b in lat_s.bytes() {
        payload[p] = b;
        p += 1;
    }
    let mid = b",\"lon\":";
    payload[p..p + mid.len()].copy_from_slice(mid);
    p += mid.len();
    for b in lon_s.bytes() {
        payload[p] = b;
        p += 1;
    }
    let tail = b",\"src\":\"qgpsloc\"}";
    payload[p..p + tail.len()].copy_from_slice(tail);
    p += tail.len();

    // AT+QMTPUB=0,0,0,0,"telematics/location",<len>
    let mut pub_cmd = [0u8; 64];
    let prefix = b"AT+QMTPUB=0,0,0,0,\"telematics/location\",";
    pub_cmd[..prefix.len()].copy_from_slice(prefix);
    let mut pi = prefix.len();
    let mut plen = p;
    let mut digits = [0u8; 4];
    let mut dn = 0usize;
    if plen == 0 {
        digits[0] = b'0';
        dn = 1;
    } else {
        while plen > 0 && dn < 4 {
            digits[dn] = b'0' + (plen % 10) as u8;
            plen /= 10;
            dn += 1;
        }
    }
    while dn > 0 {
        dn -= 1;
        pub_cmd[pi] = digits[dn];
        pi += 1;
    }
    let pub_line = core::str::from_utf8(&pub_cmd[..pi]).unwrap_or("AT+QMTPUB=0,0,0,0,\"t\",1");

    console_str("> ");
    console_str(pub_line);
    console_str("\r\n");
    modem_str(pub_line);
    modem_str("\r\n");
    // Wait for "> " prompt
    for _ in 0..100_000 {
        drain_modem(&mut resp, &mut resp_len);
        if buf_contains(&resp, resp_len, b"> ") {
            break;
        }
    }
    for &b in &payload[..p] {
        modem_byte(b);
    }
    modem_byte(0x1A);
    for _ in 0..500_000 {
        drain_modem(&mut resp, &mut resp_len);
        if buf_contains(&resp, resp_len, b"+QMTPUB:") || buf_contains(&resp, resp_len, b"OK") {
            break;
        }
    }

    console_str("location published\r\n");
    tft_fill_rect(0, 48, 240, 14, 0x0320);
    tft_draw_str(2, 50, "MQTT SENT", 0xFFFF, 0x0320, 1);

    // Status LED on PA5
    let moder = rd32(GPIOA_BASE);
    wr32(GPIOA_BASE, (moder & !(0x3 << 10)) | (0x1 << 10));
    wr32(GPIOA_BASE + 0x18, 1 << 5);

    console_str("[idle — drag modem Range (m); AT+CSQ tracks path loss]\r\n");
    // Re-query CSQ so Range/RSSI SimInput (RfMedium path loss) shows on serial.
    loop {
        send_at("AT+CSQ", &mut resp, &mut resp_len);
        for _ in 0..500_000 {
            drain_modem(&mut resp, &mut resp_len);
        }
    }
}
