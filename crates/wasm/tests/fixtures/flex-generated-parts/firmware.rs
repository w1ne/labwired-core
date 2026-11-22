// SPDX-License-Identifier: MIT
// Portable aquarium controller: read the water temperature from a DS18B20
// on a bit-banged 1-Wire line, switch an active-low heater relay below
// 20.00 °C, and show the reading on an LCD1602 I2C backpack, a TM1637
// display and a 3-LED WS2812 ring (blue while heating, green otherwise).
// Nothing here names an MCU: every line and the bus are host ports.
#![no_std]

#[link(wasm_import_module = "labwired")]
extern "C" {
    fn read(port: u32) -> f64;
    fn write(port: u32, value: f64);
    fn advance_ns(delta: u32);
    fn i2c_transfer(
        bus: u32,
        address: u32,
        write_ptr: *const u8,
        write_len: u32,
        read_ptr: *mut u8,
        read_len: u32,
    ) -> u32;
    fn console_write(ptr: *const u8, len: u32);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

fn us(delay: u32) {
    unsafe { advance_ns(delay * 1_000) };
}

fn pin(port: u32, high: bool) {
    unsafe { write(port, if high { 1.0 } else { 0.0 }) };
}

// ── 1-Wire, standard speed (Maxim AN126) ────────────────────────────────

struct OneWire {
    drive: u32,
    line: u32,
}

impl OneWire {
    fn sample(&self) -> bool {
        unsafe { read(self.line) != 0.0 }
    }
    fn reset(&self) -> bool {
        pin(self.drive, false);
        us(480);
        pin(self.drive, true);
        us(70);
        let presence = !self.sample();
        us(410);
        presence
    }
    fn write_bit(&self, bit: bool) {
        pin(self.drive, false);
        us(if bit { 6 } else { 60 });
        pin(self.drive, true);
        us(if bit { 64 } else { 10 });
    }
    fn read_bit(&self) -> bool {
        pin(self.drive, false);
        us(3);
        pin(self.drive, true);
        us(10);
        let bit = self.sample();
        us(57);
        bit
    }
    fn write(&self, byte: u8) {
        for bit in 0..8 {
            self.write_bit(byte >> bit & 1 == 1);
        }
    }
    fn read(&self) -> u8 {
        let mut byte = 0;
        for bit in 0..8 {
            byte |= u8::from(self.read_bit()) << bit;
        }
        byte
    }
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        let mut b = byte;
        for _ in 0..8 {
            let mix = (crc ^ b) & 1;
            crc >>= 1;
            if mix != 0 {
                crc ^= 0x8C;
            }
            b >>= 1;
        }
    }
    crc
}

/// The temperature in 1/16 °C, or `None` with no probe or a bad CRC.
fn read_temperature(bus: &OneWire) -> Option<i16> {
    if !bus.reset() {
        return None;
    }
    bus.write(0xCC); // SKIP ROM
    bus.write(0x44); // CONVERT T
    let mut polls = 0;
    while !bus.read_bit() {
        us(10_000);
        polls += 1;
        if polls > 100 {
            return None;
        }
    }
    if !bus.reset() {
        return None;
    }
    bus.write(0xCC);
    bus.write(0xBE); // READ SCRATCHPAD
    let mut scratchpad = [0u8; 9];
    for byte in scratchpad.iter_mut() {
        *byte = bus.read();
    }
    if crc8(&scratchpad[..8]) != scratchpad[8] {
        return None;
    }
    Some(i16::from_le_bytes([scratchpad[0], scratchpad[1]]))
}

// ── LCD1602 over a PCF8574 backpack (LiquidCrystal_I2C wiring) ───────────

const LCD_ADDRESS: u32 = 0x27;
const BACKLIGHT: u8 = 0x08;
const ENABLE: u8 = 0x04;

fn lcd_port(bus: u32, value: u8) {
    let frame = [value];
    unsafe { i2c_transfer(bus, LCD_ADDRESS, frame.as_ptr(), 1, core::ptr::null_mut(), 0) };
    us(1);
}

fn lcd_nibble(bus: u32, nibble: u8, rs: bool) {
    let port = (nibble << 4) | BACKLIGHT | u8::from(rs);
    lcd_port(bus, port | ENABLE);
    lcd_port(bus, port);
    us(40);
}

fn lcd_byte(bus: u32, byte: u8, rs: bool) {
    lcd_nibble(bus, byte >> 4, rs);
    lcd_nibble(bus, byte & 0x0F, rs);
}

fn lcd_print(bus: u32, text: &[u8]) {
    for &byte in text {
        lcd_byte(bus, byte, true);
    }
}

fn lcd_init(bus: u32) {
    us(50_000);
    for nibble in [0x3, 0x3, 0x3, 0x2] {
        lcd_nibble(bus, nibble, false);
        us(4_500);
    }
    lcd_byte(bus, 0x28, false); // 4-bit, 2 lines, 5x8
    lcd_byte(bus, 0x0C, false); // display on, no cursor
    lcd_byte(bus, 0x01, false); // clear
    us(2_000);
    lcd_byte(bus, 0x06, false); // increment
}

// ── TM1637 ───────────────────────────────────────────────────────────────

const DIGITS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];

struct Tm1637 {
    clk: u32,
    dio: u32,
}

impl Tm1637 {
    fn set(&self, port: u32, high: bool) {
        pin(port, high);
        us(5);
    }
    fn start(&self) {
        self.set(self.dio, false);
    }
    fn stop(&self) {
        self.set(self.clk, false);
        self.set(self.dio, false);
        self.set(self.clk, true);
        self.set(self.dio, true);
    }
    fn byte(&self, byte: u8) {
        for bit in 0..8 {
            self.set(self.clk, false);
            self.set(self.dio, byte >> bit & 1 == 1);
            self.set(self.clk, true);
        }
        self.set(self.clk, false);
        self.set(self.dio, true);
        self.set(self.clk, true);
        self.set(self.clk, false);
    }
    fn show(&self, digits: [u8; 4]) {
        self.start();
        self.byte(0x40);
        self.stop();
        self.start();
        self.byte(0xC0);
        for digit in digits {
            self.byte(DIGITS[digit as usize]);
        }
        self.stop();
        self.start();
        self.byte(0x8F);
        self.stop();
    }
}

// ── WS2812 (T0H 400 ns, T1H 800 ns, 1250 ns per bit) ────────────────────

fn ws2812(port: u32, grb: &[u8]) {
    for &byte in grb {
        for bit in (0..8).rev() {
            let one = byte >> bit & 1 == 1;
            pin(port, true);
            unsafe { advance_ns(if one { 800 } else { 400 }) };
            pin(port, false);
            unsafe { advance_ns(if one { 450 } else { 850 }) };
        }
    }
    us(60);
}

// ── text without an allocator ────────────────────────────────────────────

struct Text {
    bytes: [u8; 40],
    len: usize,
}

impl Text {
    fn new() -> Self {
        Self { bytes: [0; 40], len: 0 }
    }
    fn push(&mut self, text: &[u8]) -> &mut Self {
        for &byte in text {
            self.bytes[self.len] = byte;
            self.len += 1;
        }
        self
    }
    /// A value in hundredths, as `18.50` or `-3.25`.
    fn centi(&mut self, value: i32) -> &mut Self {
        if value < 0 {
            self.push(b"-");
        }
        let value = value.unsigned_abs();
        let whole = value / 100;
        if whole >= 10 {
            self.push(&[b'0' + (whole / 10 % 10) as u8]);
        }
        self.push(&[b'0' + (whole % 10) as u8, b'.']);
        self.push(&[b'0' + (value / 10 % 10) as u8, b'0' + (value % 10) as u8])
    }
    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// One control pass. Returns the raw temperature in 1/16 °C, or 0x8000 when
/// the probe did not answer.
#[no_mangle]
pub extern "C" fn run(
    dq_drive: u32,
    dq: u32,
    heater: u32,
    ring: u32,
    clk: u32,
    dio: u32,
    i2c: u32,
) -> u32 {
    let probe = OneWire { drive: dq_drive, line: dq };
    pin(dq_drive, true);
    lcd_init(i2c);
    let Some(raw) = read_temperature(&probe) else {
        lcd_print(i2c, b"No probe");
        let line = b"no probe\n";
        unsafe { console_write(line.as_ptr(), line.len() as u32) };
        return 0x8000;
    };
    let centi = i32::from(raw) * 100 / 16;
    let heating = centi < 2_000;
    // The relay module input is active-low.
    pin(heater, !heating);

    let mut line1 = Text::new();
    line1.push(b"Water ").centi(centi).push(b"C");
    lcd_print(i2c, line1.as_bytes());
    lcd_byte(i2c, 0x80 | 0x40, false);
    lcd_print(i2c, if heating { b"Heater ON" } else { b"Heater off" });

    let shown = centi.clamp(0, 9_999) as u32;
    Tm1637 { clk, dio }.show([
        (shown / 1000) as u8,
        (shown / 100 % 10) as u8,
        (shown / 10 % 10) as u8,
        (shown % 10) as u8,
    ]);

    let colour: [u8; 3] = if heating { [0x00, 0x00, 0x40] } else { [0x40, 0x00, 0x00] };
    let mut frame = [0u8; 9];
    for led in 0..3 {
        frame[led * 3..led * 3 + 3].copy_from_slice(&colour);
    }
    ws2812(ring, &frame);

    let mut log = Text::new();
    log.push(b"water ")
        .centi(centi)
        .push(if heating { b" C heater on\n" } else { b" C heater off\n" });
    unsafe { console_write(log.as_bytes().as_ptr(), log.len as u32) };
    raw as u16 as u32
}
