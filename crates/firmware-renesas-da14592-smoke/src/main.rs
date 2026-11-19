#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// Renode platforms/cpus/renesas-da14592.repl instance bases. UART = DesignWare
// APB 16550 with a 4-byte stride (THR @0x00, LSR @0x14). GPIO offsets from
// Renode RenesasDA14_GPIO.cs; driver use from Zephyr gpio_smartbond.c.
const UART0: usize = 0x5002_0000;
const UART1: usize = 0x5002_0100;
const UART_THR: usize = 0x00;
const UART_LSR: usize = 0x14;
const LSR_THRE: u32 = 1 << 5;
const LSR_TEMT: u32 = 1 << 6;

const GPIO: usize = 0x5002_0600;
const P0_DATA: usize = GPIO;
const P1_DATA: usize = GPIO + 0x04;
const P0_SET: usize = GPIO + 0x08;
const P1_SET: usize = GPIO + 0x0C;
const P0_RESET: usize = GPIO + 0x10;
const P1_RESET: usize = GPIO + 0x14;
const P0_WEAK: usize = GPIO + 0xA4;
const P1_WEAK: usize = GPIO + 0xA8;
// PUPD [9:8]: 0 input, 1 pull-up, 2 pull-down (reset), 3 output.
const MODE_IN_PU: u32 = 0x100;
const MODE_IN_PD: u32 = 0x200;
const MODE_OUT: u32 = 0x300;

const fn mode(port: usize, pin: usize) -> usize {
    GPIO + 0x18 + 4 * (16 * port + pin)
}

const LED: u32 = 1 << 5; // P0.5: pulsed for the gpio_edges assertion
const P1_PIN: u32 = 1 << 2; // P1.2
const PULL_PIN: usize = 3; // P0.3

const SRAM: usize = 0x2000_0100;
const SRAM_LOW: usize = 0x0080_0100;
const SRAM_TOP_WORD: usize = 0x2001_7FFC;
const ROM_MIRROR: usize = 0x0090_0000;
const EFLASH_DATA: usize = 0x3100_0010;
const EFLASH_DATA_LOW: usize = 0x00A0_0010;
// Smoke summary: 0xDA140000 | pass bits.
const SUMMARY: usize = 0x2001_7F00;

fn rd(addr: usize) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: usize, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn putc(uart: usize, byte: u8) {
    let mut remaining = 100_000u32;
    while rd(uart + UART_LSR) & LSR_THRE == 0 && remaining > 0 {
        remaining -= 1;
    }
    wr(uart + UART_THR, u32::from(byte));
}

fn puts(uart: usize, s: &[u8]) {
    for byte in s {
        putc(uart, *byte);
    }
}

#[entry]
fn main() -> ! {
    let mut pass = 0u32;
    let mut marks = [b'F'; 10];
    let mut check = |i: usize, ok: bool| {
        if ok {
            pass |= 1 << i;
            marks[i] = b'P';
        }
    };

    // 0-1: both UART LSRs reset to THRE | TEMT.
    for (i, uart) in [UART0, UART1].into_iter().enumerate() {
        check(i, rd(uart + UART_LSR) == LSR_THRE | LSR_TEMT);
    }

    // 2: every pad resets to input with pull-down, so both DATA words read 0.
    check(
        2,
        rd(mode(0, 0)) == MODE_IN_PD
            && rd(mode(1, 15)) == MODE_IN_PD
            && rd(P0_DATA) == 0
            && rd(P1_DATA) == 0,
    );

    // 3: P0_SET / P0_RESET act on the P0 latch of an output pad; SET reads 0.
    wr(mode(0, 5), MODE_OUT);
    wr(P0_SET, LED | (1 << 6)); // P0.6 stays an input with pull-down
    let high = rd(P0_DATA);
    let set_reads = rd(P0_SET);
    wr(P0_RESET, LED);
    check(3, high == LED && set_reads == 0 && rd(P0_DATA) == 0);

    // 4: P1 has its own DATA/SET/RESET words; P0 is not touched.
    wr(mode(1, 2), MODE_OUT);
    wr(P1_SET, P1_PIN);
    let p1 = rd(P1_DATA);
    let p0 = rd(P0_DATA);
    wr(P1_RESET, P1_PIN);
    check(4, p1 == P1_PIN && p0 == 0 && rd(P1_DATA) == 0);

    // 5: an input pad reads its PUPD pull (up 1, down 0).
    wr(mode(0, PULL_PIN), MODE_IN_PU);
    let up = rd(P0_DATA) & (1 << PULL_PIN);
    wr(mode(0, PULL_PIN), MODE_IN_PD);
    let down = rd(P0_DATA) & (1 << PULL_PIN);
    check(5, up == 1 << PULL_PIN && down == 0);

    // 6: P0/P1_WEAK_CTRL are separate 16-bit words.
    wr(P0_WEAK, 0x1_0010);
    wr(P1_WEAK, 0x0020);
    check(6, rd(P0_WEAK) == 0x10 && rd(P1_WEAK) == 0x20);
    wr(P0_WEAK, 0);
    wr(P1_WEAK, 0);

    // 7: SRAM at 0x20000000 is the same store as its 0x800000 view.
    wr(SRAM, 0xDA14_0592);
    let low = rd(SRAM_LOW);
    wr(SRAM_LOW, 0x0592_DA14);
    check(7, low == 0xDA14_0592 && rd(SRAM) == 0x0592_DA14);

    // 8: the ROM view at 0x900000 shows the vector table at 0x0.
    check(8, rd(ROM_MIRROR) == rd(0) && rd(ROM_MIRROR + 4) == rd(4));

    // 9: eFlash data area holds a word, also seen at 0xA00000; SRAM top.
    wr(EFLASH_DATA, 0xEF1A_5400);
    wr(SRAM_TOP_WORD, 0x0001_7FFC);
    check(
        9,
        rd(EFLASH_DATA_LOW) == 0xEF1A_5400 && rd(SRAM_TOP_WORD) == 0x0001_7FFC,
    );

    puts(UART0, b"DA14592 M33 UART0 smoke\r\n");
    // P0.5: three pulses for the gpio_edges assertion.
    for _ in 0..3 {
        wr(P0_SET, LED);
        wr(P0_RESET, LED);
    }
    wr(SUMMARY, 0xDA14_0000 | pass);
    puts(UART0, b"DA14592 checks ");
    puts(UART0, &marks);
    puts(UART0, b" DONE\r\n");
    puts(UART1, b"DA14592 UART1 line\r\n");
    loop {
        core::hint::spin_loop();
    }
}
