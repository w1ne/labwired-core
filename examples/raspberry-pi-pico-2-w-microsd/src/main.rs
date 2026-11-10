#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// Raspberry Pi Pico 2 W / RP2350 register map.
const UART0_DR: *mut u32 = 0x4007_0000 as *mut u32;
const SPI0: usize = 0x4008_0000;
const SPI0_CR0: *mut u32 = (SPI0 + 0x00) as *mut u32;
const SPI0_CR1: *mut u32 = (SPI0 + 0x04) as *mut u32;
const SPI0_DR: *mut u32 = (SPI0 + 0x08) as *mut u32;
const SPI0_SR: *const u32 = (SPI0 + 0x0c) as *const u32;
const SPI0_CPSR: *mut u32 = (SPI0 + 0x10) as *mut u32;

const IO_BANK0: usize = 0x4002_8000;
const GPIO16_CTRL: *mut u32 = (IO_BANK0 + 8 * 16 + 4) as *mut u32;
const GPIO17_CTRL: *mut u32 = (IO_BANK0 + 8 * 17 + 4) as *mut u32;
const GPIO18_CTRL: *mut u32 = (IO_BANK0 + 8 * 18 + 4) as *mut u32;
const GPIO19_CTRL: *mut u32 = (IO_BANK0 + 8 * 19 + 4) as *mut u32;
const SIO_GPIO_OUT_SET: *mut u32 = 0xD000_0014 as *mut u32;
const SIO_GPIO_OUT_CLR: *mut u32 = 0xD000_0018 as *mut u32;
const SIO_GPIO_OE_SET: *mut u32 = 0xD000_0024 as *mut u32;

const GPIO_FUNC_SPI: u32 = 1;
const GPIO_FUNC_SIO: u32 = 5;
const CARD_CS_MASK: u32 = 1 << 17;
const SSP_SR_TNF: u32 = 1 << 1;
const SSP_SR_RNE: u32 = 1 << 2;
const MAX_POLLS: usize = 64;
const README_LBA: u32 = 290;
const README: &[u8] = b"LabWired SD card demo.\r\n";

#[entry]
fn main() -> ! {
    unsafe {
        setup_pins();
        setup_spi0();

        let result = read_demo_file();
        if result {
            print_uart("PICO2W_MICROSD_READ=PASS\n");
        } else {
            print_uart("PICO2W_MICROSD_READ=FAIL\n");
        }
    }

    loop {
        cortex_m::asm::wfi();
    }
}

unsafe fn setup_pins() {
    // Pico 2 W header mapping: GP16=MISO, GP17=CS, GP18=SCK, GP19=MOSI.
    core::ptr::write_volatile(GPIO16_CTRL, GPIO_FUNC_SPI);
    core::ptr::write_volatile(GPIO18_CTRL, GPIO_FUNC_SPI);
    core::ptr::write_volatile(GPIO19_CTRL, GPIO_FUNC_SPI);
    core::ptr::write_volatile(GPIO17_CTRL, GPIO_FUNC_SIO);
    core::ptr::write_volatile(SIO_GPIO_OE_SET, CARD_CS_MASK);
    core::ptr::write_volatile(SIO_GPIO_OUT_SET, CARD_CS_MASK);
}

unsafe fn setup_spi0() {
    // PL022 mode 0, 8-bit words. At the RP2350 150 MHz system clock, CPSDVSR=2
    // and SCR=255 keep initial SD-card clocks below the 400 kHz startup limit.
    core::ptr::write_volatile(SPI0_CR1, 0);
    core::ptr::write_volatile(SPI0_CPSR, 2);
    core::ptr::write_volatile(SPI0_CR0, (255 << 8) | 7);
    core::ptr::write_volatile(SPI0_CR1, 1 << 1);
}

unsafe fn read_demo_file() -> bool {
    // Supply at least 74 clocks with CS high before the first command.
    for _ in 0..10 {
        let _ = spi_transfer(0xFF);
    }
    card_select(true);

    if send_command(0, 0, 0x95) != Some(0x01) {
        card_select(false);
        return false;
    }

    // CMD8 / R7: voltage range and check pattern must echo 0x000001AA.
    if send_command(8, 0x0000_01AA, 0x87) != Some(0x01) {
        card_select(false);
        return false;
    }
    let r7 = [
        spi_transfer(0xFF),
        spi_transfer(0xFF),
        spi_transfer(0xFF),
        spi_transfer(0xFF),
    ];
    if r7 != [0, 0, 1, 0xAA] {
        card_select(false);
        return false;
    }

    // ACMD41 exits idle, then CMD58 confirms an SDHC card.
    if send_command(55, 0, 0x01) != Some(0x01)
        || send_command(41, 0x4000_0000, 0x01) != Some(0x00)
        || send_command(58, 0, 0x01) != Some(0x00)
    {
        card_select(false);
        return false;
    }
    let ocr = [
        spi_transfer(0xFF),
        spi_transfer(0xFF),
        spi_transfer(0xFF),
        spi_transfer(0xFF),
    ];
    if ocr != [0xC0, 0xFF, 0x80, 0] {
        card_select(false);
        return false;
    }

    // Read the first data sector of the fixed FAT16 /README.TXT demo file.
    if send_command(17, README_LBA, 0x01) != Some(0x00) {
        card_select(false);
        return false;
    }
    let mut saw_token = false;
    for _ in 0..MAX_POLLS {
        if spi_transfer(0xFF) == 0xFE {
            saw_token = true;
            break;
        }
    }
    if !saw_token {
        card_select(false);
        return false;
    }

    for expected in README {
        if spi_transfer(0xFF) != *expected {
            card_select(false);
            return false;
        }
    }
    card_select(false);
    true
}

unsafe fn send_command(command: u8, argument: u32, crc: u8) -> Option<u8> {
    let _ = spi_transfer(0x40 | command);
    let _ = spi_transfer((argument >> 24) as u8);
    let _ = spi_transfer((argument >> 16) as u8);
    let _ = spi_transfer((argument >> 8) as u8);
    let _ = spi_transfer(argument as u8);
    let _ = spi_transfer(crc);

    for _ in 0..MAX_POLLS {
        let response = spi_transfer(0xFF);
        if response & 0x80 == 0 {
            return Some(response);
        }
    }
    None
}

unsafe fn spi_transfer(byte: u8) -> u8 {
    for _ in 0..MAX_POLLS {
        if core::ptr::read_volatile(SPI0_SR) & SSP_SR_TNF != 0 {
            break;
        }
    }
    core::ptr::write_volatile(SPI0_DR, u32::from(byte));
    for _ in 0..MAX_POLLS {
        if core::ptr::read_volatile(SPI0_SR) & SSP_SR_RNE != 0 {
            return core::ptr::read_volatile(SPI0_DR) as u8;
        }
    }
    0xFF
}

unsafe fn card_select(selected: bool) {
    if selected {
        core::ptr::write_volatile(SIO_GPIO_OUT_CLR, CARD_CS_MASK);
    } else {
        core::ptr::write_volatile(SIO_GPIO_OUT_SET, CARD_CS_MASK);
    }
}

fn print_uart(text: &str) {
    for byte in text.bytes() {
        unsafe { core::ptr::write_volatile(UART0_DR, u32::from(byte)) };
    }
}
