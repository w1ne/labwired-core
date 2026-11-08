#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Initialize the ESP32-C3-LCDkit's on-board GC9A01 over SPI2 and paint red.
//! This profile exercises only GPIO, SPI2, the LCD controller and UART0 TX.

use panic_halt as _;
use riscv_rt::entry;

const GPIO: usize = 0x6000_4000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO + 0x24) as *mut u32;
const GPIO_FUNC_OUT_SEL: usize = GPIO + 0x554;
const IO_MUX: usize = 0x6000_9000;

const SPI2: usize = 0x6002_4000;
const SPI_CMD: *mut u32 = (SPI2 + 0x00) as *mut u32;
const SPI_CLOCK: *mut u32 = (SPI2 + 0x0C) as *mut u32;
const SPI_USER: *mut u32 = (SPI2 + 0x10) as *mut u32;
const SPI_USER1: *mut u32 = (SPI2 + 0x14) as *mut u32;
const SPI_MS_DLEN: *mut u32 = (SPI2 + 0x1C) as *mut u32;
const SPI_MISC: *mut u32 = (SPI2 + 0x20) as *mut u32;
const SPI_W0: *mut u32 = (SPI2 + 0x98) as *mut u32;
const SPI_SLAVE: *mut u32 = (SPI2 + 0xE0) as *mut u32;
const SPI_CLK_GATE: *mut u32 = (SPI2 + 0xE8) as *mut u32;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const GPIO_OUT_FUNC: u32 = 128;
const GPIO_MATRIX_FSPICLK: u32 = 63;
const GPIO_MATRIX_FSPID: u32 = 65;
const SPI_UPDATE: u32 = 1 << 23;
const SPI_USR: u32 = 1 << 24;
const LCD_CS: u32 = 1 << 7;
const LCD_DC: u32 = 1 << 2;
const LCD_BACKLIGHT: u32 = 1 << 5;

#[inline(always)]
unsafe fn write_gpio(pin: u32, high: bool) {
    let mask = 1u32 << pin;
    core::ptr::write_volatile(if high { GPIO_OUT_W1TS } else { GPIO_OUT_W1TC }, mask);
}

unsafe fn route_gpio_output(pin: u32) {
    core::ptr::write_volatile(
        (GPIO_FUNC_OUT_SEL + pin as usize * 4) as *mut u32,
        GPIO_OUT_FUNC,
    );
}

unsafe fn route_spi_output(pin: u32, signal: u32) {
    core::ptr::write_volatile((GPIO_FUNC_OUT_SEL + pin as usize * 4) as *mut u32, signal);
}

fn uart_str(text: &str) {
    for byte in text.bytes() {
        unsafe { core::ptr::write_volatile(UART0_FIFO, u32::from(byte)) }
    }
}

unsafe fn spi_write(bytes: &[u8]) {
    for chunk in bytes.chunks(64) {
        for (word_index, word_bytes) in chunk.chunks(4).enumerate() {
            let mut word = 0u32;
            for (byte_index, byte) in word_bytes.iter().enumerate() {
                word |= u32::from(*byte) << (byte_index * 8);
            }
            core::ptr::write_volatile(SPI_W0.add(word_index), word);
        }

        core::ptr::write_volatile(SPI_MS_DLEN, (chunk.len() as u32 * 8) - 1);
        core::ptr::write_volatile(SPI_CMD, SPI_UPDATE);
        while core::ptr::read_volatile(SPI_CMD) & SPI_UPDATE != 0 {}
        write_gpio(7, false);
        core::ptr::write_volatile(SPI_CMD, SPI_USR);
        while core::ptr::read_volatile(SPI_CMD) & SPI_USR != 0 {}
        write_gpio(7, true);
    }
}

unsafe fn lcd_command(command: u8) {
    write_gpio(2, false);
    spi_write(&[command]);
}

unsafe fn lcd_data(data: &[u8]) {
    write_gpio(2, true);
    spi_write(data);
}

unsafe fn lcd_window(x0: u16, y0: u16, x1: u16, y1: u16) {
    let columns = [(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8];
    let rows = [(y0 >> 8) as u8, y0 as u8, (y1 >> 8) as u8, y1 as u8];
    lcd_command(0x2A); // CASET
    lcd_data(&columns);
    lcd_command(0x2B); // RASET
    lcd_data(&rows);
}

#[entry]
fn main() -> ! {
    unsafe {
        // Route C3 FSPI SCK/MOSI to the board's IO1/IO0 LCD pins. Set IO_MUX
        // pads to the GPIO matrix function, then enable their output drivers.
        for pin in [0u32, 1, 2, 5, 7] {
            let mux = (IO_MUX + 4 + pin as usize * 4) as *mut u32;
            let current = core::ptr::read_volatile(mux);
            core::ptr::write_volatile(mux, current & !(0b111 << 12));
        }
        route_spi_output(1, GPIO_MATRIX_FSPICLK);
        route_spi_output(0, GPIO_MATRIX_FSPID);
        route_gpio_output(2);
        route_gpio_output(5);
        route_gpio_output(7);
        core::ptr::write_volatile(
            GPIO_ENABLE_W1TS,
            (1 << 0) | (1 << 1) | LCD_DC | LCD_BACKLIGHT | LCD_CS,
        );

        write_gpio(7, true); // deselect LCD
        write_gpio(2, false); // command level
        write_gpio(5, false); // backlight off while initializing

        core::ptr::write_volatile(SPI_CLK_GATE, 7);
        core::ptr::write_volatile(SPI_SLAVE, 0);
        // APB source / (N + 1) = 80 MHz / 20 = 4 MHz. H=((N + 1)/2)-1,
        // L=N; PRE=0. A conservative panel clock keeps the smoke simple.
        core::ptr::write_volatile(SPI_CLOCK, (19 << 12) | (9 << 6) | 19);
        core::ptr::write_volatile(SPI_USER, (1 << 6) | (1 << 7) | (1 << 27));
        core::ptr::write_volatile(SPI_USER1, 0);
        core::ptr::write_volatile(SPI_MISC, 0x3E);

        lcd_command(0x11); // SLPOUT
        lcd_command(0x36); // MADCTL: top-left origin
        lcd_data(&[0x00]);
        lcd_command(0x3A); // COLMOD (the model stores RGB565)
        lcd_data(&[0x55]);
        lcd_window(0, 0, 7, 7);
        lcd_command(0x2C); // RAMWR
        let mut red = [0u8; 8 * 8 * 2];
        for pixel in red.chunks_exact_mut(2) {
            pixel.copy_from_slice(&[0xF8, 0x00]);
        }
        lcd_data(&red);
        lcd_command(0x29); // DISPON
        write_gpio(5, true); // LCD_BL_CTRL
        uart_str("ESP32_C3_LCDKIT_GC9A01_OK\n");
    }

    loop {}
}
